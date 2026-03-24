use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    error::{McpSubagentError, Result},
    spec::{runtime_policy::WorkingDirPolicy, AgentSpec},
    types::RunRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceMode {
    InPlace,
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
    #[serde(default)]
    pub notes: Vec<String>,
}

pub fn prepare_workspace(
    spec: &AgentSpec,
    request: &RunRequest,
    state_dir: &Path,
    handle_id: &str,
) -> Result<PreparedWorkspace> {
    let source_path = resolve_source_path(&request.working_dir)?;
    match spec.runtime.working_dir_policy {
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
    copy_dir_recursively(&source_path, &workspace_path)?;
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
            copy_dir_recursively(&source_path, &workspace_path)?;
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use crate::{
        runtime::workspace::{prepare_workspace, WorkspaceMode},
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{FileConflictPolicy, RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
            AgentSpec,
        },
        types::{RunMode, RunRequest},
    };

    fn sample_spec(policy: WorkingDirPolicy) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "writer".to_string(),
                description: "write code".to_string(),
                provider: Provider::Ollama,
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
                sandbox: SandboxPolicy::WorkspaceWrite,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
        }
    }

    #[test]
    fn in_place_uses_source_directory() {
        let temp = tempdir().expect("tempdir");
        let request = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            working_dir: temp.path().to_path_buf(),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::InPlace),
            &request,
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
        let request = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            working_dir: source.clone(),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::TempCopy),
            &request,
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
        let request = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            working_dir: source,
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let prepared = prepare_workspace(
            &sample_spec(WorkingDirPolicy::GitWorktree),
            &request,
            temp.path(),
            "h3",
        )
        .expect("prepare");
        assert_eq!(prepared.mode, WorkspaceMode::GitWorktreeFallbackTempCopy);
        assert!(!prepared.notes.is_empty());
    }
}
