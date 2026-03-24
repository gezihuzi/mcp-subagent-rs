use std::path::Path;

use rmcp::ErrorData;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    mcp::state::WorkspaceRecord,
    runtime::{
        claude_runner::{claude_runner_from_env, supports_provider as claude_supports_provider},
        codex_runner::{codex_runner_from_env, supports_provider as codex_supports_provider},
        context::{ContextCompiler, DefaultContextCompiler},
        dispatcher::{DispatchResult, Dispatcher, RunMetadata, RunStatus},
        gemini_runner::{gemini_runner_from_env, supports_provider as gemini_supports_provider},
        memory::resolve_memory,
        mock_runner::{MockRunPlan, MockRunner, RunnerTerminalState},
        summary::{StructuredSummary, VerificationStatus},
        workspace::{prepare_workspace, PreparedWorkspace, WorkspaceMode},
    },
    spec::validate::validate_agent_spec,
    types::{ResolvedMemory, RunRequest},
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

    let result = if claude_supports_provider(&spec.core.provider) {
        run_dispatch_claude(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else if codex_supports_provider(&spec.core.provider) {
        run_dispatch_codex(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else if gemini_supports_provider(&spec.core.provider) {
        run_dispatch_gemini(spec, &effective_request, handle_id, resolved_memory.clone()).await
    } else {
        run_dispatch_mock(spec, &effective_request, handle_id, resolved_memory)
    }?;

    Ok(DispatchEnvelope {
        result,
        workspace: workspace_record,
    })
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

fn run_dispatch_mock(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    let mock_summary = build_mock_summary(&spec.core.name, request);
    let dispatcher = Dispatcher::new(
        DefaultContextCompiler,
        MockRunner::new(MockRunPlan::Succeeded {
            summary: mock_summary,
        }),
    );

    let mut dispatched = dispatcher
        .run(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    dispatched.metadata.handle_id = parse_handle_id(handle_id);
    dispatched.metadata.workspace_path = request.working_dir.clone();
    Ok(dispatched)
}

async fn run_dispatch_codex(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = codex_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

async fn run_dispatch_claude(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = claude_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

async fn run_dispatch_gemini(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    resolved_memory: ResolvedMemory,
) -> std::result::Result<DispatchResult, ErrorData> {
    validate_agent_spec(spec).map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let compiler = DefaultContextCompiler;
    let compiled = compiler
        .compile(spec, request, resolved_memory)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let execution = gemini_runner_from_env()
        .execute(spec, request, &compiled)
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let summary = compiler
        .parse_summary(&execution.stdout, &execution.stderr)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let (status, error_message) = match execution.terminal_state {
        RunnerTerminalState::Succeeded => (RunStatus::Succeeded, None),
        RunnerTerminalState::Failed { message } => (RunStatus::Failed, Some(message)),
        RunnerTerminalState::TimedOut => (
            RunStatus::TimedOut,
            Some("runner exceeded timeout".to_string()),
        ),
        RunnerTerminalState::Cancelled => (
            RunStatus::Cancelled,
            Some("runner cancelled by request".to_string()),
        ),
    };
    let metadata = build_terminal_metadata(spec, request, handle_id, status, error_message);

    Ok(DispatchResult {
        metadata,
        summary,
        stdout: execution.stdout,
        stderr: execution.stderr,
    })
}

fn build_terminal_metadata(
    spec: &crate::spec::AgentSpec,
    request: &RunRequest,
    handle_id: &str,
    status: RunStatus,
    error_message: Option<String>,
) -> RunMetadata {
    let now = OffsetDateTime::now_utc();
    RunMetadata {
        handle_id: parse_handle_id(handle_id),
        created_at: now,
        updated_at: now,
        status: status.clone(),
        status_history: vec![
            RunStatus::Received,
            RunStatus::Validating,
            RunStatus::ProbingProvider,
            RunStatus::PreparingWorkspace,
            RunStatus::ResolvingMemory,
            RunStatus::CompilingContext,
            RunStatus::Launching,
            RunStatus::Running,
            RunStatus::Collecting,
            RunStatus::ParsingSummary,
            RunStatus::Finalizing,
            status,
        ],
        provider: spec.core.provider.clone(),
        agent_name: spec.core.name.clone(),
        workspace_path: request.working_dir.clone(),
        error_message,
    }
}

fn build_mock_summary(agent_name: &str, request: &RunRequest) -> StructuredSummary {
    let touched_files = request
        .selected_files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect::<Vec<_>>();

    StructuredSummary {
        summary: format!("Mock run completed for task: {}", request.task),
        key_findings: vec![format!(
            "Agent `{}` executed through dispatcher mock runner.",
            agent_name
        )],
        artifacts: Vec::new(),
        open_questions: Vec::new(),
        next_steps: vec![
            "Replace mock runner with provider runner for production use.".to_string(),
        ],
        exit_code: 0,
        verification_status: VerificationStatus::Passed,
        touched_files,
    }
}
