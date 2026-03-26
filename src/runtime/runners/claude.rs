use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;

use crate::{
    error::{McpSubagentError, Result},
    runtime::runners::{
        streaming::collect_streaming_output, AgentRunner, RunnerExecution, RunnerOutputObserver,
        RunnerTerminalState,
    },
    spec::{
        runtime_policy::{ApprovalPolicy, SandboxPolicy},
        AgentSpec,
    },
    types::{CompiledContext, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone)]
pub struct ClaudeRunner {
    executable: PathBuf,
}

impl Default for ClaudeRunner {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("claude"),
        }
    }
}

impl ClaudeRunner {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    async fn execute_internal(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        compiled: &CompiledContext,
        observer: Option<&mut dyn RunnerOutputObserver>,
    ) -> Result<RunnerExecution> {
        let prompt = compose_prompt(compiled);
        let schema_file = std::env::temp_dir().join(format!(
            "mcp-subagent-summary-schema-{}.json",
            uuid::Uuid::now_v7()
        ));
        let schema = schemars::schema_for!(crate::runtime::summary::ProviderSummary);
        let schema_json = serde_json::to_string_pretty(&schema).map_err(McpSubagentError::Json)?;
        fs::write(&schema_file, schema_json).map_err(McpSubagentError::Io)?;
        let timeout = Duration::from_secs(spec.runtime.timeout_secs.max(1));
        let permission_mode = resolve_permission_mode(spec)?;

        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("--print")
            .arg("--output-format")
            .arg("text")
            .arg("--json-schema")
            .arg(&schema_file)
            .arg("--permission-mode")
            .arg(permission_mode)
            .arg("--add-dir")
            .arg(&task_spec.working_dir)
            .arg(&prompt)
            .current_dir(&task_spec.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(model) = spec.core.model.as_deref() {
            command.arg("--model").arg(model);
        }

        let (status, stdout, stderr, timed_out) = match observer {
            Some(output_observer) => {
                let mut child = command.spawn().map_err(McpSubagentError::Io)?;
                let observed =
                    collect_streaming_output(&mut child, timeout, output_observer).await?;
                (
                    observed.status,
                    observed.stdout,
                    observed.stderr,
                    observed.timed_out,
                )
            }
            None => match tokio::time::timeout(timeout, command.output()).await {
                Ok(waited) => {
                    let output = waited.map_err(McpSubagentError::Io)?;
                    (
                        output.status,
                        String::from_utf8_lossy(&output.stdout).to_string(),
                        String::from_utf8_lossy(&output.stderr).to_string(),
                        false,
                    )
                }
                Err(_) => {
                    let _ = fs::remove_file(&schema_file);
                    return Ok(RunnerExecution {
                        terminal_state: RunnerTerminalState::TimedOut,
                        stdout: String::new(),
                        stderr: format!(
                            "claude execution exceeded timeout of {}s",
                            timeout.as_secs()
                        ),
                    });
                }
            },
        };

        let _ = fs::remove_file(&schema_file);

        if timed_out {
            return Ok(RunnerExecution {
                terminal_state: RunnerTerminalState::TimedOut,
                stdout,
                stderr: if stderr.is_empty() {
                    format!(
                        "claude execution exceeded timeout of {}s",
                        timeout.as_secs()
                    )
                } else {
                    stderr
                },
            });
        }

        let terminal_state = if status.success() {
            RunnerTerminalState::Succeeded
        } else {
            let exit_code = status.code().unwrap_or(-1);
            let mut message = format!("claude exited with code {exit_code}");
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

    pub async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        _hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        self.execute_internal(spec, task_spec, compiled, None).await
    }

    pub async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        _hints: &WorkflowHints,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        self.execute_internal(spec, task_spec, compiled, Some(observer))
            .await
    }
}

#[async_trait]
impl AgentRunner for ClaudeRunner {
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        ClaudeRunner::execute_task(self, spec, task_spec, hints, compiled).await
    }

    async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        ClaudeRunner::execute_task_with_observer(self, spec, task_spec, hints, compiled, observer)
            .await
    }
}

