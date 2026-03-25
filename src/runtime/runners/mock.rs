use crate::{
    error::Result,
    runtime::runners::{AgentRunner, RunnerExecution, RunnerTerminalState},
    runtime::summary::{StructuredSummary, SUMMARY_END_SENTINEL, SUMMARY_START_SENTINEL},
    spec::AgentSpec,
    types::{CompiledContext, RunRequest, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone)]
pub enum MockRunPlan {
    Succeeded {
        summary: StructuredSummary,
    },
    SucceededFromRequest,
    Failed {
        message: String,
        stdout: String,
        stderr: String,
    },
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct MockRunner {
    plan: MockRunPlan,
}

impl MockRunner {
    pub fn new(plan: MockRunPlan) -> Self {
        Self { plan }
    }
}

#[async_trait::async_trait]
impl AgentRunner for MockRunner {
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        _hints: &WorkflowHints,
        _compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let execution = match &self.plan {
            MockRunPlan::Succeeded { summary } => {
                let summary_json = serde_json::to_string_pretty(summary)?;
                RunnerExecution {
                    terminal_state: RunnerTerminalState::Succeeded,
                    stdout: format!(
                        "mock runner completed\n{}\n{}\n{}\n",
                        SUMMARY_START_SENTINEL, summary_json, SUMMARY_END_SENTINEL
                    ),
                    stderr: String::new(),
                }
            }
            MockRunPlan::SucceededFromRequest => {
                let summary_json = serde_json::to_string_pretty(
                    &build_mock_summary_from_task_spec(spec, task_spec),
                )?;
                RunnerExecution {
                    terminal_state: RunnerTerminalState::Succeeded,
                    stdout: format!(
                        "mock runner completed\n{}\n{}\n{}\n",
                        SUMMARY_START_SENTINEL, summary_json, SUMMARY_END_SENTINEL
                    ),
                    stderr: String::new(),
                }
            }
            MockRunPlan::Failed {
                message,
                stdout,
                stderr,
            } => RunnerExecution {
                terminal_state: RunnerTerminalState::Failed {
                    message: message.clone(),
                },
                stdout: stdout.clone(),
                stderr: stderr.clone(),
            },
            MockRunPlan::TimedOut => RunnerExecution {
                terminal_state: RunnerTerminalState::TimedOut,
                stdout: "mock runner timed out".to_string(),
                stderr: String::new(),
            },
            MockRunPlan::Cancelled => RunnerExecution {
                terminal_state: RunnerTerminalState::Cancelled,
                stdout: String::new(),
                stderr: "mock runner cancelled".to_string(),
            },
        };
        Ok(execution)
    }

    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let task_spec = request.to_task_spec();
        let hints = request.to_workflow_hints();
        self.execute_task(spec, &task_spec, &hints, compiled).await
    }
}

fn build_mock_summary_from_task_spec(spec: &AgentSpec, task_spec: &TaskSpec) -> StructuredSummary {
    let touched_files = task_spec
        .selected_files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect::<Vec<_>>();

    StructuredSummary {
        summary: format!("Mock run completed for task: {}", task_spec.task),
        key_findings: vec![format!(
            "Agent `{}` executed through dispatcher mock runner.",
            spec.core.name
        )],
        artifacts: Vec::new(),
        open_questions: Vec::new(),
        next_steps: vec![
            "Replace mock runner with provider runner for production use.".to_string(),
        ],
        exit_code: 0,
        verification_status: crate::runtime::summary::VerificationStatus::Passed,
        touched_files,
        plan_refs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        runtime::{
            runners::mock::{MockRunPlan, MockRunner},
            runners::{AgentRunner, RunnerTerminalState},
            summary::{StructuredSummary, VerificationStatus},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, RunRequest},
    };

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "review code".to_string(),
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
            task: "review".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        }
    }

    #[tokio::test]
    async fn mock_runner_success_wraps_summary_json() {
        let runner = MockRunner::new(MockRunPlan::Succeeded {
            summary: StructuredSummary {
                summary: "ok".to_string(),
                key_findings: vec!["a".to_string()],
                artifacts: Vec::new(),
                open_questions: Vec::new(),
                next_steps: Vec::new(),
                exit_code: 0,
                verification_status: VerificationStatus::Passed,
                touched_files: Vec::new(),
                plan_refs: Vec::new(),
            },
        });

        let execution = runner
            .execute(
                &sample_spec(),
                &sample_request(),
                &CompiledContext {
                    system_prefix: String::new(),
                    injected_prompt: String::new(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        assert_eq!(execution.terminal_state, RunnerTerminalState::Succeeded);
        assert!(execution
            .stdout
            .contains("<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>"));
    }
}
