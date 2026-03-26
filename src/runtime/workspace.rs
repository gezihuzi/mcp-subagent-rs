use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    error::{McpSubagentError, Result},
    spec::{
        runtime_policy::{DelegationContextPolicy, SandboxPolicy, WorkingDirPolicy},
        AgentSpec, Provider,
    },
    types::{TaskSpec, WorkflowHints},
};

const GEMINI_RESEARCH_SCRATCH_ENV: &str = "MCP_SUBAGENT_GEMINI_RESEARCH_SCRATCH_DIR";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceMode {
    InPlace,
    StableScratch,
    TempCopy,
    GitWorktree,
    GitWorktreeFallbackTempCopy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedWorkspace {
    pub source_path: PathBuf,
    pub workspace_path: PathBuf,
    pub mode: WorkspaceMode,
    pub notes: Vec<String>,
}

pub fn prepare_workspace(
    spec: &AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    state_dir: &Path,
    handle_id: &str,
) -> Result<PreparedWorkspace> {
    let source_path = resolve_source_path(&task_spec.working_dir)?;
    match spec.runtime.working_dir_policy {
        WorkingDirPolicy::Auto => {
            prepare_auto_workspace(spec, task_spec, hints, source_path, state_dir, handle_id)
        }
        WorkingDirPolicy::InPlace => Ok(PreparedWorkspace {
            source_path: source_path.clone(),
            workspace_path: source_path,
            mode: WorkspaceMode::InPlace,
            notes: Vec::new(),
        }),
        WorkingDirPolicy::TempCopy => {
            prepare_temp_copy_workspace(source_path, state_dir, handle_id)
        }
        WorkingDirPolicy::GitWorktree => {
            prepare_git_worktree_workspace(source_path, state_dir, handle_id)
        }
    }
}

fn prepare_auto_workspace(
    spec: &AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    source_path: PathBuf,
    state_dir: &Path,
    handle_id: &str,
) -> Result<PreparedWorkspace> {
    prepare_auto_workspace_with_scratch_override(
        spec,
        task_spec,
        hints,
        source_path,
        state_dir,
        handle_id,
        None,
    )
}

fn prepare_auto_workspace_with_scratch_override(
    spec: &AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    source_path: PathBuf,
    state_dir: &Path,
    handle_id: &str,
    scratch_override: Option<&Path>,
) -> Result<PreparedWorkspace> {
    let stage = hints
        .stage
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let read_stage = matches!(stage.as_str(), "research" | "plan");
    let read_only = matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly);

    if let Some(prepared) = maybe_prepare_gemini_research_scratch_workspace(
        spec,
        task_spec,
        hints,
        &source_path,
        &stage,
        read_only,
        scratch_override,
    )? {
        return Ok(prepared);
    }

    if read_only || read_stage {
        let mut notes = Vec::new();
        if read_only {
            notes.push("auto policy selected in-place workspace for read-only sandbox".to_string());
        }
        if read_stage {
            notes.push(format!(
                "auto policy selected in-place workspace for stage `{}`",
                hints.stage.as_deref().unwrap_or_default()
            ));
        }
        return Ok(PreparedWorkspace {
            source_path: source_path.clone(),
            workspace_path: source_path,
            mode: WorkspaceMode::InPlace,
            notes,
        });
    }

    prepare_git_worktree_workspace(source_path, state_dir, handle_id)
}

fn maybe_prepare_gemini_research_scratch_workspace(
    spec: &AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    source_path: &Path,
    stage: &str,
    read_only: bool,
    scratch_override: Option<&Path>,
) -> Result<Option<PreparedWorkspace>> {
    if !should_use_gemini_research_scratch(spec, task_spec, hints, stage, read_only) {
        return Ok(None);
    }

    let workspace_path = resolve_stable_gemini_research_scratch_dir(scratch_override)?;
    let notes = vec![
        format!(
            "auto policy selected stable scratch workspace for Gemini research-only task: {}",
            workspace_path.display()
        ),
        format!("original working_dir: {}", source_path.display()),
    ];

    Ok(Some(PreparedWorkspace {
        source_path: source_path.to_path_buf(),
        workspace_path,
        mode: WorkspaceMode::StableScratch,
        notes,
    }))
}

