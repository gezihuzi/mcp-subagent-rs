use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool_handler, ErrorData, ServerHandler, ServiceExt,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{
    error::McpSubagentError,
    mcp::helpers::{resolve_effective_run_mode, resolve_preferred_run_mode, run_mode_label},
    mcp::persistence::{load_run_record_from_disk, persist_run_record},
    mcp::state::{build_execution_policy_snapshot, PolicySnapshot, RunRecord, RuntimeState},
    mcp::tools::build_tool_router,
    probe::{ProviderProbe, ProviderProber, SystemProviderProber},
    runtime::workspace::resolve_source_path,
    spec::{
        registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
        runtime_policy::{FileConflictPolicy, SandboxPolicy},
        Provider,
    },
    types::{RunMode, SelectedFile, TaskSpec, WorkflowHints},
};

pub use crate::mcp::dto::{
    AgentListing, ArtifactOutput, CancelAgentOutput, CodexInput, CodexOutput, GetAgentStatsInput,
    GetAgentStatsOutput, GetRunResultInput, HandleInput, ListAgentsOutput, ListRunsInput,
    ListRunsOutput, OutcomeView, ReadAgentArtifactInput, ReadAgentArtifactOutput, ReadRunLogsInput,
    ReadRunLogsOutput, RunAgentInput, RunAgentSelectedFileInput, RunEventOutput, RunUsageOutput,
    RunView, RuntimePolicySummary, WatchAgentEventsInput, WatchAgentEventsOutput, WatchRunInput,
    WatchRunOutput,
};

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpSubagentServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("mcp-subagent MCP server")
    }
}

#[derive(Debug, Clone)]
pub struct McpSubagentServer {
    tool_router: ToolRouter<Self>,
    agents_dirs: Vec<PathBuf>,
    state_dir: PathBuf,
    provider_prober: Arc<dyn ProviderProber>,
    runtime_state: Arc<Mutex<RuntimeState>>,
}

impl McpSubagentServer {
    pub fn new(agents_dirs: Vec<PathBuf>) -> Self {
        Self::new_with_state_dir_and_prober(
            agents_dirs,
            default_state_dir(),
            Arc::new(SystemProviderProber),
        )
    }

    pub fn new_with_state_dir(agents_dirs: Vec<PathBuf>, state_dir: PathBuf) -> Self {
        Self::new_with_state_dir_and_prober(agents_dirs, state_dir, Arc::new(SystemProviderProber))
    }

    pub fn new_with_state_dir_and_prober(
        agents_dirs: Vec<PathBuf>,
        state_dir: PathBuf,
        provider_prober: Arc<dyn ProviderProber>,
    ) -> Self {
        Self {
            tool_router: build_tool_router(),
            agents_dirs,
            state_dir,
            provider_prober,
            runtime_state: Arc::new(Mutex::new(RuntimeState::default())),
        }
    }

