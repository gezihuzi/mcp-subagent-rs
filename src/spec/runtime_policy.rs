use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ContextMode {
    Isolated,
    SummaryOnly,
    SelectedFiles(Vec<String>),
    ExpandedBrief,
}

impl std::fmt::Display for ContextMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum MemorySource {
    AutoProjectMemory,
    ActivePlan,
    ArchivedPlans,
    File(String),
    Glob(String),
    Inline(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum WorkingDirPolicy {
    Auto,
    InPlace,
    TempCopy,
    GitWorktree,
}

impl std::fmt::Display for WorkingDirPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum FileConflictPolicy {
    Deny,
    Serialize,
    AllowWithMergeReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

impl std::fmt::Display for SandboxPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ApprovalPolicy {
    ProviderDefault,
    Ask,
    AutoAcceptEdits,
    DenyByDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BackgroundPreference {
    PreferForeground,
    PreferBackground,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum SpawnPolicy {
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPolicy {
    #[serde(default = "default_emit_summary_json")]
    pub emit_summary_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    #[serde(default = "default_retry_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_retry_backoff_secs")]
    pub backoff_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePolicy {
    #[serde(default = "default_context_mode")]
    pub context_mode: ContextMode,
    #[serde(default = "default_memory_sources")]
    pub memory_sources: Vec<MemorySource>,
    #[serde(default = "default_working_dir_policy")]
    pub working_dir_policy: WorkingDirPolicy,
    #[serde(default = "default_file_conflict_policy")]
    pub file_conflict_policy: FileConflictPolicy,
    #[serde(default = "default_sandbox_policy")]
    pub sandbox: SandboxPolicy,
    #[serde(default = "default_approval_policy")]
    pub approval: ApprovalPolicy,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_background_preference")]
    pub background_preference: BackgroundPreference,
    #[serde(default = "default_spawn_policy")]
    pub spawn_policy: SpawnPolicy,
    #[serde(default)]
    pub artifact_policy: ArtifactPolicy,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
}

impl Default for ArtifactPolicy {
    fn default() -> Self {
        Self {
            emit_summary_json: default_emit_summary_json(),
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_attempts(),
            backoff_secs: default_retry_backoff_secs(),
        }
    }
}

impl Default for RuntimePolicy {
    fn default() -> Self {
        Self {
            context_mode: default_context_mode(),
            memory_sources: default_memory_sources(),
            working_dir_policy: default_working_dir_policy(),
            file_conflict_policy: default_file_conflict_policy(),
            sandbox: default_sandbox_policy(),
            approval: default_approval_policy(),
            max_turns: None,
            timeout_secs: default_timeout_secs(),
            background_preference: default_background_preference(),
            spawn_policy: default_spawn_policy(),
            artifact_policy: ArtifactPolicy::default(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

fn default_context_mode() -> ContextMode {
    ContextMode::Isolated
}

fn default_memory_sources() -> Vec<MemorySource> {
    vec![MemorySource::AutoProjectMemory, MemorySource::ActivePlan]
}

fn default_working_dir_policy() -> WorkingDirPolicy {
    WorkingDirPolicy::Auto
}

fn default_file_conflict_policy() -> FileConflictPolicy {
    FileConflictPolicy::Serialize
}

fn default_sandbox_policy() -> SandboxPolicy {
    SandboxPolicy::ReadOnly
}

fn default_approval_policy() -> ApprovalPolicy {
    ApprovalPolicy::ProviderDefault
}

fn default_timeout_secs() -> u64 {
    900
}

fn default_background_preference() -> BackgroundPreference {
    BackgroundPreference::PreferForeground
}

fn default_spawn_policy() -> SpawnPolicy {
    SpawnPolicy::Sync
}

fn default_emit_summary_json() -> bool {
    true
}

fn default_retry_attempts() -> u32 {
    1
}

fn default_retry_backoff_secs() -> u64 {
    1
}
