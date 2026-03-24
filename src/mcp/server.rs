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
    mcp::state::{build_execution_policy_snapshot, ExecutionPolicyRecord, RunRecord, RuntimeState},
    mcp::tools::build_tool_router,
    probe::{ProviderProbe, ProviderProber, SystemProviderProber},
    runtime::workspace::resolve_source_path,
    spec::{
        registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
        runtime_policy::{FileConflictPolicy, SandboxPolicy},
        Provider,
    },
    types::{RunMode, RunRequest, SelectedFile},
};

pub use crate::mcp::dto::{
    AgentListing, AgentStatusOutput, ArtifactOutput, CancelAgentOutput, HandleInput,
    ListAgentsOutput, ReadAgentArtifactInput, ReadAgentArtifactOutput, RunAgentInput,
    RunAgentOutput, RunAgentSelectedFileInput, RuntimePolicySummary, SpawnAgentOutput,
    SummaryOutput,
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
    ) -> std::result::Result<
        (
            LoadedAgentSpec,
            RunRequest,
            ProviderProbe,
            ExecutionPolicyRecord,
        ),
        ErrorData,
    > {
        let specs = self.load_specs()?;
        let loaded = specs
            .into_iter()
            .find(|item| item.spec.core.name == input.agent_name)
            .ok_or_else(|| {
                ErrorData::resource_not_found(
                    format!("agent not found: {}", input.agent_name),
                    None,
                )
            })?;
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

        let probe_result = self.ensure_provider_ready(&loaded.spec.core.provider)?;

        let request = RunRequest {
            task: input.task,
            task_brief: input.task_brief,
            parent_summary: input.parent_summary,
            selected_files: input
                .selected_files
                .into_iter()
                .map(|file| SelectedFile {
                    path: PathBuf::from(file.path),
                    rationale: file.rationale,
                    content: file.content,
                })
                .collect(),
            stage: input.stage,
            plan_ref: input.plan_ref,
            working_dir: input
                .working_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
            run_mode: effective_run_mode,
            acceptance_criteria: vec![
                "Return sentinel-wrapped SummaryEnvelope JSON.".to_string(),
                "Keep findings concise and actionable.".to_string(),
            ],
        };

        Ok((loaded, request, probe_result, execution_policy))
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
            format!(
                "provider `{}` is unavailable ({})",
                provider.as_str(),
                details.join("; ")
            ),
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
        request: &RunRequest,
    ) -> std::result::Result<Vec<String>, ErrorData> {
        if !matches!(
            spec.runtime.file_conflict_policy,
            FileConflictPolicy::Serialize
        ) || matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        {
            return Ok(Vec::new());
        }
        let source = resolve_source_path(&request.working_dir)
            .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
        let source_key = source.display().to_string();
        if request.selected_files.is_empty() {
            return Ok(vec![format!("{source_key}::__workspace__")]);
        }

        let mut keys = request
            .selected_files
            .iter()
            .map(|selected| {
                let scoped_path = if selected.path.is_absolute() {
                    selected.path.strip_prefix(&source).unwrap_or(&selected.path)
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
    use std::{
        collections::HashMap,
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
        runtime::summary::{
            ArtifactKind, ArtifactRef, StructuredSummary, SummaryEnvelope, SummaryParseStatus,
            VerificationStatus, SUMMARY_CONTRACT_VERSION,
        },
        spec::Provider,
    };

    use super::{
        acquire_serialize_locks_from_state, HandleInput, McpSubagentServer,
        ReadAgentArtifactInput, RunAgentInput, RunAgentSelectedFileInput,
    };
    use crate::{mcp::artifacts::build_runtime_artifacts, mcp::state::RuntimeState};

    fn write_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "Mock", "");
    }

    fn write_codex_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "Codex", "");
    }

    fn write_gemini_agent_spec(dir: &Path) {
        write_agent_spec_with_provider_and_runtime(dir, "Gemini", "");
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
working_dir_policy = "InPlace"
sandbox = "ReadOnly"
{runtime_extra}
"#
        );
        fs::write(dir.join("reviewer.agent.toml"), agent).expect("write agent");
    }

    #[derive(Debug, Clone)]
    struct TestProviderProber {
        probes: HashMap<Provider, ProviderProbe>,
    }

    impl TestProviderProber {
        fn ready() -> Self {
            Self {
                probes: HashMap::new(),
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
    }

    impl ProviderProber for TestProviderProber {
        fn probe(&self, provider: &Provider) -> ProviderProbe {
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
            .any(|note| note.contains("MissingBinary")));
    }

    #[tokio::test]
    async fn list_agents_marks_ollama_available_when_probe_ready() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider(&agents_dir, "Ollama");
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
        };
        let out = server
            .run_agent(rmcp::handler::server::wrapper::Parameters(input))
            .await
            .expect("run")
            .0;

        assert_eq!(out.status, "Succeeded");
        assert!(out
            .structured_summary
            .summary
            .contains("Mock run completed"));
        assert_eq!(out.structured_summary.verification_status, "Passed");
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
            }))
            .await
        {
            Ok(_) => panic!("run should fail when provider is unavailable"),
            Err(err) => err,
        };
        assert!(err
            .message
            .as_ref()
            .contains("provider `Codex` is unavailable"));
    }

    #[tokio::test]
    async fn run_agent_rejects_when_spawn_policy_requires_async() {
        let temp = tempdir().expect("temp");
        let agents_dir = temp.path().join("agents");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&agents_dir).expect("create agents");
        write_agent_spec_with_provider_and_runtime(
            &agents_dir,
            "Mock",
            r#"spawn_policy = "Async""#,
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
            "Mock",
            r#"background_preference = "PreferBackground""#,
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
        write_agent_spec_with_provider(&agents_dir, "Ollama");
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
            }))
            .await
        {
            Ok(_) => panic!("run should fail when provider is unavailable"),
            Err(err) => err,
        };

        assert!(err
            .message
            .as_ref()
            .contains("provider `Ollama` is unavailable"));
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

        let summary = SummaryEnvelope {
            contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
            parse_status: SummaryParseStatus::Validated,
            summary: StructuredSummary {
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
                exit_code: 0,
                verification_status: VerificationStatus::Passed,
                touched_files: Vec::new(),
                plan_refs: Vec::new(),
            },
            raw_fallback_text: None,
        };

        let (index, payloads) = build_runtime_artifacts(&summary, "", "", Some(temp.path()));
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
            "run_agent",
            "spawn_agent",
            "get_agent_status",
            "cancel_agent",
            "read_agent_artifact",
        ] {
            assert!(tools.iter().any(|tool| tool.name == expected));
        }

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
            final_status = structured_field(&status_json, "status").to_string();
            if matches!(
                final_status.as_str(),
                "Succeeded" | "Failed" | "Cancelled" | "TimedOut"
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        assert_eq!(final_status, "Succeeded");

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

        let second_spawn_res = client
            .call_tool(
                CallToolRequestParams::new("spawn_agent").with_arguments(
                    json!({
                        "agent_name": "reviewer",
                        "task": "cancel me"
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
        assert_eq!(structured_field(&cancel_json, "status"), "Cancelled");

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
        assert_eq!(status.status, "Succeeded");

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
provider = "Mock"
instructions = "write"

[runtime]
working_dir_policy = "TempCopy"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
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
            }))
            .await
            .expect("run")
            .0;

        let run_json =
            fs::read_to_string(state_dir.join("runs").join(&out.handle_id).join("run.json"))
                .expect("read run json");
        let run_obj: serde_json::Value = serde_json::from_str(&run_json).expect("parse run json");
        let run_dir = state_dir.join("runs").join(&out.handle_id);
        assert!(!run_obj["created_at"].is_null());
        assert!(!run_obj["updated_at"].is_null());
        assert!(run_obj["status_history"].is_array());
        assert_eq!(run_obj["request_snapshot"]["task"], "copy workspace");
        assert_eq!(
            run_obj["request_snapshot"]["working_dir"],
            project_dir.display().to_string()
        );
        assert_eq!(run_obj["spec_snapshot"]["name"], "writer");
        assert_eq!(run_obj["spec_snapshot"]["working_dir_policy"], "TempCopy");
        assert_eq!(run_obj["probe_result"]["provider"], "Mock");
        assert!(!run_obj["memory_resolution"].is_null());
        assert_eq!(run_obj["workspace"]["mode"], "TempCopy");
        assert!(!run_obj["execution_policy"].is_null());
        assert_eq!(run_obj["execution_policy"]["requested_run_mode"], "sync");
        assert_eq!(run_obj["execution_policy"]["effective_run_mode"], "sync");
        assert_eq!(
            run_obj["execution_policy"]["effective_run_mode_source"],
            "spec"
        );
        assert_eq!(run_obj["execution_policy"]["retry_max_attempts"], 1);
        assert_eq!(run_obj["execution_policy"]["retry_backoff_secs"], 1);
        assert_eq!(run_obj["execution_policy"]["attempts_used"], 1);
        assert_eq!(run_obj["execution_policy"]["retries_used"], 0);
        let workspace_path = run_obj["workspace"]["workspace_path"]
            .as_str()
            .expect("workspace path");
        assert!(
            !Path::new(workspace_path).exists(),
            "workspace should be cleaned after run completion"
        );
        assert!(run_obj["workspace"]["lock_key"]
            .as_str()
            .expect("lock key")
            .contains("project"));
        assert!(run_obj["workspace"]["lock_keys"].is_array());
        assert!(
            run_obj["workspace"]["lock_keys"]
                .as_array()
                .is_some_and(|keys| !keys.is_empty())
        );
        assert!(run_obj["compiled_context_markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("ROLE"));

        assert!(run_dir.join("request.json").exists());
        assert!(run_dir.join("resolved-spec.json").exists());
        assert!(run_dir.join("compiled-context.md").exists());
        assert!(run_dir.join("status.json").exists());
        assert!(run_dir.join("summary.json").exists());
        assert!(run_dir.join("summary.raw.txt").exists());
        assert!(run_dir.join("workspace.meta.json").exists());
        assert!(run_dir.join("events.ndjson").exists());
        assert!(run_dir.join("artifacts").join("index.json").exists());

        let artifact_index_json = fs::read_to_string(run_dir.join("artifacts").join("index.json"))
            .expect("read artifact index");
        let artifact_index: serde_json::Value =
            serde_json::from_str(&artifact_index_json).expect("parse artifact index");
        let first = artifact_index
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
            fs::read_to_string(run_dir.join("events.ndjson")).expect("read events file");
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
            "probe",
            "gate",
            "workspace",
            "memory",
            "policy",
            "parse",
            "cleanup",
        ] {
            assert!(
                event_names.iter().any(|event| event == required),
                "missing event `{required}` in {event_names:?}"
            );
        }

        assert!(run_dir.join("stdout.log").exists());
        assert!(run_dir.join("stderr.log").exists());
        assert!(run_dir.join("temp").exists());
    }

    #[tokio::test]
    async fn serialize_lock_blocks_until_guard_released() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let first_guard = acquire_serialize_locks_from_state(
            &state,
            vec!["repo-key::src".to_string()],
        )
        .await;
        assert_eq!(first_guard.len(), 1, "first guard");

        let state_clone = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            let start = Instant::now();
            let guard = acquire_serialize_locks_from_state(
                &state_clone,
                vec!["repo-key::src".to_string()],
            )
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
        let first_guard = acquire_serialize_locks_from_state(
            &state,
            vec!["repo-key::src".to_string()],
        )
        .await;
        assert_eq!(first_guard.len(), 1);

        let state_clone = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            let start = Instant::now();
            let guard = acquire_serialize_locks_from_state(
                &state_clone,
                vec!["repo-key::web".to_string()],
            )
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
