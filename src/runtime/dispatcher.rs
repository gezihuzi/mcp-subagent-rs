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
        runners::{AgentRunner, RunnerTerminalState},
        summary::{SummaryEnvelope, SummaryParseStatus},
    },
    spec::{
        runtime_policy::{ApprovalPolicy, ParsePolicy, SandboxPolicy, WorkingDirPolicy},
        validate::validate_agent_spec,
        workflow::WorkflowStageKind,
        Provider,
    },
    types::{ResolvedMemory, RunRequest},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
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

impl std::fmt::Display for RunStatus {
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
pub struct RunMetadata {
    pub handle_id: Uuid,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub status: RunStatus,
    pub status_history: Vec<RunStatus>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchResult {
    pub metadata: RunMetadata,
    pub summary: SummaryEnvelope,
    pub stdout: String,
    pub stderr: String,
    pub compiled_context_markdown: String,
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
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<DispatchResult> {
        let mut tracker = RunTracker::new(spec, request.working_dir.clone());

        tracker.transition(RunStatus::Validating);
        validate_agent_spec(spec)?;
        enforce_readonly_gitworktree_scope(spec, request)?;
        enforce_runtime_depth(spec, request)?;
        enforce_workflow_gate(spec, request)?;

        tracker.transition(RunStatus::ProbingProvider);
        tracker.transition(RunStatus::PreparingWorkspace);
        tracker.transition(RunStatus::ResolvingMemory);

        tracker.transition(RunStatus::CompilingContext);
        let compiled = self.compiler.compile(spec, request, memory)?;
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
        let mut final_status = RunStatus::Failed;
        let mut final_error_message = Some("dispatcher terminated unexpectedly".to_string());

        for attempt in 1..=attempt_budget {
            tracker.metadata.attempts_used = attempt;
            tracker.transition(RunStatus::Launching);
            tracker.transition(RunStatus::Running);
            let execution = self.runner.execute(spec, request, &compiled).await?;

            tracker.transition(RunStatus::Collecting);
            tracker.transition(RunStatus::ParsingSummary);
            let summary_envelope = self
                .compiler
                .parse_summary(&execution.stdout, &execution.stderr)?;
            let attempt_assessment =
                assess_attempt_outcome(&execution, &summary_envelope, &spec.runtime.parse_policy);

            let retry_exhausted = attempt >= attempt_budget;
            let can_retry = attempt_assessment.retryable && !retry_exhausted;

            final_execution = Some(execution);
            final_summary = Some(summary_envelope);
            final_status = attempt_assessment.status;
            final_error_message = attempt_assessment.error_message;

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
            }
            break;
        }

        let execution = final_execution.ok_or_else(|| {
            McpSubagentError::SpecValidation(
                "dispatcher did not collect runner execution".to_string(),
            )
        })?;
        let summary_envelope = final_summary.ok_or_else(|| {
            McpSubagentError::SpecValidation(
                "dispatcher did not collect summary envelope".to_string(),
            )
        })?;

        tracker.metadata.retry_attempts = tracker.metadata.attempts_used.saturating_sub(1);
        tracker.transition(RunStatus::Finalizing);
        tracker.finish(final_status, final_error_message);

        Ok(DispatchResult {
            metadata: tracker.metadata,
            summary: summary_envelope,
            stdout: execution.stdout,
            stderr: execution.stderr,
            compiled_context_markdown,
        })
    }
}

#[derive(Debug)]
struct AttemptAssessment {
    status: RunStatus,
    error_message: Option<String>,
    retryable: bool,
}

fn assess_attempt_outcome(
    execution: &crate::runtime::runners::RunnerExecution,
    summary_envelope: &SummaryEnvelope,
    parse_policy: &ParsePolicy,
) -> AttemptAssessment {
    match &execution.terminal_state {
        RunnerTerminalState::Succeeded => {
            if matches!(summary_envelope.parse_status, SummaryParseStatus::Validated) {
                AttemptAssessment {
                    status: RunStatus::Succeeded,
                    error_message: None,
                    retryable: false,
                }
            } else {
                AttemptAssessment {
                    status: if matches!(parse_policy, ParsePolicy::BestEffort) {
                        RunStatus::Succeeded
                    } else {
                        RunStatus::Failed
                    },
                    error_message: if matches!(parse_policy, ParsePolicy::BestEffort) {
                        None
                    } else {
                        Some(format!(
                            "structured summary parse status is {}",
                            summary_envelope.parse_status
                        ))
                    },
                    retryable: !matches!(parse_policy, ParsePolicy::BestEffort)
                        && matches!(
                            summary_envelope.parse_status,
                            SummaryParseStatus::Invalid | SummaryParseStatus::Degraded
                        ),
                }
            }
        }
        RunnerTerminalState::Failed { message } => AttemptAssessment {
            status: RunStatus::Failed,
            error_message: Some(message.clone()),
            retryable: is_retryable_error_message(message),
        },
        RunnerTerminalState::TimedOut => AttemptAssessment {
            status: RunStatus::TimedOut,
            error_message: Some("runner exceeded timeout".to_string()),
            retryable: true,
        },
        RunnerTerminalState::Cancelled => AttemptAssessment {
            status: RunStatus::Cancelled,
            error_message: Some("runner cancelled by request".to_string()),
            retryable: false,
        },
    }
}

fn is_retryable_error_message(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
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
    ]
    .iter()
    .any(|keyword| lowered.contains(keyword))
}

