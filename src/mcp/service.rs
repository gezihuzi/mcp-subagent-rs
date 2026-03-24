use std::path::Path;

use rmcp::ErrorData;
use uuid::Uuid;

use crate::{
    mcp::state::WorkspaceRecord,
    runtime::{
        claude_runner::claude_runner_from_env,
        codex_runner::codex_runner_from_env,
        context::DefaultContextCompiler,
        dispatcher::{DispatchResult, Dispatcher},
        gemini_runner::gemini_runner_from_env,
        memory::resolve_memory,
        mock_runner::{MockRunPlan, MockRunner},
        runner::AgentRunner,
        workspace::{prepare_workspace, PreparedWorkspace, WorkspaceMode},
    },
    spec::Provider,
    types::RunRequest,
};

#[derive(Debug, Clone)]
pub(crate) struct DispatchEnvelope {
    pub(crate) result: DispatchResult,
    pub(crate) workspace: WorkspaceRecord,
}

pub(crate) async fn run_dispatch(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    state_dir: &Path,
    lock_key: Option<String>,
) -> std::result::Result<DispatchEnvelope, ErrorData> {
    let prepared_workspace = prepare_workspace(spec, request, state_dir, handle_id)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let workspace_record = to_workspace_record(&prepared_workspace, lock_key);

    let mut effective_request = request.clone();
    effective_request.working_dir = prepared_workspace.workspace_path;
    let resolved_memory = resolve_memory(spec, &effective_request)
        .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;

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
    })
}

fn select_runner(provider: &Provider) -> Box<dyn AgentRunner> {
    match provider {
        Provider::Claude => Box::new(claude_runner_from_env()),
        Provider::Codex => Box::new(codex_runner_from_env()),
        Provider::Gemini => Box::new(gemini_runner_from_env()),
        Provider::Ollama => Box::new(MockRunner::new(MockRunPlan::SucceededFromRequest)),
    }
}

fn to_workspace_record(prepared: &PreparedWorkspace, lock_key: Option<String>) -> WorkspaceRecord {
    WorkspaceRecord {
        mode: match prepared.mode {
            WorkspaceMode::InPlace => "InPlace",
            WorkspaceMode::TempCopy => "TempCopy",
            WorkspaceMode::GitWorktree => "GitWorktree",
            WorkspaceMode::GitWorktreeFallbackTempCopy => "GitWorktreeFallbackTempCopy",
        }
        .to_string(),
        source_path: prepared.source_path.clone(),
        workspace_path: prepared.workspace_path.clone(),
        notes: prepared.notes.clone(),
        lock_key,
    }
}

fn parse_handle_id(handle_id: &str) -> Uuid {
    Uuid::parse_str(handle_id).unwrap_or_else(|_| Uuid::now_v7())
}
