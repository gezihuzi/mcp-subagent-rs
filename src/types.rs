use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedFile {
    pub path: PathBuf,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub task: String,
    #[serde(default)]
    pub task_brief: Option<String>,
    #[serde(default)]
    pub parent_summary: Option<String>,
    #[serde(default)]
    pub selected_files: Vec<SelectedFile>,
    pub working_dir: PathBuf,
    #[serde(default)]
    pub run_mode: RunMode,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySnippet {
    pub label: String,
    pub content: String,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMemory {
    #[serde(default)]
    pub project_memories: Vec<MemorySnippet>,
    #[serde(default)]
    pub additional_memories: Vec<MemorySnippet>,
    #[serde(default)]
    pub native_passthrough_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    InlineSummary,
    NativePassThrough,
    RawInline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextSourceRef {
    pub label: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub injection_mode: InjectionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledContext {
    pub system_prefix: String,
    pub injected_prompt: String,
    #[serde(default)]
    pub source_manifest: Vec<ContextSourceRef>,
}
