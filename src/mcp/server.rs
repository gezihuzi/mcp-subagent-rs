use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{sync::Mutex, task::JoinHandle};
use uuid::Uuid;

use crate::{
    error::McpSubagentError,
    runtime::{
        context::DefaultContextCompiler,
        dispatcher::{DispatchResult, Dispatcher, RunStatus},
        mock_runner::{MockRunPlan, MockRunner},
        summary::{ArtifactKind, ArtifactRef, StructuredSummary, VerificationStatus},
    },
    spec::registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
    types::{ResolvedMemory, RunMode, RunRequest, SelectedFile},
};

#[derive(Debug, Default)]
struct RuntimeState {
    runs: HashMap<String, RunRecord>,
    tasks: HashMap<String, JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct RunRecord {
    status: RunStatus,
    updated_at: OffsetDateTime,
    summary: Option<StructuredSummary>,
    artifact_index: Vec<ArtifactOutput>,
    artifacts: HashMap<String, String>,
    error_message: Option<String>,
    task: String,
}

impl RunRecord {
    fn running(task: String) -> Self {
        Self {
            status: RunStatus::Running,
            updated_at: OffsetDateTime::now_utc(),
            summary: None,
            artifact_index: Vec::new(),
            artifacts: HashMap::new(),
            error_message: None,
            task,
        }
    }
}

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
    runtime_state: Arc<Mutex<RuntimeState>>,
}

