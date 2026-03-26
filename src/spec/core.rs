use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Mock,
    Claude,
    Codex,
    Gemini,
    Ollama,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpecCore {
    pub name: String,
    pub description: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub instructions: String,
    #[serde(default = "default_string_vec")]
    pub allowed_tools: Vec<String>,
    #[serde(default = "default_string_vec")]
    pub disallowed_tools: Vec<String>,
    #[serde(default = "default_string_vec")]
    pub skills: Vec<String>,
    #[serde(default = "default_string_vec")]
    pub tags: Vec<String>,
    #[serde(default = "default_metadata")]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_string_vec() -> Vec<String> {
    Vec::new()
}

fn default_metadata() -> HashMap<String, serde_json::Value> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::AgentSpecCore;

    #[test]
    fn agent_spec_core_direct_deserialization_preserves_collection_defaults() {
        let core: AgentSpecCore = toml::from_str(
            r#"
name = "reviewer"
description = "review code"
provider = "codex"
instructions = "review"
"#,
        )
        .expect("agent spec core should parse");

        assert!(core.allowed_tools.is_empty());
        assert!(core.disallowed_tools.is_empty());
        assert!(core.skills.is_empty());
        assert!(core.tags.is_empty());
        assert!(core.metadata.is_empty());
    }
}
