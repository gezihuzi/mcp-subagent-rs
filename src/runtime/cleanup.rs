use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::runtime::workspace::{PreparedWorkspace, WorkspaceMode};

#[derive(Debug)]
pub(crate) struct WorkspaceCleanupGuard {
    mode: WorkspaceMode,
    source_path: PathBuf,
    workspace_path: PathBuf,
}

impl WorkspaceCleanupGuard {
    pub(crate) fn for_workspace(prepared: &PreparedWorkspace) -> Option<Self> {
        match prepared.mode {
            WorkspaceMode::InPlace | WorkspaceMode::StableScratch => None,
            _ => Some(Self {
                mode: prepared.mode.clone(),
                source_path: prepared.source_path.clone(),
                workspace_path: prepared.workspace_path.clone(),
            }),
        }
    }

    fn cleanup(&self) -> std::result::Result<(), String> {
        match self.mode {
            WorkspaceMode::InPlace | WorkspaceMode::StableScratch => Ok(()),
            WorkspaceMode::TempCopy | WorkspaceMode::GitWorktreeFallbackTempCopy => {
                remove_workspace_dir_if_exists(&self.workspace_path)
            }
            WorkspaceMode::GitWorktree => {
                cleanup_git_worktree(&self.source_path, &self.workspace_path)
            }
        }
    }
}

impl Drop for WorkspaceCleanupGuard {
    fn drop(&mut self) {
        if let Err(err) = self.cleanup() {
            tracing::warn!(
                workspace = %self.workspace_path.display(),
                mode = ?self.mode,
                "workspace cleanup failed: {err}"
            );
        }
    }
}

fn cleanup_git_worktree(
    source_path: &Path,
    workspace_path: &Path,
) -> std::result::Result<(), String> {
    match try_remove_git_worktree(source_path, workspace_path) {
        Ok(()) => Ok(()),
        Err(git_err) => {
            tracing::warn!(
                source = %source_path.display(),
                workspace = %workspace_path.display(),
                "git worktree cleanup failed, fallback to fs remove: {git_err}"
            );
            remove_workspace_dir_if_exists(workspace_path).map_err(|fs_err| {
                format!(
                    "git worktree remove failed ({git_err}); fallback remove_dir_all failed: {fs_err}"
                )
            })
        }
    }
}

fn try_remove_git_worktree(
    source_path: &Path,
    workspace_path: &Path,
) -> std::result::Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source_path)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(workspace_path)
        .output()
        .map_err(|err| format!("failed to execute `git worktree remove`: {err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("git worktree remove failed: {}", stderr.trim()))
}

fn remove_workspace_dir_if_exists(path: &Path) -> std::result::Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|err| {
        format!(
            "failed to remove workspace directory {}: {err}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::runtime::{
        cleanup::WorkspaceCleanupGuard,
        workspace::{PreparedWorkspace, WorkspaceMode},
    };

    #[test]
    fn in_place_workspace_has_no_cleanup_guard() {
        let temp = tempdir().expect("tempdir");
        let prepared = PreparedWorkspace {
            source_path: temp.path().to_path_buf(),
            workspace_path: temp.path().to_path_buf(),
            mode: WorkspaceMode::InPlace,
            notes: Vec::new(),
        };

        let guard = WorkspaceCleanupGuard::for_workspace(&prepared);
        assert!(guard.is_none());
    }

    #[test]
    fn stable_scratch_workspace_has_no_cleanup_guard() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let workspace = temp.path().join("scratch");
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&workspace).expect("create workspace");
        let prepared = PreparedWorkspace {
            source_path: source,
            workspace_path: workspace,
            mode: WorkspaceMode::StableScratch,
            notes: Vec::new(),
        };

        let guard = WorkspaceCleanupGuard::for_workspace(&prepared);
        assert!(guard.is_none());
    }

    #[test]
    fn temp_copy_workspace_is_removed_when_guard_drops() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(workspace.join("a.txt"), "hello").expect("write file");

        let prepared = PreparedWorkspace {
            source_path: source,
            workspace_path: workspace.clone(),
            mode: WorkspaceMode::TempCopy,
            notes: Vec::new(),
        };

        let guard = WorkspaceCleanupGuard::for_workspace(&prepared).expect("cleanup guard");
        assert!(workspace.exists());
        drop(guard);
        assert!(!workspace.exists());
    }

    #[test]
    fn git_worktree_cleanup_falls_back_to_remove_dir() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(workspace.join("a.txt"), "hello").expect("write file");

        let prepared = PreparedWorkspace {
            source_path: source,
            workspace_path: workspace.clone(),
            mode: WorkspaceMode::GitWorktree,
            notes: Vec::new(),
        };

        let guard = WorkspaceCleanupGuard::for_workspace(&prepared).expect("cleanup guard");
        assert!(workspace.exists());
        drop(guard);
        assert!(!workspace.exists());
    }
}
