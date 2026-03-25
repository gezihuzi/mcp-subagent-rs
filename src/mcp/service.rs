use std::path::Path;

use rmcp::ErrorData;
use uuid::Uuid;

use crate::{
    mcp::{
        persistence::append_run_event,
        state::{build_memory_resolution_snapshot, MemoryResolutionRecord, WorkspaceRecord},
    },
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
    spec::runtime_policy::{DelegationContextPolicy, NativeDiscoveryPolicy},
    spec::AgentSpec,
    spec::Provider,
    types::RunRequest,
};
use serde_json::json;

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
    let mut prepared_workspace = prepare_workspace(spec, request, state_dir, handle_id)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let _ = append_run_event(
        state_dir,
        handle_id,
        "workspace.prepare.completed",
        "preparing",
        "workspace_prepare",
        "workspace",
        "workspace preparation completed",
        json!({}),
    );
    let effective_spec = apply_workspace_runtime_overrides(spec, &mut prepared_workspace);
    let workspace_cleanup = WorkspaceCleanupGuard::for_workspace(&prepared_workspace);
    let workspace_record = to_workspace_record(&prepared_workspace, lock_keys);

    let mut effective_request = request.clone();
    effective_request.working_dir = prepared_workspace.workspace_path;
    let resolved_memory = resolve_memory(&effective_spec, &effective_request)
        .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
    attach_plan_section_acceptance_criteria(
        &effective_spec,
        &mut effective_request,
        &resolved_memory,
    );
    let memory_resolution = build_memory_resolution_snapshot(&resolved_memory);

    let runner = select_runner(&effective_spec.core.provider);
    let dispatcher = Dispatcher::new(DefaultContextCompiler, runner);
    let mut result = dispatcher
        .run_with_transition_observer(
            &effective_spec,
            &effective_request,
            resolved_memory,
            |prev, current| {
                if matches!(
                    current,
                    crate::runtime::dispatcher::RunStatus::CompilingContext
                ) {
                    let _ = append_run_event(
                        state_dir,
                        handle_id,
                        "context.compile.started",
                        "preparing",
                        "context_compile",
                        "context",
                        "context compile started",
                        json!({}),
                    );
                }
                if matches!(
                    current,
                    crate::runtime::dispatcher::RunStatus::ParsingSummary
                ) {
                    let _ = append_run_event(
                        state_dir,
                        handle_id,
                        "parse.started",
                        "running",
                        "parse",
                        "parser",
                        "summary parse started",
                        json!({}),
                    );
                }
                if prev.as_ref().is_some_and(|status| {
                    matches!(
                        status,
                        crate::runtime::dispatcher::RunStatus::CompilingContext
                    )
                }) && !matches!(
                    current,
                    crate::runtime::dispatcher::RunStatus::CompilingContext
                ) {
                    let _ = append_run_event(
                        state_dir,
                        handle_id,
                        "context.compile.completed",
                        "preparing",
                        "context_compile",
                        "context",
                        "context compile completed",
                        json!({}),
                    );
                }
                if prev.as_ref().is_some_and(|status| {
                    matches!(
                        status,
                        crate::runtime::dispatcher::RunStatus::ParsingSummary
                    )
                }) && !matches!(
                    current,
                    crate::runtime::dispatcher::RunStatus::ParsingSummary
                ) {
                    let _ = append_run_event(
                        state_dir,
                        handle_id,
                        "parse.completed",
                        "running",
                        "parse",
                        "parser",
                        "summary parse completed",
                        json!({}),
                    );
                }
            },
        )
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

fn apply_workspace_runtime_overrides(
    spec: &AgentSpec,
    prepared: &mut PreparedWorkspace,
) -> AgentSpec {
    let mut effective_spec = spec.clone();
    if matches!(prepared.mode, WorkspaceMode::StableScratch)
        && matches!(effective_spec.core.provider, Provider::Gemini)
        && matches!(
            effective_spec.runtime.native_discovery,
            NativeDiscoveryPolicy::Isolated
        )
    {
        effective_spec.runtime.native_discovery = NativeDiscoveryPolicy::Minimal;
        prepared.notes.push(
            "runtime override: stable_scratch forces Gemini native_discovery from isolated to minimal to avoid auth/trust startup loops"
                .to_string(),
        );
    }
    effective_spec
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
            WorkspaceMode::StableScratch => "stable_scratch",
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

fn attach_plan_section_acceptance_criteria(
    spec: &AgentSpec,
    request: &mut RunRequest,
    memory: &crate::types::ResolvedMemory,
) {
    if !should_attach_plan_acceptance_criteria(spec, request) {
        return;
    }

    let Some(plan_section) = memory
        .additional_memories
        .iter()
        .find(|snippet| snippet.label.starts_with("plan_section:"))
    else {
        return;
    };

    let extracted = extract_markdown_checklist_items(&plan_section.content);
    if extracted.is_empty() {
        return;
    }

    for item in extracted {
        if request
            .acceptance_criteria
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&item))
        {
            continue;
        }
        request.acceptance_criteria.push(item);
    }
}

