pub mod core;
pub mod provider_overrides;
pub mod registry;
pub mod runtime_policy;
pub mod validate;

pub use core::{AgentSpecCore, Provider};
pub use provider_overrides::ProviderOverrides;
pub use runtime_policy::RuntimePolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub core: AgentSpecCore,
    #[serde(default)]
    pub runtime: RuntimePolicy,
    #[serde(default)]
    pub provider_overrides: ProviderOverrides,
}
