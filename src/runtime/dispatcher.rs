use std::{
    collections::HashSet,
    path::{Component, PathBuf},
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::time::Duration;
use uuid::Uuid;

use crate::{
    error::{McpSubagentError, Result},
    runtime::{
        context::ContextCompiler,
        outcome::{FailureOutcome, RetryClassification, RetryInfo, RunOutcome, UsageStats},
        runners::{AgentRunner, RunnerOutputObserver, RunnerTerminalState},
        summary::{ParsedSummary, SummaryParseStatus},
        usage::NativeUsage,
    },
    spec::{
        runtime_policy::{ApprovalPolicy, ParsePolicy, SandboxPolicy, WorkingDirPolicy},
        validate::validate_agent_spec,
        workflow::WorkflowStageKind,
        Provider,
    },
    types::{ResolvedMemory, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Received,
    Validating,
    ProbingProvider,
    PreparingWorkspace,
    ResolvingMemory,
    CompilingContext,
    Launching,
    Running,
    Collecting,
    ParsingSummary,
    Finalizing,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl std::fmt::Display for RunPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Received => "received",
            Self::Validating => "validating",
            Self::ProbingProvider => "probing_provider",
            Self::PreparingWorkspace => "preparing_workspace",
            Self::ResolvingMemory => "resolving_memory",
            Self::CompilingContext => "compiling_context",
            Self::Launching => "launching",
            Self::Running => "running",
            Self::Collecting => "collecting",
            Self::ParsingSummary => "parsing_summary",
            Self::Finalizing => "finalizing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRunResult {
    pub handle_id: Uuid,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub status: RunPhase,
    pub status_history: Vec<RunPhase>,
    pub provider: Provider,
    pub agent_name: String,
    pub workspace_path: PathBuf,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub attempts_used: u32,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub max_attempts: u32,
    #[serde(default)]
    pub max_turns: Option<u32>,
    pub outcome: RunOutcome,
    pub stdout: String,
    pub stderr: String,
    pub compiled_context_markdown: String,
    #[serde(default)]
    pub native_usage: Option<NativeUsage>,
}

#[derive(Debug)]
pub struct Dispatcher<C, R> {
    compiler: C,
    runner: R,
}

impl<C, R> Dispatcher<C, R>
where
    C: ContextCompiler,
    R: AgentRunner,
{
    pub fn new(compiler: C, runner: R) -> Self {
        Self { compiler, runner }
    }

    pub async fn run(
        &self,
        spec: &crate::spec::AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        memory: ResolvedMemory,
    ) -> Result<DispatchRunResult> {
        self.run_with_observers(spec, task_spec, hints, memory, |_prev, _next| {}, None)
            .await
    }

    pub async fn run_with_transition_observer<F>(
        &self,
        spec: &crate::spec::AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        memory: ResolvedMemory,
        mut on_transition: F,
    ) -> Result<DispatchRunResult>
    where
        F: FnMut(Option<RunPhase>, RunPhase),
    {
        self.run_with_observers(spec, task_spec, hints, memory, &mut on_transition, None)
            .await
    }

    pub async fn run_with_observers<F>(
        &self,
        spec: &crate::spec::AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        memory: ResolvedMemory,
        mut on_transition: F,
        mut output_observer: Option<&mut dyn RunnerOutputObserver>,
    ) -> Result<DispatchRunResult>
    where
        F: FnMut(Option<RunPhase>, RunPhase),
    {
        let mut tracker = RunTracker::new(spec, task_spec.working_dir.clone());

        let mut previous_status = tracker.status.clone();
        tracker.transition(RunPhase::Validating);
        on_transition(Some(previous_status), RunPhase::Validating);
        validate_agent_spec(spec)?;
        enforce_readonly_gitworktree_scope(spec, hints)?;
        enforce_runtime_depth(spec, hints)?;
        enforce_workflow_gate(spec, task_spec, hints)?;

        previous_status = tracker.status.clone();
        tracker.transition(RunPhase::ProbingProvider);
        on_transition(Some(previous_status), RunPhase::ProbingProvider);
        previous_status = tracker.status.clone();
        tracker.transition(RunPhase::PreparingWorkspace);
        on_transition(Some(previous_status), RunPhase::PreparingWorkspace);
        previous_status = tracker.status.clone();
        tracker.transition(RunPhase::ResolvingMemory);
        on_transition(Some(previous_status), RunPhase::ResolvingMemory);

        previous_status = tracker.status.clone();
        tracker.transition(RunPhase::CompilingContext);
        on_transition(Some(previous_status), RunPhase::CompilingContext);
        let compiled = self.compiler.compile_task(spec, task_spec, hints, memory)?;
        let compiled_context_markdown =
            format!("{}\n\n{}", compiled.system_prefix, compiled.injected_prompt);

        let retry_policy = &spec.runtime.retry_policy;
        let configured_max_attempts = retry_policy.max_attempts.max(1);
        let max_turns = spec.runtime.max_turns;
        let turn_limit = max_turns.unwrap_or(u32::MAX);
        let attempt_budget = configured_max_attempts.min(turn_limit).max(1);
        tracker.set_attempt_budget(configured_max_attempts, max_turns);

        let mut final_execution = None;
        let mut final_summary = None;
        let mut final_status = RunPhase::Failed;
        let mut final_error_message = Some("dispatcher terminated unexpectedly".to_string());
        let mut final_retry_classification = RetryClassification::Unknown;
        let mut final_retry_classification_reason = None;

        for attempt in 1..=attempt_budget {
            tracker.attempts_used = attempt;
            previous_status = tracker.status.clone();
            tracker.transition(RunPhase::Launching);
            on_transition(Some(previous_status), RunPhase::Launching);
            previous_status = tracker.status.clone();
            tracker.transition(RunPhase::Running);
            on_transition(Some(previous_status), RunPhase::Running);
            let execution = match output_observer.as_deref_mut() {
                Some(observer) => {
                    self.runner
                        .execute_task_with_observer(spec, task_spec, hints, &compiled, observer)
                        .await?
                }
                None => {
                    self.runner
                        .execute_task(spec, task_spec, hints, &compiled)
                        .await?
                }
            };

            previous_status = tracker.status.clone();
            tracker.transition(RunPhase::Collecting);
            on_transition(Some(previous_status), RunPhase::Collecting);
            previous_status = tracker.status.clone();
            tracker.transition(RunPhase::ParsingSummary);
            on_transition(Some(previous_status), RunPhase::ParsingSummary);
            let parsed_summary = self
                .compiler
                .parse_summary(&execution.stdout, &execution.stderr)?;
            let attempt_assessment =
                assess_attempt_outcome(&execution, &parsed_summary, &spec.runtime.parse_policy);

            let retry_exhausted = attempt >= attempt_budget;
            let can_retry = attempt_assessment.retryable && !retry_exhausted;

            final_execution = Some(execution);
            final_summary = Some(parsed_summary);
            final_status = attempt_assessment.status;
            final_error_message = attempt_assessment.error_message;
            final_retry_classification = attempt_assessment.retry_classification;
            final_retry_classification_reason = attempt_assessment.retry_classification_reason;

            if can_retry {
                if retry_policy.backoff_secs > 0 {
                    tokio::time::sleep(Duration::from_secs(retry_policy.backoff_secs)).await;
                }
                continue;
            }

            if retry_exhausted && attempt_assessment.retryable {
                let exhausted_message = if max_turns.is_some_and(|turns| turns <= attempt) {
                    format!(
                        "retryable failure exhausted by max_turns={}; attempts_used={attempt}",
                        max_turns.unwrap_or(attempt)
                    )
                } else {
                    format!("retry attempts exhausted; attempts_used={attempt}")
                };
                final_error_message = match final_error_message {
                    Some(message) => Some(format!("{message}; {exhausted_message}")),
                    None => Some(exhausted_message),
                };
                final_retry_classification_reason = Some(match final_retry_classification_reason {
                    Some(reason) => format!("{reason}; retry budget exhausted"),
                    None => "retry budget exhausted".to_string(),
                });
            }
            break;
        }

        let execution = final_execution.ok_or_else(|| {
            McpSubagentError::SpecValidation(
                "dispatcher did not collect runner execution".to_string(),
            )
        })?;
        let parsed_summary = final_summary.ok_or_else(|| {
            McpSubagentError::SpecValidation(
                "dispatcher did not collect parsed summary".to_string(),
            )
        })?;

        tracker.retry_attempts = tracker.attempts_used.saturating_sub(1);
        previous_status = tracker.status.clone();
        tracker.transition(RunPhase::Finalizing);
        on_transition(Some(previous_status), RunPhase::Finalizing);
        tracker.error_message = final_error_message.clone();
        previous_status = tracker.status.clone();
        tracker.transition(final_status.clone());
        on_transition(Some(previous_status), final_status);
        let native_usage = crate::runtime::usage::parse_native_usage(
            &spec.core.provider,
            &execution.stdout,
            &execution.stderr,
        );
        let duration_ms = (tracker.updated_at - tracker.created_at)
            .whole_milliseconds()
            .max(0) as u64;
        let usage = UsageStats {
            duration_ms,
            input_tokens: native_usage.as_ref().and_then(|u| u.input_tokens),
            output_tokens: native_usage.as_ref().and_then(|u| u.output_tokens),
            total_tokens: native_usage.as_ref().and_then(|u| u.total_tokens),
            provider_exit_code: Some(infer_provider_exit_code(&execution.terminal_state)),
        };
        let outcome = match tracker.status {
            RunPhase::Succeeded => RunOutcome::Succeeded(parsed_summary.to_success_outcome(usage)),
            RunPhase::Failed => RunOutcome::Failed(FailureOutcome {
                error: tracker
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "unknown failure".to_string()),
                retry: RetryInfo {
                    classification: final_retry_classification,
                    reason: final_retry_classification_reason,
                    attempts_used: tracker.attempts_used,
                },
                partial_summary: Some(parsed_summary.summary_text().to_string()),
                usage,
            }),
            RunPhase::TimedOut => RunOutcome::TimedOut {
                elapsed_secs: duration_ms / 1000,
            },
            RunPhase::Cancelled => RunOutcome::Cancelled {
                reason: tracker
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "cancelled".to_string()),
            },
            _ => RunOutcome::Failed(FailureOutcome {
                error: format!("unexpected terminal status: {}", tracker.status),
                retry: RetryInfo {
                    classification: RetryClassification::Unknown,
                    reason: None,
                    attempts_used: tracker.attempts_used,
                },
                partial_summary: None,
                usage,
            }),
        };

        Ok(DispatchRunResult {
            handle_id: tracker.handle_id,
            created_at: tracker.created_at,
            updated_at: tracker.updated_at,
            status: tracker.status,
            status_history: tracker.status_history,
            provider: tracker.provider,
            agent_name: tracker.agent_name,
            workspace_path: tracker.workspace_path,
            error_message: tracker.error_message,
            attempts_used: tracker.attempts_used,
            retry_attempts: tracker.retry_attempts,
            max_attempts: tracker.max_attempts,
            max_turns: tracker.max_turns,
            outcome,
            stdout: execution.stdout,
            stderr: execution.stderr,
            compiled_context_markdown,
            native_usage,
        })
    }
}