fn should_attach_plan_acceptance_criteria(spec: &AgentSpec, request: &RunRequest) -> bool {
    if matches!(
        spec.runtime.delegation_context,
        DelegationContextPolicy::PlanSection
    ) {
        return true;
    }
    if request
        .stage
        .as_deref()
        .is_some_and(|stage| stage.eq_ignore_ascii_case("review"))
    {
        return true;
    }
    spec.core
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("review"))
}

fn extract_markdown_checklist_items(section: &str) -> Vec<String> {
    section
        .lines()
        .filter_map(markdown_list_item)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn markdown_list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(item) = trimmed.strip_prefix(marker) {
            return Some(item);
        }
    }

    let bytes = trimmed.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 {
        return None;
    }
    if idx >= bytes.len() || bytes[idx] != b'.' {
        return None;
    }
    idx += 1;
    if idx >= bytes.len() || bytes[idx] != b' ' {
        return None;
    }
    idx += 1;
    Some(&trimmed[idx..])
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use tempfile::tempdir;

    use crate::{
        mcp::service::run_dispatch,
        runtime::workspace::{PreparedWorkspace, WorkspaceMode},
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{
                ContextMode, DelegationContextPolicy, FileConflictPolicy, MemorySource,
                NativeDiscoveryPolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy,
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

    #[test]
    fn stable_scratch_overrides_gemini_isolated_discovery_to_minimal() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let scratch = temp.path().join("scratch");
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&scratch).expect("create scratch");

        let mut spec = sample_spec(
            WorkingDirPolicy::Auto,
            vec![MemorySource::AutoProjectMemory],
        );
        spec.core.provider = Provider::Gemini;
        spec.runtime.native_discovery = NativeDiscoveryPolicy::Isolated;
        let mut prepared = PreparedWorkspace {
            source_path: source.clone(),
            workspace_path: scratch,
            mode: WorkspaceMode::StableScratch,
            notes: Vec::new(),
        };

        let effective = super::apply_workspace_runtime_overrides(&spec, &mut prepared);
        assert_eq!(
            effective.runtime.native_discovery,
            NativeDiscoveryPolicy::Minimal
        );
        assert!(prepared
            .notes
            .iter()
            .any(|note| note.contains("forces Gemini native_discovery")));
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

    #[tokio::test]
    async fn run_dispatch_attaches_plan_section_acceptance_criteria_for_reviewer() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir_all(&source).expect("create source");
        fs::write(
            source.join("PLAN.md"),
            r#"# PLAN

## Acceptance Criteria
- Must include regression coverage.
- Must mention open risks.
"#,
        )
        .expect("write plan");

        let mut spec = sample_spec(
            WorkingDirPolicy::InPlace,
            vec![MemorySource::AutoProjectMemory],
        );
        spec.runtime.delegation_context = DelegationContextPolicy::PlanSection;
        spec.runtime.plan_section_selector = Some("Acceptance Criteria".to_string());
        spec.core.tags = vec!["review".to_string()];
        let request = sample_request(source);
        let handle = "run-plan-section-acceptance";
        let state_dir = temp.path().join("state");

        let dispatch = run_dispatch(&spec, &request, handle, &state_dir, Vec::new())
            .await
            .expect("dispatch succeeds");

        let prompt = dispatch.result.compiled_context_markdown;
        assert!(
            prompt.contains("Must include regression coverage."),
            "compiled prompt missing plan-derived acceptance criteria: {prompt}"
        );
        assert!(
            prompt.contains("Must mention open risks."),
            "compiled prompt missing plan-derived acceptance criteria: {prompt}"
        );
    }
}
