use serde::{Deserialize, Serialize};

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
    pub stage: Option<String>,
    pub plan_ref: Option<String>,
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
    #[serde(default)]
    pub producer: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct SummaryOutput {
    pub contract_version: String,
    pub parse_status: String,
    pub summary: String,
    pub key_findings: Vec<String>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
    pub verification_status: String,
    pub touched_files: Vec<String>,
    pub plan_refs: Vec<String>,
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
