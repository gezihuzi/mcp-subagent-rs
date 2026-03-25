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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectInvocation {
    pub executable: String,
    pub args: Vec<String>,
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

pub fn build_connect_invocation(
    host: ConnectHost,
    paths: &ConnectSnippetPaths,
) -> ConnectInvocation {
    match host {
        ConnectHost::Claude => ConnectInvocation {
            executable: connect_host_executable(host).to_string(),
            args: vec![
                "mcp".to_string(),
                "add".to_string(),
                "--transport".to_string(),
                "stdio".to_string(),
                "mcp-subagent".to_string(),
                "--".to_string(),
                paths.binary.display().to_string(),
                "--agents-dir".to_string(),
                paths.agents_dir.display().to_string(),
                "--state-dir".to_string(),
                paths.state_dir.display().to_string(),
                "mcp".to_string(),
            ],
        },
        ConnectHost::Codex => ConnectInvocation {
            executable: connect_host_executable(host).to_string(),
            args: vec![
                "mcp".to_string(),
                "add".to_string(),
                "mcp-subagent".to_string(),
                "--".to_string(),
                paths.binary.display().to_string(),
                "--agents-dir".to_string(),
                paths.agents_dir.display().to_string(),
                "--state-dir".to_string(),
                paths.state_dir.display().to_string(),
                "mcp".to_string(),
            ],
        },
        ConnectHost::Gemini => ConnectInvocation {
            executable: connect_host_executable(host).to_string(),
            args: vec![
                "mcp".to_string(),
                "add".to_string(),
                "mcp-subagent".to_string(),
                paths.binary.display().to_string(),
                "--agents-dir".to_string(),
                paths.agents_dir.display().to_string(),
                "--state-dir".to_string(),
                paths.state_dir.display().to_string(),
                "mcp".to_string(),
            ],
        },
    }
}

pub fn build_host_launch_invocation(host: ConnectHost) -> ConnectInvocation {
    ConnectInvocation {
        executable: connect_host_executable(host).to_string(),
        args: Vec::new(),
    }
}

pub fn connect_host_executable(host: ConnectHost) -> &'static str {
    match host {
        ConnectHost::Claude => "claude",
        ConnectHost::Codex => "codex",
        ConnectHost::Gemini => "gemini",
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
        build_connect_invocation, build_connect_snippet, build_host_launch_invocation,
        resolve_connect_snippet_paths, shell_escape_path, ConnectHost,
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

    #[test]
    fn builds_host_specific_invocations() {
        let paths = resolve_connect_snippet_paths(
            Path::new("/repo"),
            PathBuf::from("/usr/local/bin/mcp-subagent"),
            PathBuf::from("/repo/agents"),
            PathBuf::from("/repo/.mcp-subagent/state"),
        );

        let claude = build_connect_invocation(ConnectHost::Claude, &paths);
        let codex = build_connect_invocation(ConnectHost::Codex, &paths);
        let gemini = build_connect_invocation(ConnectHost::Gemini, &paths);

        assert_eq!(claude.executable, "claude");
        assert_eq!(
            claude.args,
            vec![
                "mcp",
                "add",
                "--transport",
                "stdio",
                "mcp-subagent",
                "--",
                "/usr/local/bin/mcp-subagent",
                "--agents-dir",
                "/repo/agents",
                "--state-dir",
                "/repo/.mcp-subagent/state",
                "mcp",
            ]
        );

        assert_eq!(codex.executable, "codex");
        assert!(codex.args.starts_with(&[
            "mcp".to_string(),
            "add".to_string(),
            "mcp-subagent".to_string(),
            "--".to_string(),
        ]));

        assert_eq!(gemini.executable, "gemini");
        assert_eq!(gemini.args[0], "mcp");
        assert_eq!(gemini.args[1], "add");
        assert_eq!(gemini.args[2], "mcp-subagent");
        assert_eq!(gemini.args[3], "/usr/local/bin/mcp-subagent");
    }

    #[test]
    fn builds_host_launch_invocation_for_each_host() {
        let claude = build_host_launch_invocation(ConnectHost::Claude);
        let codex = build_host_launch_invocation(ConnectHost::Codex);
        let gemini = build_host_launch_invocation(ConnectHost::Gemini);

        assert_eq!(claude.executable, "claude");
        assert!(claude.args.is_empty());
        assert_eq!(codex.executable, "codex");
        assert!(codex.args.is_empty());
        assert_eq!(gemini.executable, "gemini");
        assert!(gemini.args.is_empty());
    }
}
