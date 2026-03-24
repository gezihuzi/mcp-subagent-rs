use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    error::{McpSubagentError, Result},
    runtime::{
        context::ContextCompiler,
        runner::{AgentRunner, RunnerTerminalState},
        summary::{SummaryEnvelope, SummaryParseStatus},
    },
    spec::{validate::validate_agent_spec, workflow::WorkflowStageKind, Provider},
    types::{ResolvedMemory, RunRequest},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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
        enforce_runtime_depth(spec, request)?;
        enforce_workflow_gate(spec, request)?;

        tracker.transition(RunStatus::ProbingProvider);
        tracker.transition(RunStatus::PreparingWorkspace);
        tracker.transition(RunStatus::ResolvingMemory);

        tracker.transition(RunStatus::CompilingContext);
        let compiled = self.compiler.compile(spec, request, memory)?;
        let compiled_context_markdown =
            format!("{}\n\n{}", compiled.system_prefix, compiled.injected_prompt);

        tracker.transition(RunStatus::Launching);
        tracker.transition(RunStatus::Running);
        let execution = self.runner.execute(spec, request, &compiled).await?;

        tracker.transition(RunStatus::Collecting);
        tracker.transition(RunStatus::ParsingSummary);
        let summary_envelope = self
            .compiler
            .parse_summary(&execution.stdout, &execution.stderr)?;

        tracker.transition(RunStatus::Finalizing);
        match execution.terminal_state {
            RunnerTerminalState::Succeeded => {
                if matches!(summary_envelope.parse_status, SummaryParseStatus::Validated) {
                    tracker.finish(RunStatus::Succeeded, None);
                } else {
                    tracker.finish(
                        RunStatus::Failed,
                        Some(format!(
                            "structured summary parse status is {:?}",
                            summary_envelope.parse_status
                        )),
                    );
                }
            }
            RunnerTerminalState::Failed { message } => {
                tracker.finish(RunStatus::Failed, Some(message));
            }
            RunnerTerminalState::TimedOut => {
                tracker.finish(
                    RunStatus::TimedOut,
                    Some("runner exceeded timeout".to_string()),
                );
            }
            RunnerTerminalState::Cancelled => {
                tracker.finish(
                    RunStatus::Cancelled,
                    Some("runner cancelled by request".to_string()),
                );
            }
        }

        Ok(DispatchResult {
            metadata: tracker.metadata,
            summary: summary_envelope,
            stdout: execution.stdout,
            stderr: execution.stderr,
            compiled_context_markdown,
        })
    }
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

    if !matches!(stage, WorkflowStageKind::Build | WorkflowStageKind::Review) {
        return Ok(());
    }

    let gate = &workflow.require_plan_when;
    let touched_files_triggered = gate
        .require_plan_if_touched_files_ge
        .is_some_and(|threshold| request.selected_files.len() as u32 >= threshold);
    let runtime_triggered = gate
        .require_plan_if_estimated_runtime_minutes_ge
        .is_some_and(|threshold| (spec.runtime.timeout_secs / 60) >= threshold as u64);
    let async_triggered = gate.require_plan_if_parallel_agents
        && matches!(request.run_mode, crate::types::RunMode::Async);
    let requires_plan = touched_files_triggered || runtime_triggered || async_triggered;

    if !requires_plan {
        return Ok(());
    }

    if has_plan_file(request) {
        return Ok(());
    }

    Err(McpSubagentError::SpecValidation(
        "workflow plan required before Build/Review stage: PLAN.md is missing".to_string(),
    ))
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
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use crate::{
        runtime::{
            context::DefaultContextCompiler,
            dispatcher::{Dispatcher, RunStatus},
            mock_runner::{MockRunPlan, MockRunner},
            summary::{StructuredSummary, SummaryParseStatus, VerificationStatus},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
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

    fn sample_spec_with_depth_limit(max_runtime_depth: u8) -> AgentSpec {
        let mut spec = sample_spec_with_plan_gate();
        if let Some(workflow) = spec.workflow.as_mut() {
            workflow.max_runtime_depth = max_runtime_depth;
        }
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
    async fn build_stage_requires_plan_when_gate_hits() {
        let temp = tempdir().expect("tempdir");
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("Build".to_string());
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
        request.stage = Some("Build".to_string());
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
    async fn rejects_stage_not_enabled_in_workflow_stages() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );
        let mut request = sample_request();
        request.stage = Some("Research".to_string());

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
        request.stage = Some("Build".to_string());

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
}
