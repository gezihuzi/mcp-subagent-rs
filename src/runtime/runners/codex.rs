use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::{
    error::{McpSubagentError, Result},
    runtime::runners::{
        streaming::collect_streaming_output, AgentRunner, RunnerExecution, RunnerOutputObserver,
        RunnerTerminalState,
    },
    spec::{
        provider_overrides::{CodexSandboxMode, ReasoningEffort},
        runtime_policy::{ApprovalPolicy, SandboxPolicy},
        AgentSpec,
    },
    types::{CompiledContext, RunRequest},
};

#[derive(Debug, Clone)]
pub struct CodexRunner {
    executable: PathBuf,
}

impl Default for CodexRunner {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("codex"),
        }
    }
}

impl CodexRunner {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    async fn execute_internal(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
        mut observer: Option<&mut dyn RunnerOutputObserver>,
    ) -> Result<RunnerExecution> {
        let prompt = compose_prompt(compiled);
        let output_file = std::env::temp_dir().join(format!(
            "mcp-subagent-codex-last-message-{}.txt",
            uuid::Uuid::now_v7()
        ));
        let schema_file = std::env::temp_dir().join(format!(
            "mcp-subagent-summary-schema-{}.json",
            uuid::Uuid::now_v7()
        ));
        let schema_json = build_codex_output_schema_json()?;
        fs::write(&schema_file, schema_json).map_err(McpSubagentError::Io)?;
        let timeout = Duration::from_secs(spec.runtime.timeout_secs.max(1));
        validate_approval_policy(spec)?;

        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("exec")
            .arg("--skip-git-repo-check")
            .arg("--sandbox")
            .arg(resolve_sandbox(spec))
            .arg("--cd")
            .arg(&request.working_dir)
            .arg("--output-last-message")
            .arg(&output_file)
            .arg("--output-schema")
            .arg(&schema_file)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(model) = spec.core.model.as_deref() {
            command.arg("--model").arg(model);
        }
        if let Some(reasoning) = spec
            .provider_overrides
            .codex
            .as_ref()
            .and_then(|override_cfg| override_cfg.model_reasoning_effort.as_ref())
        {
            command.arg("-c").arg(format!(
                "model_reasoning_effort=\"{}\"",
                map_reasoning_effort(reasoning)
            ));
        }

        let mut child = command.spawn().map_err(McpSubagentError::Io)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(McpSubagentError::Io)?;
        }

        let (status, mut stdout, stderr, timed_out) = match observer.as_deref_mut() {
            Some(output_observer) => {
                let observed =
                    collect_streaming_output(&mut child, timeout, output_observer).await?;
                (
                    observed.status,
                    observed.stdout,
                    observed.stderr,
                    observed.timed_out,
                )
            }
            None => match tokio::time::timeout(timeout, child.wait_with_output()).await {
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
                    let _ = fs::remove_file(&output_file);
                    let _ = fs::remove_file(&schema_file);
                    return Ok(RunnerExecution {
                        terminal_state: RunnerTerminalState::TimedOut,
                        stdout: String::new(),
                        stderr: format!(
                            "codex execution exceeded timeout of {}s",
                            timeout.as_secs()
                        ),
                    });
                }
            },
        };

        if timed_out {
            let _ = fs::remove_file(&output_file);
            let _ = fs::remove_file(&schema_file);
            return Ok(RunnerExecution {
                terminal_state: RunnerTerminalState::TimedOut,
                stdout,
                stderr: if stderr.is_empty() {
                    format!("codex execution exceeded timeout of {}s", timeout.as_secs())
                } else {
                    stderr
                },
            });
        }

        if let Ok(last_message) = fs::read_to_string(&output_file) {
            if !last_message.trim().is_empty() {
                if !stdout.is_empty() && !stdout.ends_with('\n') {
                    stdout.push('\n');
                }
                stdout.push_str(&last_message);
            }
        }
        let _ = fs::remove_file(&output_file);
        let _ = fs::remove_file(&schema_file);

        let terminal_state = if status.success() {
            RunnerTerminalState::Succeeded
        } else {
            let exit_code = status.code().unwrap_or(-1);
            let mut message = format!("codex exited with code {exit_code}");
            if let Some(summary_line) = summarize_codex_stderr(&stderr) {
                message.push_str(": ");
                message.push_str(&summary_line);
            }
            RunnerTerminalState::Failed { message }
        };

        Ok(RunnerExecution {
            terminal_state,
            stdout,
            stderr,
        })
    }

    pub async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        self.execute_internal(spec, request, compiled, None).await
    }
}

