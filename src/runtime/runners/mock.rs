use crate::{
    error::Result,
    runtime::runners::{AgentRunner, RunnerExecution, RunnerTerminalState},
    runtime::{
        outcome::{SuccessOutcome, UsageStats},
        summary::{
            ProviderSummary, SummaryParseStatus, VerificationStatus, SUMMARY_END_SENTINEL,
            SUMMARY_START_SENTINEL,
        },
    },
    spec::AgentSpec,
    types::{CompiledContext, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone)]
pub enum MockRunPlan {
    Succeeded {
        envelope: ProviderSummary,
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
            MockRunPlan::Succeeded { envelope } => {
                let summary_json = serde_json::to_string_pretty(envelope)?;
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
                    &build_mock_envelope_from_task_spec(spec, task_spec),
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
}

fn build_mock_success_outcome(spec: &AgentSpec, task_spec: &TaskSpec) -> SuccessOutcome {
    let touched_files = task_spec
        .selected_files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect::<Vec<_>>();

    SuccessOutcome {
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
        verification: VerificationStatus::Passed,
        usage: UsageStats::ZERO,
        parse_status: SummaryParseStatus::Validated,
        touched_files,
        plan_refs: Vec::new(),
    }
}

fn build_mock_envelope_from_task_spec(spec: &AgentSpec, task_spec: &TaskSpec) -> ProviderSummary {
    let success = build_mock_success_outcome(spec, task_spec);
    ProviderSummary {
        summary: success.summary,
        key_findings: success.key_findings,
        artifacts: success.artifacts,
        open_questions: success.open_questions,
        next_steps: success.next_steps,
        verification: success.verification,
        touched_files: success.touched_files,
        plan_refs: success.plan_refs,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        runtime::{
            runners::mock::{MockRunPlan, MockRunner},
            runners::{AgentRunner, RunnerTerminalState},
            summary::{ProviderSummary, VerificationStatus},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, TaskSpec, WorkflowHints},
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

    fn sample_task_spec() -> TaskSpec {
        TaskSpec {
            task: "review".to_string(),
            task_brief: None,
            acceptance_criteria: Vec::new(),
            selected_files: Vec::new(),
            working_dir: PathBuf::from("."),
        }
    }

    #[tokio::test]
    async fn mock_runner_success_wraps_summary_json() {
        let runner = MockRunner::new(MockRunPlan::Succeeded {
            envelope: ProviderSummary {
                summary: "ok".to_string(),
                key_findings: vec!["a".to_string()],
                artifacts: Vec::new(),
                open_questions: Vec::new(),
                next_steps: Vec::new(),
                verification: VerificationStatus::Passed,
                touched_files: Vec::new(),
                plan_refs: Vec::new(),
            },
        });

        let execution = runner
            .execute_task(
                &sample_spec(),
                &sample_task_spec(),
                &WorkflowHints {
                    run_mode: RunMode::Sync,
                    ..WorkflowHints::default()
                },
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