    pub async fn serve_stdio(self) -> std::result::Result<(), McpSubagentError> {
        let server = self
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|err| McpSubagentError::McpServer(err.to_string()))?;
        let _ = server
            .waiting()
            .await
            .map_err(|err| McpSubagentError::McpServer(err.to_string()))?;
        Ok(())
    }

    pub(crate) fn load_specs(&self) -> std::result::Result<Vec<LoadedAgentSpec>, ErrorData> {
        load_agent_specs_from_dirs(&self.agents_dirs)
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))
    }

    pub(crate) fn prepare_run(
        &self,
        input: RunAgentInput,
        requested_run_mode: RunMode,
    ) -> std::result::Result<(LoadedAgentSpec, TaskSpec, WorkflowHints, PolicySnapshot), ErrorData>
    {
        let specs = self.load_specs()?;
        let mut loaded = specs
            .into_iter()
            .find(|item| item.spec.core.name == input.agent_name)
            .ok_or_else(|| {
                ErrorData::resource_not_found(
                    format!("agent not found: {}", input.agent_name),
                    None,
                )
            })?;

        if let Some(policy) = input.working_dir_policy_override.clone() {
            loaded.spec.runtime.working_dir_policy = policy;
        }
        let (preferred_run_mode, preferred_run_mode_source) =
            resolve_preferred_run_mode(&loaded.spec);
        let (effective_run_mode, run_mode_source, should_reject_mode_mismatch) =
            resolve_effective_run_mode(
                requested_run_mode.clone(),
                preferred_run_mode.clone(),
                preferred_run_mode_source,
            );
        let execution_policy = build_execution_policy_snapshot(
            &loaded.spec,
            requested_run_mode.clone(),
            effective_run_mode.clone(),
            run_mode_source,
        );
        if should_reject_mode_mismatch {
            let tool_hint = match effective_run_mode {
                RunMode::Sync => "run_agent",
                RunMode::Async => "spawn_agent",
            };
            return Err(ErrorData::invalid_params(
                format!(
                    "agent `{}` execution mode resolved to `{}` by runtime policy; use `{tool_hint}` instead",
                    loaded.spec.core.name,
                    run_mode_label(&effective_run_mode),
                ),
                None,
            ));
        }

        let task_spec = TaskSpec {
            task: input.task,
            task_brief: input.task_brief,
            acceptance_criteria: vec![
                "Return sentinel-wrapped ProviderSummary JSON.".to_string(),
                "Keep findings concise and actionable.".to_string(),
            ],
            selected_files: input
                .selected_files
                .into_iter()
                .map(|file| SelectedFile {
                    path: PathBuf::from(file.path),
                    rationale: file.rationale,
                    content: file.content,
                })
                .collect(),
            working_dir: input
                .working_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        let hints = WorkflowHints {
            stage: input.stage,
            plan_ref: input.plan_ref,
            parent_summary: input.parent_summary,
            run_mode: effective_run_mode,
        };

        Ok((loaded, task_spec, hints, execution_policy))
    }

    pub(crate) fn probe_provider(&self, provider: &Provider) -> ProviderProbe {
        self.provider_prober.probe(provider)
    }

    pub(crate) fn ensure_provider_ready(
        &self,
        provider: &Provider,
    ) -> std::result::Result<ProviderProbe, ErrorData> {
        let probe = self.probe_provider(provider);
        if probe.is_available() {
            return Ok(probe);
        }

        let mut details = Vec::new();
        details.push(format!("status={}", probe.status));
        if let Some(version) = &probe.version {
            details.push(format!("version={version}"));
        }
        details.extend(probe.notes);

        Err(ErrorData::invalid_params(
            provider_unavailable_message(provider, &details),
            None,
        ))
    }

    pub(crate) async fn get_or_load_run_record(
        &self,
        handle_id: &str,
    ) -> std::result::Result<RunRecord, ErrorData> {
        {
            let state = self.runtime_state.lock().await;
            if let Some(record) = state.runs.get(handle_id) {
                return Ok(record.clone());
            }
        }

        let loaded = load_run_record_from_disk(&self.state_dir, handle_id)?;
        let Some(record) = loaded else {
            return Err(ErrorData::resource_not_found(
                format!("handle not found: {handle_id}"),
                None,
            ));
        };

        let mut state = self.runtime_state.lock().await;
        state.runs.insert(handle_id.to_string(), record.clone());
        Ok(record)
    }

    /// Wait for an in-progress async run task to complete.
    /// This is used by the CLI `spawn` command so the process doesn't exit
    /// before the background task has a chance to persist its results.
    pub async fn wait_for_run(&self, handle_id: &str) {
        let task = {
            let mut state = self.runtime_state.lock().await;
            state.tasks.remove(handle_id)
        };
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub(crate) async fn upsert_and_persist_run(
        &self,
        handle_id: &str,
        record: RunRecord,
    ) -> std::result::Result<(), ErrorData> {
        {
            let mut state = self.runtime_state.lock().await;
            state.runs.insert(handle_id.to_string(), record.clone());
        }
        persist_run_record(&self.state_dir, handle_id, &record)
    }

    pub(crate) fn conflict_lock_keys(
        &self,
        spec: &crate::spec::AgentSpec,
        task_spec: &TaskSpec,
    ) -> std::result::Result<Vec<String>, ErrorData> {
        if !matches!(
            spec.runtime.file_conflict_policy,
            FileConflictPolicy::Serialize
        ) || matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        {
            return Ok(Vec::new());
        }
        let source = resolve_source_path(&task_spec.working_dir)
            .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
        let source_key = source.display().to_string();
        if task_spec.selected_files.is_empty() {
            return Ok(vec![format!("{source_key}::__workspace__")]);
        }

        let mut keys = task_spec
            .selected_files
            .iter()
            .map(|selected| {
                let scoped_path = if selected.path.is_absolute() {
                    selected
                        .path
                        .strip_prefix(&source)
                        .unwrap_or(&selected.path)
                } else {
                    selected.path.as_path()
                };
                let scope = scoped_path
                    .components()
                    .find_map(|component| match component {
                        Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "__workspace__".to_string());
                format!("{source_key}::{scope}")
            })
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    pub(crate) async fn acquire_serialize_locks(
        &self,
        lock_keys: Vec<String>,
    ) -> Vec<OwnedMutexGuard<()>> {
        acquire_serialize_locks_from_state(&self.runtime_state, lock_keys).await
    }

    pub(crate) fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub(crate) fn runtime_state(&self) -> Arc<Mutex<RuntimeState>> {
        Arc::clone(&self.runtime_state)
    }

    pub(crate) fn provider_prober(&self) -> Arc<dyn ProviderProber> {
        Arc::clone(&self.provider_prober)
    }
}

pub(crate) fn provider_unavailable_message(provider: &Provider, details: &[String]) -> String {
    format!(
        "provider `{}` is unavailable ({})",
        provider.as_str(),
        details.join("; ")
    )
}

pub(crate) async fn acquire_serialize_locks_from_state(
    state: &Arc<Mutex<RuntimeState>>,
    mut lock_keys: Vec<String>,
) -> Vec<OwnedMutexGuard<()>> {
    if lock_keys.is_empty() {
        return Vec::new();
    }
    lock_keys.sort();
    lock_keys.dedup();

    let mut guards = Vec::with_capacity(lock_keys.len());
    for key in lock_keys {
        let lock = {
            let mut guard = state.lock().await;
            guard
                .serialize_locks
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        guards.push(lock.lock_owned().await);
    }
    guards
}

fn default_state_dir() -> PathBuf {
    PathBuf::from(".mcp-subagent/state")
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{
        collections::HashMap,
        env,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::{Duration, Instant},
    };

    use rmcp::{
        model::{CallToolRequestParams, ClientInfo},
        ClientHandler, ServiceExt,
    };
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    use crate::{
        probe::{ProbeStatus, ProviderCapabilities, ProviderProbe, ProviderProber},
        runtime::outcome::{RunOutcome, SuccessOutcome, UsageStats},
        runtime::summary::{ArtifactKind, ArtifactRef, SummaryParseStatus, VerificationStatus},
        spec::{runtime_policy::WorkingDirPolicy, Provider},
    };

    use super::{
        acquire_serialize_locks_from_state, HandleInput, McpSubagentServer, ReadAgentArtifactInput,
        RunAgentInput, RunAgentSelectedFileInput,
    };
    use crate::{mcp::artifacts::build_runtime_artifacts, mcp::state::RuntimeState};

    fn write_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "mock", "");
    }

    fn write_codex_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "codex", "");
    }

    fn write_gemini_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "gemini", "");
    }

    fn write_agent_spec_with_provider(dir: &Path, provider: &str) {
        write_agent_spec_with_provider_and_runtime(dir, provider, "");
    }

    fn write_agent_spec_with_provider_and_runtime(dir: &Path, provider: &str, runtime_extra: &str) {
        let agent = format!(
            r#"
[core]
name = "reviewer"
description = "review code"
provider = "{provider}"
instructions = "review"

[runtime]
working_dir_policy = "in_place"
sandbox = "read_only"
{runtime_extra}
"#
        );
        fs::write(dir.join("reviewer.agent.toml"), agent).expect("write agent");
    }

    fn write_named_agent_spec_with_provider_and_runtime(
        dir: &Path,
        name: &str,
        provider: &str,
        runtime_extra: &str,
    ) {
        let agent = format!(
            r#"
[core]
name = "{name}"
description = "review code"
provider = "{provider}"
instructions = "review"

[runtime]
working_dir_policy = "in_place"
sandbox = "read_only"
{runtime_extra}
"#
        );
        fs::write(dir.join(format!("{name}.agent.toml")), agent).expect("write named agent");
    }

    #[cfg(unix)]
    fn write_executable_script(path: &Path, script: &str) {
        fs::write(path, script).expect("write script");
        let mut perms = fs::metadata(path).expect("script metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod script");
    }

    struct EnvVarGuard {
        key: String,
        old: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &str, value: &Path) -> Self {
            let old = env::var_os(key);
            // Safety: tests intentionally override process env for provider binary resolution.
            unsafe { env::set_var(key, value) };
            Self {
                key: key.to_string(),
                old,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(old) = self.old.as_ref() {
                // Safety: restoring previous test-local env snapshot.
                unsafe { env::set_var(&self.key, old) };
            } else {
                // Safety: restoring by removing test-local env override.
                unsafe { env::remove_var(&self.key) };
            }
        }
    }

    #[derive(Debug, Clone)]
    struct TestProviderProber {
        probes: HashMap<Provider, ProviderProbe>,
        delays: HashMap<Provider, Duration>,
    }

    impl TestProviderProber {
        fn ready() -> Self {
            Self {
                probes: HashMap::new(),
                delays: HashMap::new(),
            }
        }

        fn with_status(mut self, provider: Provider, status: ProbeStatus, note: &str) -> Self {
            self.probes.insert(
                provider.clone(),
                ProviderProbe {
                    provider: provider.clone(),
                    executable: provider_binary(&provider),
                    version: Some("test-version".to_string()),
                    status,
                    capabilities: provider_capabilities(&provider),
                    validated_flags: provider_validated_flags(&provider),
                    notes: vec![note.to_string()],
                },
            );
            self
        }

        fn with_delay(mut self, provider: Provider, delay: Duration) -> Self {
            self.delays.insert(provider, delay);
            self
        }
    }

    impl ProviderProber for TestProviderProber {
        fn probe(&self, provider: &Provider) -> ProviderProbe {
            if let Some(delay) = self.delays.get(provider) {
                std::thread::sleep(*delay);
            }
            self.probes
                .get(provider)
                .cloned()
                .unwrap_or_else(|| ProviderProbe {
                    provider: provider.clone(),
                    executable: provider_binary(provider),
                    version: Some("test-version".to_string()),
                    status: ProbeStatus::Ready,
                    capabilities: provider_capabilities(provider),
                    validated_flags: provider_validated_flags(provider),
                    notes: Vec::new(),
                })
        }
    }

    fn provider_binary(provider: &Provider) -> std::path::PathBuf {
        match provider {
            Provider::Mock => "mock",
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Gemini => "gemini",
            Provider::Ollama => "ollama",
        }
        .into()
    }

    fn provider_capabilities(provider: &Provider) -> ProviderCapabilities {
        match provider {
            Provider::Mock => ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: false,
                experimental: false,
            },
            Provider::Claude => ProviderCapabilities {
                supports_background_native: true,
                supports_native_project_memory: true,
                experimental: false,
            },
            Provider::Codex => ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: true,
                experimental: false,
            },
            Provider::Gemini => ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: true,
                experimental: true,
            },
            Provider::Ollama => ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: false,
                experimental: false,
            },
        }
    }

    fn provider_validated_flags(provider: &Provider) -> Vec<String> {
        match provider {
            Provider::Mock => Vec::new(),
            Provider::Claude => vec![
                "--permission-mode".to_string(),
                "--add-dir".to_string(),
                "--output-format".to_string(),
                "--json-schema".to_string(),
            ],
            Provider::Codex => vec![
                "--sandbox".to_string(),
                "--ask-for-approval".to_string(),
                "--output-last-message".to_string(),
                "--output-schema".to_string(),
            ],
            Provider::Gemini => vec![
                "--approval-mode".to_string(),
                "--include-directories".to_string(),
                "--output-format".to_string(),
            ],
            Provider::Ollama => Vec::new(),
        }
    }

    fn make_server(
        agents_dir: std::path::PathBuf,
        state_dir: std::path::PathBuf,
    ) -> McpSubagentServer {
        McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(TestProviderProber::ready()),
        )
    }

    #[tokio::test]
    async fn list_agents_tool_returns_agent() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);
        let server = make_server(agents_dir, state_dir);

        let out = server.list_agents().await.expect("list").0;
        assert_eq!(out.agents.len(), 1);
        assert_eq!(out.agents[0].name, "reviewer");
    }

    #[tokio::test]
    async fn list_agents_marks_provider_unavailable() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_gemini_agent_spec(&agents_dir);

        let server = McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(TestProviderProber::ready().with_status(
                Provider::Gemini,
                ProbeStatus::MissingBinary,
                "gemini CLI not installed",
            )),
        );

        let out = server.list_agents().await.expect("list").0;
        assert_eq!(out.agents.len(), 1);
        assert!(!out.agents[0].available);
        assert!(out.agents[0]
            .capability_notes
            .iter()
            .any(|note| note.contains("missing_binary")));
    }

    #[tokio::test]
    async fn list_agents_marks_ollama_available_when_probe_ready() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider(&agents_dir, "ollama");
        let server = make_server(agents_dir, state_dir);

        let out = server.list_agents().await.expect("list").0;
        assert_eq!(out.agents.len(), 1);
        assert!(out.agents[0].available);
        assert!(out.agents[0]
            .capability_notes
            .iter()
            .any(|note| note.contains("provider_tier: local")));
    }

    #[tokio::test]
    async fn run_agent_tool_returns_structured_summary() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);
        let server = make_server(agents_dir, state_dir);

        let input = RunAgentInput {
            agent_name: "reviewer".to_string(),
            task: "review parser".to_string(),
            task_brief: Some("review parser".to_string()),
            parent_summary: None,
            selected_files: vec![RunAgentSelectedFileInput {
                path: "src/parser.rs".to_string(),
                rationale: Some("hotspot".to_string()),
                content: None,
            }],
            stage: None,
            plan_ref: None,
            working_dir: None,
            working_dir_policy_override: None,
        };
        let out = server
            .run_agent(rmcp::handler::server::wrapper::Parameters(input))
            .await
            .expect("run")
            .0;

        assert_eq!(out.phase, "succeeded");
        assert!(out.terminal);
        match out.outcome {
            Some(super::OutcomeView::Succeeded { summary, .. }) => {
                assert!(summary.contains("Mock run completed"));
            }
            other => panic!("expected succeeded outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_agent_applies_working_dir_policy_override() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);
        let server = make_server(agents_dir, state_dir.clone());

        let out = server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "override workspace policy".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: Some(WorkingDirPolicy::Direct),
            }))
            .await
            .expect("run")
            .0;
        assert!(out.terminal);

        let run_json =
            fs::read_to_string(state_dir.join("runs").join(&out.handle_id).join("run.json"))
                .expect("read run json");
        let run_obj: serde_json::Value = serde_json::from_str(&run_json).expect("parse run json");
        assert_eq!(run_obj["spec_snapshot"]["working_dir_policy"], "direct");
    }

    #[tokio::test]
    async fn run_agent_rejects_unavailable_provider() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_codex_agent_spec(&agents_dir);

        let server = McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(TestProviderProber::ready().with_status(
                Provider::Codex,
                ProbeStatus::MissingBinary,
                "codex CLI not installed",
            )),
        );

        let err = match server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "review parser".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
        {
            Ok(_) => panic!("run should fail when provider is unavailable"),
            Err(err) => err,
        };
        assert!(err
            .message
            .as_ref()
            .contains("provider `codex` is unavailable"));
    }

    #[tokio::test]
    async fn spawn_agent_returns_before_slow_probe_completes() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);

        let server = McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(
                TestProviderProber::ready().with_delay(Provider::Mock, Duration::from_millis(900)),
            ),
        );

        let started = Instant::now();
        let spawn = server
            .spawn_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "slow probe".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
            .expect("spawn")
            .0;
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(300),
            "spawn took {:?}, expected < 300ms",
            elapsed
        );
        assert_eq!(spawn.phase, "accepted");
        assert!(!spawn.terminal);
        assert!(spawn.outcome.is_none());
        assert!(!spawn.created_at.is_empty());

        let status = server
            .get_agent_status(rmcp::handler::server::wrapper::Parameters(HandleInput {
                handle_id: spawn.handle_id.clone(),
            }))
            .await
            .expect("status")
            .0;
        assert!(!status.terminal);

        server.wait_for_run(&spawn.handle_id).await;
        let finished = server
            .get_agent_status(rmcp::handler::server::wrapper::Parameters(HandleInput {
                handle_id: spawn.handle_id,
            }))
            .await
            .expect("final status")
            .0;
        assert!(finished.terminal);
        assert!(matches!(
            finished.outcome,
            Some(super::OutcomeView::Succeeded { .. })
        ));
    }

    #[tokio::test]
    async fn spawn_agent_accepts_then_fails_when_provider_unavailable() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_codex_agent_spec(&agents_dir);

        let server = McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(TestProviderProber::ready().with_status(
                Provider::Codex,
                ProbeStatus::MissingBinary,
                "codex CLI not installed",
            )),
        );

        let spawn = server
            .spawn_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "should fail async".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
            .expect("spawn")
            .0;
        assert_eq!(spawn.phase, "accepted");
        assert!(!spawn.terminal);
        assert!(spawn.outcome.is_none());
        assert!(!spawn.created_at.is_empty());

        server.wait_for_run(&spawn.handle_id).await;

        let status = server
            .get_agent_status(rmcp::handler::server::wrapper::Parameters(HandleInput {
                handle_id: spawn.handle_id,
            }))
            .await
            .expect("status")
            .0;
        assert!(status.terminal);
        match status.outcome {
            Some(super::OutcomeView::Failed { error, .. }) => {
                assert!(error.contains("provider `codex` is unavailable"));
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn watch_agent_events_surfaces_runtime_delta_for_gemini_and_claude() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&agents_dir).expect("create agents");
        fs::create_dir_all(&workspace_dir).expect("create workspace");

        write_named_agent_spec_with_provider_and_runtime(
            &agents_dir,
            "gemini-research",
            "gemini",
            r#"timeout_secs = 10"#,
        );
        write_named_agent_spec_with_provider_and_runtime(
            &agents_dir,
            "claude-review",
            "claude",
            r#"timeout_secs = 10"#,
        );

        let gemini_script_path = temp.path().join("fake-gemini-stream.sh");
        let claude_script_path = temp.path().join("fake-claude-stream.sh");
        let script = r#"#!/bin/sh
set -eu
echo "chunk-stdout-1"
echo "chunk-stderr-1" >&2
sleep 1
cat <<'EOF'
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": [],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": ["src/lib.rs"]
}
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
EOF
exit 0
"#;
        write_executable_script(&gemini_script_path, script);
        write_executable_script(&claude_script_path, script);

        let _gemini_env = EnvVarGuard::set_path("MCP_SUBAGENT_GEMINI_BIN", &gemini_script_path);
        let _claude_env = EnvVarGuard::set_path("MCP_SUBAGENT_CLAUDE_BIN", &claude_script_path);

        let server = make_server(agents_dir, state_dir);

        for agent_name in ["gemini-research", "claude-review"] {
            let spawn = server
                .spawn_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                    agent_name: agent_name.to_string(),
                    task: "stream delta probe".to_string(),
                    task_brief: None,
                    parent_summary: None,
                    selected_files: Vec::new(),
                    stage: None,
                    plan_ref: None,
                    working_dir: Some(workspace_dir.display().to_string()),
                    working_dir_policy_override: None,
                }))
                .await
                .expect("spawn")
                .0;
            assert_eq!(spawn.phase, "accepted");

            let started = Instant::now();
            let mut terminal_observed = false;

            for _ in 0..150 {
                let events = server
                    .watch_agent_events(rmcp::handler::server::wrapper::Parameters(
                        crate::mcp::dto::WatchAgentEventsInput {
                            handle_id: spawn.handle_id.clone(),
                            since_seq: Some(0),
                            limit: Some(200),
                            phase: None,
                            phase_timeout_secs: None,
                        },
                    ))
                    .await
                    .expect("watch events")
                    .0;
                if events.terminal {
                    terminal_observed = true;
                }
                if terminal_observed && started.elapsed() >= Duration::from_millis(120) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }

            server.wait_for_run(&spawn.handle_id).await;
            let final_status = server
                .get_agent_status(rmcp::handler::server::wrapper::Parameters(HandleInput {
                    handle_id: spawn.handle_id.clone(),
                }))
                .await
                .expect("final status")
                .0;
            assert!(final_status.terminal);
            assert!(matches!(
                final_status.outcome,
                Some(super::OutcomeView::Succeeded { .. })
            ));

            let final_events = server
                .watch_agent_events(rmcp::handler::server::wrapper::Parameters(
                    crate::mcp::dto::WatchAgentEventsInput {
                        handle_id: spawn.handle_id.clone(),
                        since_seq: Some(0),
                        limit: Some(300),
                        phase: None,
                        phase_timeout_secs: None,
                    },
                ))
                .await
                .expect("final watch events")
                .0;
            let final_event_names = final_events
                .events
                .iter()
                .map(|event| event.event.as_str())
                .collect::<Vec<_>>();
            assert!(
                final_event_names.contains(&"provider.first_output"),
                "expected provider.first_output for {agent_name}; events={final_event_names:?}"
            );
            let first_delta = final_events
                .events
                .iter()
                .find(|event| {
                    event.event == "provider.stdout.delta" || event.event == "provider.stderr.delta"
                })
                .unwrap_or_else(|| {
                    panic!(
                        "expected provider delta event for {agent_name}; events={final_event_names:?}"
                    )
                });
            let delta_at = time::OffsetDateTime::parse(
                &first_delta.timestamp,
                &time::format_description::well_known::Rfc3339,
            )
            .expect("parse first delta timestamp");
            let completed_event = final_events
                .events
                .iter()
                .find(|event| event.event == "run.completed")
                .expect("run.completed event");
            let completed_at = time::OffsetDateTime::parse(
                &completed_event.timestamp,
                &time::format_description::well_known::Rfc3339,
            )
            .expect("parse run.completed timestamp");
            let delta_lead = if completed_at >= delta_at {
                Duration::from_millis((completed_at - delta_at).whole_milliseconds() as u64)
            } else {
                Duration::from_secs(0)
            };
            assert!(
                delta_lead >= Duration::from_millis(600),
                "expected runtime delta event to lead completion by >=600ms for {agent_name}, got {:?}",
                delta_lead
            );
        }
    }

    #[tokio::test]
    async fn run_agent_rejects_when_spawn_policy_requires_async() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider_and_runtime(
            &agents_dir,
            "mock",
            r#"spawn_policy = "async""#,
        );
        let server = make_server(agents_dir, state_dir);

        let err = match server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "review parser".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
        {
            Ok(_) => panic!("run should fail for async-only policy"),
            Err(err) => err,
        };

        assert!(err
            .message
            .as_ref()
            .contains("execution mode resolved to `async`"));
        assert!(err.message.as_ref().contains("use `spawn_agent`"));
    }

    #[tokio::test]
    async fn run_agent_rejects_when_background_prefers_async() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider_and_runtime(
            &agents_dir,
            "mock",
            r#"background_preference = "prefer_background""#,
        );
        let server = make_server(agents_dir, state_dir);

        let err = match server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "review parser".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
        {
            Ok(_) => panic!("run should fail for background async preference"),
            Err(err) => err,
        };

        assert!(err
            .message
            .as_ref()
            .contains("execution mode resolved to `async`"));
        assert!(err.message.as_ref().contains("use `spawn_agent`"));
    }

    #[tokio::test]
    async fn run_agent_rejects_unavailable_ollama_provider() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider(&agents_dir, "ollama");
        let server = McpSubagentServer::new_with_state_dir_and_prober(
            vec![agents_dir],
            state_dir,
            Arc::new(TestProviderProber::ready().with_status(
                Provider::Ollama,
                ProbeStatus::MissingBinary,
                "ollama CLI not installed",
            )),
        );

        let err = match server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "review parser".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
        {
            Ok(_) => panic!("run should fail when provider is unavailable"),
            Err(err) => err,
        };

        assert!(err
            .message
            .as_ref()
            .contains("provider `ollama` is unavailable"));
    }

    #[derive(Debug, Clone, Default)]
    struct DummyClient;

    impl ClientHandler for DummyClient {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    fn structured_field<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("missing field `{key}` in {value}"))
    }

    #[test]
    fn declared_workspace_artifacts_are_persisted_in_index_and_payloads() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("report.md"), "# report").expect("write report");

        let outcome = RunOutcome::Succeeded(SuccessOutcome {
            summary: "done".to_string(),
            key_findings: vec!["one".to_string()],
            artifacts: vec![ArtifactRef {
                path: PathBuf::from("report.md"),
                kind: ArtifactKind::ReportMarkdown,
                description: "markdown report".to_string(),
                media_type: Some("text/markdown".to_string()),
            }],
            open_questions: Vec::new(),
            next_steps: Vec::new(),
            verification: VerificationStatus::Passed,
            usage: UsageStats::ZERO,
            parse_status: SummaryParseStatus::Validated,
            touched_files: Vec::new(),
            plan_refs: Vec::new(),
        });

        let (index, payloads) = build_runtime_artifacts(&outcome, "", "", Some(temp.path()));
        assert!(index.iter().any(|item| item.path == "report.md"));
        assert_eq!(
            payloads.get("report.md").expect("report payload"),
            "# report"
        );
    }

    #[tokio::test]
    async fn mcp_transport_roundtrip_for_all_tools() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);

        let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
        let server = make_server(agents_dir, state_dir);
        let server_handle = tokio::spawn(async move {
            let running = server.serve(server_transport).await.expect("server init");
            let _ = running.waiting().await.expect("server wait");
        });

        let client = DummyClient
            .serve(client_transport)
            .await
            .expect("client init");

        let tools = client.list_all_tools().await.expect("list tools");
        for expected in [
            "list_agents",
            "list_runs",
            "codex",
            "run_agent",
            "spawn_agent",
            "get_agent_status",
            "get_run_result",
            "get_agent_stats",
            "watch_agent_events",
            "cancel_agent",
            "read_agent_artifact",
            "read_run_logs",
            "watch_run",
        ] {
            assert!(tools.iter().any(|tool| tool.name == expected));
        }

        let codex_res = client
            .call_tool(
                CallToolRequestParams::new("codex").with_arguments(
                    json!({
                        "agent_name": "reviewer",
                        "task": "codex-style review parser",
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("codex run");
        let codex_json = codex_res
            .structured_content
            .expect("codex has structured content");
        assert_eq!(
            codex_json
                .get("render_style")
                .and_then(|value| value.as_str()),
            Some("codex")
        );
        let codex_run = codex_json.get("run").expect("run output");
        assert_eq!(
            codex_run
                .get("outcome")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("succeeded")
        );
        assert!(codex_json
            .get("rendered")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.contains("P1")));

        let spawn_res = client
            .call_tool(
                CallToolRequestParams::new("spawn_agent").with_arguments(
                    json!({
                        "agent_name": "reviewer",
                        "task": "review parser",
                        "selected_files": [{"path": "src/parser.rs"}]
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("spawn");
        let spawn_json = spawn_res
            .structured_content
            .expect("spawn has structured content");
        assert_eq!(structured_field(&spawn_json, "phase"), "accepted");
        assert_eq!(
            spawn_json.get("terminal").and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(spawn_json
            .get("created_at")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.is_empty()));
        let handle_id = structured_field(&spawn_json, "handle_id").to_string();

        let mut final_status = String::new();
        for _ in 0..30 {
            let status_res = client
                .call_tool(
                    CallToolRequestParams::new("get_agent_status").with_arguments(
                        json!({"handle_id": handle_id})
                            .as_object()
                            .expect("object")
                            .clone(),
                    ),
                )
                .await
                .expect("status");
            let status_json = status_res
                .structured_content
                .expect("status has structured content");
            final_status = status_json
                .get("outcome")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if status_json
                .get("terminal")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        assert_eq!(final_status, "succeeded");

        let status_after_done = client
            .call_tool(
                CallToolRequestParams::new("get_agent_status").with_arguments(
                    json!({"handle_id": handle_id.clone()})
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
            )
            .await
            .expect("status after done");
        let status_after_done_json = status_after_done
            .structured_content
            .expect("status after done structured");
        assert_eq!(
            status_after_done_json
                .get("terminal")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(status_after_done_json.get("outcome").is_some());
        assert_eq!(
            status_after_done_json
                .get("outcome")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("succeeded")
        );

        let artifact_res = client
            .call_tool(
                CallToolRequestParams::new("read_agent_artifact").with_arguments(
                    json!({
                        "handle_id": handle_id,
                        "path": "summary.json"
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("read artifact");
        let artifact_json = artifact_res
            .structured_content
            .expect("artifact has structured content");
        assert!(structured_field(&artifact_json, "content").contains("Mock run completed"));

        let list_runs_res = client
            .call_tool(
                CallToolRequestParams::new("list_runs")
                    .with_arguments(json!({ "limit": 5 }).as_object().expect("object").clone()),
            )
            .await
            .expect("list runs");
        let list_runs_json = list_runs_res
            .structured_content
            .expect("list_runs has structured content");
        let run_rows = list_runs_json
            .get("runs")
            .and_then(|value| value.as_array())
            .expect("runs array");
        assert!(run_rows
            .iter()
            .any(|row| row.get("handle_id").and_then(|value| value.as_str())
                == Some(handle_id.as_str())));
        let listed = run_rows
            .iter()
            .find(|row| {
                row.get("handle_id").and_then(|value| value.as_str()) == Some(handle_id.as_str())
            })
            .expect("listed run row");
        assert!(listed.get("phase").is_some());
        assert!(listed.get("terminal").is_some());
        assert!(listed.get("outcome").is_some());

        let result_res = client
            .call_tool(
                CallToolRequestParams::new("get_run_result").with_arguments(
                    json!({ "handle_id": handle_id.clone() })
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
            )
            .await
            .expect("get run result");
        let result_json = result_res
            .structured_content
            .expect("get_run_result has structured content");
        assert_eq!(
            result_json
                .get("terminal")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        let outcome_json = result_json.get("outcome").expect("outcome");
        assert_eq!(structured_field(outcome_json, "status"), "succeeded");
        assert!(structured_field(outcome_json, "summary").contains("Mock run completed"));

        let logs_res = client
            .call_tool(
                CallToolRequestParams::new("read_run_logs").with_arguments(
                    json!({ "handle_id": handle_id.clone(), "stream": "stdout" })
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
            )
            .await
            .expect("read run logs");
        let logs_json = logs_res
            .structured_content
            .expect("read_run_logs has structured content");
        assert!(logs_json.get("stdout").is_some());
        assert!(logs_json.get("stderr").is_some());

        let watch_res = client
            .call_tool(
                CallToolRequestParams::new("watch_run").with_arguments(
                    json!({
                        "handle_id": handle_id.clone(),
                        "timeout_secs": 1,
                        "phase": "completed",
                        "phase_timeout_secs": 1
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("watch run");
        let watch_json = watch_res
            .structured_content
            .expect("watch_run has structured content");
        let watch_run_json = watch_json.get("run").expect("run");
        assert_eq!(
            watch_run_json
                .get("terminal")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            watch_run_json
                .get("outcome")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("succeeded")
        );
        assert_eq!(
            watch_json
                .get("timed_out")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(watch_json.get("phase_timeout_hit").is_some());
        assert!(watch_json.get("block_reason").is_some());
        assert!(watch_json.get("advice").is_some());

        let stats_res = client
            .call_tool(
                CallToolRequestParams::new("get_agent_stats").with_arguments(
                    json!({ "handle_id": handle_id.clone() })
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
            )
            .await
            .expect("get stats");
        let stats_json = stats_res
            .structured_content
            .expect("get_agent_stats has structured content");
        assert_eq!(structured_field(&stats_json, "status"), "succeeded");
        assert!(stats_json.get("usage").is_some());
        assert!(stats_json.get("wall_ms").is_some());
        assert!(stats_json.get("block_reason").is_some());
        assert!(stats_json.get("advice").is_some());
        assert!(stats_json.get("workspace_prepare_ms").is_some());
        assert!(stats_json.get("provider_boot_ms").is_some());
        assert!(stats_json.get("wait_reasons").is_some());

        let events_res = client
            .call_tool(
                CallToolRequestParams::new("watch_agent_events").with_arguments(
                    json!({
                        "handle_id": handle_id.clone(),
                        "since_seq": 0,
                        "limit": 50,
                        "phase": "completed",
                        "phase_timeout_secs": 1
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("watch events");
        let events_json = events_res
            .structured_content
            .expect("watch_agent_events has structured content");
        assert_eq!(structured_field(&events_json, "status"), "succeeded");
        let events = events_json
            .get("events")
            .and_then(|value| value.as_array())
            .expect("events array");
        assert!(!events.is_empty(), "expected incremental events");
        assert!(events_json.get("current_phase").is_some());
        assert!(events_json.get("current_phase_age_ms").is_some());
        assert!(events_json.get("phase_timeout_hit").is_some());
        assert!(events_json.get("block_reason").is_some());
        assert!(events_json.get("advice").is_some());

        let all_events_res = client
            .call_tool(
                CallToolRequestParams::new("watch_agent_events").with_arguments(
                    json!({
                        "handle_id": handle_id.clone(),
                        "since_seq": 0,
                        "limit": 300
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("watch all events");
        let all_events_json = all_events_res
            .structured_content
            .expect("watch all events structured");
        let all_event_names = all_events_json
            .get("events")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| entry.get("event").and_then(|value| value.as_str()))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for expected in [
            "workspace.prepare.completed",
            "context.compile.started",
            "context.compile.completed",
            "parse.started",
            "parse.completed",
        ] {
            assert!(
                all_event_names.iter().any(|event| event == expected),
                "expected event `{expected}`, got {all_event_names:?}"
            );
        }
        let context_started = all_event_names
            .iter()
            .position(|name| name == "context.compile.started")
            .expect("context.compile.started");
        let context_completed = all_event_names
            .iter()
            .position(|name| name == "context.compile.completed")
            .expect("context.compile.completed");
        let parse_started = all_event_names
            .iter()
            .position(|name| name == "parse.started")
            .expect("parse.started");
        let parse_completed = all_event_names
            .iter()
            .position(|name| name == "parse.completed")
            .expect("parse.completed");
        let completed = all_event_names
            .iter()
            .position(|name| name == "run.completed")
            .expect("run.completed");
        assert!(context_started < context_completed);
        assert!(context_completed < parse_started);
        assert!(parse_started < parse_completed);
        assert!(parse_completed < completed);

        let second_spawn_res = client
            .call_tool(
                CallToolRequestParams::new("spawn_agent").with_arguments(
                    json!({
                        "agent_name": "reviewer",
                        "task": "cancel me",
                        "selected_files": []
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("spawn second");
        let second_handle = structured_field(
            &second_spawn_res
                .structured_content
                .expect("spawn2 structured"),
            "handle_id",
        )
        .to_string();

        let cancel_res = client
            .call_tool(
                CallToolRequestParams::new("cancel_agent").with_arguments(
                    json!({"handle_id": second_handle})
                        .as_object()
                        .expect("object")
                        .clone(),
                ),
            )
            .await
            .expect("cancel");
        let cancel_json = cancel_res
            .structured_content
            .expect("cancel structured content");
        assert_eq!(structured_field(&cancel_json, "status"), "cancelled");
        let cancelled_events = client
            .call_tool(
                CallToolRequestParams::new("watch_agent_events").with_arguments(
                    json!({
                        "handle_id": second_handle,
                        "since_seq": 0,
                        "limit": 50
                    })
                    .as_object()
                    .expect("object")
                    .clone(),
                ),
            )
            .await
            .expect("watch cancelled events");
        let cancelled_events_json = cancelled_events
            .structured_content
            .expect("watch cancelled structured");
        let cancelled_event_names = cancelled_events_json
            .get("events")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| entry.get("event").and_then(|value| value.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(
            cancelled_event_names.contains(&"run.cancelled"),
            "expected run.cancelled event, got {cancelled_event_names:?}"
        );

        client.cancel().await.expect("cancel client");
        server_handle.await.expect("server join");
    }

    #[tokio::test]
    async fn restart_can_query_persisted_runs_and_reject_invalid_path() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec(&agents_dir);

        let server = make_server(agents_dir.clone(), state_dir.clone());
        let run_out = server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "reviewer".to_string(),
                task: "persist me".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: vec![RunAgentSelectedFileInput {
                    path: "src/lib.rs".to_string(),
                    rationale: None,
                    content: None,
                }],
                stage: None,
                plan_ref: None,
                working_dir: None,
                working_dir_policy_override: None,
            }))
            .await
            .expect("run")
            .0;
        let handle_id = run_out.handle_id;
        drop(server);

        let restarted = make_server(agents_dir, state_dir);
        let status = restarted
            .get_agent_status(rmcp::handler::server::wrapper::Parameters(HandleInput {
                handle_id: handle_id.clone(),
            }))
            .await
            .expect("status")
            .0;
        assert!(status.terminal);
        assert!(matches!(
            status.outcome,
            Some(super::OutcomeView::Succeeded { .. })
        ));

        let artifact = restarted
            .read_agent_artifact(rmcp::handler::server::wrapper::Parameters(
                ReadAgentArtifactInput {
                    handle_id: handle_id.clone(),
                    path: "summary.json".to_string(),
                },
            ))
            .await
            .expect("read summary")
            .0;
        assert!(artifact.content.contains("Mock run completed"));

        let invalid = match restarted
            .read_agent_artifact(rmcp::handler::server::wrapper::Parameters(
                ReadAgentArtifactInput {
                    handle_id,
                    path: "../escape.txt".to_string(),
                },
            ))
            .await
        {
            Ok(_) => panic!("invalid path should fail"),
            Err(err) => err,
        };
        assert!(invalid.message.as_ref().contains("invalid artifact path"));
    }

    #[tokio::test]
    async fn run_agent_tempcopy_persists_workspace_metadata() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&agents_dir).expect("create agents");
        fs::create_dir_all(&project_dir).expect("create project");
        fs::write(project_dir.join("hello.txt"), "workspace source").expect("write source");

        let agent = r#"
[core]
name = "writer"
description = "write code"
provider = "mock"
instructions = "write"

[runtime]
working_dir_policy = "temp_copy"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
"#;
        fs::write(agents_dir.join("writer.agent.toml"), agent).expect("write agent");
        let server = make_server(agents_dir, state_dir.clone());

        let out = server
            .run_agent(rmcp::handler::server::wrapper::Parameters(RunAgentInput {
                agent_name: "writer".to_string(),
                task: "copy workspace".to_string(),
                task_brief: None,
                parent_summary: None,
                selected_files: Vec::new(),
                stage: None,
                plan_ref: None,
                working_dir: Some(project_dir.display().to_string()),
                working_dir_policy_override: None,
            }))
            .await
            .expect("run")
            .0;

        let run_json =
            fs::read_to_string(state_dir.join("runs").join(&out.handle_id).join("run.json"))
                .expect("read run json");
        let run_obj: serde_json::Value = serde_json::from_str(&run_json).expect("parse run json");
        let run_dir = state_dir.join("runs").join(&out.handle_id);
        assert!(!run_obj["state"]["created_at"].is_null());
        assert!(!run_obj["state"]["updated_at"].is_null());
        assert!(run_obj["state"]["status_history"].is_array());
        assert_eq!(run_obj["task_spec"]["task"], "copy workspace");
        assert_eq!(
            run_obj["task_spec"]["working_dir"],
            project_dir.display().to_string()
        );
        assert_eq!(run_obj["spec_snapshot"]["name"], "writer");
        assert_eq!(run_obj["spec_snapshot"]["working_dir_policy"], "temp_copy");
        assert_eq!(run_obj["state"]["probe_result"]["provider"], "mock");
        assert!(!run_obj["state"]["memory_resolution"].is_null());
        assert_eq!(run_obj["state"]["workspace"]["mode"], "temp_copy");
        assert!(!run_obj["state"]["policy"].is_null());
        assert_eq!(run_obj["state"]["policy"]["requested_run_mode"], "sync");
        assert_eq!(run_obj["state"]["policy"]["effective_run_mode"], "sync");
        assert_eq!(
            run_obj["state"]["policy"]["effective_run_mode_source"],
            "spec"
        );
        assert_eq!(run_obj["state"]["policy"]["retry_max_attempts"], 1);
        assert_eq!(run_obj["state"]["policy"]["retry_backoff_secs"], 1);
        assert_eq!(run_obj["state"]["policy"]["attempts_used"], 1);
        assert_eq!(run_obj["state"]["policy"]["retries_used"], 0);
        let workspace_path = run_obj["state"]["workspace"]["workspace_path"]
            .as_str()
            .expect("workspace path");
        assert!(
            !Path::new(workspace_path).exists(),
            "workspace should be cleaned after run completion"
        );
        assert!(run_obj["state"]["workspace"]["lock_key"]
            .as_str()
            .expect("lock key")
            .contains("project"));
        assert!(run_obj["state"]["workspace"]["lock_keys"].is_array());
        assert!(run_obj["state"]["workspace"]["lock_keys"]
            .as_array()
            .is_some_and(|keys| !keys.is_empty()));
        assert!(run_obj["state"]["compiled_context_markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("ROLE"));

        assert!(run_dir.join("run.json").exists());
        assert!(run_dir.join("compiled-context.md").exists());
        assert!(run_dir.join("events.jsonl").exists());
        assert!(run_dir.join("stdout.log").exists());
        assert!(run_dir.join("stderr.log").exists());

        assert!(!run_dir.join("request.json").exists());
        assert!(!run_dir.join("resolved-spec.json").exists());
        assert!(!run_dir.join("status.json").exists());
        assert!(!run_dir.join("summary.json").exists());
        assert!(!run_dir.join("summary.raw.txt").exists());
        assert!(!run_dir.join("workspace.meta.json").exists());
        assert!(!run_dir.join("events.ndjson").exists());

        let first = run_obj["artifact_index"]
            .as_array()
            .and_then(|items| items.first())
            .expect("artifact index item");
        assert!(first.get("kind").is_some());
        assert!(first.get("path").is_some());
        assert!(first.get("media_type").is_some());
        assert!(first.get("producer").is_some());
        assert!(first.get("created_at").is_some());
        assert!(first.get("description").is_some());

        let events_text =
            fs::read_to_string(run_dir.join("events.jsonl")).expect("read events file");
        let event_names = events_text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|event| {
                event
                    .get("event")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        for required in [
            "workspace.prepare.completed",
            "context.compile.started",
            "context.compile.completed",
            "parse.started",
            "parse.completed",
        ] {
            assert!(
                event_names.iter().any(|event| event == required),
                "missing event `{required}` in {event_names:?}"
            );
        }
    }

    #[tokio::test]
    async fn serialize_lock_blocks_until_guard_released() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let first_guard =
            acquire_serialize_locks_from_state(&state, vec!["repo-key::src".to_string()]).await;
        assert_eq!(first_guard.len(), 1, "first guard");

        let state_clone = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            let start = Instant::now();
            let guard =
                acquire_serialize_locks_from_state(&state_clone, vec!["repo-key::src".to_string()])
                    .await;
            let elapsed = start.elapsed();
            drop(guard);
            elapsed
        });

        tokio::time::sleep(Duration::from_millis(90)).await;
        assert!(
            !waiter.is_finished(),
            "second guard should still be waiting"
        );

        drop(first_guard);
        let elapsed = waiter.await.expect("waiter join");
        assert!(
            elapsed >= Duration::from_millis(80),
            "lock wait should be observable, elapsed={elapsed:?}"
        );
    }

    #[tokio::test]
    async fn serialize_lock_allows_non_conflicting_scopes() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let first_guard =
            acquire_serialize_locks_from_state(&state, vec!["repo-key::src".to_string()]).await;
        assert_eq!(first_guard.len(), 1);

        let state_clone = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            let start = Instant::now();
            let guard =
                acquire_serialize_locks_from_state(&state_clone, vec!["repo-key::web".to_string()])
                    .await;
            let elapsed = start.elapsed();
            drop(guard);
            elapsed
        });

        let elapsed = waiter.await.expect("waiter join");
        assert!(
            elapsed < Duration::from_millis(80),
            "non-conflicting lock should not block, elapsed={elapsed:?}"
        );
        drop(first_guard);
    }
}