impl McpSubagentServer {
    pub fn new(agents_dirs: Vec<PathBuf>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            agents_dirs,
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
    ) -> std::result::Result<(LoadedAgentSpec, RunRequest), ErrorData> {
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

        Ok((loaded, request))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct AgentListing {
    pub name: String,
    pub description: String,
    pub provider: String,
    pub available: bool,
    pub runtime_policy: RuntimePolicySummary,
    pub capability_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RuntimePolicySummary {
    pub context_mode: String,
    pub working_dir_policy: String,
    pub sandbox: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListAgentsOutput {
    pub agents: Vec<AgentListing>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunAgentSelectedFileInput {
    pub path: String,
    pub rationale: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunAgentInput {
    pub agent_name: String,
    pub task: String,
    pub task_brief: Option<String>,
    pub parent_summary: Option<String>,
    #[serde(default)]
    pub selected_files: Vec<RunAgentSelectedFileInput>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct HandleInput {
    pub handle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ReadAgentArtifactInput {
    pub handle_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ArtifactOutput {
    pub path: String,
    pub kind: String,
    pub description: String,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct SummaryOutput {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
    pub verification_status: String,
    pub touched_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunAgentOutput {
    pub handle_id: String,
    pub status: String,
    pub structured_summary: SummaryOutput,
    pub artifact_index: Vec<ArtifactOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct SpawnAgentOutput {
    pub handle_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct AgentStatusOutput {
    pub handle_id: String,
    pub status: String,
    pub updated_at: String,
    pub error_message: Option<String>,
    pub structured_summary: Option<SummaryOutput>,
    pub artifact_index: Vec<ArtifactOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CancelAgentOutput {
    pub handle_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ReadAgentArtifactOutput {
    pub handle_id: String,
    pub path: String,
    pub content: String,
}

#[tool_router]
impl McpSubagentServer {
    #[tool(description = "List all available mcp-subagent agent specs.")]
    pub async fn list_agents(&self) -> std::result::Result<Json<ListAgentsOutput>, ErrorData> {
        let loaded = self.load_specs()?;
        let agents = loaded
            .into_iter()
            .map(|loaded| {
                let runtime = loaded.spec.runtime;
                AgentListing {
                    name: loaded.spec.core.name,
                    description: loaded.spec.core.description,
                    provider: loaded.spec.core.provider.as_str().to_string(),
                    available: true,
                    runtime_policy: RuntimePolicySummary {
                        context_mode: format!("{:?}", runtime.context_mode),
                        working_dir_policy: format!("{:?}", runtime.working_dir_policy),
                        sandbox: format!("{:?}", runtime.sandbox),
                        timeout_secs: runtime.timeout_secs,
                    },
                    capability_notes: Vec::new(),
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
        let (loaded, request) = self.prepare_run(input)?;
        let result = run_dispatch(&loaded.spec, &request)?;

        let (artifact_index, artifacts) =
            build_runtime_artifacts(&result.summary, &result.stdout, &result.stderr);
        let output = RunAgentOutput {
            handle_id: result.metadata.handle_id.to_string(),
            status: format!("{:?}", result.metadata.status),
            structured_summary: map_summary_output(&result.summary),
            artifact_index: artifact_index.clone(),
        };

        let mut state = self.runtime_state.lock().await;
        state.runs.insert(
            output.handle_id.clone(),
            RunRecord {
                status: result.metadata.status,
                updated_at: OffsetDateTime::now_utc(),
                summary: Some(result.summary),
                artifact_index,
                artifacts,
                error_message: result.metadata.error_message,
                task: request.task,
            },
        );

        Ok(Json(output))
    }

    #[tool(description = "Spawn an agent asynchronously and return handle_id immediately.")]
    pub async fn spawn_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<SpawnAgentOutput>, ErrorData> {
        let (loaded, request) = self.prepare_run(input)?;
        let handle_id = Uuid::now_v7().to_string();

        {
            let mut state = self.runtime_state.lock().await;
            state
                .runs
                .insert(handle_id.clone(), RunRecord::running(request.task.clone()));
        }

        let state = Arc::clone(&self.runtime_state);
        let task_handle_id = handle_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let dispatch = run_dispatch(&loaded.spec, &request);

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
                    let (artifact_index, artifacts) =
                        build_runtime_artifacts(&result.summary, &result.stdout, &result.stderr);
                    record.status = result.metadata.status;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.error_message = result.metadata.error_message;
                    record.summary = Some(result.summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                }
                Err(err) => {
                    let summary = failed_summary(err.message.clone().into_owned());
                    let (artifact_index, artifacts) = build_runtime_artifacts(&summary, "", "");
                    record.status = RunStatus::Failed;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.error_message = Some(err.to_string());
                    record.summary = Some(summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                }
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
        let state = self.runtime_state.lock().await;
        let record = state.runs.get(&input.handle_id).ok_or_else(|| {
            ErrorData::resource_not_found(format!("handle not found: {}", input.handle_id), None)
        })?;

        Ok(Json(AgentStatusOutput {
            handle_id: input.handle_id,
            status: format!("{:?}", record.status),
            updated_at: format_time(record.updated_at),
            error_message: record.error_message.clone(),
            structured_summary: record.summary.as_ref().map(map_summary_output),
            artifact_index: record.artifact_index.clone(),
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
            record.error_message = Some("cancelled by user request".to_string());
            if record.summary.is_none() {
                let summary = cancelled_summary(record.task.clone());
                let (artifact_index, artifacts) = build_runtime_artifacts(&summary, "", "");
                record.summary = Some(summary);
                record.artifact_index = artifact_index;
                record.artifacts = artifacts;
            }
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
        let state = self.runtime_state.lock().await;
        let run = state.runs.get(&input.handle_id).ok_or_else(|| {
            ErrorData::resource_not_found(format!("handle not found: {}", input.handle_id), None)
        })?;

        let content = run.artifacts.get(&input.path).ok_or_else(|| {
            ErrorData::resource_not_found(
                format!(
                    "artifact not found for handle {}: {}",
                    input.handle_id, input.path
                ),
                None,
            )
        })?;

        Ok(Json(ReadAgentArtifactOutput {
            handle_id: input.handle_id,
            path: input.path,
            content: content.clone(),
        }))
    }
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

fn map_artifact_output(artifact: &ArtifactRef) -> ArtifactOutput {
    ArtifactOutput {
        path: artifact.path.display().to_string(),
        kind: format!("{:?}", artifact.kind),
        description: artifact.description.clone(),
        media_type: artifact.media_type.clone(),
    }
}

fn run_dispatch(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
) -> std::result::Result<DispatchResult, ErrorData> {
    let mock_summary = build_mock_summary(&spec.core.name, request);
    let dispatcher = Dispatcher::new(
        DefaultContextCompiler,
        MockRunner::new(MockRunPlan::Succeeded {
            summary: mock_summary,
        }),
    );

    dispatcher
        .run(spec, request, ResolvedMemory::default())
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))
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

fn build_runtime_artifacts(
    summary: &StructuredSummary,
    stdout: &str,
    stderr: &str,
) -> (Vec<ArtifactOutput>, HashMap<String, String>) {
    let mut index = summary
        .artifacts
        .iter()
        .map(map_artifact_output)
        .collect::<Vec<_>>();
    let mut payloads = HashMap::new();

    if let Ok(summary_json) = serde_json::to_string_pretty(summary) {
        index.push(ArtifactOutput {
            path: "summary.json".to_string(),
            kind: format!("{:?}", ArtifactKind::SummaryJson),
            description: "Structured summary JSON".to_string(),
            media_type: Some("application/json".to_string()),
        });
        payloads.insert("summary.json".to_string(), summary_json);
    }

    if !stdout.is_empty() {
        index.push(ArtifactOutput {
            path: "stdout.txt".to_string(),
            kind: format!("{:?}", ArtifactKind::StdoutText),
            description: "Captured stdout".to_string(),
            media_type: Some("text/plain".to_string()),
        });
        payloads.insert("stdout.txt".to_string(), stdout.to_string());
    }

    if !stderr.is_empty() {
        index.push(ArtifactOutput {
            path: "stderr.txt".to_string(),
            kind: format!("{:?}", ArtifactKind::StderrText),
            description: "Captured stderr".to_string(),
            media_type: Some("text/plain".to_string()),
        });
        payloads.insert("stderr.txt".to_string(), stderr.to_string());
    }

    (index, payloads)
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
    use std::{fs, path::Path, time::Duration};

    use rmcp::{
        model::{CallToolRequestParams, ClientInfo},
        ClientHandler, ServiceExt,
    };
    use serde_json::json;
    use tempfile::tempdir;

    use super::{McpSubagentServer, RunAgentInput, RunAgentSelectedFileInput};

    fn write_agent_spec(dir: &Path) {
        let agent = r#"
[core]
name = "reviewer"
description = "review code"
provider = "Codex"
instructions = "review"

[runtime]
working_dir_policy = "InPlace"
sandbox = "ReadOnly"
"#;
        fs::write(dir.join("reviewer.agent.toml"), agent).expect("write agent");
    }

    #[tokio::test]
    async fn list_agents_tool_returns_agent() {
        let temp = tempdir().expect("temp");
        let path = temp.path().to_path_buf();
        write_agent_spec(&path);
        let server = McpSubagentServer::new(vec![path]);

        let out = server.list_agents().await.expect("list").0;
        assert_eq!(out.agents.len(), 1);
        assert_eq!(out.agents[0].name, "reviewer");
    }

    #[tokio::test]
    async fn run_agent_tool_returns_structured_summary() {
        let temp = tempdir().expect("temp");
        let path = temp.path().to_path_buf();
        write_agent_spec(&path);
        let server = McpSubagentServer::new(vec![path]);

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

    #[tokio::test]
    async fn mcp_transport_roundtrip_for_all_tools() {
        let temp = tempdir().expect("temp");
        let path = temp.path().to_path_buf();
        write_agent_spec(&path);

        let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
        let server = McpSubagentServer::new(vec![path]);
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
}
