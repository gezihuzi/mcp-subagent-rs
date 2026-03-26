use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;

use crate::{
    error::{McpSubagentError, Result},
    runtime::runners::{AgentRunner, RunnerExecution, RunnerTerminalState},
    spec::AgentSpec,
    types::{CompiledContext, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone)]
pub struct OllamaRunner {
    executable: PathBuf,
}

impl Default for OllamaRunner {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("ollama"),
        }
    }
}

impl OllamaRunner {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    pub async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        _hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let prompt = compose_prompt(compiled);
        let timeout = Duration::from_secs(spec.runtime.timeout_secs.max(1));
        let model = resolve_model(spec)?;

        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("run")
            .arg(model)
            .arg(&prompt)
            .current_dir(&task_spec.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let output = match tokio::time::timeout(timeout, command.output()).await {
            Ok(waited) => waited.map_err(McpSubagentError::Io)?,
            Err(_) => {
                return Ok(RunnerExecution {
                    terminal_state: RunnerTerminalState::TimedOut,
                    stdout: String::new(),
                    stderr: format!(
                        "ollama execution exceeded timeout of {}s",
                        timeout.as_secs()
                    ),
                });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let terminal_state = if output.status.success() {
            RunnerTerminalState::Succeeded
        } else {
            let exit_code = output.status.code().unwrap_or(-1);
            let mut message = format!("ollama exited with code {exit_code}");
            if !stderr.trim().is_empty() {
                let first_line = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or_default();
                if !first_line.is_empty() {
                    message.push_str(": ");
                    message.push_str(first_line.trim());
                }
            }
            RunnerTerminalState::Failed { message }
        };

        Ok(RunnerExecution {
            terminal_state,
            stdout,
            stderr,
        })
    }
}

#[async_trait]
impl AgentRunner for OllamaRunner {
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        OllamaRunner::execute_task(self, spec, task_spec, hints, compiled).await
    }
}

fn compose_prompt(compiled: &CompiledContext) -> String {
    format!(
        "{}\n\n{}",
        compiled.system_prefix.trim(),
        compiled.injected_prompt.trim()
    )
}

fn resolve_model(spec: &AgentSpec) -> Result<String> {
    if let Some(model) = spec.core.model.as_ref() {
        let trimmed = model.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Ok(model) = std::env::var("MCP_SUBAGENT_OLLAMA_MODEL") {
        let trimmed = model.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    Err(McpSubagentError::SpecValidation(
        "Ollama requires `core.model` or MCP_SUBAGENT_OLLAMA_MODEL".to_string(),
    ))
}

pub fn supports_provider(provider: &crate::spec::Provider) -> bool {
    matches!(provider, crate::spec::Provider::Ollama)
}

pub fn from_env() -> OllamaRunner {
    let configured = std::env::var("MCP_SUBAGENT_OLLAMA_BIN").ok();
    match configured {
        Some(path) if !path.trim().is_empty() => OllamaRunner::new(Path::new(&path).to_path_buf()),
        _ => OllamaRunner::default(),
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use tempfile::tempdir;

    use crate::{
        runtime::{runners::ollama::OllamaRunner, runners::RunnerTerminalState},
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, TaskSpec, WorkflowHints},
    };

    fn sample_spec(timeout_secs: u64) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "local-builder".to_string(),
                description: "local build".to_string(),
                provider: Provider::Ollama,
                model: Some("qwen2.5-coder:7b".to_string()),
                instructions: "You are a local coding model".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: Default::default(),
            },
            runtime: RuntimePolicy {
                sandbox: SandboxPolicy::ReadOnly,
                working_dir_policy: WorkingDirPolicy::InPlace,
                timeout_secs,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    fn sample_task_spec(working_dir: PathBuf) -> TaskSpec {
        TaskSpec {
            task: "implement parser".to_string(),
            task_brief: None,
            acceptance_criteria: Vec::new(),
            selected_files: Vec::new(),
            working_dir,
        }
    }

    fn sample_hints() -> WorkflowHints {
        WorkflowHints {
            run_mode: RunMode::Sync,
            ..WorkflowHints::default()
        }
    }

    #[tokio::test]
    async fn ollama_runner_succeeds_with_summary_stdout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-ollama.sh");
        let script = r#"#!/bin/sh
set -eu
echo "stub output"
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["next"],
  "verification_status": "Passed",
  "touched_files": ["src/lib.rs"]
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
EOF
exit 0
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = OllamaRunner::new(script_path);
        let execution = runner
            .execute_task(
                &sample_spec(30),
                &sample_task_spec(dir.path().to_path_buf()),
                &sample_hints(),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        assert_eq!(execution.terminal_state, RunnerTerminalState::Succeeded);
        assert!(execution.stdout.contains("MCP_SUBAGENT_SUMMARY_JSON_START"));
    }

    #[tokio::test]
    async fn ollama_runner_reports_nonzero_exit_as_failed() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-ollama-fail.sh");
        let script = r#"#!/bin/sh
set -eu
echo "model missing" >&2
exit 7
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = OllamaRunner::new(script_path);
        let execution = runner
            .execute_task(
                &sample_spec(30),
                &sample_task_spec(dir.path().to_path_buf()),
                &sample_hints(),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        match execution.terminal_state {
            RunnerTerminalState::Failed { message } => {
                assert!(message.contains("code 7"));
            }
            other => panic!("unexpected terminal state: {other:?}"),
        }
        assert!(execution.stderr.contains("model missing"));
    }

    #[tokio::test]
    async fn ollama_runner_marks_timeout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-ollama-timeout.sh");
        let script = r#"#!/bin/sh
set -eu
sleep 2
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = OllamaRunner::new(script_path);
        let execution = runner
            .execute_task(
                &sample_spec(1),
                &sample_task_spec(dir.path().to_path_buf()),
                &sample_hints(),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        assert_eq!(execution.terminal_state, RunnerTerminalState::TimedOut);
    }

    #[tokio::test]
    async fn ollama_runner_requires_model() {
        let dir = tempdir().expect("tempdir");
        let runner = OllamaRunner::new(PathBuf::from("ollama"));
        let mut spec = sample_spec(30);
        spec.core.model = None;

        let err = runner
            .execute_task(
                &spec,
                &sample_task_spec(dir.path().to_path_buf()),
                &sample_hints(),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect_err("model should be required");
        assert!(
            err.to_string().contains("requires `core.model`"),
            "unexpected error: {err}"
        );
    }
}
