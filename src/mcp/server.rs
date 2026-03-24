use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::{
    error::McpSubagentError,
    mcp::artifacts::{
        build_runtime_artifacts, read_artifact_from_disk, sanitize_relative_artifact_path,
    },
    mcp::persistence::{load_run_record_from_disk, persist_run_record},
    mcp::state::{
        append_status_if_terminal, build_probe_result_snapshot, build_run_request_snapshot,
        build_run_spec_snapshot, RunRecord, RuntimeState, WorkspaceRecord,
    },
    probe::{ProbeStatus, ProviderProbe, ProviderProber, SystemProviderProber},
    runtime::{
        claude_runner::{claude_runner_from_env, supports_provider as claude_supports_provider},
        codex_runner::{codex_runner_from_env, supports_provider as codex_supports_provider},
        context::{ContextCompiler, DefaultContextCompiler},
        dispatcher::{DispatchResult, Dispatcher, RunMetadata, RunStatus},
        gemini_runner::{gemini_runner_from_env, supports_provider as gemini_supports_provider},
        memory::resolve_memory,
        mock_runner::{MockRunPlan, MockRunner, RunnerTerminalState},
        summary::{StructuredSummary, VerificationStatus},
        workspace::{prepare_workspace, resolve_source_path, PreparedWorkspace, WorkspaceMode},
    },
    spec::{
        registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
        runtime_policy::{FileConflictPolicy, SandboxPolicy},
        validate::validate_agent_spec,
        Provider,
    },
    types::{ResolvedMemory, RunMode, RunRequest, SelectedFile},
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
            tool_router: Self::tool_router(),
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

    fn load_specs(&self) -> std::result::Result<Vec<LoadedAgentSpec>, ErrorData> {
        load_agent_specs_from_dirs(&self.agents_dirs)
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))
    }

    fn prepare_run(
        &self,
        input: RunAgentInput,
    ) -> std::result::Result<(LoadedAgentSpec, RunRequest, ProviderProbe), ErrorData> {
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
            working_dir: input
                .working_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
            run_mode: RunMode::Sync,
            acceptance_criteria: vec![
                "Return sentinel-wrapped StructuredSummary JSON.".to_string(),
                "Keep findings concise and actionable.".to_string(),
            ],
        };

        Ok((loaded, request, probe_result))
    }

    fn probe_provider(&self, provider: &Provider) -> ProviderProbe {
        self.provider_prober.probe(provider)
    }

    fn ensure_provider_ready(
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

    async fn get_or_load_run_record(
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

    async fn upsert_and_persist_run(
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

    fn conflict_lock_key(
        &self,
        spec: &crate::spec::AgentSpec,
        request: &RunRequest,
    ) -> std::result::Result<Option<String>, ErrorData> {
        if !matches!(
            spec.runtime.file_conflict_policy,
            FileConflictPolicy::Serialize
        ) || matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        {
            return Ok(None);
        }
        let source = resolve_source_path(&request.working_dir)
            .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
        Ok(Some(source.display().to_string()))
    }

    async fn acquire_serialize_lock(
        &self,
        lock_key: Option<String>,
    ) -> Option<OwnedMutexGuard<()>> {
        acquire_serialize_lock_from_state(&self.runtime_state, lock_key).await
    }
}

async fn acquire_serialize_lock_from_state(
    state: &Arc<Mutex<RuntimeState>>,
    lock_key: Option<String>,
) -> Option<OwnedMutexGuard<()>> {
    let key = lock_key?;
    let lock = {
        let mut guard = state.lock().await;
        guard
            .serialize_locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    Some(lock.lock_owned().await)
}

#[tool_router]
impl McpSubagentServer {
    #[tool(description = "List all available mcp-subagent agent specs.")]
    pub async fn list_agents(&self) -> std::result::Result<Json<ListAgentsOutput>, ErrorData> {
        let loaded = self.load_specs()?;
        let mut probe_cache: HashMap<Provider, ProviderProbe> = HashMap::new();
        let agents = loaded
            .into_iter()
            .map(|loaded| {
                let provider = loaded.spec.core.provider.clone();
                let runtime = loaded.spec.runtime;
                let probe = probe_cache
                    .entry(provider.clone())
                    .or_insert_with(|| self.probe_provider(&provider))
                    .clone();
                AgentListing {
                    name: loaded.spec.core.name,
                    description: loaded.spec.core.description,
                    provider: provider.as_str().to_string(),
                    available: probe.is_available(),
                    runtime_policy: RuntimePolicySummary {
                        context_mode: format!("{:?}", runtime.context_mode),
                        working_dir_policy: format!("{:?}", runtime.working_dir_policy),
                        sandbox: format!("{:?}", runtime.sandbox),
                        timeout_secs: runtime.timeout_secs,
                    },
                    capability_notes: build_capability_notes(&probe),
                }
            })
            .collect();

        Ok(Json(ListAgentsOutput { agents }))
    }

    #[tool(description = "Run an agent synchronously and return structured summary.")]
    pub async fn run_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<RunAgentOutput>, ErrorData> {
        let (loaded, request, probe_result) = self.prepare_run(input)?;
        let request_snapshot = build_run_request_snapshot(&request);
        let spec_snapshot = build_run_spec_snapshot(&loaded.spec);
        let probe_snapshot = build_probe_result_snapshot(&probe_result);
        let run_created_at = OffsetDateTime::now_utc();
        let handle_id = Uuid::now_v7().to_string();
        let lock_key = self.conflict_lock_key(&loaded.spec, &request)?;
        let _serialize_guard = self.acquire_serialize_lock(lock_key.clone()).await;
        let dispatch = run_dispatch(
            &loaded.spec,
            &request,
            &handle_id,
            &self.state_dir,
            lock_key.clone(),
        )
        .await?;
        let result = dispatch.result;

        let (artifact_index, artifacts) = build_runtime_artifacts(
            &result.summary,
            &result.stdout,
            &result.stderr,
            Some(&dispatch.workspace.workspace_path),
        );
        let output = RunAgentOutput {
            handle_id: handle_id.clone(),
            status: format!("{:?}", result.metadata.status),
            structured_summary: map_summary_output(&result.summary),
            artifact_index: artifact_index.clone(),
        };

        let record = RunRecord {
            status: result.metadata.status,
            created_at: run_created_at,
            updated_at: OffsetDateTime::now_utc(),
            status_history: result.metadata.status_history,
            summary: Some(result.summary),
            artifact_index,
            artifacts,
            error_message: result.metadata.error_message,
            task: request.task,
            request_snapshot: Some(request_snapshot),
            spec_snapshot: Some(spec_snapshot),
            probe_result: Some(probe_snapshot),
            workspace: Some(dispatch.workspace),
        };
        self.upsert_and_persist_run(&handle_id, record).await?;

        Ok(Json(output))
    }

    #[tool(description = "Spawn an agent asynchronously and return handle_id immediately.")]
    pub async fn spawn_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<SpawnAgentOutput>, ErrorData> {
        let (loaded, request, probe_result) = self.prepare_run(input)?;
        let handle_id = Uuid::now_v7().to_string();
        let running_record = RunRecord::running(
            request.task.clone(),
            Some(build_run_request_snapshot(&request)),
            Some(build_run_spec_snapshot(&loaded.spec)),
            Some(build_probe_result_snapshot(&probe_result)),
        );
        let lock_key = self.conflict_lock_key(&loaded.spec, &request)?;

        self.upsert_and_persist_run(&handle_id, running_record)
            .await?;

        let state = Arc::clone(&self.runtime_state);
        let state_dir = self.state_dir.clone();
        let task_handle_id = handle_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let _serialize_guard =
                acquire_serialize_lock_from_state(&state, lock_key.clone()).await;
            let dispatch = run_dispatch(
                &loaded.spec,
                &request,
                &task_handle_id,
                &state_dir,
                lock_key.clone(),
            )
            .await;

            let mut guard = state.lock().await;
            guard.tasks.remove(&task_handle_id);
            let Some(record) = guard.runs.get_mut(&task_handle_id) else {
                return;
            };

            if matches!(record.status, RunStatus::Cancelled) {
                return;
            }

            match dispatch {
                Ok(result) => {
                    let dispatch_result = result.result;
                    let (artifact_index, artifacts) = build_runtime_artifacts(
                        &dispatch_result.summary,
                        &dispatch_result.stdout,
                        &dispatch_result.stderr,
                        Some(&result.workspace.workspace_path),
                    );
                    record.status = dispatch_result.metadata.status;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.status_history = dispatch_result.metadata.status_history;
                    record.error_message = dispatch_result.metadata.error_message;
                    record.summary = Some(dispatch_result.summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.workspace = Some(result.workspace);
                }
                Err(err) => {
                    let summary = failed_summary(err.message.clone().into_owned());
                    let (artifact_index, artifacts) =
                        build_runtime_artifacts(&summary, "", "", None);
                    record.status = RunStatus::Failed;
                    record.updated_at = OffsetDateTime::now_utc();
                    append_status_if_terminal(&mut record.status_history, RunStatus::Failed);
                    record.error_message = Some(err.to_string());
                    record.summary = Some(summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.workspace = None;
                }
            }

            if let Err(err) = persist_run_record(&state_dir, &task_handle_id, record) {
                record.error_message = Some(format!("failed to persist run state: {err}"));
            }
        });

        {
            let mut state = self.runtime_state.lock().await;
            state.tasks.insert(handle_id.clone(), task);
        }

        Ok(Json(SpawnAgentOutput {
            handle_id,
            status: format!("{:?}", RunStatus::Running),
        }))
    }

    #[tool(description = "Get current status for an async agent run.")]
    pub async fn get_agent_status(
        &self,
        Parameters(input): Parameters<HandleInput>,
    ) -> std::result::Result<Json<AgentStatusOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let structured_summary = record.summary.as_ref().map(map_summary_output);

        Ok(Json(AgentStatusOutput {
            handle_id: input.handle_id,
            status: format!("{:?}", record.status),
            updated_at: format_time(record.updated_at),
            error_message: record.error_message,
            structured_summary,
            artifact_index: record.artifact_index,
        }))
    }

    #[tool(description = "Cancel an async agent run if still in progress.")]
    pub async fn cancel_agent(
        &self,
        Parameters(input): Parameters<HandleInput>,
    ) -> std::result::Result<Json<CancelAgentOutput>, ErrorData> {
        let mut state = self.runtime_state.lock().await;

        let existing_status = state
            .runs
            .get(&input.handle_id)
            .map(|run| run.status.clone())
            .ok_or_else(|| {
                ErrorData::resource_not_found(
                    format!("handle not found: {}", input.handle_id),
                    None,
                )
            })?;

        if matches!(
            existing_status,
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled | RunStatus::TimedOut
        ) {
            return Ok(Json(CancelAgentOutput {
                handle_id: input.handle_id,
                status: format!("{:?}", existing_status),
            }));
        }

        if let Some(task) = state.tasks.remove(&input.handle_id) {
            task.abort();
        }

        if let Some(record) = state.runs.get_mut(&input.handle_id) {
            record.status = RunStatus::Cancelled;
            record.updated_at = OffsetDateTime::now_utc();
            append_status_if_terminal(&mut record.status_history, RunStatus::Cancelled);
            record.error_message = Some("cancelled by user request".to_string());
            if record.summary.is_none() {
                let summary = cancelled_summary(record.task.clone());
                let (artifact_index, artifacts) = build_runtime_artifacts(&summary, "", "", None);
                record.summary = Some(summary);
                record.artifact_index = artifact_index;
                record.artifacts = artifacts;
            }
            persist_run_record(&self.state_dir, &input.handle_id, record)?;
        }

        Ok(Json(CancelAgentOutput {
            handle_id: input.handle_id,
            status: format!("{:?}", RunStatus::Cancelled),
        }))
    }

    #[tool(description = "Read a UTF-8 text artifact by run handle and path.")]
    pub async fn read_agent_artifact(
        &self,
        Parameters(input): Parameters<ReadAgentArtifactInput>,
    ) -> std::result::Result<Json<ReadAgentArtifactOutput>, ErrorData> {
        if sanitize_relative_artifact_path(&input.path).is_none() {
            return Err(ErrorData::invalid_params(
                format!("invalid artifact path: {}", input.path),
                None,
            ));
        }

        let mut run = self.get_or_load_run_record(&input.handle_id).await?;
        let content = if let Some(content) = run.artifacts.get(&input.path) {
            content.clone()
        } else {
            let content = read_artifact_from_disk(&self.state_dir, &input.handle_id, &input.path)?
                .ok_or_else(|| {
                    ErrorData::resource_not_found(
                        format!(
                            "artifact not found for handle {}: {}",
                            input.handle_id, input.path
                        ),
                        None,
                    )
                })?;
            run.artifacts.insert(input.path.clone(), content.clone());
            let mut state = self.runtime_state.lock().await;
            if let Some(existing) = state.runs.get_mut(&input.handle_id) {
                existing
                    .artifacts
                    .insert(input.path.clone(), content.clone());
            }
            content
        };

        Ok(Json(ReadAgentArtifactOutput {
            handle_id: input.handle_id,
            path: input.path,
            content,
        }))
    }
}

fn default_state_dir() -> PathBuf {
    PathBuf::from(".mcp-subagent/state")
}

fn build_capability_notes(probe: &ProviderProbe) -> Vec<String> {
    let mut notes = Vec::new();
    notes.push(format!("probe_status: {}", probe.status));
    if let Some(version) = &probe.version {
        notes.push(format!("detected_version: {version}"));
    }
    if matches!(probe.status, ProbeStatus::MissingBinary) {
        notes.push(format!(
            "install `{}` and ensure it is in PATH",
            probe.executable.display()
        ));
    }
    notes.extend(probe.notes.clone());
    notes
}

fn map_summary_output(summary: &StructuredSummary) -> SummaryOutput {
    SummaryOutput {
        summary: summary.summary.clone(),
        key_findings: summary.key_findings.clone(),
        open_questions: summary.open_questions.clone(),
        next_steps: summary.next_steps.clone(),
        exit_code: summary.exit_code,
        verification_status: format!("{:?}", summary.verification_status),
        touched_files: summary.touched_files.clone(),
    }
}

#[derive(Debug, Clone)]
struct DispatchEnvelope {
    result: DispatchResult,
    workspace: WorkspaceRecord,
}

async fn run_dispatch(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    state_dir: &Path,
    lock_key: Option<String>,
) -> std::result::Result<DispatchEnvelope, ErrorData> {
    let prepared_workspace = prepare_workspace(spec, request, state_dir, handle_id)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let workspace_record = to_workspace_record(&prepared_workspace, lock_key);
    let mut effective_request = request.clone();
    effective_request.working_dir = prepared_workspace.workspace_path;
    let resolved_memory = resolve_memory(spec, &effective_request)
        .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;

    let result = if claude_supports_provider(&spec.core.provider) {
        run_dispatch_claude(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else if codex_supports_provider(&spec.core.provider) {
        run_dispatch_codex(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else if gemini_supports_provider(&spec.core.provider) {
        run_dispatch_gemini(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else {
        run_dispatch_mock(spec, &effective_request, handle_id, resolved_memory)
    }?;

    Ok(DispatchEnvelope {
        result,
        workspace: workspace_record,
    })
}

fn to_workspace_record(prepared: &PreparedWorkspace, lock_key: Option<String>) -> WorkspaceRecord {
    WorkspaceRecord {
        mode: match prepared.mode {
            WorkspaceMode::InPlace => "InPlace",
            WorkspaceMode::TempCopy => "TempCopy",
            WorkspaceMode::GitWorktree => "GitWorktree",
            WorkspaceMode::GitWorktreeFallbackTempCopy => "GitWorktreeFallbackTempCopy",
        }
        .to_string(),
        source_path: prepared.source_path.clone(),
        workspace_path: prepared.workspace_path.clone(),
        notes: prepared.notes.clone(),
        lock_key,
    }
}

fn parse_handle_id(handle_id: &str) -> Uuid {
    Uuid::parse_str(handle_id).unwrap_or_else(|_| Uuid::now_v7())
}

fn run_dispatch_mock(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    let mock_summary = build_mock_summary(&spec.core.name, request);
    let dispatcher = Dispatcher::new(
        DefaultContextCompiler,
        MockRunner::new(MockRunPlan::Succeeded {
            summary: mock_summary,
        }),
    );

    let mut dispatched = dispatcher
        .run(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    dispatched.metadata.handle_id = parse_handle_id(handle_id);
    dispatched.metadata.workspace_path = request.working_dir.clone();
    Ok(dispatched)
}

async fn run_dispatch_codex(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = codex_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

async fn run_dispatch_claude(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = claude_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

async fn run_dispatch_gemini(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = gemini_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

fn build_terminal_metadata(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    status: RunStatus,
    error_message: Option<String>,
) -> RunMetadata {
    let now = OffsetDateTime::now_utc();
    RunMetadata {
        handle_id: parse_handle_id(handle_id),
        created_at: now,
        updated_at: now,
        status: status.clone(),
        status_history: vec![
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
            status,
        ],
        provider: spec.core.provider.clone(),
        agent_name: spec.core.name.clone(),
        workspace_path: request.working_dir.clone(),
        error_message,
    }
}

fn build_mock_summary(agent_name: &str, request: &RunRequest) -> StructuredSummary {
    let touched_files = request
        .selected_files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect::<Vec<_>>();

    StructuredSummary {
        summary: format!("Mock run completed for task: {}", request.task),
        key_findings: vec![format!(
            "Agent `{}` executed through dispatcher mock runner.",
            agent_name
        )],
        artifacts: Vec::new(),
        open_questions: Vec::new(),
        next_steps: vec![
            "Replace mock runner with provider runner for production use.".to_string(),
        ],
        exit_code: 0,
        verification_status: VerificationStatus::Passed,
        touched_files,
    }
}

fn failed_summary(message: String) -> StructuredSummary {
    StructuredSummary {
        summary: "Run failed before structured output was collected.".to_string(),
        key_findings: vec![message],
        artifacts: Vec::new(),
        open_questions: vec!["Inspect server logs for failure details.".to_string()],
        next_steps: vec!["Retry the run with corrected configuration.".to_string()],
        exit_code: 1,
        verification_status: VerificationStatus::NotRun,
        touched_files: Vec::new(),
    }
}

fn cancelled_summary(task: String) -> StructuredSummary {
    StructuredSummary {
        summary: format!("Run cancelled before completion for task: {task}"),
        key_findings: vec!["User requested cancellation".to_string()],
        artifacts: Vec::new(),
        open_questions: Vec::new(),
        next_steps: vec!["Re-run the task if cancellation was accidental.".to_string()],
        exit_code: 130,
        verification_status: VerificationStatus::NotRun,
        touched_files: Vec::new(),
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
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
        runtime::summary::{ArtifactKind, ArtifactRef, StructuredSummary, VerificationStatus},
        spec::Provider,
    };

    use super::{
        acquire_serialize_lock_from_state, build_runtime_artifacts, HandleInput, McpSubagentServer,
        ReadAgentArtifactInput, RunAgentInput, RunAgentSelectedFileInput,
    };
    use crate::mcp::state::RuntimeState;

    fn write_agent_spec(dir: &Path) {
        write_agent_spec_with_provider(dir, "Ollama");
    }

    fn write_codex_agent_spec(dir: &Path) {
        write_agent_spec_with_provider(dir, "Codex");
    }

    fn write_gemini_agent_spec(dir: &Path) {
        write_agent_spec_with_provider(dir, "Gemini");
    }

    fn write_agent_spec_with_provider(dir: &Path, provider: &str) {
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
                    notes: Vec::new(),
                })
        }
    }

    fn provider_binary(provider: &Provider) -> std::path::PathBuf {
        match provider {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Gemini => "gemini",
            Provider::Ollama => "ollama",
        }
        .into()
    }

    fn provider_capabilities(provider: &Provider) -> ProviderCapabilities {
        match provider {
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

        let summary = StructuredSummary {
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
provider = "Ollama"
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
                working_dir: Some(project_dir.display().to_string()),
            }))
            .await
            .expect("run")
            .0;

        let run_json =
            fs::read_to_string(state_dir.join("runs").join(&out.handle_id).join("run.json"))
                .expect("read run json");
        let run_obj: serde_json::Value = serde_json::from_str(&run_json).expect("parse run json");
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
        assert_eq!(run_obj["probe_result"]["provider"], "Ollama");
        assert_eq!(run_obj["workspace"]["mode"], "TempCopy");
        let workspace_path = run_obj["workspace"]["workspace_path"]
            .as_str()
            .expect("workspace path");
        assert!(Path::new(workspace_path).join("hello.txt").exists());
        assert!(run_obj["workspace"]["lock_key"]
            .as_str()
            .expect("lock key")
            .contains("project"));
        let run_dir = state_dir.join("runs").join(&out.handle_id);
        assert!(run_dir.join("stdout.log").exists());
        assert!(run_dir.join("stderr.log").exists());
        assert!(run_dir.join("temp").exists());
    }

    #[tokio::test]
    async fn serialize_lock_blocks_until_guard_released() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let first_guard = acquire_serialize_lock_from_state(&state, Some("repo-key".to_string()))
            .await
            .expect("first guard");

        let state_clone = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            let start = Instant::now();
            let guard =
                acquire_serialize_lock_from_state(&state_clone, Some("repo-key".to_string()))
                    .await
                    .expect("second guard");
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
}
