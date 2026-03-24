use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    error::Result,
    runtime::{
        context::ContextCompiler,
        mock_runner::{RunnerTerminalState, RuntimeRunner},
        summary::StructuredSummary,
    },
    spec::{validate::validate_agent_spec, Provider},
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
    pub summary: StructuredSummary,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub struct Dispatcher<C, R> {
    compiler: C,
    runner: R,
}

impl<C, R> Dispatcher<C, R>
where
    C: ContextCompiler,
    R: RuntimeRunner,
{
    pub fn new(compiler: C, runner: R) -> Self {
        Self { compiler, runner }
    }

    pub fn run(
        &self,
        spec: &crate::spec::AgentSpec,
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<DispatchResult> {
        let mut tracker = RunTracker::new(spec, request.working_dir.clone());

        tracker.transition(RunStatus::Validating);
        validate_agent_spec(spec)?;

        tracker.transition(RunStatus::ProbingProvider);
        tracker.transition(RunStatus::PreparingWorkspace);
        tracker.transition(RunStatus::ResolvingMemory);

        tracker.transition(RunStatus::CompilingContext);
        let compiled = self.compiler.compile(spec, request, memory)?;

        tracker.transition(RunStatus::Launching);
        tracker.transition(RunStatus::Running);
        let execution = self.runner.execute(spec, request, &compiled)?;

        tracker.transition(RunStatus::Collecting);
        tracker.transition(RunStatus::ParsingSummary);
        let summary = self
            .compiler
            .parse_summary(&execution.stdout, &execution.stderr)?;

        tracker.transition(RunStatus::Finalizing);
        match execution.terminal_state {
            RunnerTerminalState::Succeeded => {
                tracker.finish(RunStatus::Succeeded, None);
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
            summary,
            stdout: execution.stdout,
            stderr: execution.stderr,
        })
    }
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

    use crate::{
        runtime::{
            context::DefaultContextCompiler,
            dispatcher::{Dispatcher, RunStatus},
            mock_runner::{MockRunPlan, MockRunner},
            summary::{StructuredSummary, VerificationStatus},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{ResolvedMemory, RunMode, RunRequest},
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
        }
    }

    fn sample_request() -> RunRequest {
        RunRequest {
            task: "review parser".to_string(),
            task_brief: Some("review parser".to_string()),
            parent_summary: None,
            selected_files: Vec::new(),
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
        }
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

    #[test]
    fn dispatch_reaches_succeeded() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: success_summary(),
            }),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Succeeded);
        assert_common_lifecycle(&result.metadata.status_history);
        assert_eq!(
            result.summary.verification_status,
            VerificationStatus::Passed
        );
    }

    #[test]
    fn dispatch_reaches_failed_and_keeps_summary() {
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
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Failed);
        assert_common_lifecycle(&result.metadata.status_history);
        assert_eq!(
            result.summary.verification_status,
            VerificationStatus::ParseFailed
        );
        assert_eq!(
            result.metadata.error_message.as_deref(),
            Some("mock failure")
        );
    }

    #[test]
    fn dispatch_reaches_timed_out() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::TimedOut),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::TimedOut);
        assert_common_lifecycle(&result.metadata.status_history);
    }

    #[test]
    fn dispatch_reaches_cancelled() {
        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Cancelled),
        );

        let result = dispatcher
            .run(&sample_spec(), &sample_request(), ResolvedMemory::default())
            .expect("dispatch run");

        assert_eq!(result.metadata.status, RunStatus::Cancelled);
        assert_common_lifecycle(&result.metadata.status_history);
    }
}
