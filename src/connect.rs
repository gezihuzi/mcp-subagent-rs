use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectHost {
    Claude,
    Codex,
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectSnippetPaths {
    pub binary: PathBuf,
    pub agents_dir: PathBuf,
    pub state_dir: PathBuf,
}

pub fn resolve_connect_snippet_paths(
    cwd: &Path,
    binary: PathBuf,
    agents_dir: PathBuf,
    state_dir: PathBuf,
) -> ConnectSnippetPaths {
    ConnectSnippetPaths {
        binary: absolutize(cwd, binary),
        agents_dir: absolutize(cwd, agents_dir),
        state_dir: absolutize(cwd, state_dir),
    }
}

pub fn build_connect_snippet(host: ConnectHost, paths: &ConnectSnippetPaths) -> String {
    let binary = shell_escape_path(&paths.binary);
    let agents_dir = shell_escape_path(&paths.agents_dir);
    let state_dir = shell_escape_path(&paths.state_dir);

    match host {
        ConnectHost::Claude => format!(
            "claude mcp add --transport stdio mcp-subagent -- \\\n  {binary} \\\n  --agents-dir {agents_dir} \\\n  --state-dir {state_dir} \\\n  mcp"
        ),
        ConnectHost::Codex => format!(
            "codex mcp add mcp-subagent -- \\\n  {binary} \\\n  --agents-dir {agents_dir} \\\n  --state-dir {state_dir} \\\n  mcp"
        ),
        ConnectHost::Gemini => format!(
            "gemini mcp add mcp-subagent \\\n  {binary} \\\n  --agents-dir {agents_dir} \\\n  --state-dir {state_dir} \\\n  mcp"
        ),
    }
}

pub fn shell_escape_path(path: &Path) -> String {
    shell_escape(path.to_string_lossy().as_ref())
}

fn shell_escape(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn absolutize(base: &Path, path: PathBuf) -> PathBuf {
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    normalize_lexically(path)
}

fn normalize_lexically(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        build_connect_snippet, resolve_connect_snippet_paths, shell_escape_path, ConnectHost,
    };

    #[test]
    fn resolves_relative_paths_to_absolute() {
        let cwd = Path::new("/tmp/workspace");
        let paths = resolve_connect_snippet_paths(
            cwd,
            PathBuf::from("bin/mcp-subagent"),
            PathBuf::from("./agents"),
            PathBuf::from(".mcp-subagent/state"),
        );

        assert_eq!(
            paths.binary,
            PathBuf::from("/tmp/workspace/bin/mcp-subagent")
        );
        assert_eq!(paths.agents_dir, PathBuf::from("/tmp/workspace/agents"));
        assert_eq!(
            paths.state_dir,
            PathBuf::from("/tmp/workspace/.mcp-subagent/state")
        );
    }

    #[test]
    fn shell_escape_handles_spaces_and_single_quotes() {
        let escaped = shell_escape_path(Path::new("/tmp/with space/agent's/bin"));
        assert_eq!(escaped, "'/tmp/with space/agent'\"'\"'s/bin'");
    }

    #[test]
    fn builds_host_specific_snippets() {
        let paths = resolve_connect_snippet_paths(
            Path::new("/repo"),
            PathBuf::from("/usr/local/bin/mcp-subagent"),
            PathBuf::from("/repo/agents"),
            PathBuf::from("/repo/.mcp-subagent/state"),
        );

        let claude = build_connect_snippet(ConnectHost::Claude, &paths);
        let codex = build_connect_snippet(ConnectHost::Codex, &paths);
        let gemini = build_connect_snippet(ConnectHost::Gemini, &paths);

        assert!(claude.starts_with("claude mcp add --transport stdio mcp-subagent --"));
        assert!(codex.starts_with("codex mcp add mcp-subagent --"));
        assert!(gemini.starts_with("gemini mcp add mcp-subagent"));
        assert!(claude.contains("'/usr/local/bin/mcp-subagent'"));
        assert!(codex.contains("--agents-dir '/repo/agents'"));
        assert!(gemini.contains("--state-dir '/repo/.mcp-subagent/state'"));
    }
}
