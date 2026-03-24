use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOverrides {
    #[serde(default)]
    pub claude: Option<ClaudeOverrides>,
    #[serde(default)]
    pub codex: Option<CodexOverrides>,
    #[serde(default)]
    pub gemini: Option<GeminiOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeOverrides {
    #[serde(default)]
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexSandboxMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexOverrides {
    #[serde(default)]
    pub model_reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub sandbox_mode: Option<CodexSandboxMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeminiOverrides {
    #[serde(default)]
    pub experimental_subagents: Option<bool>,
}