#[async_trait]
impl AgentRunner for CodexRunner {
    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        CodexRunner::execute(self, spec, request, compiled).await
    }

    async fn execute_with_observer(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        self.execute_internal(spec, request, compiled, Some(observer))
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

fn build_codex_output_schema_json() -> Result<String> {
    let schema = schemars::schema_for!(crate::runtime::summary::SummaryEnvelope);
    let mut schema_value = serde_json::to_value(schema).map_err(McpSubagentError::Json)?;
    normalize_openai_strict_schema(&mut schema_value);
    serde_json::to_string_pretty(&schema_value).map_err(McpSubagentError::Json)
}

fn normalize_openai_strict_schema(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                normalize_openai_strict_schema(child);
            }
            if let Some(Value::Object(properties)) = map.get("properties") {
                let mut keys = properties.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                map.insert(
                    "required".to_string(),
                    Value::Array(keys.into_iter().map(Value::String).collect()),
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_openai_strict_schema(item);
            }
        }
        _ => {}
    }
}

fn summarize_codex_stderr(stderr: &str) -> Option<String> {
    let lines = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines
        .iter()
        .find(|line| line.starts_with("ERROR:"))
        .map(|line| (*line).to_string())
        .or_else(|| {
            lines
                .iter()
                .find(|line| line.contains("invalid_json_schema"))
                .map(|line| (*line).to_string())
        })
        .or_else(|| {
            lines
                .iter()
                .find(|line| line.to_ascii_lowercase().contains("error"))
                .map(|line| (*line).to_string())
        })
        .or_else(|| lines.first().map(|line| (*line).to_string()))
}

fn resolve_sandbox(spec: &AgentSpec) -> &'static str {
    if let Some(codex_override) = spec.provider_overrides.codex.as_ref() {
        if let Some(mode) = codex_override.sandbox_mode.as_ref() {
            return match mode {
                CodexSandboxMode::ReadOnly => "read-only",
                CodexSandboxMode::WorkspaceWrite => "workspace-write",
                CodexSandboxMode::FullAccess => "danger-full-access",
            };
        }
    }

    match spec.runtime.sandbox {
        SandboxPolicy::ReadOnly => "read-only",
        SandboxPolicy::WorkspaceWrite => "workspace-write",
        SandboxPolicy::FullAccess => "danger-full-access",
    }
}

fn map_reasoning_effort(value: &ReasoningEffort) -> &'static str {
    match value {
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
    }
}

fn validate_approval_policy(spec: &AgentSpec) -> Result<()> {
    match spec.runtime.approval {
        ApprovalPolicy::ProviderDefault | ApprovalPolicy::DenyByDefault => Ok(()),
        ApprovalPolicy::Ask => Err(McpSubagentError::SpecValidation(
            "Codex approval policy `Ask` is not yet validated for current CLI mapping".to_string(),
        )),
        ApprovalPolicy::AutoAcceptEdits => Err(McpSubagentError::SpecValidation(
            "Codex approval policy `AutoAcceptEdits` is not yet validated for current CLI mapping"
                .to_string(),
        )),
    }
}

pub fn supports_provider(provider: &crate::spec::Provider) -> bool {
    matches!(provider, crate::spec::Provider::Codex)
}