#[derive(Debug)]
struct AttemptAssessment {
    status: RunPhase,
    error_message: Option<String>,
    retryable: bool,
    retry_classification: RetryClassification,
    retry_classification_reason: Option<String>,
}

fn assess_attempt_outcome(
    execution: &crate::runtime::runners::RunnerExecution,
    summary: &ParsedSummary,
    parse_policy: &ParsePolicy,
) -> AttemptAssessment {
    match &execution.terminal_state {
        RunnerTerminalState::Succeeded => {
            if matches!(summary.parse_status(), SummaryParseStatus::Validated) {
                AttemptAssessment {
                    status: RunPhase::Succeeded,
                    error_message: None,
                    retryable: false,
                    retry_classification: RetryClassification::NonRetryable,
                    retry_classification_reason: Some(
                        "runner succeeded with validated structured summary".to_string(),
                    ),
                }
            } else {
                AttemptAssessment {
                    status: if matches!(parse_policy, ParsePolicy::BestEffort) {
                        RunPhase::Succeeded
                    } else {
                        RunPhase::Failed
                    },
                    error_message: if matches!(parse_policy, ParsePolicy::BestEffort) {
                        None
                    } else {
                        Some(format!(
                            "structured summary parse status is {}",
                            summary.parse_status()
                        ))
                    },
                    retryable: !matches!(parse_policy, ParsePolicy::BestEffort)
                        && matches!(
                            summary.parse_status(),
                            SummaryParseStatus::Invalid | SummaryParseStatus::Degraded
                        ),
                    retry_classification: if matches!(parse_policy, ParsePolicy::BestEffort) {
                        RetryClassification::NonRetryable
                    } else {
                        RetryClassification::Retryable
                    },
                    retry_classification_reason: Some(
                        if matches!(parse_policy, ParsePolicy::BestEffort) {
                            format!(
                                "parse_status={} accepted by best_effort policy",
                                summary.parse_status()
                            )
                        } else {
                            format!(
                                "parse_status={} requires retry under strict policy",
                                summary.parse_status()
                            )
                        },
                    ),
                }
            }
        }
        RunnerTerminalState::Failed { message } => {
            let classification = classify_error_message(message);
            AttemptAssessment {
                status: RunPhase::Failed,
                error_message: Some(message.clone()),
                retryable: classification.retryable,
                retry_classification: classification.classification,
                retry_classification_reason: Some(classification.reason),
            }
        }
        RunnerTerminalState::TimedOut => AttemptAssessment {
            status: RunPhase::TimedOut,
            error_message: Some("runner exceeded timeout".to_string()),
            retryable: true,
            retry_classification: RetryClassification::Retryable,
            retry_classification_reason: Some("runner execution timed out".to_string()),
        },
        RunnerTerminalState::Cancelled => AttemptAssessment {
            status: RunPhase::Cancelled,
            error_message: Some("runner cancelled by request".to_string()),
            retryable: false,
            retry_classification: RetryClassification::NonRetryable,
            retry_classification_reason: Some("runner cancelled by user request".to_string()),
        },
    }
}