fn should_use_gemini_research_scratch(
    spec: &AgentSpec,
    task_spec: &TaskSpec,
    hints: &WorkflowHints,
    stage: &str,
    read_only: bool,
) -> bool {
    if !matches!(spec.core.provider, Provider::Gemini) {
        return false;
    }
    if !read_only {
        return false;
    }
    if !matches!(
        spec.runtime.delegation_context,
        DelegationContextPolicy::Minimal
    ) {
        return false;
    }
    if !task_spec.selected_files.is_empty() || hints.plan_ref.is_some() {
        return false;
    }

    let stage_research = matches!(stage, "research" | "plan");
    let tag_research = spec
        .core
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("research"));
    stage_research || tag_research
}

fn resolve_stable_gemini_research_scratch_dir(override_path: Option<&Path>) -> Result<PathBuf> {
    let cwd = std::env::current_dir().map_err(McpSubagentError::Io)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let env_override = std::env::var(GEMINI_RESEARCH_SCRATCH_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    resolve_stable_gemini_research_scratch_dir_with(
        override_path,
        env_override.as_deref(),
        &cwd,
        home.as_deref(),
    )
}

fn resolve_stable_gemini_research_scratch_dir_with(
    override_path: Option<&Path>,
    env_override: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
) -> Result<PathBuf> {
    let mut candidate = if let Some(path) = override_path {
        path.to_path_buf()
    } else if let Some(path) = env_override {
        PathBuf::from(path)
    } else if let Some(home_dir) = home {
        home_dir
            .join(".mcp-subagent")
            .join("provider-workspaces")
            .join("gemini")
            .join("research")
    } else {
        cwd.join(".mcp-subagent")
            .join("provider-workspaces")
            .join("gemini")
            .join("research")
    };

    if !candidate.is_absolute() {
        candidate = cwd.join(candidate);
    }
    fs::create_dir_all(&candidate).map_err(McpSubagentError::Io)?;
    candidate.canonicalize().map_err(|err| {
        McpSubagentError::Io(std::io::Error::new(
            err.kind(),
            format!(
                "failed to resolve stable Gemini scratch workspace ({}): {err}",
                candidate.display()
            ),
        ))
    })
}

pub fn resolve_source_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let normalized = absolute.canonicalize().map_err(|err| {
        McpSubagentError::Io(std::io::Error::new(
            err.kind(),
            format!(
                "working_dir does not exist or is inaccessible ({}): {err}",
                absolute.display()
            ),
        ))
    })?;
    Ok(normalized)
}

fn prepare_temp_copy_workspace(
    source_path: PathBuf,
    state_dir: &Path,
    handle_id: &str,
) -> Result<PreparedWorkspace> {
    let workspace_path = state_dir.join("runs").join(handle_id).join("workspace");
    if workspace_path.exists() {
        fs::remove_dir_all(&workspace_path)?;
    }
    if let Err(err) = copy_dir_recursively(&source_path, &workspace_path) {
        remove_dir_if_exists_best_effort(&workspace_path);
        return Err(err);
    }
    Ok(PreparedWorkspace {
        source_path,
        workspace_path,
        mode: WorkspaceMode::TempCopy,
        notes: Vec::new(),
    })
}

fn prepare_git_worktree_workspace(
    source_path: PathBuf,
    state_dir: &Path,
    handle_id: &str,
) -> Result<PreparedWorkspace> {
    let workspace_path = state_dir.join("runs").join(handle_id).join("workspace");
    if workspace_path.exists() {
        fs::remove_dir_all(&workspace_path)?;
    }

    match try_create_git_worktree(&source_path, &workspace_path) {
        Ok(()) => Ok(PreparedWorkspace {
            source_path,
            workspace_path,
            mode: WorkspaceMode::GitWorktree,
            notes: Vec::new(),
        }),
        Err(reason) => {
            if let Err(err) = copy_dir_recursively(&source_path, &workspace_path) {
                remove_dir_if_exists_best_effort(&workspace_path);
                return Err(err);
            }
            Ok(PreparedWorkspace {
                source_path,
                workspace_path,
                mode: WorkspaceMode::GitWorktreeFallbackTempCopy,
                notes: vec![format!("git_worktree fallback to temp_copy: {reason}")],
            })
        }
    }
}

