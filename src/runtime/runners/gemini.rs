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
        runtime_policy::{ApprovalPolicy, NativeDiscoveryPolicy, SandboxPolicy},
        AgentSpec,
    },
    types::{CompiledContext, RunRequest, TaskSpec, WorkflowHints},
};

const GEMINI_DISCOVERY_TEMP_PREFIX: &str = "mcp-subagent-gemini-discovery";
const ISOLATED_FALLBACK_NOTE: &str =
    "mcp-subagent: isolated native discovery failed with auth-like error; retried with minimal discovery.";

#[derive(Debug, Clone)]
pub struct GeminiRunner {
    executable: PathBuf,
}

struct GeminiExecuteOptions<'a> {
    prompt: &'a str,
    approval_mode: &'a str,
    timeout: Duration,
    discovery_policy: &'a NativeDiscoveryPolicy,
    observer: Option<&'a mut dyn RunnerOutputObserver>,
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

    async fn execute_internal(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
        observer: Option<&mut dyn RunnerOutputObserver>,
    ) -> Result<RunnerExecution> {
        let prompt = compose_prompt(compiled);
        let timeout = Duration::from_secs(spec.runtime.timeout_secs.max(1));
        let approval_mode = resolve_approval_mode(spec)?;
        let discovery_policy = spec.runtime.native_discovery.clone();

        if let Some(observer_ref) = observer {
            let execution = self
                .execute_once(
                    spec,
                    request,
                    GeminiExecuteOptions {
                        prompt: &prompt,
                        approval_mode,
                        timeout,
                        discovery_policy: &discovery_policy,
                        observer: Some(&mut *observer_ref),
                    },
                )
                .await?;
            if matches!(discovery_policy, NativeDiscoveryPolicy::Isolated)
                && should_retry_isolated_with_minimal(&execution)
            {
                let mut retried = self
                    .execute_once(
                        spec,
                        request,
                        GeminiExecuteOptions {
                            prompt: &prompt,
                            approval_mode,
                            timeout,
                            discovery_policy: &NativeDiscoveryPolicy::Minimal,
                            observer: Some(&mut *observer_ref),
                        },
                    )
                    .await?;
                retried.stderr = merge_fallback_stderr(&execution.stderr, &retried.stderr);
                return Ok(retried);
            }
            return Ok(execution);
        }

        let execution = self
            .execute_once(
                spec,
                request,
                GeminiExecuteOptions {
                    prompt: &prompt,
                    approval_mode,
                    timeout,
                    discovery_policy: &discovery_policy,
                    observer: None,
                },
            )
            .await?;

        if matches!(discovery_policy, NativeDiscoveryPolicy::Isolated)
            && should_retry_isolated_with_minimal(&execution)
        {
            let mut retried = self
                .execute_once(
                    spec,
                    request,
                    GeminiExecuteOptions {
                        prompt: &prompt,
                        approval_mode,
                        timeout,
                        discovery_policy: &NativeDiscoveryPolicy::Minimal,
                        observer: None,
                    },
                )
                .await?;
            retried.stderr = merge_fallback_stderr(&execution.stderr, &retried.stderr);
            return Ok(retried);
        }

        Ok(execution)
    }

    pub async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let task_spec = request.to_task_spec();
        let hints = request.to_workflow_hints();
        GeminiRunner::execute_task(self, spec, &task_spec, &hints, compiled).await
    }

    pub async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        let request = RunRequest::from_parts(task_spec, hints);
        self.execute_internal(spec, &request, compiled, None).await
    }

    pub async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        let request = RunRequest::from_parts(task_spec, hints);
        self.execute_internal(spec, &request, compiled, Some(observer))
            .await
    }

    async fn execute_once(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        options: GeminiExecuteOptions<'_>,
    ) -> Result<RunnerExecution> {
        let launch = prepare_discovery_launch(options.discovery_policy, &request.working_dir)?;
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("--prompt")
            .arg(options.prompt)
            .arg("--output-format")
            .arg("text")
            .arg("--approval-mode")
            .arg(options.approval_mode)
            .arg("--include-directories")
            .arg(&launch.include_dir)
            .current_dir(&launch.current_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(model) = spec.core.model.as_deref() {
            command.arg("--model").arg(model);
        }
        for (key, value) in &launch.env_overrides {
            command.env(key, value);
        }

        let (status, stdout, stderr, timed_out) = match options.observer {
            Some(output_observer) => {
                let mut child = command.spawn().map_err(McpSubagentError::Io)?;
                let observed =
                    collect_streaming_output(&mut child, options.timeout, output_observer).await?;
                (
                    observed.status,
                    observed.stdout,
                    observed.stderr,
                    observed.timed_out,
                )
            }
            None => match tokio::time::timeout(options.timeout, command.output()).await {
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
                    cleanup_discovery_dirs(&launch.cleanup_dirs);
                    return Ok(RunnerExecution {
                        terminal_state: RunnerTerminalState::TimedOut,
                        stdout: String::new(),
                        stderr: format!(
                            "gemini execution exceeded timeout of {}s",
                            options.timeout.as_secs()
                        ),
                    });
                }
            },
        };
        cleanup_discovery_dirs(&launch.cleanup_dirs);

        if timed_out {
            return Ok(RunnerExecution {
                terminal_state: RunnerTerminalState::TimedOut,
                stdout,
                stderr: if stderr.is_empty() {
                    format!(
                        "gemini execution exceeded timeout of {}s",
                        options.timeout.as_secs()
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

#[derive(Debug)]
struct DiscoveryLaunch {
    include_dir: PathBuf,
    current_dir: PathBuf,
    env_overrides: Vec<(String, String)>,
    cleanup_dirs: Vec<PathBuf>,
}

fn prepare_discovery_launch(
    policy: &NativeDiscoveryPolicy,
    working_dir: &Path,
) -> Result<DiscoveryLaunch> {
    let include_dir = resolve_include_dir(working_dir)?;
    match policy {
        NativeDiscoveryPolicy::Inherit | NativeDiscoveryPolicy::Allowlist => Ok(DiscoveryLaunch {
            include_dir: include_dir.clone(),
            current_dir: include_dir,
            env_overrides: Vec::new(),
            cleanup_dirs: Vec::new(),
        }),
        NativeDiscoveryPolicy::Minimal => {
            let launch_dir = create_temp_dir("minimal-launch")?;
            Ok(DiscoveryLaunch {
                include_dir,
                current_dir: launch_dir.clone(),
                env_overrides: Vec::new(),
                cleanup_dirs: vec![launch_dir],
            })
        }
        NativeDiscoveryPolicy::Isolated => {
            let root = create_temp_dir("isolated-root")?;
            let launch_dir = root.join("launch");
            let home_dir = root.join("home");
            let xdg_config = home_dir.join(".config");
            let xdg_data = home_dir.join(".local/share");
            let xdg_cache = home_dir.join(".cache");
            for dir in [&launch_dir, &home_dir, &xdg_config, &xdg_data, &xdg_cache] {
                fs::create_dir_all(dir).map_err(McpSubagentError::Io)?;
            }
            Ok(DiscoveryLaunch {
                include_dir,
                current_dir: launch_dir,
                env_overrides: vec![
                    ("HOME".to_string(), home_dir.display().to_string()),
                    (
                        "XDG_CONFIG_HOME".to_string(),
                        xdg_config.display().to_string(),
                    ),
                    ("XDG_DATA_HOME".to_string(), xdg_data.display().to_string()),
                    (
                        "XDG_CACHE_HOME".to_string(),
                        xdg_cache.display().to_string(),
                    ),
                    (
                        "MCP_SUBAGENT_NATIVE_DISCOVERY".to_string(),
                        "isolated".to_string(),
                    ),
                ],
                cleanup_dirs: vec![root],
            })
        }
    }
}

fn resolve_include_dir(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .map_err(McpSubagentError::Io)?
        .join(path))
}

fn create_temp_dir(label: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "{GEMINI_DISCOVERY_TEMP_PREFIX}-{label}-{}",
        uuid::Uuid::now_v7()
    ));
    fs::create_dir_all(&path).map_err(McpSubagentError::Io)?;
    Ok(path)
}

fn cleanup_discovery_dirs(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_dir_all(path);
    }
}

fn should_retry_isolated_with_minimal(execution: &RunnerExecution) -> bool {
    if !matches!(execution.terminal_state, RunnerTerminalState::Failed { .. }) {
        return false;
    }
    let lowered = execution.stderr.to_ascii_lowercase();
    [
        "auth required",
        "authentication",
        "login",
        "credential",
        "api key",
        "unauthorized",
        "permission denied",
    ]
    .iter()
    .any(|keyword| lowered.contains(keyword))
}

fn merge_fallback_stderr(primary_stderr: &str, retried_stderr: &str) -> String {
    let mut merged = String::new();
    if !retried_stderr.trim().is_empty() {
        merged.push_str(retried_stderr.trim());
        merged.push('\n');
    }
    merged.push_str(ISOLATED_FALLBACK_NOTE);
    if !primary_stderr.trim().is_empty() {
        merged.push_str("\ninitial isolated stderr:\n");
        merged.push_str(primary_stderr.trim());
    }
    merged
}

#[async_trait]
impl AgentRunner for GeminiRunner {
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        GeminiRunner::execute_task(self, spec, task_spec, hints, compiled).await
    }

    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        GeminiRunner::execute(self, spec, request, compiled).await
    }

    async fn execute_with_observer(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        let task_spec = request.to_task_spec();
        let hints = request.to_workflow_hints();
        GeminiRunner::execute_task_with_observer(self, spec, &task_spec, &hints, compiled, observer)
            .await
    }

    async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        GeminiRunner::execute_task_with_observer(self, spec, task_spec, hints, compiled, observer)
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

fn resolve_approval_mode(spec: &AgentSpec) -> Result<&'static str> {
    match spec.runtime.approval {
        ApprovalPolicy::ProviderDefault | ApprovalPolicy::DenyByDefault => {
            match spec.runtime.sandbox {
                SandboxPolicy::ReadOnly => Ok("default"),
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
        runtime::runners::{
            gemini::GeminiRunner, AgentRunner, RunnerOutputObserver, RunnerOutputStream,
            RunnerTerminalState,
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{
                ApprovalPolicy, NativeDiscoveryPolicy, RuntimePolicy, SandboxPolicy,
                WorkingDirPolicy,
            },
            AgentSpec,
        },
        types::{CompiledContext, RunMode, RunRequest},
    };

    fn sample_spec(timeout_secs: u64, native_discovery: NativeDiscoveryPolicy) -> AgentSpec {
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
                native_discovery,
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
                &sample_spec(30, NativeDiscoveryPolicy::Inherit),
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
    async fn gemini_runner_maps_readonly_to_default_approval_mode() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-approval.sh");
        let script = r#"#!/bin/sh
set -eu
approval_mode=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --approval-mode)
      approval_mode="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
[ "$approval_mode" = "default" ] || {
  echo "unexpected approval mode: $approval_mode" >&2
  exit 31
}
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
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

        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(30, NativeDiscoveryPolicy::Inherit),
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
                &sample_spec(30, NativeDiscoveryPolicy::Inherit),
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
    async fn gemini_runner_execute_with_observer_streams_output_chunks() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-stream.sh");
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
  "exit_code": 0,
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

        let runner = GeminiRunner::new(script_path);
        let mut observer = CollectingObserver::default();
        let execution = <GeminiRunner as AgentRunner>::execute_with_observer(
            &runner,
            &sample_spec(30, NativeDiscoveryPolicy::Inherit),
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
                &sample_spec(1, NativeDiscoveryPolicy::Inherit),
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
        let mut spec = sample_spec(30, NativeDiscoveryPolicy::Inherit);
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

    #[tokio::test]
    async fn gemini_runner_minimal_discovery_uses_isolated_launch_cwd() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-minimal-discovery.sh");
        let script = r#"#!/bin/sh
set -eu
include=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --include-directories)
      include="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
echo "PWD:$PWD"
echo "INCLUDE:$include"
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
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

        let working_dir = dir.path().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace");
        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(30, NativeDiscoveryPolicy::Minimal),
                &sample_request(working_dir.clone()),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        let pwd_line = execution
            .stdout
            .lines()
            .find(|line| line.starts_with("PWD:"))
            .expect("pwd line");
        assert_ne!(
            pwd_line.trim_start_matches("PWD:"),
            working_dir.display().to_string()
        );
        assert!(execution
            .stdout
            .contains(&format!("INCLUDE:{}", working_dir.display())));
    }

    #[tokio::test]
    async fn gemini_runner_isolated_discovery_falls_back_to_minimal_on_auth_error() {
        let dir = tempdir().expect("tempdir");
        let script_path = dir.path().join("fake-gemini-isolated-fallback.sh");
        let script = r#"#!/bin/sh
set -eu
if echo "${HOME:-}" | grep -q "mcp-subagent-gemini-discovery"; then
  echo "authentication required in isolated profile" >&2
  exit 42
fi
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": [],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
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

        let working_dir = dir.path().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace");
        let runner = GeminiRunner::new(script_path);
        let execution = runner
            .execute(
                &sample_spec(30, NativeDiscoveryPolicy::Isolated),
                &sample_request(working_dir),
                &CompiledContext {
                    system_prefix: "sys".to_string(),
                    injected_prompt: "prompt".to_string(),
                    source_manifest: Vec::new(),
                },
            )
            .await
            .expect("execute");

        assert_eq!(execution.terminal_state, RunnerTerminalState::Succeeded);
        assert!(execution.stderr.contains("retried with minimal discovery"));
    }
}