fn infer_provider_exit_code(terminal_state: &RunnerTerminalState) -> i32 {
    match terminal_state {
        RunnerTerminalState::Succeeded => 0,
        RunnerTerminalState::Failed { .. } => 1,
        RunnerTerminalState::TimedOut => 124,
        RunnerTerminalState::Cancelled => 130,
    }
}

#[derive(Debug)]
struct RetryMessageClassification {
    classification: RetryClassification,
    retryable: bool,
    reason: String,
}

fn classify_error_message(message: &str) -> RetryMessageClassification {
    let lowered = message.to_ascii_lowercase();
    let retryable_keywords = [
        "timeout",
        "timed out",
        "temporary",
        "try again",
        "429",
        "rate limit",
        "network",
        "connection",
        "unavailable",
        "econnreset",
        "broken pipe",
    ];
    if let Some(keyword) = retryable_keywords
        .iter()
        .find(|keyword| lowered.contains(**keyword))
    {
        return RetryMessageClassification {
            classification: RetryClassification::Retryable,
            retryable: true,
            reason: format!("matched retryable keyword `{keyword}`"),
        };
    }

    let non_retryable_keywords = [
        "invalid_json_schema",
        "invalid schema",
        "invalid_request_error",
        "permission denied",
        "unauthorized",
        "missing binary",
        "spec validation",
        "unsupported",
    ];
    if let Some(keyword) = non_retryable_keywords
        .iter()
        .find(|keyword| lowered.contains(**keyword))
    {
        return RetryMessageClassification {
            classification: RetryClassification::NonRetryable,
            retryable: false,
            reason: format!("matched non-retryable keyword `{keyword}`"),
        };
    }

    RetryMessageClassification {
        classification: RetryClassification::Unknown,
        retryable: false,
        reason: "no retryable/non-retryable keyword matched".to_string(),
    }
}

fn enforce_workflow_gate(
    spec: &crate::spec::AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
) -> Result<()> {
    let Some(workflow) = spec.workflow.as_ref() else {
        return Ok(());
    };
    if !workflow.enabled {
        return Ok(());
    }

    let Some(stage_raw) = hints.stage.as_deref() else {
        return Ok(());
    };
    let stage = parse_stage_kind(stage_raw)?;

    if !workflow.stages.is_empty() && !workflow.stages.contains(&stage) {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow stage `{stage_raw}` is not enabled in workflow.stages"
        )));
    }
    if !workflow.allowed_stages.is_empty() && !workflow.allowed_stages.contains(&stage) {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow stage `{stage_raw}` is not in workflow.allowed_stages"
        )));
    }
    enforce_stage_agent_routing(spec, &stage, stage_raw)?;
    enforce_review_policy(spec, task_spec, hints, &stage, stage_raw)?;

    if !matches!(stage, WorkflowStageKind::Build | WorkflowStageKind::Review) {
        return Ok(());
    }

    let gate = &workflow.require_plan_when;
    let triggered_reasons = collect_plan_gate_triggered_reasons(spec, task_spec, hints, gate);
    if triggered_reasons.is_empty() {
        return Ok(());
    }

    if has_plan_file(task_spec, hints) {
        return Ok(());
    }

    Err(McpSubagentError::SpecValidation(format!(
        "workflow plan required before Build/Review stage: PLAN.md is missing (triggered_by={})",
        triggered_reasons.join(",")
    )))
}

fn enforce_runtime_depth(spec: &crate::spec::AgentSpec, hints: &WorkflowHints) -> Result<()> {
    let Some(workflow) = spec.workflow.as_ref() else {
        return Ok(());
    };
    if !workflow.enabled {
        return Ok(());
    }

    let depth = infer_runtime_depth(hints);
    if depth > workflow.max_runtime_depth {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow runtime depth exceeded: depth={} max_runtime_depth={}",
            depth, workflow.max_runtime_depth
        )));
    }
    Ok(())
}

fn enforce_readonly_gitworktree_scope(
    spec: &crate::spec::AgentSpec,
    hints: &WorkflowHints,
) -> Result<()> {
    if !matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        || !matches!(
            spec.runtime.working_dir_policy,
            WorkingDirPolicy::GitWorktree
        )
    {
        return Ok(());
    }

    let Some(stage_raw) = hints.stage.as_deref() else {
        return Err(McpSubagentError::SpecValidation(
            "ReadOnly + GitWorktree requires explicit stage Research or Plan".to_string(),
        ));
    };
    let stage = parse_stage_kind(stage_raw)?;
    if matches!(stage, WorkflowStageKind::Research | WorkflowStageKind::Plan) {
        return Ok(());
    }

    Err(McpSubagentError::SpecValidation(format!(
        "ReadOnly + GitWorktree is only allowed for Research/Plan stage; received `{stage_raw}`"
    )))
}

fn infer_runtime_depth(hints: &WorkflowHints) -> u8 {
    let Some(parent_summary) = hints.parent_summary.as_deref() else {
        return 0;
    };
    parse_runtime_depth_marker(parent_summary)
        .unwrap_or(0)
        .saturating_add(1)
}

fn parse_runtime_depth_marker(parent_summary: &str) -> Option<u8> {
    parent_summary
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '|'))
        .find_map(|token| {
            token
                .strip_prefix("runtime_depth=")
                .or_else(|| token.strip_prefix("runtime_depth:"))
                .and_then(|value| value.parse::<u8>().ok())
        })
}

fn parse_stage_kind(stage: &str) -> Result<WorkflowStageKind> {
    match stage.to_ascii_lowercase().as_str() {
        "research" => Ok(WorkflowStageKind::Research),
        "plan" => Ok(WorkflowStageKind::Plan),
        "build" => Ok(WorkflowStageKind::Build),
        "review" => Ok(WorkflowStageKind::Review),
        "archive" => Ok(WorkflowStageKind::Archive),
        _ => Err(McpSubagentError::SpecValidation(format!(
            "invalid workflow stage `{stage}`; expected Research/Plan/Build/Review/Archive"
        ))),
    }
}

