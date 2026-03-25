use std::path::Path;

use rmcp::ErrorData;
use uuid::Uuid;

use crate::{
    mcp::state::{build_memory_resolution_snapshot, MemoryResolutionRecord, WorkspaceRecord},
    runtime::{
        cleanup::WorkspaceCleanupGuard,
        context::DefaultContextCompiler,
        dispatcher::{DispatchResult, Dispatcher},
        memory::resolve_memory,
        runners::{
            self,
            mock::{MockRunPlan, MockRunner},
            AgentRunner,
        },
        workspace::{prepare_workspace, PreparedWorkspace, WorkspaceMode},
    },
    spec::Provider,
    types::RunRequest,
};

#[derive(Debug)]
pub(crate) struct DispatchEnvelope {
    pub(crate) result: DispatchResult,
    pub(crate) workspace: WorkspaceRecord,
    pub(crate) memory_resolution: MemoryResolutionRecord,
    pub(crate) _workspace_cleanup: Option<WorkspaceCleanupGuard>,
}

pub(crate) async fn run_dispatch(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    state_dir: &Path,
    lock_keys: Vec<String>,
) -> std::result::Result<DispatchEnvelope, ErrorData> {
    let prepared_workspace = prepare_workspace(spec, request, state_dir, handle_id)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let workspace_cleanup = WorkspaceCleanupGuard::for_workspace(&prepared_workspace);
    let workspace_record = to_workspace_record(&prepared_workspace, lock_keys);

    let mut effective_request = request.clone();
    effective_request.working_dir = prepared_workspace.workspace_path;
    let resolved_memory = resolve_memory(spec, &effective_request)
        .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
    let memory_resolution = build_memory_resolution_snapshot(&resolved_memory);

    let runner = select_runner(&spec.core.provider);
    let dispatcher = Dispatcher::new(DefaultContextCompiler, runner);
    let mut result = dispatcher
        .run(spec, &effective_request, resolved_memory)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    result.metadata.handle_id = parse_handle_id(handle_id);
    result.metadata.workspace_path = effective_request.working_dir.clone();

    Ok(DispatchEnvelope {
        result,
        workspace: workspace_record,
        memory_resolution,
        _workspace_cleanup: workspace_cleanup,
    })
}

fn select_runner(provider: &Provider) -> Box<dyn AgentRunner> {
    match provider {
        Provider::Mock => Box::new(MockRunner::new(MockRunPlan::SucceededFromRequest)),
        Provider::Claude => Box::new(runners::claude::from_env()),
        Provider::Codex => Box::new(runners::codex::from_env()),
        Provider::Gemini => Box::new(runners::gemini::from_env()),
        Provider::Ollama => Box::new(runners::ollama::from_env()),
    }
}

fn to_workspace_record(prepared: &PreparedWorkspace, lock_keys: Vec<String>) -> WorkspaceRecord {
    let lock_key = lock_keys.first().cloned();
    WorkspaceRecord {
        mode: match prepared.mode {
            WorkspaceMode::InPlace => "in_place",
            WorkspaceMode::TempCopy => "temp_copy",
            WorkspaceMode::GitWorktree => "git_worktree",
            WorkspaceMode::GitWorktreeFallbackTempCopy => "git_worktree_fallback_temp_copy",
        }
        .to_string(),
        source_path: prepared.source_path.clone(),
        workspace_path: prepared.workspace_path.clone(),
        notes: prepared.notes.clone(),
        lock_key,
        lock_keys,
    }
}

fn parse_handle_id(handle_id: &str) -> Uuid {
    Uuid::parse_str(handle_id).unwrap_or_else(|_| Uuid::now_v7())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use tempfile::tempdir;

    use crate::{
        mcp::service::run_dispatch,
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{
                ContextMode, FileConflictPolicy, MemorySource, RuntimePolicy, SandboxPolicy,
                WorkingDirPolicy,
            },
            AgentSpec,
        },
        types::{RunMode, RunRequest},
    };

    fn sample_spec(
        working_dir_policy: WorkingDirPolicy,
        memory_sources: Vec<MemorySource>,
    ) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "writer".to_string(),
                description: "write code".to_string(),
                provider: Provider::Mock,
                model: None,
                instructions: "write".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: HashMap::new(),
            },
            runtime: RuntimePolicy {
                context_mode: ContextMode::Isolated,
                memory_sources,
                working_dir_policy,
                file_conflict_policy: FileConflictPolicy::Serialize,
                sandbox: SandboxPolicy::WorkspaceWrite,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    fn sample_request(working_dir: std::path::PathBuf) -> RunRequest {
        RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            stage: None,
            plan_ref: None,
            working_dir,
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        }
    }

    #[tokio::test]
    async fn run_dispatch_cleans_temp_workspace_after_success() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir_all(&source).expect("create source");
        fs::write(source.join("a.txt"), "hello").expect("write source");

        let spec = sample_spec(
            WorkingDirPolicy::TempCopy,
            vec![MemorySource::AutoProjectMemory],
        );
        let request = sample_request(source);
        let handle = "run-success-cleanup";
        let state_dir = temp.path().join("state");

        let dispatch = run_dispatch(&spec, &request, handle, &state_dir, Vec::new())
            .await
            .expect("dispatch succeeds");
        let workspace_path = dispatch.workspace.workspace_path.clone();
        assert!(workspace_path.exists());

        drop(dispatch);
        assert!(!workspace_path.exists());
    }

    #[tokio::test]
    async fn run_dispatch_error_path_cleans_temp_workspace() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir_all(&source).expect("create source");
        fs::write(source.join("a.txt"), "hello").expect("write source");

        let spec = sample_spec(
            WorkingDirPolicy::TempCopy,
            vec![MemorySource::Glob("missing/**/*.md".to_string())],
        );
        let request = sample_request(source);
        let handle = "run-error-cleanup";
        let state_dir = temp.path().join("state");

        let err = run_dispatch(&spec, &request, handle, &state_dir, Vec::new())
            .await
            .expect_err("dispatch should fail at memory resolve");
        assert!(err
            .message
            .as_ref()
            .contains("Glob memory source did not match any files"));

        let workspace_path = state_dir.join("runs").join(handle).join("workspace");
        assert!(!workspace_path.exists());
    }
}
