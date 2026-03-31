pub mod codex;

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, rmcp::schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RenderStyle {
    #[default]
    Default,
    Codex,
}