fn has_plan_file(task_spec: &TaskSpec, hints: &WorkflowHints) -> bool {
    if let Some(plan_ref) = hints.plan_ref.as_deref() {
        let plan_path = task_spec.working_dir.join(plan_ref);
        if plan_path.is_file() {
            return true;
        }
    }

    task_spec.working_dir.join("PLAN.md").is_file()
        || task_spec
            .working_dir
            .join(".mcp-subagent/PLAN.md")
            .is_file()
}

fn enforce_stage_agent_routing(
    spec: &crate::spec::AgentSpec,
    stage: &WorkflowStageKind,
    stage_raw: &str,
) -> Result<()> {
    let profile = agent_stage_profile(spec);
    let stage_signal_is_planning = contains_any_keyword(
        &profile,
        &[
            "research",
            "investigat",
            "analy",
            "scan",
            "plan",
            "planner",
            "strategy",
            "study",
            "survey",
        ],
    );
    let stage_signal_is_reviewer = contains_any_keyword(
        &profile,
        &[
            "review",
            "reviewer",
            "audit",
            "correctness",
            "style",
            "maintainability",
            "quality",
            "verification",
            "validate",
            "lint",
        ],
    );
    let stage_signal_is_builder = contains_any_keyword(
        &profile,
        &[
            "build",
            "builder",
            "coder",
            "implement",
            "write code",
            "frontend",
            "backend",
            "patch",
            "fix",
            "refactor",
        ],
    );

    match stage {
        WorkflowStageKind::Research | WorkflowStageKind::Plan => {
            if stage_signal_is_planning {
                return Ok(());
            }
            Err(McpSubagentError::SpecValidation(format!(
                "workflow stage `{stage_raw}` should use a planning/research agent (agent=`{}` tags={:?})",
                spec.core.name, spec.core.tags
            )))
        }
        WorkflowStageKind::Review => {
            if stage_signal_is_reviewer {
                return Ok(());
            }
            if stage_signal_is_builder {
                return Err(McpSubagentError::SpecValidation(format!(
                    "workflow stage `{stage_raw}` should prioritize reviewer agents (agent=`{}` tags={:?})",
                    spec.core.name, spec.core.tags
                )));
            }
            Ok(())
        }
        WorkflowStageKind::Build | WorkflowStageKind::Archive => Ok(()),
    }
}