fn compose_prompt(compiled: &CompiledContext) -> String {
    format!(
        "{}\n\n{}",
        compiled.system_prefix.trim(),
        compiled.injected_prompt.trim()
    )
}

fn resolve_permission_mode(spec: &AgentSpec) -> Result<String> {
    if let Some(mode) = spec
        .provider_overrides
        .claude
        .as_ref()
        .and_then(|override_cfg| override_cfg.permission_mode.as_ref())
    {
        if !matches!(
            mode.as_str(),
            "default" | "acceptEdits" | "plan" | "dontAsk" | "bypassPermissions"
        ) {
            return Err(McpSubagentError::SpecValidation(format!(
                "unsupported Claude permission_mode override `{mode}`; supported: default|acceptEdits|plan|dontAsk|bypassPermissions"
            )));
        }
        return Ok(mode.clone());
    }

    let mapped = match spec.runtime.approval {
        ApprovalPolicy::ProviderDefault => match spec.runtime.sandbox {
            SandboxPolicy::ReadOnly => "plan",
            SandboxPolicy::WorkspaceWrite => "acceptEdits",
            SandboxPolicy::FullAccess => "bypassPermissions",
        },
        ApprovalPolicy::DenyByDefault => "plan",
        ApprovalPolicy::AutoAcceptEdits => "acceptEdits",
        ApprovalPolicy::Ask => {
            return Err(McpSubagentError::SpecValidation(
                "Claude approval policy `Ask` is not yet validated for current CLI mapping"
                    .to_string(),
            ))
        }
    };

    Ok(mapped.to_string())
}

pub fn supports_provider(provider: &crate::spec::Provider) -> bool {
    matches!(provider, crate::spec::Provider::Claude)
}

pub fn from_env() -> ClaudeRunner {
    let configured = std::env::var("MCP_SUBAGENT_CLAUDE_BIN").ok();
    match configured {
        Some(path) if !path.trim().is_empty() => ClaudeRunner::new(Path::new(&path).to_path_buf()),
        _ => ClaudeRunner::default(),
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use tempfile::tempdir;

    use crate::{
        runtime::runners::{
            claude::ClaudeRunner, AgentRunner, RunnerOutputObserver, RunnerOutputStream,
            RunnerTerminalState,
        },
        spec::{
            core::{AgentSpecCore, Provider},
            provider_overrides::{ClaudeOverrides, ProviderOverrides},
            runtime_policy::{ApprovalPolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, TaskSpec, WorkflowHints},
    };

    fn sample_spec(timeout_secs: u64) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "review".to_string(),
                provider: Provider::Claude,
                model: None,
                instructions: "You are a reviewer".to_string(),
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
            task: "review parser".to_string(),
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

    #[derive(Default)]
    struct CollectingObserver {
        events: Vec<(RunnerOutputStream, String)>,
    }

    impl RunnerOutputObserver for CollectingObserver {
        fn on_output(&mut self, stream: RunnerOutputStream, chunk: &str) {
            self.events.push((stream, chunk.to_string()));
        }
    }

    #[tokio::test]
    async fn claude_runner_succeeds_with_summary_stdout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude.sh");
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

        let runner = ClaudeRunner::new(script_path);
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
    async fn claude_runner_passes_json_schema_flag() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude-schema.sh");
        let script = r#"#!/bin/sh
set -eu
schema_file=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --json-schema)
      schema_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
[ -n "$schema_file" ] || { echo "missing --json-schema" >&2; exit 21; }
[ -f "$schema_file" ] || { echo "schema file not found" >&2; exit 22; }
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["next"],
  "verification_status": "Passed",
  "touched_files": ["src/lib.rs"],
  "plan_refs": []
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
EOF
exit 0
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = ClaudeRunner::new(script_path);
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
    }

    #[tokio::test]
    async fn claude_runner_reports_nonzero_exit_as_failed() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude-fail.sh");
        let script = r#"#!/bin/sh
set -eu
echo "auth required" >&2
exit 6
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = ClaudeRunner::new(script_path);
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
                assert!(message.contains("code 6"));
            }
            other => panic!("unexpected terminal state: {other:?}"),
        }
        assert!(execution.stderr.contains("auth required"));
    }

    #[tokio::test]
    async fn claude_runner_execute_with_observer_streams_output_chunks() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude-stream.sh");
        let script = r#"#!/bin/sh