fn try_create_git_worktree(
    source_path: &Path,
    workspace_path: &Path,
) -> std::result::Result<(), String> {
    let inside_git = Command::new("git")
        .arg("-C")
        .arg(source_path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .map_err(|err| format!("failed to execute git probe: {err}"))?;
    if !inside_git.status.success() {
        let stderr = String::from_utf8_lossy(&inside_git.stderr);
        return Err(format!(
            "not a git worktree root or git unavailable: {}",
            stderr.trim()
        ));
    }

    let add = Command::new("git")
        .arg("-C")
        .arg(source_path)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(workspace_path)
        .arg("HEAD")
        .output()
        .map_err(|err| format!("failed to execute git worktree add: {err}"))?;
    if !add.status.success() {
        let stderr = String::from_utf8_lossy(&add.stderr);
        return Err(format!("git worktree add failed: {}", stderr.trim()));
    }
    Ok(())
}

fn copy_dir_recursively(source_path: &Path, destination_path: &Path) -> Result<()> {
    fs::create_dir_all(destination_path)?;
    for entry in WalkDir::new(source_path).follow_links(false) {
        let entry = entry.map_err(|err| {
            McpSubagentError::Io(std::io::Error::other(format!(
                "failed to walk source directory {}: {err}",
                source_path.display()
            )))
        })?;
        let src = entry.path();
        let rel = src.strip_prefix(source_path).map_err(|err| {
            McpSubagentError::Io(std::io::Error::other(format!(
                "failed to strip source prefix {} from {}: {err}",
                source_path.display(),
                src.display()
            )))
        })?;
        if rel.as_os_str().is_empty() {
            continue;
        }

        let dst = destination_path.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst)?;
            continue;
        }

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, &dst)?;
    }
    Ok(())
}