fn enforce_review_policy(
    spec: &crate::spec::AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    stage: &WorkflowStageKind,
    stage_raw: &str,
) -> Result<()> {
    if !matches!(stage, WorkflowStageKind::Review) {
        return Ok(());
    }
    let Some(workflow) = spec.workflow.as_ref() else {
        return Ok(());
    };
    if !workflow.enabled {
        return Ok(());
    }

    let policy = &workflow.review_policy;
    let high_risk =
        !collect_plan_gate_triggered_reasons(spec, task_spec, hints, &workflow.require_plan_when)
            .is_empty();
    let required_style = policy.require_style_review || high_risk;
    let required_correctness = policy.require_correctness_review;
    if !required_correctness && !required_style {
        return Ok(());
    }

    let profile = agent_stage_profile(spec);
    let current_tracks = detect_review_tracks(&profile);
    let parent_tracks = detect_parent_summary_review_tracks(hints.parent_summary.as_deref());
    let has_correctness = current_tracks.correctness || parent_tracks.correctness;
    let has_style = current_tracks.style || parent_tracks.style;

    if required_correctness && !has_correctness {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow review policy requires correctness review on stage `{stage_raw}` (agent=`{}` tags={:?})",
            spec.core.name, spec.core.tags
        )));
    }
    if required_style && !has_style {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow review policy requires style review on stage `{stage_raw}` (agent=`{}` tags={:?})",
            spec.core.name, spec.core.tags
        )));
    }

    if required_correctness
        && required_style
        && !current_tracks.correctness
        && !current_tracks.style
    {
        return Err(McpSubagentError::SpecValidation(format!(
            "workflow dual review requires reviewer track evidence on stage `{stage_raw}` (agent=`{}`)",
            spec.core.name
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct ReviewTrackCoverage {
    correctness: bool,
    style: bool,
}

fn detect_review_tracks(profile: &str) -> ReviewTrackCoverage {
    let correctness = contains_any_keyword(
        profile,
        &[
            "correctness",
            "logic",
            "regression",
            "bug",
            "safety",
            "security",
            "verify",
            "validation",
        ],
    );
    let style = contains_any_keyword(
        profile,
        &[
            "style",
            "maintainability",
            "readability",
            "naming",
            "consistency",
            "clean code",
        ],
    );
    ReviewTrackCoverage { correctness, style }
}

fn detect_parent_summary_review_tracks(parent_summary: Option<&str>) -> ReviewTrackCoverage {
    let Some(parent_summary) = parent_summary else {
        return ReviewTrackCoverage::default();
    };
    let lowered = parent_summary.to_lowercase();
    detect_review_tracks(&lowered)
}

fn agent_stage_profile(spec: &crate::spec::AgentSpec) -> String {
    let mut profile = String::new();
    profile.push_str(&spec.core.name);
    profile.push('\n');
    profile.push_str(&spec.core.description);
    profile.push('\n');
    profile.push_str(&spec.core.instructions);
    profile.push('\n');
    for tag in &spec.core.tags {
        profile.push_str(tag);
        profile.push('\n');
    }
    profile.to_lowercase()
}

fn collect_plan_gate_triggered_reasons(
    spec: &crate::spec::AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    gate: &crate::spec::workflow::WorkflowGatePolicy,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if gate
        .require_plan_if_touched_files_ge
        .is_some_and(|threshold| task_spec.selected_files.len() as u32 >= threshold)
    {
        reasons.push("touched_files_ge".to_string());
    }
    if gate
        .require_plan_if_estimated_runtime_minutes_ge
        .is_some_and(|threshold| (spec.runtime.timeout_secs / 60) >= threshold as u64)
    {
        reasons.push("estimated_runtime_minutes_ge".to_string());
    }
    if gate.require_plan_if_parallel_agents
        && matches!(hints.run_mode, crate::types::RunMode::Async)
    {
        reasons.push("parallel_agents".to_string());
    }
    if gate.require_plan_if_cross_module && detect_cross_module_request(task_spec) {
        reasons.push("cross_module".to_string());
    }
    if gate.require_plan_if_new_interface && detect_new_interface_request(task_spec) {
        reasons.push("new_interface".to_string());
    }
    if gate.require_plan_if_migration && detect_migration_request(task_spec) {
        reasons.push("migration".to_string());
    }
    if gate.require_plan_if_human_approval_point && detect_human_approval_point(spec, task_spec) {
        reasons.push("human_approval_point".to_string());
    }

    reasons
}

fn detect_cross_module_request(task_spec: &TaskSpec) -> bool {
    let mut roots = HashSet::new();

    for selected in &task_spec.selected_files {
        let root = top_level_module_root(task_spec, &selected.path);
        if let Some(root) = root {
            roots.insert(root);
        }
        if roots.len() >= 2 {
            return true;
        }
    }

    let text = workflow_signal_text(task_spec);
    contains_any_keyword(
        &text,
        &[
            "cross module",
            "cross-module",
            "multi-module",
            "multiple modules",
            "跨模块",
            "多个模块",
        ],
    )
}

fn top_level_module_root(task_spec: &TaskSpec, selected_path: &std::path::Path) -> Option<String> {
    let effective_path = if selected_path.is_absolute() {
        selected_path
            .strip_prefix(&task_spec.working_dir)
            .unwrap_or(selected_path)
    } else {
        selected_path
    };

    effective_path
        .components()
        .find_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
            _ => None,
        })
}

fn detect_new_interface_request(task_spec: &TaskSpec) -> bool {
    let text = workflow_signal_text(task_spec);
    contains_any_keyword(
        &text,
        &[
            "new interface",
            "new api",
            "public api",
            "new endpoint",
            "breaking change",
            "trait",
            "新增接口",
            "新接口",
            "公开接口",
            "新增api",
            "新增 endpoint",
        ],
    )
}

fn detect_migration_request(task_spec: &TaskSpec) -> bool {
    let text = workflow_signal_text(task_spec);
    contains_any_keyword(
        &text,
        &[
            "migration",
            "migrate",
            "database migration",
            "schema migration",
            "upgrade",
            "backfill",
            "数据迁移",
            "迁移",
            "升级",
        ],
    )
}

fn detect_human_approval_point(spec: &crate::spec::AgentSpec, task_spec: &TaskSpec) -> bool {
    if matches!(spec.runtime.approval, ApprovalPolicy::Ask) {
        return true;
    }
    let text = workflow_signal_text(task_spec);
    contains_any_keyword(
        &text,
        &[
            "human approval",
            "approval required",
            "needs approval",
            "manual approval",
            "人工审批",
            "需要审批",
            "审批点",
        ],
    )
}

fn workflow_signal_text(task_spec: &TaskSpec) -> String {
    let mut text = String::new();
    text.push_str(&task_spec.task);
    text.push('\n');
    if let Some(task_brief) = task_spec.task_brief.as_deref() {
        text.push_str(task_brief);
    }
    text.to_lowercase()
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

#[derive(Debug)]
struct RunTracker {
    handle_id: Uuid,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    status: RunPhase,
    status_history: Vec<RunPhase>,
    provider: Provider,
    agent_name: String,
    workspace_path: PathBuf,
    error_message: Option<String>,
    attempts_used: u32,
    retry_attempts: u32,
    max_attempts: u32,
    max_turns: Option<u32>,
}

impl RunTracker {
    fn new(spec: &crate::spec::AgentSpec, workspace_path: PathBuf) -> Self {
        let now = OffsetDateTime::now_utc();
        let handle_id = Uuid::now_v7();
        let status = RunPhase::Received;
        Self {
            handle_id,
            created_at: now,
            updated_at: now,
            status: status.clone(),
            status_history: vec![status],
            provider: spec.core.provider.clone(),
            agent_name: spec.core.name.clone(),
            workspace_path,
            error_message: None,
            attempts_used: 0,
            retry_attempts: 0,
            max_attempts: 1,
            max_turns: None,
        }
    }

    fn transition(&mut self, status: RunPhase) {
        self.status = status.clone();
        self.updated_at = OffsetDateTime::now_utc();
        self.status_history.push(status);
    }

    fn set_attempt_budget(&mut self, max_attempts: u32, max_turns: Option<u32>) {
        self.max_attempts = max_attempts;
        self.max_turns = max_turns;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use tempfile::tempdir;

    use crate::{
        runtime::{
            context::DefaultContextCompiler,
            dispatcher::{Dispatcher, RunPhase},
            outcome::RunOutcome,
            runners::{
                mock::{MockRunPlan, MockRunner},
                AgentRunner, RunnerExecution, RunnerTerminalState,
            },
            summary::{
                ProviderSummary, SummaryParseStatus, VerificationStatus, SUMMARY_END_SENTINEL,
                SUMMARY_START_SENTINEL,
            },
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{
                ApprovalPolicy, ParsePolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy,
            },
            workflow::{WorkflowGatePolicy, WorkflowSpec, WorkflowStageKind},
            AgentSpec,
        },
        types::{ResolvedMemory, RunMode, SelectedFile, TaskSpec, WorkflowHints},
    };

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "review".to_string(),
                provider: Provider::Codex,
                model: None,
                instructions: "review".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: Default::default(),
            },
            runtime: RuntimePolicy {
                sandbox: SandboxPolicy::ReadOnly,
                working_dir_policy: WorkingDirPolicy::InPlace,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    #[derive(Debug, Clone)]
    struct SequenceRunner {
        executions: Arc<Mutex<Vec<RunnerExecution>>>,
    }

    impl SequenceRunner {
        fn new(executions: Vec<RunnerExecution>) -> Self {
            Self {
                executions: Arc::new(Mutex::new(executions)),
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentRunner for SequenceRunner {
        async fn execute_task(
            &self,
            _spec: &AgentSpec,
            _task_spec: &TaskSpec,
            _hints: &WorkflowHints,
            _compiled: &crate::types::CompiledContext,
        ) -> crate::error::Result<RunnerExecution> {
            let mut executions = self.executions.lock().expect("lock sequence");
            if executions.is_empty() {
                return Ok(RunnerExecution {
                    terminal_state: RunnerTerminalState::Failed {
                        message: "sequence runner exhausted".to_string(),
                    },
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            Ok(executions.remove(0))
        }
    }

    fn sample_task_spec() -> TaskSpec {
        TaskSpec {
            task: "review parser".to_string(),
            task_brief: Some("review parser".to_string()),
            acceptance_criteria: Vec::new(),
            selected_files: Vec::new(),
            working_dir: PathBuf::from("."),
        }
    }

    fn sample_hints() -> WorkflowHints {
        WorkflowHints {
            run_mode: RunMode::Sync,
            ..WorkflowHints::default()
        }
    }

    fn success_summary() -> ProviderSummary {
        ProviderSummary {
            summary: "ok".to_string(),
            key_findings: vec!["one".to_string()],
            artifacts: Vec::new(),
            open_questions: Vec::new(),
            next_steps: Vec::new(),
            verification: VerificationStatus::Passed,
            touched_files: vec!["src/parser.rs".to_string()],
            plan_refs: vec!["step-1".to_string()],
        }
    }

    fn succeeded_execution(summary: ProviderSummary) -> RunnerExecution {
        let summary_json = serde_json::to_string_pretty(&summary).expect("serialize summary");
        RunnerExecution {
            terminal_state: RunnerTerminalState::Succeeded,
            stdout: format!(
                "{}\n{}\n{}\n",
                SUMMARY_START_SENTINEL, summary_json, SUMMARY_END_SENTINEL
            ),
            stderr: String::new(),
        }
    }

    fn sample_spec_with_plan_gate() -> AgentSpec {
        let mut spec = sample_spec();
        spec.workflow = Some(WorkflowSpec {
            enabled: true,
            require_plan_when: WorkflowGatePolicy {
                require_plan_if_touched_files_ge: Some(1),
                require_plan_if_cross_module: false,
                require_plan_if_parallel_agents: false,
                require_plan_if_new_interface: false,
                require_plan_if_migration: false,
                require_plan_if_human_approval_point: false,
                require_plan_if_estimated_runtime_minutes_ge: None,
            },
            stages: vec![WorkflowStageKind::Build],
            ..WorkflowSpec::default()
        });
        spec
    }

    fn sample_spec_with_custom_plan_gate(gate: WorkflowGatePolicy) -> AgentSpec {
        let mut spec = sample_spec();
        spec.workflow = Some(WorkflowSpec {
            enabled: true,
            require_plan_when: gate,
            stages: vec![WorkflowStageKind::Build],
            ..WorkflowSpec::default()
        });
        spec
    }

    fn sample_spec_with_depth_limit(max_runtime_depth: u8) -> AgentSpec {
        let mut spec = sample_spec_with_plan_gate();
        if let Some(workflow) = spec.workflow.as_mut() {
            workflow.max_runtime_depth = max_runtime_depth;
        }
        spec
    }

    fn sample_spec_for_stage_routing(
        name: &str,
        tags: &[&str],
        stages: Vec<WorkflowStageKind>,
    ) -> AgentSpec {
        let mut spec = sample_spec();
        spec.core.name = name.to_string();
        spec.core.description = format!("agent profile {name}");
        spec.core.instructions = tags.join(" ");
        spec.core.tags = tags.iter().map(|tag| tag.to_string()).collect();
        spec.workflow = Some(WorkflowSpec {
            enabled: true,
            require_plan_when: WorkflowGatePolicy {
                require_plan_if_touched_files_ge: None,
                require_plan_if_cross_module: false,
                require_plan_if_parallel_agents: false,
                require_plan_if_new_interface: false,
                require_plan_if_migration: false,
                require_plan_if_human_approval_point: false,
                require_plan_if_estimated_runtime_minutes_ge: None,
            },
            stages: stages.clone(),
            allowed_stages: stages,
            ..WorkflowSpec::default()
        });
        spec
    }

    fn assert_common_lifecycle(status_history: &[RunPhase]) {
        for status in [
            RunPhase::Received,
            RunPhase::Validating,
            RunPhase::ProbingProvider,
            RunPhase::PreparingWorkspace,
            RunPhase::ResolvingMemory,
            RunPhase::CompilingContext,
            RunPhase::Launching,
            RunPhase::Running,
            RunPhase::Collecting,
            RunPhase::ParsingSummary,
            RunPhase::Finalizing,
        ] {
            assert!(
                status_history.contains(&status),
                "missing status in lifecycle: {status:?}"
            );
        }
    }

    #[tokio::test]
    async fn dispatch_reaches_succeeded() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Succeeded);
        assert_common_lifecycle(&result.status_history);
        match &result.outcome {
            RunOutcome::Succeeded(success) => {
                assert_eq!(success.verification, VerificationStatus::Passed);
                assert_eq!(success.parse_status, SummaryParseStatus::Validated);
            }
            other => panic!("expected succeeded outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_result_to_run_outcome_maps_success_fields() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        let outcome = result.outcome.clone();
        match outcome {
            RunOutcome::Succeeded(success) => {
                assert_eq!(success.summary, "ok");
                assert_eq!(success.key_findings, vec!["one".to_string()]);
                assert_eq!(success.touched_files, vec!["src/parser.rs".to_string()]);
                assert_eq!(success.parse_status, SummaryParseStatus::Validated);
                assert_eq!(success.verification, VerificationStatus::Passed);
                assert_eq!(success.usage.provider_exit_code, Some(0));
            }
            other => panic!("expected succeeded outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_result_to_run_outcome_maps_failure_retry_fields() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Failed {
                message: "network timeout from provider".to_string(),
                stdout: "plain stdout".to_string(),
                stderr: "plain stderr".to_string(),
            }),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        let outcome = result.outcome.clone();
        match outcome {
            RunOutcome::Failed(failure) => {
                assert!(failure.error.contains("network timeout from provider"));
                assert_eq!(
                    failure.retry.classification,
                    super::RetryClassification::Retryable
                );
                assert_eq!(failure.retry.attempts_used, 1);
                assert!(failure
                    .partial_summary
                    .as_deref()
                    .is_some_and(|text| !text.is_empty()));
                assert_eq!(failure.usage.provider_exit_code, Some(1));
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_best_effort_succeeds_when_summary_is_degraded() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            SequenceRunner::new(vec![RunnerExecution {
                terminal_state: RunnerTerminalState::Succeeded,
                stdout: "plain text without summary envelope".to_string(),
                stderr: String::new(),
            }]),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Succeeded);
        match &result.outcome {
            RunOutcome::Succeeded(success) => {
                assert_eq!(success.parse_status, SummaryParseStatus::Degraded);
            }
            other => panic!("expected succeeded outcome, got {other:?}"),
        }
        assert_eq!(result.error_message, None);
    }

    #[tokio::test]
    async fn dispatch_strict_fails_when_summary_is_degraded() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            SequenceRunner::new(vec![RunnerExecution {
                terminal_state: RunnerTerminalState::Succeeded,
                stdout: "plain text without summary envelope".to_string(),
                stderr: String::new(),
            }]),
        );
        let mut spec = sample_spec();
        spec.runtime.parse_policy = ParsePolicy::Strict;

        let result = dispatcher
            .run(
                &spec,
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Failed);
        match &result.outcome {
            RunOutcome::Failed(failure) => {
                assert_eq!(
                    failure.retry.classification,
                    super::RetryClassification::Retryable
                );
                assert!(failure
                    .partial_summary
                    .as_deref()
                    .is_some_and(|text| !text.is_empty()));
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert!(result
            .error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("structured summary parse status is Degraded")));
    }

    #[tokio::test]
    async fn dispatch_reaches_failed_and_keeps_summary() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Failed {
                message: "mock failure".to_string(),
                stdout: "plain stdout".to_string(),
                stderr: "plain stderr".to_string(),
            }),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Failed);
        assert_common_lifecycle(&result.status_history);
        match &result.outcome {
            RunOutcome::Failed(failure) => {
                assert_eq!(
                    failure.retry.classification,
                    super::RetryClassification::Unknown
                );
                assert!(failure
                    .partial_summary
                    .as_deref()
                    .is_some_and(|text| !text.is_empty()));
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert_eq!(result.error_message.as_deref(), Some("mock failure"));
    }

    #[tokio::test]
    async fn dispatch_reaches_timed_out() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::TimedOut),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::TimedOut);
        assert!(matches!(result.outcome, RunOutcome::TimedOut { .. }));
        assert_common_lifecycle(&result.status_history);
    }

    #[tokio::test]
    async fn dispatch_reaches_cancelled() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Cancelled),
        );

        let result = dispatcher
            .run(
                &sample_spec(),
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Cancelled);
        assert!(matches!(result.outcome, RunOutcome::Cancelled { .. }));
        assert_common_lifecycle(&result.status_history);
    }

    #[tokio::test]
    async fn dispatch_retries_transient_failure_and_succeeds() {
        let mut spec = sample_spec();
        spec.runtime.retry_policy.max_attempts = 2;
        spec.runtime.retry_policy.backoff_secs = 0;
        spec.runtime.max_turns = Some(2);

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            SequenceRunner::new(vec![
                RunnerExecution {
                    terminal_state: RunnerTerminalState::Failed {
                        message: "network timeout from provider".to_string(),
                    },
                    stdout: "transient".to_string(),
                    stderr: String::new(),
                },
                succeeded_execution(success_summary()),
            ]),
        );

        let result = dispatcher
            .run(
                &spec,
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Succeeded);
        assert_eq!(result.attempts_used, 2);
        assert_eq!(result.retry_attempts, 1);
        assert_eq!(result.max_attempts, 2);
        assert_eq!(result.max_turns, Some(2));
        assert!(matches!(result.outcome, RunOutcome::Succeeded(_)));
        assert_eq!(result.error_message, None);
    }

    #[tokio::test]
    async fn dispatch_stops_retry_when_max_turns_reached() {
        let mut spec = sample_spec();
        spec.runtime.retry_policy.max_attempts = 3;
        spec.runtime.retry_policy.backoff_secs = 0;
        spec.runtime.max_turns = Some(1);

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            SequenceRunner::new(vec![
                RunnerExecution {
                    terminal_state: RunnerTerminalState::Failed {
                        message: "network unavailable".to_string(),
                    },
                    stdout: "transient".to_string(),
                    stderr: String::new(),
                },
                succeeded_execution(success_summary()),
            ]),
        );

        let result = dispatcher
            .run(
                &spec,
                &sample_task_spec(),
                &sample_hints(),
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch run");

        assert_eq!(result.status, RunPhase::Failed);
        assert_eq!(result.attempts_used, 1);
        assert_eq!(result.retry_attempts, 0);
        assert_eq!(result.max_attempts, 3);
        assert_eq!(result.max_turns, Some(1));
        match &result.outcome {
            RunOutcome::Failed(failure) => {
                assert_eq!(
                    failure.retry.classification,
                    super::RetryClassification::Retryable
                );
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert!(result
            .error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("max_turns=1")));
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();
        task_spec.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let err = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("missing plan should fail");
        assert!(
            err.to_string().contains("plan required"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn build_stage_passes_when_plan_exists() {
        let temp = tempdir().expect("tempdir");
        std::fs::write(temp.path().join("PLAN.md"), "# plan").expect("write plan");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();
        task_spec.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let result = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch should pass with plan");
        assert_eq!(result.status, RunPhase::Succeeded);
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_cross_module_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();
        task_spec.selected_files = vec![
            SelectedFile {
                path: PathBuf::from("src/a.rs"),
                rationale: None,
                content: None,
            },
            SelectedFile {
                path: PathBuf::from("web/app.ts"),
                rationale: None,
                content: None,
            },
        ];

        let gate = WorkflowGatePolicy {
            require_plan_if_touched_files_ge: None,
            require_plan_if_cross_module: true,
            require_plan_if_parallel_agents: false,
            require_plan_if_new_interface: false,
            require_plan_if_migration: false,
            require_plan_if_human_approval_point: false,
            require_plan_if_estimated_runtime_minutes_ge: None,
        };

        let err = dispatcher
            .run(
                &sample_spec_with_custom_plan_gate(gate),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("missing plan should fail when cross_module trigger hits");
        assert!(err.to_string().contains("cross_module"));
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_new_interface_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();
        task_spec.task = "add new public API endpoint for parser".to_string();

        let gate = WorkflowGatePolicy {
            require_plan_if_touched_files_ge: None,
            require_plan_if_cross_module: false,
            require_plan_if_parallel_agents: false,
            require_plan_if_new_interface: true,
            require_plan_if_migration: false,
            require_plan_if_human_approval_point: false,
            require_plan_if_estimated_runtime_minutes_ge: None,
        };

        let err = dispatcher
            .run(
                &sample_spec_with_custom_plan_gate(gate),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("missing plan should fail when new_interface trigger hits");
        assert!(err.to_string().contains("new_interface"));
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_migration_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();
        task_spec.task = "run database migration from v1 to v2".to_string();

        let gate = WorkflowGatePolicy {
            require_plan_if_touched_files_ge: None,
            require_plan_if_cross_module: false,
            require_plan_if_parallel_agents: false,
            require_plan_if_new_interface: false,
            require_plan_if_migration: true,
            require_plan_if_human_approval_point: false,
            require_plan_if_estimated_runtime_minutes_ge: None,
        };

        let err = dispatcher
            .run(
                &sample_spec_with_custom_plan_gate(gate),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("missing plan should fail when migration trigger hits");
        assert!(err.to_string().contains("migration"));
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_human_approval_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());
        task_spec.working_dir = temp.path().to_path_buf();

        let gate = WorkflowGatePolicy {
            require_plan_if_touched_files_ge: None,
            require_plan_if_cross_module: false,
            require_plan_if_parallel_agents: false,
            require_plan_if_new_interface: false,
            require_plan_if_migration: false,
            require_plan_if_human_approval_point: true,
            require_plan_if_estimated_runtime_minutes_ge: None,
        };

        let mut spec = sample_spec_with_custom_plan_gate(gate);
        spec.runtime.approval = ApprovalPolicy::Ask;

        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("missing plan should fail when human_approval_point trigger hits");
        assert!(err.to_string().contains("human_approval_point"));
    }

    #[tokio::test]
    async fn rejects_stage_not_enabled_in_workflow_stages() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("research".to_string());

        let err = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("stage outside workflow.stages should fail");
        assert!(
            err.to_string()
                .contains("is not enabled in workflow.stages"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn rejects_runtime_depth_exceeding_workflow_limit() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.parent_summary = Some("runtime_depth=1 previous nested run".to_string());
        hints.stage = Some("build".to_string());

        let err = dispatcher
            .run(
                &sample_spec_with_depth_limit(1),
                &task_spec,
                &hints,
                ResolvedMemory::default(),
            )
            .await
            .expect_err("depth > max_runtime_depth should fail");
        assert!(
            err.to_string().contains("runtime depth exceeded"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn readonly_gitworktree_allows_research_stage() {
        let mut spec = sample_spec();
        spec.runtime.sandbox = SandboxPolicy::ReadOnly;
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("research".to_string());

        let result = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect("readonly+gitworktree should pass in research stage");
        assert_eq!(result.status, RunPhase::Succeeded);
    }

    #[tokio::test]
    async fn readonly_gitworktree_rejects_build_stage() {
        let mut spec = sample_spec();
        spec.runtime.sandbox = SandboxPolicy::ReadOnly;
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("build".to_string());

        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("readonly+gitworktree should fail in build stage");
        assert!(err.to_string().contains("only allowed for Research/Plan"));
    }

    #[tokio::test]
    async fn readonly_gitworktree_requires_explicit_stage() {
        let mut spec = sample_spec();
        spec.runtime.sandbox = SandboxPolicy::ReadOnly;
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let hints = sample_hints();

        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("readonly+gitworktree should require explicit stage");
        assert!(err
            .to_string()
            .contains("requires explicit stage Research or Plan"));
    }

    #[tokio::test]
    async fn research_stage_rejects_non_planning_agent() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("research".to_string());

        let spec = sample_spec_for_stage_routing(
            "backend-coder",
            &["build", "backend", "codex"],
            vec![WorkflowStageKind::Research],
        );
        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("research stage should reject non-planning agent");
        assert!(err.to_string().contains("planning/research agent"));
    }

    #[tokio::test]
    async fn plan_stage_allows_research_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("plan".to_string());

        let spec = sample_spec_for_stage_routing(
            "fast-researcher",
            &["research", "read-only"],
            vec![WorkflowStageKind::Plan],
        );
        let result = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect("plan stage should allow research agent profile");
        assert_eq!(result.status, RunPhase::Succeeded);
    }

    #[tokio::test]
    async fn review_stage_rejects_builder_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("review".to_string());

        let spec = sample_spec_for_stage_routing(
            "frontend-builder",
            &["build", "frontend", "ui"],
            vec![WorkflowStageKind::Review],
        );
        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("review stage should reject builder-like agent");
        assert!(err.to_string().contains("prioritize reviewer agents"));
    }

    #[tokio::test]
    async fn review_stage_allows_reviewer_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("review".to_string());

        let spec = sample_spec_for_stage_routing(
            "correctness-reviewer",
            &["review", "correctness"],
            vec![WorkflowStageKind::Review],
        );
        let result = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect("review stage should allow reviewer agent");
        assert_eq!(result.status, RunPhase::Succeeded);
    }

    #[tokio::test]
    async fn review_stage_requires_dual_tracks_for_high_risk_without_parent_evidence() {
        let mut spec = sample_spec_for_stage_routing(
            "correctness-reviewer",
            &["review", "correctness"],
            vec![WorkflowStageKind::Review],
        );
        if let Some(workflow) = spec.workflow.as_mut() {
            workflow.require_plan_when.require_plan_if_touched_files_ge = Some(1);
            workflow.require_plan_when.require_plan_if_parallel_agents = false;
            workflow.require_plan_when.require_plan_if_cross_module = false;
            workflow.require_plan_when.require_plan_if_new_interface = false;
            workflow.require_plan_when.require_plan_if_migration = false;
            workflow
                .require_plan_when
                .require_plan_if_human_approval_point = false;
            workflow
                .require_plan_when
                .require_plan_if_estimated_runtime_minutes_ge = None;
            workflow.review_policy.require_correctness_review = true;
            workflow.review_policy.require_style_review = false;
        }

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("review".to_string());
        task_spec.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let err = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect_err("high risk review should require style evidence");
        assert!(err.to_string().contains("requires style review"));
    }

    #[tokio::test]
    async fn review_stage_accepts_dual_tracks_with_parent_summary_evidence() {
        let mut spec = sample_spec_for_stage_routing(
            "correctness-reviewer",
            &["review", "correctness"],
            vec![WorkflowStageKind::Review],
        );
        if let Some(workflow) = spec.workflow.as_mut() {
            workflow.require_plan_when.require_plan_if_touched_files_ge = Some(1);
            workflow.require_plan_when.require_plan_if_parallel_agents = false;
            workflow.require_plan_when.require_plan_if_cross_module = false;
            workflow.require_plan_when.require_plan_if_new_interface = false;
            workflow.require_plan_when.require_plan_if_migration = false;
            workflow
                .require_plan_when
                .require_plan_if_human_approval_point = false;
            workflow
                .require_plan_when
                .require_plan_if_estimated_runtime_minutes_ge = None;
            workflow.review_policy.require_correctness_review = true;
            workflow.review_policy.require_style_review = false;
        }

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                envelope: success_summary(),
            }),
        );
        let mut task_spec = sample_task_spec();
        let mut hints = sample_hints();
        hints.stage = Some("review".to_string());
        task_spec.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];
        hints.parent_summary = Some("previous style review confirmed maintainability".to_string());

        let result = dispatcher
            .run(&spec, &task_spec, &hints, ResolvedMemory::default())
            .await
            .expect("parent summary style evidence should satisfy dual review");
        assert_eq!(result.status, RunPhase::Succeeded);
    }

    #[test]
    fn classify_error_message_marks_non_retryable_schema_errors() {
        let classified = super::classify_error_message(
            "codex exited with code 1: invalid_json_schema for response format",
        );
        assert_eq!(
            classified.classification,
            super::RetryClassification::NonRetryable
        );
        assert!(!classified.retryable);
    }

    #[test]
    fn classify_error_message_marks_unknown_when_unmatched() {
        let classified = super::classify_error_message("runner failed with unknown reason");
        assert_eq!(
            classified.classification,
            super::RetryClassification::Unknown
        );
        assert!(!classified.retryable);
    }
}
