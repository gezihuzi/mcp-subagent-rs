pub mod core;
pub mod provider_overrides;
pub mod registry;
pub mod runtime_policy;
pub mod validate;
pub mod workflow;

pub use core::{AgentSpecCore, Provider};
pub use provider_overrides::ProviderOverrides;
pub use runtime_policy::RuntimePolicy;
use serde::{Deserialize, Serialize};
pub use workflow::WorkflowSpec;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub core: AgentSpecCore,
    #[serde(default = "default_runtime_policy")]
    pub runtime: RuntimePolicy,
    #[serde(default = "default_provider_overrides")]
    pub provider_overrides: ProviderOverrides,
    pub workflow: Option<WorkflowSpec>,
}

fn default_runtime_policy() -> RuntimePolicy {
    RuntimePolicy::default()
}

fn default_provider_overrides() -> ProviderOverrides {
    ProviderOverrides::default()
}

#[cfg(test)]
mod tests {
    use super::AgentSpec;

    #[test]
    fn agent_spec_direct_deserialization_preserves_top_level_defaults() {
        let spec: AgentSpec = toml::from_str(
            r#"
[core]
name = "reviewer"
description = "review code"
provider = "codex"
instructions = "review"
"#,
        )
        .expect("agent spec should parse");

        assert_eq!(spec.runtime.timeout_secs, 900);
        assert!(spec.provider_overrides.claude.is_none());
        assert!(spec.provider_overrides.codex.is_none());
        assert!(spec.provider_overrides.gemini.is_none());
        assert!(spec.workflow.is_none());
    }
}