fn remove_dir_if_exists_best_effort(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = fs::remove_dir_all(path);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use crate::{
        runtime::workspace::{
            prepare_auto_workspace_with_scratch_override, prepare_workspace, WorkspaceMode,
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{
                DelegationContextPolicy, FileConflictPolicy, RuntimePolicy, SandboxPolicy,
                WorkingDirPolicy,
            },
            AgentSpec,
        },
        types::{RunMode, TaskSpec, WorkflowHints},
    };

    fn sample_spec(policy: WorkingDirPolicy) -> AgentSpec {
        sample_spec_with_provider(policy, SandboxPolicy::WorkspaceWrite, Provider::Mock)
    }

    fn sample_spec_with_sandbox(policy: WorkingDirPolicy, sandbox: SandboxPolicy) -> AgentSpec {
        sample_spec_with_provider(policy, sandbox, Provider::Mock)
    }

    fn sample_spec_with_provider(
        policy: WorkingDirPolicy,
        sandbox: SandboxPolicy,
        provider: Provider,
    ) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "writer".to_string(),
                description: "write code".to_string(),
                provider,
                model: None,
                instructions: "do work".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: HashMap::new(),
            },
            runtime: RuntimePolicy {
                working_dir_policy: policy,
                file_conflict_policy: FileConflictPolicy::Serialize,
                sandbox,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    fn sample_task_spec(working_dir: std::path::PathBuf) -> TaskSpec {
        TaskSpec {
            task: "task".to_string(),
            task_brief: None,
            acceptance_criteria: Vec::new(),
            selected_files: Vec::new(),
            working_dir,
        }
    }

    fn sample_hints(stage: Option<&str>) -> WorkflowHints {
        WorkflowHints {
            stage: stage.map(str::to_string),
            run_mode: RunMode::Sync,
            ..WorkflowHints::default()
        }
    }

    #[test]
    fn in_place_uses_source_directory() {
        let temp = tempdir().expect("tempdir");
        let task_spec = sample_task_spec(temp.path().to_path_buf());
        let hints = sample_hints(None);

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::InPlace),
            &task_spec,
            &hints,
            temp.path(),
            "h1",
        )
        .expect("prepare");
        assert_eq!(prepared.mode, WorkspaceMode::InPlace);
        assert_eq!(
            prepared.workspace_path,
            temp.path().canonicalize().expect("canon")
        );
    }

    #[test]
    fn temp_copy_creates_isolated_workspace() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("a.txt"), "hello").expect("write source");
        let task_spec = sample_task_spec(source.clone());
        let hints = sample_hints(None);

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::TempCopy),
            &task_spec,
            &hints,
            temp.path(),
            "h2",
        )
        .expect("prepare");
        assert_eq!(prepared.mode, WorkspaceMode::TempCopy);
        assert!(prepared.workspace_path.join("a.txt").exists());
    }

    #[test]
    fn git_worktree_falls_back_for_non_git_directory() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).expect("create source");
        let task_spec = sample_task_spec(source);
        let hints = sample_hints(None);

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::GitWorktree),
            &task_spec,
            &hints,
            temp.path(),
            "h3",
        )
        .expect("prepare");
        assert_eq!(prepared.mode, WorkspaceMode::GitWorktreeFallbackTempCopy);
        assert!(!prepared.notes.is_empty());
    }

    #[test]
    fn auto_policy_uses_in_place_for_read_only_task() {
        let temp = tempdir().expect("tempdir");
        let task_spec = sample_task_spec(temp.path().to_path_buf());
        let hints = sample_hints(Some("research"));

        let prepared = prepare_workspace(
            &sample_spec_with_sandbox(WorkingDirPolicy::Auto, SandboxPolicy::ReadOnly),
            &task_spec,
            &hints,
            temp.path(),
            "h4",
        )
        .expect("prepare");

        assert_eq!(prepared.mode, WorkspaceMode::InPlace);
        assert!(prepared
            .notes
            .iter()
            .any(|note| note.contains("read-only sandbox")));
    }

    #[test]
    fn auto_policy_prefers_worktree_for_write_task() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).expect("create source");
        let task_spec = sample_task_spec(source);
        let hints = sample_hints(Some("build"));

        let prepared = prepare_workspace(
            &sample_spec_with_sandbox(WorkingDirPolicy::Auto, SandboxPolicy::WorkspaceWrite),
            &task_spec,
            &hints,
            temp.path(),
            "h5",
        )
        .expect("prepare");

        assert!(matches!(
            prepared.mode,
            WorkspaceMode::GitWorktree | WorkspaceMode::GitWorktreeFallbackTempCopy
        ));
    }

    #[test]
    fn auto_policy_routes_gemini_research_profile_to_stable_scratch() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let scratch = temp.path().join("scratch").join("gemini-research");
        std::fs::create_dir_all(&source).expect("create source");

        let mut task_spec = sample_task_spec(source.clone());
        task_spec.task = "Search official website".to_string();
        let hints = sample_hints(Some("research"));

        let mut spec = sample_spec_with_provider(
            WorkingDirPolicy::Auto,
            SandboxPolicy::ReadOnly,
            Provider::Gemini,
        );
        spec.runtime.delegation_context = DelegationContextPolicy::Minimal;
        spec.core.tags = vec!["research".to_string()];

        let prepared = prepare_auto_workspace_with_scratch_override(
            &spec,
            &task_spec,
            &hints,
            source.canonicalize().expect("source canonicalized"),
            temp.path(),
            "h6",
            Some(&scratch),
        )
        .expect("prepare");

        assert_eq!(prepared.mode, WorkspaceMode::StableScratch);
        assert_eq!(
            prepared.workspace_path,
            scratch.canonicalize().expect("scratch canonicalized")
        );
        assert_eq!(
            prepared.source_path,
            source.canonicalize().expect("source canonicalized")
        );
        assert!(prepared
            .notes
            .iter()
            .any(|note| note.contains("stable scratch workspace")));
    }

    #[test]
    fn auto_policy_keeps_in_place_when_gemini_research_has_selected_files() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let scratch = temp.path().join("scratch").join("gemini-research");
        std::fs::create_dir_all(&source).expect("create source");

        let mut task_spec = sample_task_spec(source.clone());
        task_spec.task = "Inspect src/lib.rs".to_string();
        task_spec.selected_files = vec![crate::types::SelectedFile {
            path: std::path::PathBuf::from("src/lib.rs"),
            rationale: None,
            content: None,
        }];
        let hints = sample_hints(Some("research"));

        let mut spec = sample_spec_with_provider(
            WorkingDirPolicy::Auto,
            SandboxPolicy::ReadOnly,
            Provider::Gemini,
        );
        spec.runtime.delegation_context = DelegationContextPolicy::Minimal;
        spec.core.tags = vec!["research".to_string()];

        let prepared = prepare_auto_workspace_with_scratch_override(
            &spec,
            &task_spec,
            &hints,
            source.canonicalize().expect("source canonicalized"),
            temp.path(),
            "h7",
            Some(&scratch),
        )
        .expect("prepare");

        assert_eq!(prepared.mode, WorkspaceMode::InPlace);
        assert_eq!(
            prepared.workspace_path,
            source.canonicalize().expect("source canonicalized")
        );
    }

    #[test]
    fn resolve_stable_gemini_scratch_dir_uses_home_when_unset() {
        let temp = tempdir().expect("tempdir");
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(&home).expect("create home");

        let resolved =
            super::resolve_stable_gemini_research_scratch_dir_with(None, None, &cwd, Some(&home))
                .expect("resolve scratch");
        let expected = home
            .join(".mcp-subagent")
            .join("provider-workspaces")
            .join("gemini")
            .join("research")
            .canonicalize()
            .expect("canonicalize expected");
        assert_eq!(resolved, expected);
    }
}
