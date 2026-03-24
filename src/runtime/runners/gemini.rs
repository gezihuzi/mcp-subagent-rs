use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;

use crate::{
    error::{McpSubagentError, Result},
    runtime::runners::{AgentRunner, RunnerExecution, RunnerTerminalState},
    spec::{
        runtime_policy::{ApprovalPolicy, SandboxPolicy},
        AgentSpec,
    },
    types::{CompiledContext, RunRequest},
};

#[derive(Debug, Clone)]
pub struct GeminiRunner {
    executable: PathBuf,
}

impl Default for GeminiRunner {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("gemini"),
        }
    }
}

impl GeminiRunner {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    pub async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let prompt = compose_prompt(compiled);
        let timeout = Duration::from_secs(spec.runtime.timeout_secs.max(1));
        let approval_mode = resolve_approval_mode(spec)?;

        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("--prompt")
            .arg(&prompt)
            .arg("--output-format")
            .arg("text")
            .arg("--approval-mode")
            .arg(approval_mode)
            .arg("--include-directories")
            .arg(&request.working_dir)
            .current_dir(&request.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(model) = spec.core.model.as_deref() {
            command.arg("--model").arg(model);
        }

        let output = match tokio::time::timeout(timeout, command.output()).await {
            Ok(waited) => waited.map_err(McpSubagentError::Io)?,
            Err(_) => {
                return Ok(RunnerExecution {
                    terminal_state: RunnerTerminalState::TimedOut,
                    stdout: String::new(),
                    stderr: format!(
                        "gemini execution exceeded timeout of {}s",
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
            let mut message = format!("gemini exited with code {exit_code}");
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
impl AgentRunner for GeminiRunner {
    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        GeminiRunner::execute(self, spec, request, compiled).await
    }
}

fn compose_prompt(compiled: &CompiledContext) -> String {
    format!(
        "{}\n\n{}",
        compiled.system_prefix.trim(),
        compiled.injected_prompt.trim()
    )
}

fn resolve_approval_mode(spec: &AgentSpec) -> Result<&'static str> {
    match spec.runtime.approval {
        ApprovalPolicy::ProviderDefault | ApprovalPolicy::DenyByDefault => {
            match spec.runtime.sandbox {
                SandboxPolicy::ReadOnly => Ok("plan"),
                SandboxPolicy::WorkspaceWrite => Ok("auto_edit"),
                SandboxPolicy::FullAccess => Ok("yolo"),
            }
        }
        ApprovalPolicy::Ask => Err(McpSubagentError::SpecValidation(
            "Gemini approval policy `Ask` is not yet validated for current CLI mapping".to_string(),
        )),
        ApprovalPolicy::AutoAcceptEdits => Err(McpSubagentError::SpecValidation(
            "Gemini approval policy `AutoAcceptEdits` is not yet validated for current CLI mapping"
                .to_string(),
        )),
    }
}

pub fn supports_provider(provider: &crate::spec::Provider) -> bool {
    matches!(provider, crate::spec::Provider::Gemini)
}

pub fn from_env() -> GeminiRunner {
    let configured = std::env::var("MCP_SUBAGENT_GEMINI_BIN").ok();
    match configured {
        Some(path) if !path.trim().is_empty() => GeminiRunner::new(Path::new(&path).to_path_buf()),
        _ => GeminiRunner::default(),
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use tempfile::tempdir;

    use crate::{
        runtime::{runners::gemini::GeminiRunner, runners::RunnerTerminalState},
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{ApprovalPolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, RunRequest},
    };

    fn sample_spec(timeout_secs: u64) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "investigator".to_string(),
                description: "investigate".to_string(),
                provider: Provider::Gemini,
                model: None,
                instructions: "You are an investigator".to_string(),
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

    fn sample_request(working_dir: PathBuf) -> RunRequest {
        RunRequest {
            task: "investigate parser".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            stage: None,
            plan_ref: None,
            working_dir,
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        }
    }

    #[tokio::test]
    async fn gemini_runner_succeeds_with_summary_stdout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini.sh");
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
  "exit_code": 0,
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

        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(30),
                &sample_request(dir.path().to_path_buf()),
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
    async fn gemini_runner_reports_nonzero_exit_as_failed() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-fail.sh");
        let script = r#"#!/bin/sh
set -eu
echo "auth required" >&2
exit 9
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(30),
                &sample_request(dir.path().to_path_buf()),
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
                assert!(message.contains("code 9"));
            }
            other => panic!("unexpected terminal state: {other:?}"),
        }
        assert!(execution.stderr.contains("auth required"));
    }

    #[tokio::test]
    async fn gemini_runner_marks_timeout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-timeout.sh");
        let script = r#"#!/bin/sh
set -eu
sleep 2
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(1),
                &sample_request(dir.path().to_path_buf()),
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
    async fn gemini_runner_rejects_unvalidated_approval_policy() {
        let dir = tempdir().expect("tempdir");
        let mut spec = sample_spec(30);
        spec.runtime.approval = ApprovalPolicy::Ask;
        let runner = GeminiRunner::new(PathBuf::from("gemini"));

        let err = runner
            .execute(
                &spec,
                &sample_request(dir.path().to_path_buf()),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect_err("Ask should be rejected until validated");
        assert!(
            err.to_string().contains("not yet validated"),
            "unexpected error: {err}"
        );
    }
}
