use crate::{render::RenderStyle, spec::runtime_policy::WorkingDirPolicy};
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
    pub selected_files: Vec<RunAgentSelectedFileInput>,
    pub stage: Option<String>,
    pub plan_ref: Option<String>,
    pub working_dir: Option<String>,
    pub working_dir_policy_override: Option<WorkingDirPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CodexInput {
    pub task: String,
    pub task_brief: Option<String>,
    pub parent_summary: Option<String>,
    #[serde(default)]
    pub selected_files: Vec<RunAgentSelectedFileInput>,
    pub stage: Option<String>,
    pub plan_ref: Option<String>,
    pub working_dir: Option<String>,
    pub working_dir_policy_override: Option<WorkingDirPolicy>,
    pub agent_name: Option<String>,
    pub render_style: Option<RenderStyle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CodexOutput {
    pub run: RunView,
    pub rendered: String,
    pub render_style: RenderStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct HandleInput {
    pub handle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct DenyPermissionInput {
    pub handle_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct PermissionDecisionOutput {
    pub handle_id: String,
    pub status: String,
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
    pub producer: Option<String>,
    pub created_at: Option<String>,
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
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OutcomeView {
    Succeeded {
        summary: String,
        key_findings: Vec<String>,
        touched_files: Vec<String>,
        artifacts: Vec<ArtifactOutput>,
        usage: RunUsageOutput,
    },
    Failed {
        error: String,
        retry_classification: String,
        partial_summary: Option<String>,
        usage: RunUsageOutput,
    },
    Cancelled {
        reason: String,
    },
    TimedOut {
        elapsed_secs: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunView {
    pub handle_id: String,
    pub agent_name: String,
    pub task_brief: Option<String>,
    pub phase: String,
    pub terminal: bool,
    pub outcome: Option<OutcomeView>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListRunsInput {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListRunsOutput {
    pub runs: Vec<RunView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GetRunResultInput {
    pub handle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ReadRunLogsInput {
    pub handle_id: String,
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
    pub interval_ms: Option<u64>,
    pub timeout_secs: Option<u64>,
    pub phase: Option<String>,
    pub phase_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchRunOutput {
    pub run: RunView,
    pub timed_out: bool,
    pub phase_timeout_hit: bool,
    pub block_reason: Option<String>,
    pub advice: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WatchAgentEventsInput {
    pub handle_id: String,
    pub since_seq: Option<u64>,
    pub limit: Option<usize>,
    pub phase: Option<String>,
    pub phase_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RunEventOutput {
    pub seq: Option<u64>,
    pub event: String,
    pub timestamp: String,
    pub state: Option<String>,
    pub phase: Option<String>,
    pub source: Option<String>,
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
    pub next_seq: Option<u64>,
    pub current_phase: Option<String>,
    pub current_phase_age_ms: Option<u64>,
    pub phase_timeout_hit: bool,
    pub block_reason: Option<String>,
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
    pub state: Option<String>,
    pub phase: Option<String>,
    pub last_event_at: Option<String>,
    pub last_event_age_ms: Option<u64>,
    pub stalled: bool,
    pub block_reason: Option<String>,
    pub advice: Vec<String>,
    pub queue_ms: Option<u64>,
    pub provider_probe_ms: Option<u64>,
    pub workspace_prepare_ms: Option<u64>,
    pub provider_boot_ms: Option<u64>,
    pub execution_ms: Option<u64>,
    pub first_output_ms: Option<u64>,
    pub first_output_warned: bool,
    pub first_output_warning_at: Option<String>,
    pub current_wait_reason: Option<String>,
    pub wait_reasons: Vec<String>,
    pub wall_ms: Option<u64>,
    pub usage: RunUsageOutput,
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

#[cfg(test)]
mod tests {
    use crate::{render::RenderStyle, spec::runtime_policy::WorkingDirPolicy};

    use super::{CodexInput, RunAgentInput};

    #[test]
    fn run_agent_input_deserializes_working_dir_policy_override_enum() {
        let raw = serde_json::json!({
            "agent_name": "reviewer",
            "task": "review parser",
            "task_brief": null,
            "parent_summary": null,
            "selected_files": [],
            "stage": null,
            "plan_ref": null,
            "working_dir": null,
            "working_dir_policy_override": "direct"
        });
        let parsed: RunAgentInput = serde_json::from_value(raw).expect("deserialize input");
        assert_eq!(
            parsed.working_dir_policy_override,
            Some(WorkingDirPolicy::Direct)
        );
    }

    #[test]
    fn run_agent_input_rejects_unknown_working_dir_policy_override() {
        let raw = serde_json::json!({
            "agent_name": "reviewer",
            "task": "review parser",
            "task_brief": null,
            "parent_summary": null,
            "selected_files": [],
            "stage": null,
            "plan_ref": null,
            "working_dir": null,
            "working_dir_policy_override": "bad-policy"
        });
        let err = serde_json::from_value::<RunAgentInput>(raw).expect_err("must reject override");
        assert!(
            err.to_string().contains("unknown variant"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn codex_input_defaults_selected_files() {
        let raw = serde_json::json!({
            "task": "review parser"
        });
        let parsed: CodexInput = serde_json::from_value(raw).expect("deserialize codex input");
        assert!(parsed.selected_files.is_empty());
    }

    #[test]
    fn codex_input_deserializes_render_style_enum() {
        let raw = serde_json::json!({
            "task": "review parser",
            "render_style": "codex"
        });
        let parsed: CodexInput = serde_json::from_value(raw).expect("deserialize codex input");
        assert_eq!(parsed.render_style, Some(RenderStyle::Codex));
    }
}
