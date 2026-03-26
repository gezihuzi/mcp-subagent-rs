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
    #[serde(default)]
    pub runtime: RuntimePolicy,
    #[serde(default)]
    pub provider_overrides: ProviderOverrides,
    pub workflow: Option<WorkflowSpec>,
}
