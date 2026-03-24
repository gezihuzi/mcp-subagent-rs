use std::{collections::HashMap, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    mcp::dto::ArtifactOutput,
    probe::ProviderProbe,
    runtime::{dispatcher::RunStatus, summary::StructuredSummary},
    spec::{
        runtime_policy::{
            ApprovalPolicy, ContextMode, FileConflictPolicy, MemorySource, SandboxPolicy,
            WorkingDirPolicy,
        },
        AgentSpec,
    },
    types::{RunMode, RunRequest},
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
    pub(crate) summary: Option<StructuredSummary>,
    pub(crate) artifact_index: Vec<ArtifactOutput>,
    pub(crate) artifacts: HashMap<String, String>,
    pub(crate) error_message: Option<String>,
    pub(crate) task: String,
    pub(crate) request_snapshot: Option<RunRequestSnapshot>,
    pub(crate) spec_snapshot: Option<RunSpecSnapshot>,
    pub(crate) probe_result: Option<ProbeResultRecord>,
    pub(crate) workspace: Option<WorkspaceRecord>,
}

impl RunRecord {
    pub(crate) fn running(
        task: String,
        request_snapshot: Option<RunRequestSnapshot>,
        spec_snapshot: Option<RunSpecSnapshot>,
        probe_result: Option<ProbeResultRecord>,
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
            workspace: None,
        }
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedFileSnapshot {
    path: PathBuf,
    #[serde(default)]
    rationale: Option<String>,
    has_inlined_content: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRequestSnapshot {
    task: String,
    #[serde(default)]
    task_brief: Option<String>,
    parent_summary_present: bool,
    selected_files: Vec<SelectedFileSnapshot>,
    working_dir: PathBuf,
    run_mode: RunMode,
    acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunSpecSnapshot {
    name: String,
    provider: String,
    #[serde(default)]
    model: Option<String>,
    context_mode: String,
    working_dir_policy: String,
    file_conflict_policy: String,
    sandbox: String,
    approval: String,
    timeout_secs: u64,
    memory_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeResultRecord {
    provider: String,
    executable: PathBuf,
    #[serde(default)]
    version: Option<String>,
    status: String,
    notes: Vec<String>,
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
    pub(crate) summary: Option<StructuredSummary>,
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
    pub(crate) workspace: Option<WorkspaceRecord>,
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
            workspace: value.workspace.clone(),
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

pub(crate) fn append_status_if_terminal(status_history: &mut Vec<RunStatus>, status: RunStatus) {
    if status_history.last().is_some_and(|last| *last == status) {
        return;
    }
    status_history.push(status);
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
        MemorySource::File(path) => format!("File({path})"),
        MemorySource::Glob(pattern) => format!("Glob({pattern})"),
        MemorySource::Inline(_) => "Inline(<content>)".to_string(),
    }
}