fn enforce_workflow_gate(spec: &crate::spec::AgentSpec, request: &RunRequest) -> Result<()> {
    let Some(workflow) = spec.workflow.as_ref() else {
        return Ok(());
    };
    if !workflow.enabled {
        return Ok(());
    }

    let Some(stage_raw) = request.stage.as_deref() else {
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
    enforce_review_policy(spec, request, &stage, stage_raw)?;

    if !matches!(stage, WorkflowStageKind::Build | WorkflowStageKind::Review) {
        return Ok(());
    }

    let gate = &workflow.require_plan_when;
    let triggered_reasons = collect_plan_gate_triggered_reasons(spec, request, gate);
    if triggered_reasons.is_empty() {
        return Ok(());
    }

    if has_plan_file(request) {
        return Ok(());
    }

    Err(McpSubagentError::SpecValidation(format!(
        "workflow plan required before Build/Review stage: PLAN.md is missing (triggered_by={})",
        triggered_reasons.join(",")
    )))
}

fn enforce_runtime_depth(spec: &crate::spec::AgentSpec, request: &RunRequest) -> Result<()> {
    let Some(workflow) = spec.workflow.as_ref() else {
        return Ok(());
    };
    if !workflow.enabled {
        return Ok(());
    }

    let depth = infer_runtime_depth(request);
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
    request: &RunRequest,
) -> Result<()> {
    if !matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        || !matches!(
            spec.runtime.working_dir_policy,
            WorkingDirPolicy::GitWorktree
        )
    {
        return Ok(());
    }

    let Some(stage_raw) = request.stage.as_deref() else {
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

fn infer_runtime_depth(request: &RunRequest) -> u8 {
    let Some(parent_summary) = request.parent_summary.as_deref() else {
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

fn has_plan_file(request: &RunRequest) -> bool {
    if let Some(plan_ref) = request.plan_ref.as_deref() {
        let plan_path = request.working_dir.join(plan_ref);
        if plan_path.is_file() {
            return true;
        }
    }

    request.working_dir.join("PLAN.md").is_file()
        || request.working_dir.join(".mcp-subagent/PLAN.md").is_file()
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
    request: &RunRequest,
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
        !collect_plan_gate_triggered_reasons(spec, request, &workflow.require_plan_when).is_empty();
    let required_style = policy.require_style_review || high_risk;
    let required_correctness = policy.require_correctness_review;
    if !required_correctness && !required_style {
        return Ok(());
    }

    let profile = agent_stage_profile(spec);
    let current_tracks = detect_review_tracks(&profile);
    let parent_tracks = detect_parent_summary_review_tracks(request.parent_summary.as_deref());
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
    request: &RunRequest,
    gate: &crate::spec::workflow::WorkflowGatePolicy,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if gate
        .require_plan_if_touched_files_ge
        .is_some_and(|threshold| request.selected_files.len() as u32 >= threshold)
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
        && matches!(request.run_mode, crate::types::RunMode::Async)
    {
        reasons.push("parallel_agents".to_string());
    }
    if gate.require_plan_if_cross_module && detect_cross_module_request(request) {
        reasons.push("cross_module".to_string());
    }
    if gate.require_plan_if_new_interface && detect_new_interface_request(request) {
        reasons.push("new_interface".to_string());
    }
    if gate.require_plan_if_migration && detect_migration_request(request) {
        reasons.push("migration".to_string());
    }
    if gate.require_plan_if_human_approval_point && detect_human_approval_point(spec, request) {
        reasons.push("human_approval_point".to_string());
    }

    reasons
}

fn detect_cross_module_request(request: &RunRequest) -> bool {
    let mut roots = HashSet::new();

    for selected in &request.selected_files {
        let root = top_level_module_root(request, &selected.path);
        if let Some(root) = root {
            roots.insert(root);
        }
        if roots.len() >= 2 {
            return true;
        }
    }

    let text = workflow_signal_text(request);
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

fn top_level_module_root(request: &RunRequest, selected_path: &std::path::Path) -> Option<String> {
    let effective_path = if selected_path.is_absolute() {
        selected_path
            .strip_prefix(&request.working_dir)
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

fn detect_new_interface_request(request: &RunRequest) -> bool {
    let text = workflow_signal_text(request);
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

fn detect_migration_request(request: &RunRequest) -> bool {
    let text = workflow_signal_text(request);
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

fn detect_human_approval_point(spec: &crate::spec::AgentSpec, request: &RunRequest) -> bool {
    if matches!(spec.runtime.approval, ApprovalPolicy::Ask) {
        return true;
    }
    let text = workflow_signal_text(request);
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

fn workflow_signal_text(request: &RunRequest) -> String {
    let mut text = String::new();
    text.push_str(&request.task);
    text.push('\n');
    if let Some(task_brief) = request.task_brief.as_deref() {
        text.push_str(task_brief);
    }
    text.to_lowercase()
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

#[derive(Debug)]
struct RunTracker {
    metadata: RunMetadata,
}

impl RunTracker {
    fn new(spec: &crate::spec::AgentSpec, workspace_path: PathBuf) -> Self {
        let now = OffsetDateTime::now_utc();
        let handle_id = Uuid::now_v7();
        let status = RunStatus::Received;
        let metadata = RunMetadata {
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
        };
        Self { metadata }
    }

    fn transition(&mut self, status: RunStatus) {
        self.metadata.status = status.clone();
        self.metadata.updated_at = OffsetDateTime::now_utc();
        self.metadata.status_history.push(status);
    }

    fn finish(&mut self, status: RunStatus, error_message: Option<String>) {
        self.metadata.error_message = error_message;
        self.transition(status);
    }

    fn set_attempt_budget(&mut self, max_attempts: u32, max_turns: Option<u32>) {
        self.metadata.max_attempts = max_attempts;
        self.metadata.max_turns = max_turns;
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
            dispatcher::{Dispatcher, RunStatus},
            runners::{
                mock::{MockRunPlan, MockRunner},
                AgentRunner, RunnerExecution, RunnerTerminalState,
            },
            summary::{
                StructuredSummary, SummaryParseStatus, VerificationStatus, SUMMARY_END_SENTINEL,
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
        types::{ResolvedMemory, RunMode, RunRequest, SelectedFile},
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
        async fn execute(
            &self,
            _spec: &AgentSpec,
            _request: &RunRequest,
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

    fn sample_request() -> RunRequest {
        RunRequest {
            task: "review parser".to_string(),
            task_brief: Some("review parser".to_string()),
            parent_summary: None,
            selected_files: Vec::new(),
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        }
    }

    fn success_summary() -> StructuredSummary {
        StructuredSummary {
            summary: "ok".to_string(),
            key_findings: vec!["one".to_string()],
            artifacts: Vec::new(),
            open_questions: Vec::new(),
            next_steps: Vec::new(),
            exit_code: 0,
            verification_status: VerificationStatus::Passed,
            touched_files: vec!["src/parser.rs".to_string()],
            plan_refs: vec!["step-1".to_string()],
        }
    }

    fn succeeded_execution(summary: StructuredSummary) -> RunnerExecution {
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

    fn assert_common_lifecycle(status_history: &[RunStatus]) {
        for status in [
            RunStatus::Received,
            RunStatus::Validating,
            RunStatus::ProbingProvider,
            RunStatus::PreparingWorkspace,
            RunStatus::ResolvingMemory,
            RunStatus::CompilingContext,
            RunStatus::Launching,
            RunStatus::Running,
            RunStatus::Collecting,
            RunStatus::ParsingSummary,
            RunStatus::Finalizing,
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
                summary: success_summary(),
            }),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Succeeded);
        assert_common_lifecycle(&result.metadata.status_history);
        assert_eq!(
            result.summary.summary.verification_status,
            VerificationStatus::Passed
        );
        assert_eq!(result.summary.parse_status, SummaryParseStatus::Validated);
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
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Succeeded);
        assert_eq!(result.summary.parse_status, SummaryParseStatus::Degraded);
        assert_eq!(result.metadata.error_message, None);
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
            .run(&spec, &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Failed);
        assert_eq!(result.summary.parse_status, SummaryParseStatus::Degraded);
        assert!(result
            .metadata
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
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Failed);
        assert_common_lifecycle(&result.metadata.status_history);
        assert_eq!(result.summary.parse_status, SummaryParseStatus::Degraded);
        assert_eq!(
            result.summary.summary.verification_status,
            VerificationStatus::NotRun
        );
        assert_eq!(
            result.metadata.error_message.as_deref(),
            Some("mock failure")
        );
    }

    #[tokio::test]
    async fn dispatch_reaches_timed_out() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::TimedOut),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::TimedOut);
        assert_common_lifecycle(&result.metadata.status_history);
    }

    #[tokio::test]
    async fn dispatch_reaches_cancelled() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Cancelled),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Cancelled);
        assert_common_lifecycle(&result.metadata.status_history);
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
            .run(&spec, &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Succeeded);
        assert_eq!(result.metadata.attempts_used, 2);
        assert_eq!(result.metadata.retry_attempts, 1);
        assert_eq!(result.metadata.max_attempts, 2);
        assert_eq!(result.metadata.max_turns, Some(2));
        assert_eq!(result.metadata.error_message, None);
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
            .run(&spec, &sample_request(), ResolvedMemory::default())
            .await
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Failed);
        assert_eq!(result.metadata.attempts_used, 1);
        assert_eq!(result.metadata.retry_attempts, 0);
        assert_eq!(result.metadata.max_attempts, 3);
        assert_eq!(result.metadata.max_turns, Some(1));
        assert!(result
            .metadata
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();
        request.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let err = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();
        request.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let result = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &request,
                ResolvedMemory::default(),
            )
            .await
            .expect("dispatch should pass with plan");
        assert_eq!(result.metadata.status, RunStatus::Succeeded);
    }

    #[tokio::test]
    async fn build_stage_requires_plan_when_cross_module_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();
        request.selected_files = vec![
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
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();
        request.task = "add new public API endpoint for parser".to_string();

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
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();
        request.task = "run database migration from v1 to v2".to_string();

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
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());
        request.working_dir = temp.path().to_path_buf();

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
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect_err("missing plan should fail when human_approval_point trigger hits");
        assert!(err.to_string().contains("human_approval_point"));
    }

    #[tokio::test]
    async fn rejects_stage_not_enabled_in_workflow_stages() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("research".to_string());

        let err = dispatcher
            .run(
                &sample_spec_with_plan_gate(),
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.parent_summary = Some("runtime_depth=1 previous nested run".to_string());
        request.stage = Some("build".to_string());

        let err = dispatcher
            .run(
                &sample_spec_with_depth_limit(1),
                &request,
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("research".to_string());

        let result = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect("readonly+gitworktree should pass in research stage");
        assert_eq!(result.metadata.status, RunStatus::Succeeded);
    }

    #[tokio::test]
    async fn readonly_gitworktree_rejects_build_stage() {
        let mut spec = sample_spec();
        spec.runtime.sandbox = SandboxPolicy::ReadOnly;
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("build".to_string());

        let err = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
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
                summary: success_summary(),
            }),
        );
        let request = sample_request();

        let err = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("research".to_string());

        let spec = sample_spec_for_stage_routing(
            "backend-coder",
            &["build", "backend", "codex"],
            vec![WorkflowStageKind::Research],
        );
        let err = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect_err("research stage should reject non-planning agent");
        assert!(err.to_string().contains("planning/research agent"));
    }

    #[tokio::test]
    async fn plan_stage_allows_research_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("plan".to_string());

        let spec = sample_spec_for_stage_routing(
            "fast-researcher",
            &["research", "read-only"],
            vec![WorkflowStageKind::Plan],
        );
        let result = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect("plan stage should allow research agent profile");
        assert_eq!(result.metadata.status, RunStatus::Succeeded);
    }

    #[tokio::test]
    async fn review_stage_rejects_builder_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("review".to_string());

        let spec = sample_spec_for_stage_routing(
            "frontend-builder",
            &["build", "frontend", "ui"],
            vec![WorkflowStageKind::Review],
        );
        let err = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect_err("review stage should reject builder-like agent");
        assert!(err.to_string().contains("prioritize reviewer agents"));
    }

    #[tokio::test]
    async fn review_stage_allows_reviewer_agent_profile() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("review".to_string());

        let spec = sample_spec_for_stage_routing(
            "correctness-reviewer",
            &["review", "correctness"],
            vec![WorkflowStageKind::Review],
        );
        let result = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect("review stage should allow reviewer agent");
        assert_eq!(result.metadata.status, RunStatus::Succeeded);
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("review".to_string());
        request.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];

        let err = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
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
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("review".to_string());
        request.selected_files = vec![SelectedFile {
            path: PathBuf::from("src/a.rs"),
            rationale: None,
            content: None,
        }];
        request.parent_summary =
            Some("previous style review confirmed maintainability".to_string());

        let result = dispatcher
            .run(&spec, &request, ResolvedMemory::default())
            .await
            .expect("parent summary style evidence should satisfy dual review");
        assert_eq!(result.metadata.status, RunStatus::Succeeded);
    }
}
