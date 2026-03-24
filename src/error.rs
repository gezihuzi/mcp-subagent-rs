use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpSubagentError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("spec validation failed: {0}")]
    SpecValidation(String),

    #[error("mcp server error: {0}")]
    McpServer(String),
}

pub type Result<T> = std::result::Result<T, McpSubagentError>;