set -eu
echo "stdout-chunk-1"
sleep 0.1
echo "stdout-chunk-2"
echo "stderr-chunk-1" >&2
sleep 0.1
echo "stderr-chunk-2" >&2
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "verification_status": "Passed",
  "touched_files": []
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
EOF
exit 0
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = ClaudeRunner::new(script_path);
        let mut observer = CollectingObserver::default();
        let execution = <ClaudeRunner as AgentRunner>::execute_task_with_observer(
            &runner,
            &sample_spec(30),
            &sample_task_spec(dir.path().to_path_buf()),
            &sample_hints(),
            &CompiledContext {
                system_prefix: "sys".to_string(),
                injected_prompt: "prompt".to_string(),
                source_manifest: Vec::new(),
            },
            &mut observer,
        )
        .await
        .expect("execute with observer");

        assert_eq!(execution.terminal_state, RunnerTerminalState::Succeeded);
        assert!(
            observer.events.iter().any(|(stream, chunk)| matches!(
                stream,
                RunnerOutputStream::Stdout
            ) && chunk.contains("stdout-chunk")),
            "observer should receive stdout chunks"
        );
        assert!(
            observer.events.iter().any(|(stream, chunk)| matches!(
                stream,
                RunnerOutputStream::Stderr
            ) && chunk.contains("stderr-chunk")),
            "observer should receive stderr chunks"
        );
    }

    #[tokio::test]
    async fn claude_runner_marks_timeout() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude-timeout.sh");
        let script = r#"#!/bin/sh
set -eu
sleep 2
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = ClaudeRunner::new(script_path);
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
    async fn claude_runner_rejects_invalid_permission_mode_override() {
        let dir = tempdir().expect("tempdir");
        let mut spec = sample_spec(30);
        spec.provider_overrides = ProviderOverrides {
            claude: Some(ClaudeOverrides {
                permission_mode: Some("unknown_mode".to_string()),
            }),
            codex: None,
            gemini: None,
        };
        let runner = ClaudeRunner::new(PathBuf::from("claude"));

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
            .expect_err("invalid permission_mode should fail");
        assert!(
            err.to_string()
                .contains("unsupported Claude permission_mode"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn claude_runner_rejects_legacy_auto_permission_mode_override() {
        let dir = tempdir().expect("tempdir");
        let mut spec = sample_spec(30);
        spec.provider_overrides = ProviderOverrides {
            claude: Some(ClaudeOverrides {
                permission_mode: Some("auto".to_string()),
            }),
            codex: None,
            gemini: None,
        };
        let runner = ClaudeRunner::new(PathBuf::from("claude"));

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
            .expect_err("legacy auto permission_mode should fail");
        assert!(
            err.to_string()
                .contains("supported: default|acceptEdits|plan|dontAsk|bypassPermissions"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn claude_runner_maps_full_access_to_bypass_permissions() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-claude-perm.sh");
        let script = r#"#!/bin/sh
set -eu
permission_mode=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --permission-mode)
      permission_mode="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
[ "$permission_mode" = "bypassPermissions" ] || {
  echo "unexpected permission mode: $permission_mode" >&2
  exit 33
}
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "verification_status": "Passed",
  "touched_files": []
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
EOF
exit 0
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let mut spec = sample_spec(30);
        spec.runtime.sandbox = SandboxPolicy::FullAccess;
        let runner = ClaudeRunner::new(script_path);
        let execution = runner
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
            .expect("execute");

        assert_eq!(execution.terminal_state, RunnerTerminalState::Succeeded);
    }

    #[tokio::test]
    async fn claude_runner_rejects_unvalidated_approval_policy() {
        let dir = tempdir().expect("tempdir");
        let mut spec = sample_spec(30);
        spec.runtime.approval = ApprovalPolicy::Ask;
        let runner = ClaudeRunner::new(PathBuf::from("claude"));

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
            .expect_err("Ask should be rejected until validated");
        assert!(
            err.to_string().contains("not yet validated"),
            "unexpected error: {err}"
        );
    }
}
