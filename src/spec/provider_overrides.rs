use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOverrides {
    pub claude: Option<ClaudeOverrides>,
    pub codex: Option<CodexOverrides>,
    pub gemini: Option<GeminiOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeOverrides {
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
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub sandbox_mode: Option<CodexSandboxMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeminiOverrides {
    pub experimental_subagents: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::ProviderOverrides;

    #[test]
    fn provider_overrides_option_fields_deserialize_without_default_annotations() {
        let overrides: ProviderOverrides = toml::from_str(
            r#"
[codex]
"#,
        )
        .expect("provider overrides should parse");

        assert!(overrides.claude.is_none());
        let codex = overrides.codex.expect("codex override should exist");
        assert!(codex.model_reasoning_effort.is_none());
        assert!(codex.sandbox_mode.is_none());
        assert!(overrides.gemini.is_none());
    }
}