pub fn from_env() -> CodexRunner {
    let configured = std::env::var("MCP_SUBAGENT_CODEX_BIN").ok();
    match configured {
        Some(path) if !path.trim().is_empty() => CodexRunner::new(Path::new(&path).to_path_buf()),
        _ => CodexRunner::default(),
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use serde_json::Value;
    use tempfile::tempdir;

    use crate::{
        runtime::{
            runners::codex::{
                build_codex_output_schema_json, normalize_openai_strict_schema,
                summarize_codex_stderr, CodexRunner,
            },
            runners::{AgentRunner, RunnerOutputObserver, RunnerOutputStream, RunnerTerminalState},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{ApprovalPolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{CompiledContext, RunMode, RunRequest},
    };

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "review".to_string(),
                provider: Provider::Codex,
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
                timeout_secs: 30,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    fn sample_request(working_dir: PathBuf) -> RunRequest {
        RunRequest {
            task: "review parser".to_string(),
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
    async fn codex_runner_reads_last_message_file() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-codex.sh");
        let script = r#"#!/bin/sh
set -eu
output_file=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output-last-message)
      output_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
cat >/dev/null
cat >"$output_file" <<'EOF'
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
echo "stub stdout"
exit 0
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = CodexRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(),
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
    async fn codex_runner_passes_output_schema_flag() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-codex-schema.sh");
        let script = r#"#!/bin/sh
set -eu
output_file=""
schema_file=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-last-message)
      output_file="$2"
      shift 2
      ;;
    --output-schema)
      schema_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
[ -n "$schema_file" ] || { echo "missing --output-schema" >&2; exit 12; }
[ -f "$schema_file" ] || { echo "schema file not found" >&2; exit 13; }
cat >/dev/null
cat >"$output_file" <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["next"],
  "exit_code": 0,
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

        let runner = CodexRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(),
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
    }

    #[tokio::test]
    async fn codex_runner_reports_nonzero_exit_as_failed() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-codex-fail.sh");
        let script = r#"#!/bin/sh
set -eu
echo "auth required" >&2
exit 7
"#;
        fs::write(&script_path, script).expect("write script");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod");

        let runner = CodexRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(),
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
                assert!(message.contains("code 7"));
            }
            other => panic!("unexpected terminal state: {other:?}"),
        }
        assert!(execution.stderr.contains("auth required"));
    }

    #[tokio::test]
    async fn codex_runner_execute_with_observer_streams_output_chunks() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-codex-stream.sh");
        let script = r#"#!/bin/sh
set -eu
output_file=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-last-message)
      output_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
cat >/dev/null
echo "stdout-chunk-1"
sleep 0.1
echo "stdout-chunk-2"
echo "stderr-chunk-1" >&2
sleep 0.1
echo "stderr-chunk-2" >&2
cat >"$output_file" <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": [],
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

        let runner = CodexRunner::new(script_path);
        let mut observer = CollectingObserver::default();
        let execution = <CodexRunner as AgentRunner>::execute_with_observer(
            &runner,
            &sample_spec(),
            &sample_request(dir.path().to_path_buf()),
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
    async fn codex_runner_rejects_unvalidated_approval_policy() {
        let dir = tempdir().expect("tempdir");
        let mut spec = sample_spec();
        spec.runtime.approval = ApprovalPolicy::Ask;
        let runner = CodexRunner::new(PathBuf::from("codex"));

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

    #[test]
    fn strict_schema_marks_all_properties_as_required() {
        let mut value = serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "media_type": {"type": ["string", "null"]}
            },
            "required": ["summary"]
        });

        normalize_openai_strict_schema(&mut value);

        let required = value
            .get("required")
            .and_then(Value::as_array)
            .expect("required array");
        let rendered = required
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(rendered.contains(&"summary"));
        assert!(rendered.contains(&"media_type"));
    }

    #[test]
    fn schema_json_includes_media_type_in_required_list() {
        let schema_json = build_codex_output_schema_json().expect("schema");
        assert!(
            schema_json.contains("\"media_type\""),
            "schema should include media_type property"
        );
        assert!(
            schema_json.contains("\"required\""),
            "schema should include required arrays"
        );
    }

    #[test]
    fn summarize_stderr_prefers_error_lines() {
        let stderr = "\
OpenAI Codex v0.116.0 (research preview)
ERROR: {\"error\":{\"code\":\"invalid_json_schema\"}}
warning: trailing message
";
        let line = summarize_codex_stderr(stderr).expect("summary line");
        assert!(line.starts_with("ERROR:"));
    }
}
