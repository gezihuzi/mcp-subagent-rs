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
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub last_event_at: Option<String>,
    #[serde(default)]
    pub last_event_age_ms: Option<u64>,
    #[serde(default)]
    pub stalled: Option<bool>,
    #[serde(default)]
    pub block_reason: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListRunsInput {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunUsageOutput {
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub provider: String,
    pub model: Option<String>,
    pub provider_exit_code: Option<i32>,
    pub retries: u32,
    pub token_source: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_prompt_bytes: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunListingOutput {
    pub handle_id: String,
    pub status: String,
    pub updated_at: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub last_event_at: Option<String>,
    #[serde(default)]
    pub last_event_age_ms: Option<u64>,
    #[serde(default)]
    pub stalled: Option<bool>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub block_reason: Option<String>,
    pub provider: Option<String>,
    pub agent: Option<String>,
    pub task: String,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListRunsOutput {
    pub runs: Vec<RunListingOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetRunResultInput {
    pub handle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetRunResultOutput {
    pub contract_version: String,
    pub handle_id: String,
    pub status: String,
    pub updated_at: String,
    pub error_message: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub normalization_status: String,
    pub summary: Option<String>,
    pub native_result: Option<String>,
    pub normalized_result: Option<SummaryOutput>,
    pub provider_exit_code: Option<i32>,
    pub retries: u32,
    pub retry_classification: String,
    pub classification_reason: Option<String>,
    pub usage: RunUsageOutput,
    pub artifact_index: Vec<ArtifactOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ReadRunLogsInput {
    pub handle_id: String,
    #[serde(default)]
    pub stream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ReadRunLogsOutput {
    pub handle_id: String,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchRunInput {
    pub handle_id: String,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub phase_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchRunOutput {
    pub handle_id: String,
    pub status: String,
    pub updated_at: String,
    pub error_message: Option<String>,
    #[serde(default)]
    pub current_phase: Option<String>,
    #[serde(default)]
    pub current_phase_age_ms: Option<u64>,
    #[serde(default)]
    pub phase_timeout_hit: bool,
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub advice: Vec<String>,
    pub terminal: bool,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchAgentEventsInput {
    pub handle_id: String,
    #[serde(default)]
    pub since_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub phase_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunEventOutput {
    #[serde(default)]
    pub seq: Option<u64>,
    pub event: String,
    pub timestamp: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchAgentEventsOutput {
    pub handle_id: String,
    pub status: String,
    pub updated_at: String,
    pub terminal: bool,
    pub events: Vec<RunEventOutput>,
    #[serde(default)]
    pub next_seq: Option<u64>,
    #[serde(default)]
    pub current_phase: Option<String>,
    #[serde(default)]
    pub current_phase_age_ms: Option<u64>,
    #[serde(default)]
    pub phase_timeout_hit: bool,
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub advice: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetAgentStatsInput {
    pub handle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetAgentStatsOutput {
    pub handle_id: String,
    pub status: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub last_event_at: Option<String>,
    #[serde(default)]
    pub last_event_age_ms: Option<u64>,
    pub stalled: bool,
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub queue_ms: Option<u64>,
    #[serde(default)]
    pub provider_probe_ms: Option<u64>,
    #[serde(default)]
    pub workspace_prepare_ms: Option<u64>,
    #[serde(default)]
    pub provider_boot_ms: Option<u64>,
    #[serde(default)]
    pub execution_ms: Option<u64>,
    #[serde(default)]
    pub first_output_ms: Option<u64>,
    #[serde(default)]
    pub first_output_warned: bool,
    #[serde(default)]
    pub first_output_warning_at: Option<String>,
    #[serde(default)]
    pub current_wait_reason: Option<String>,
    #[serde(default)]
    pub wait_reasons: Vec<String>,
    #[serde(default)]
    pub wall_ms: Option<u64>,
    pub usage: RunUsageOutput,
}
