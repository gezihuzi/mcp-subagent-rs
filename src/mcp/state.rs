use std::{collections::HashMap, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    mcp::dto::ArtifactOutput,
    probe::ProviderProbe,
    runtime::{dispatcher::RunStatus, summary::SummaryEnvelope},
    spec::{
        runtime_policy::{
            ApprovalPolicy, BackgroundPreference, ContextMode, FileConflictPolicy, MemorySource,
            RetryPolicy, SandboxPolicy, SpawnPolicy, WorkingDirPolicy,
        },
        AgentSpec,
    },
    types::{ResolvedMemory, RunMode, RunRequest},
};

#[derive(Debug, Default)]
pub(crate) struct RuntimeState {
    pub(crate) runs: HashMap<String, RunRecord>,
    pub(crate) tasks: HashMap<String, JoinHandle<()>>,
    pub(crate) serialize_locks: HashMap<String, Arc<Mutex<()>>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RunRecord {
    pub(crate) status: RunStatus,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
    pub(crate) status_history: Vec<RunStatus>,
    pub(crate) summary: Option<SummaryEnvelope>,
    pub(crate) artifact_index: Vec<ArtifactOutput>,
    pub(crate) artifacts: HashMap<String, String>,
    pub(crate) error_message: Option<String>,
    pub(crate) task: String,
    pub(crate) request_snapshot: Option<RunRequestSnapshot>,
    pub(crate) spec_snapshot: Option<RunSpecSnapshot>,
    pub(crate) probe_result: Option<ProbeResultRecord>,
    pub(crate) memory_resolution: Option<MemoryResolutionRecord>,
    pub(crate) workspace: Option<WorkspaceRecord>,
    pub(crate) compiled_context_markdown: Option<String>,
    pub(crate) execution_policy: Option<ExecutionPolicyRecord>,
}

impl RunRecord {
    pub(crate) fn running(
        task: String,
        request_snapshot: Option<RunRequestSnapshot>,
        spec_snapshot: Option<RunSpecSnapshot>,
        probe_result: Option<ProbeResultRecord>,
        execution_policy: Option<ExecutionPolicyRecord>,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            status: RunStatus::Running,
            created_at: now,
            updated_at: now,
            status_history: vec![
                RunStatus::Received,
                RunStatus::Validating,
                RunStatus::ProbingProvider,
                RunStatus::Running,
            ],
            summary: None,
            artifact_index: Vec::new(),
            artifacts: HashMap::new(),
            error_message: None,
            task,
            request_snapshot,
            spec_snapshot,
            probe_result,
            memory_resolution: None,
            workspace: None,
            compiled_context_markdown: None,
            execution_policy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PolicyValueSource {
    Default,
    Spec,
    Override,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionPolicyRecord {
    pub(crate) requested_run_mode: RunMode,
    pub(crate) effective_run_mode: RunMode,
    pub(crate) effective_run_mode_source: PolicyValueSource,
    pub(crate) spawn_policy: String,
    pub(crate) spawn_policy_source: PolicyValueSource,
    pub(crate) background_preference: String,
    pub(crate) background_preference_source: PolicyValueSource,
    #[serde(default)]
    pub(crate) max_turns: Option<u32>,
    pub(crate) max_turns_source: PolicyValueSource,
    pub(crate) retry_max_attempts: u32,
    pub(crate) retry_backoff_secs: u64,
    pub(crate) retry_policy_source: PolicyValueSource,
    #[serde(default)]
    pub(crate) attempts_used: Option<u32>,
    #[serde(default)]
    pub(crate) retries_used: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceRecord {
    pub(crate) mode: String,
    pub(crate) source_path: PathBuf,
    pub(crate) workspace_path: PathBuf,
    #[serde(default)]
    pub(crate) notes: Vec<String>,
    #[serde(default)]
    pub(crate) lock_key: Option<String>,
    #[serde(default)]
    pub(crate) lock_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedFileSnapshot {
    pub(crate) path: PathBuf,
    #[serde(default)]
    pub(crate) rationale: Option<String>,
    pub(crate) has_inlined_content: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRequestSnapshot {
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) task_brief: Option<String>,
    pub(crate) parent_summary_present: bool,
    pub(crate) selected_files: Vec<SelectedFileSnapshot>,
    #[serde(default)]
    pub(crate) stage: Option<String>,
    #[serde(default)]
    pub(crate) plan_ref: Option<String>,
    pub(crate) working_dir: PathBuf,
    pub(crate) run_mode: RunMode,
    pub(crate) acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunSpecSnapshot {
    pub(crate) name: String,
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) model: Option<String>,
    pub(crate) context_mode: String,
    pub(crate) working_dir_policy: String,
    pub(crate) file_conflict_policy: String,
    pub(crate) sandbox: String,
    pub(crate) approval: String,
    pub(crate) timeout_secs: u64,
    pub(crate) memory_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeResultRecord {
    pub(crate) provider: String,
    pub(crate) executable: PathBuf,
    #[serde(default)]
    pub(crate) version: Option<String>,
    pub(crate) status: String,
    pub(crate) notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryResolutionRecord {
    pub(crate) project_memory_count: usize,
    pub(crate) additional_memory_count: usize,
    pub(crate) native_passthrough_count: usize,
    #[serde(default)]
    pub(crate) project_memory_labels: Vec<String>,
    #[serde(default)]
    pub(crate) additional_memory_labels: Vec<String>,
    #[serde(default)]
    pub(crate) native_passthrough_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedRunRecord {
    pub(crate) status: RunStatus,
    #[serde(default)]
    pub(crate) created_at: Option<OffsetDateTime>,
    pub(crate) updated_at: OffsetDateTime,
    #[serde(default)]
    pub(crate) status_history: Vec<RunStatus>,
    pub(crate) summary: Option<SummaryEnvelope>,
    pub(crate) artifact_index: Vec<ArtifactOutput>,
    pub(crate) error_message: Option<String>,
    pub(crate) task: String,
    #[serde(default)]
    pub(crate) request_snapshot: Option<RunRequestSnapshot>,
    #[serde(default)]
    pub(crate) spec_snapshot: Option<RunSpecSnapshot>,
    #[serde(default)]
    pub(crate) probe_result: Option<ProbeResultRecord>,
    #[serde(default)]
    pub(crate) memory_resolution: Option<MemoryResolutionRecord>,
    #[serde(default)]
    pub(crate) workspace: Option<WorkspaceRecord>,
    #[serde(default)]
    pub(crate) compiled_context_markdown: Option<String>,
    #[serde(default)]
    pub(crate) execution_policy: Option<ExecutionPolicyRecord>,
}

impl From<&RunRecord> for PersistedRunRecord {
    fn from(value: &RunRecord) -> Self {
        Self {
            status: value.status.clone(),
            created_at: Some(value.created_at),
            updated_at: value.updated_at,
            status_history: value.status_history.clone(),
            summary: value.summary.clone(),
            artifact_index: value.artifact_index.clone(),
            error_message: value.error_message.clone(),
            task: value.task.clone(),
            request_snapshot: value.request_snapshot.clone(),
            spec_snapshot: value.spec_snapshot.clone(),
            probe_result: value.probe_result.clone(),
            memory_resolution: value.memory_resolution.clone(),
            workspace: value.workspace.clone(),
            compiled_context_markdown: value.compiled_context_markdown.clone(),
            execution_policy: value.execution_policy.clone(),
        }
    }
}

pub(crate) fn build_run_request_snapshot(request: &RunRequest) -> RunRequestSnapshot {
    RunRequestSnapshot {
        task: request.task.clone(),
        task_brief: request.task_brief.clone(),
        parent_summary_present: request.parent_summary.is_some(),
        selected_files: request
            .selected_files
            .iter()
            .map(|selected| SelectedFileSnapshot {
                path: selected.path.clone(),
                rationale: selected.rationale.clone(),
                has_inlined_content: selected.content.is_some(),
            })
            .collect(),
        stage: request.stage.clone(),
        plan_ref: request.plan_ref.clone(),
        working_dir: request.working_dir.clone(),
        run_mode: request.run_mode.clone(),
        acceptance_criteria: request.acceptance_criteria.clone(),
    }
}

pub(crate) fn build_run_spec_snapshot(spec: &AgentSpec) -> RunSpecSnapshot {
    RunSpecSnapshot {
        name: spec.core.name.clone(),
        provider: spec.core.provider.as_str().to_string(),
        model: spec.core.model.clone(),
        context_mode: context_mode_to_str(&spec.runtime.context_mode),
        working_dir_policy: working_dir_policy_to_str(&spec.runtime.working_dir_policy),
        file_conflict_policy: file_conflict_policy_to_str(&spec.runtime.file_conflict_policy),
        sandbox: sandbox_policy_to_str(&spec.runtime.sandbox),
        approval: approval_policy_to_str(&spec.runtime.approval),
        timeout_secs: spec.runtime.timeout_secs,
        memory_sources: spec
            .runtime
            .memory_sources
            .iter()
            .map(memory_source_to_str)
            .collect(),
    }
}

pub(crate) fn build_probe_result_snapshot(probe: &ProviderProbe) -> ProbeResultRecord {
    ProbeResultRecord {
        provider: probe.provider.as_str().to_string(),
        executable: probe.executable.clone(),
        version: probe.version.clone(),
        status: probe.status.to_string(),
        notes: probe.notes.clone(),
    }
}

pub(crate) fn build_memory_resolution_snapshot(memory: &ResolvedMemory) -> MemoryResolutionRecord {
    MemoryResolutionRecord {
        project_memory_count: memory.project_memories.len(),
        additional_memory_count: memory.additional_memories.len(),
        native_passthrough_count: memory.native_passthrough_paths.len(),
        project_memory_labels: memory
            .project_memories
            .iter()
            .map(|item| item.label.clone())
            .collect(),
        additional_memory_labels: memory
            .additional_memories
            .iter()
            .map(|item| item.label.clone())
            .collect(),
        native_passthrough_paths: memory.native_passthrough_paths.clone(),
    }
}

pub(crate) fn append_status_if_terminal(status_history: &mut Vec<RunStatus>, status: RunStatus) {
    if status_history.last().is_some_and(|last| *last == status) {
        return;
    }
    status_history.push(status);
}

pub(crate) fn build_execution_policy_snapshot(
    spec: &AgentSpec,
    requested_run_mode: RunMode,
    effective_run_mode: RunMode,
    effective_run_mode_source: PolicyValueSource,
) -> ExecutionPolicyRecord {
    let default_runtime = crate::spec::runtime_policy::RuntimePolicy::default();
    let runtime = &spec.runtime;
    ExecutionPolicyRecord {
        requested_run_mode,
        effective_run_mode,
        effective_run_mode_source,
        spawn_policy: spawn_policy_to_str(&runtime.spawn_policy),
        spawn_policy_source: source_from_default(
            &runtime.spawn_policy,
            &default_runtime.spawn_policy,
        ),
        background_preference: background_preference_to_str(&runtime.background_preference),
        background_preference_source: source_from_default(
            &runtime.background_preference,
            &default_runtime.background_preference,
        ),
        max_turns: runtime.max_turns,
        max_turns_source: if runtime.max_turns.is_some() {
            PolicyValueSource::Spec
        } else {
            PolicyValueSource::Default
        },
        retry_max_attempts: runtime.retry_policy.max_attempts,
        retry_backoff_secs: runtime.retry_policy.backoff_secs,
        retry_policy_source: retry_policy_source(
            &runtime.retry_policy,
            &default_runtime.retry_policy,
        ),
        attempts_used: None,
        retries_used: None,
    }
}

pub(crate) fn apply_execution_policy_outcome(
    policy: &mut Option<ExecutionPolicyRecord>,
    attempts_used: u32,
    retries_used: u32,
) {
    if let Some(policy) = policy.as_mut() {
        policy.attempts_used = Some(attempts_used);
        policy.retries_used = Some(retries_used);
    }
}

fn source_from_default<T: PartialEq>(value: &T, default: &T) -> PolicyValueSource {
    if value == default {
        PolicyValueSource::Default
    } else {
        PolicyValueSource::Spec
    }
}

fn retry_policy_source(value: &RetryPolicy, default: &RetryPolicy) -> PolicyValueSource {
    source_from_default(value, default)
}

fn context_mode_to_str(mode: &ContextMode) -> String {
    match mode {
        ContextMode::Isolated => "Isolated".to_string(),
        ContextMode::SummaryOnly => "SummaryOnly".to_string(),
        ContextMode::SelectedFiles(paths) => format!("SelectedFiles({})", paths.join(",")),
        ContextMode::ExpandedBrief => "ExpandedBrief".to_string(),
    }
}

fn working_dir_policy_to_str(policy: &WorkingDirPolicy) -> String {
    match policy {
        WorkingDirPolicy::Auto => "Auto".to_string(),
        WorkingDirPolicy::InPlace => "InPlace".to_string(),
        WorkingDirPolicy::TempCopy => "TempCopy".to_string(),
        WorkingDirPolicy::GitWorktree => "GitWorktree".to_string(),
    }
}

fn file_conflict_policy_to_str(policy: &FileConflictPolicy) -> String {
    match policy {
        FileConflictPolicy::Deny => "Deny".to_string(),
        FileConflictPolicy::Serialize => "Serialize".to_string(),
        FileConflictPolicy::AllowWithMergeReview => "AllowWithMergeReview".to_string(),
    }
}

fn sandbox_policy_to_str(policy: &SandboxPolicy) -> String {
    match policy {
        SandboxPolicy::ReadOnly => "ReadOnly".to_string(),
        SandboxPolicy::WorkspaceWrite => "WorkspaceWrite".to_string(),
        SandboxPolicy::FullAccess => "FullAccess".to_string(),
    }
}

fn approval_policy_to_str(policy: &ApprovalPolicy) -> String {
    match policy {
        ApprovalPolicy::ProviderDefault => "ProviderDefault".to_string(),
        ApprovalPolicy::Ask => "Ask".to_string(),
        ApprovalPolicy::AutoAcceptEdits => "AutoAcceptEdits".to_string(),
        ApprovalPolicy::DenyByDefault => "DenyByDefault".to_string(),
    }
}

fn memory_source_to_str(source: &MemorySource) -> String {
    match source {
        MemorySource::AutoProjectMemory => "AutoProjectMemory".to_string(),
        MemorySource::ActivePlan => "ActivePlan".to_string(),
        MemorySource::ArchivedPlans => "ArchivedPlans".to_string(),
        MemorySource::File(path) => format!("File({path})"),
        MemorySource::Glob(pattern) => format!("Glob({pattern})"),
        MemorySource::Inline(_) => "Inline(<content>)".to_string(),
    }
}

fn spawn_policy_to_str(policy: &SpawnPolicy) -> String {
    match policy {
        SpawnPolicy::Sync => "Sync".to_string(),
        SpawnPolicy::Async => "Async".to_string(),
    }
}

fn background_preference_to_str(preference: &BackgroundPreference) -> String {
    match preference {
        BackgroundPreference::PreferForeground => "PreferForeground".to_string(),
        BackgroundPreference::PreferBackground => "PreferBackground".to_string(),
    }
}
