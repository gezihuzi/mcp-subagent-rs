use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::McpSubagentError,
    runtime::{
        context::DefaultContextCompiler,
        dispatcher::Dispatcher,
        mock_runner::{MockRunPlan, MockRunner},
        summary::{ArtifactRef, StructuredSummary, VerificationStatus},
    },
    spec::registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
    types::{ResolvedMemory, RunMode, RunRequest, SelectedFile},
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
}

impl McpSubagentServer {
    pub fn new(agents_dirs: Vec<PathBuf>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            agents_dirs,
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
            task: input.task.clone(),
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

        let touched_files = request
            .selected_files
            .iter()
            .map(|f| f.path.display().to_string())
            .collect::<Vec<_>>();

        let mock_summary = StructuredSummary {
            summary: format!("Mock run completed for task: {}", request.task),
            key_findings: vec![format!(
                "Agent `{}` executed through dispatcher mock runner.",
                loaded.spec.core.name
            )],
            artifacts: Vec::new(),
            open_questions: Vec::new(),
            next_steps: vec![
                "Replace mock runner with provider runner for production use.".to_string(),
            ],
            exit_code: 0,
            verification_status: VerificationStatus::Passed,
            touched_files,
        };

        let dispatcher = Dispatcher::new(
            DefaultContextCompiler,
            MockRunner::new(MockRunPlan::Succeeded {
                summary: mock_summary,
            }),
        );

        let result = dispatcher
            .run(&loaded.spec, &request, ResolvedMemory::default())
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

        let output = RunAgentOutput {
            handle_id: result.metadata.handle_id.to_string(),
            status: format!("{:?}", result.metadata.status),
            structured_summary: map_summary_output(&result.summary),
            artifact_index: result
                .summary
                .artifacts
                .iter()
                .map(map_artifact_output)
                .collect(),
        };

        Ok(Json(output))
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

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

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

    #[tokio::test]
    async fn mcp_transport_roundtrip_for_list_and_run() {
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
        assert!(tools.iter().any(|tool| tool.name == "list_agents"));
        assert!(tools.iter().any(|tool| tool.name == "run_agent"));

        let list_res = client
            .call_tool(CallToolRequestParams::new("list_agents"))
            .await
            .expect("call list_agents");
        assert!(list_res.structured_content.is_some());

        let run_res = client
            .call_tool(
                CallToolRequestParams::new("run_agent").with_arguments(
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
            .expect("call run_agent");
        assert!(run_res.structured_content.is_some());

        client.cancel().await.expect("cancel client");
        server_handle.await.expect("server join");
    }
}
