use std::{
    fs,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    connect::{
        build_connect_snippet, resolve_connect_snippet_paths, ConnectHost, ConnectSnippetPaths,
    },
    error::Result,
    spec::registry::load_agent_specs_from_dirs,
};

const PRESET_CATALOG_VERSION: &str = "v0.8.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitPreset {
    ClaudeOpusSupervisor,
    CodexPrimaryBuilder,
    GeminiFrontendTeam,
    LocalOllamaFallback,
    MinimalSingleProvider,
}

impl InitPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeOpusSupervisor => "claude-opus-supervisor",
            Self::CodexPrimaryBuilder => "codex-primary-builder",
            Self::GeminiFrontendTeam => "gemini-frontend-team",
            Self::LocalOllamaFallback => "local-ollama-fallback",
            Self::MinimalSingleProvider => "minimal-single-provider",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InitReport {
    pub preset: String,
    pub preset_catalog_version: String,
    pub root: PathBuf,
    pub agents_dir: PathBuf,
    pub created_files: Vec<PathBuf>,
    pub overwritten_files: Vec<PathBuf>,
    pub generated_agent_count: usize,
    pub notes: Vec<String>,
}

pub fn init_workspace(root: &Path, preset: InitPreset, force: bool) -> Result<InitReport> {
    init_with_preset(root, preset, force)
}

fn init_with_preset(root: &Path, preset: InitPreset, force: bool) -> Result<InitReport> {
    let root = root.to_path_buf();
    let agents_dir = root.join("agents");
    let config_path = root.join(".mcp-subagent/config.toml");
    let readme_path = root.join("README.mcp-subagent.md");
    let plan_path = root.join("PLAN.md");
    let agent_templates = preset_agent_templates(preset);

    let mut files = vec![plan_path.clone(), config_path.clone(), readme_path.clone()];
    files.extend(
        agent_templates
            .iter()
            .map(|(name, _)| agents_dir.join(name))
            .collect::<Vec<_>>(),
    );

    let mut overwritten_files = Vec::new();
    if !force {
        if let Some(existing) = files.iter().find(|path| path.exists()) {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "refusing to overwrite existing file: {} (use --force)",
                    existing.display()
                ),
            )
            .into());
        }
    } else {
        overwritten_files.extend(files.iter().filter(|path| path.exists()).cloned());
    }

    for path in &files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }

    write(&plan_path, &plan_template())?;
    write(&config_path, &config_template())?;
    let cwd = std::env::current_dir()?;
    let binary = std::env::current_exe()?;
    let connect_paths = resolve_connect_snippet_paths(
        &cwd,
        binary,
        agents_dir.clone(),
        root.join(".mcp-subagent/state"),
    );
    write(&readme_path, &readme_template(preset, &connect_paths))?;
    for (name, content) in &agent_templates {
        write(&agents_dir.join(name), content)?;
    }

    let generated = load_agent_specs_from_dirs(std::slice::from_ref(&agents_dir))?;
    let state_dir = root.join(".mcp-subagent/state");
    let validate_note = format!(
        "Run `mcp-subagent validate --agents-dir {}` to verify generated specs.",
        agents_dir.display()
    );
    let doctor_note = format!(
        "Run `mcp-subagent doctor --agents-dir {} --state-dir {}` to inspect provider readiness.",
        agents_dir.display(),
        state_dir.display()
    );

    Ok(InitReport {
        preset: preset.as_str().to_string(),
        preset_catalog_version: PRESET_CATALOG_VERSION.to_string(),
        root: root.clone(),
        agents_dir,
        created_files: files,
        overwritten_files,
        generated_agent_count: generated.len(),
        notes: vec![
            format!("Preset catalog version: {PRESET_CATALOG_VERSION}"),
            validate_note,
            doctor_note,
            "Use `mcp-subagent mcp` for stdio MCP transport.".to_string(),
        ],
    })
}

fn preset_agent_templates(preset: InitPreset) -> Vec<(&'static str, &'static str)> {
    match preset {
        InitPreset::ClaudeOpusSupervisor => vec![
            ("fast-researcher.agent.toml", FAST_RESEARCHER_AGENT),
            ("backend-coder.agent.toml", BACKEND_CODER_AGENT),
            ("frontend-builder.agent.toml", FRONTEND_BUILDER_AGENT),
            (
                "correctness-reviewer.agent.toml",
                CORRECTNESS_REVIEWER_AGENT,
            ),
            ("style-reviewer.agent.toml", STYLE_REVIEWER_AGENT),
            (
                "local-fallback-coder.agent.toml",
                LOCAL_FALLBACK_CODER_AGENT,
            ),
        ],
        InitPreset::CodexPrimaryBuilder => vec![
            ("backend-coder.agent.toml", BACKEND_CODER_AGENT),
            (
                "correctness-reviewer.agent.toml",
                CORRECTNESS_REVIEWER_AGENT,
            ),
            (
                "codex-style-reviewer.agent.toml",
                CODEX_STYLE_REVIEWER_AGENT,
            ),
        ],
        InitPreset::GeminiFrontendTeam => vec![
            ("fast-researcher.agent.toml", FAST_RESEARCHER_AGENT),
            ("frontend-builder.agent.toml", FRONTEND_BUILDER_AGENT),
            (
                "gemini-style-reviewer.agent.toml",
                GEMINI_STYLE_REVIEWER_AGENT,
            ),
        ],
        InitPreset::LocalOllamaFallback => vec![
            (
                "local-fallback-coder.agent.toml",
                LOCAL_FALLBACK_CODER_AGENT,
            ),
            ("fast-researcher.agent.toml", FAST_RESEARCHER_AGENT),
        ],
        InitPreset::MinimalSingleProvider => {
            vec![(
                "single-provider-coder.agent.toml",
                SINGLE_PROVIDER_CODER_AGENT,
            )]
        }
    }
}

fn write(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content)?;
    Ok(())
}

fn plan_template() -> String {
    r#"# PLAN.md

## Objective

State the target outcome in one sentence.

## Scope

- In scope:
- Out of scope:

## Stages

1. Research: gather facts and risks
2. Plan: update this file with concrete steps
3. Build: execute small, reviewable changes
4. Review: correctness + style checks
5. Archive: persist decisions and summaries

## Steps

1. [ ] Step 1
2. [ ] Step 2
3. [ ] Step 3

## Risks

- Risk:
- Mitigation:

## Validation

- `cargo test -q`
- `cargo run -- --agents-dir ./agents validate`
"#
    .to_string()
}

fn config_template() -> String {
    r#"[server]
transport = "stdio"
log_level = "info"

[paths]
agents_dirs = ["./agents"]
state_dir = ".mcp-subagent/state"
"#
    .to_string()
}

fn readme_template(preset: InitPreset, connect_paths: &ConnectSnippetPaths) -> String {
    let claude_snippet = build_connect_snippet(ConnectHost::Claude, connect_paths);
    let codex_snippet = build_connect_snippet(ConnectHost::Codex, connect_paths);
    let gemini_snippet = build_connect_snippet(ConnectHost::Gemini, connect_paths);

    format!(
        r#"# README.mcp-subagent

This workspace was initialized by:

```bash
mcp-subagent init --preset {}
```

Catalog version: `{}`

Available presets:

- `claude-opus-supervisor`
- `codex-primary-builder`
- `gemini-frontend-team`
- `local-ollama-fallback`
- `minimal-single-provider`

## Quick Start

```bash
mcp-subagent validate --agents-dir ./agents
mcp-subagent doctor --agents-dir ./agents
mcp-subagent list-agents --agents-dir ./agents
```

## MCP Integration (stdio)

Claude Code:

```bash
{}
```

Codex CLI:

```bash
{}
```

Gemini CLI:

```bash
{}
```

Apply integration directly at any time:

```bash
mcp-subagent connect --host claude
mcp-subagent connect --host codex
mcp-subagent connect --host gemini
```

Or print connect snippets only:

```bash
mcp-subagent connect-snippet --host claude
mcp-subagent connect-snippet --host codex
mcp-subagent connect-snippet --host gemini
```
"#,
        preset.as_str(),
        PRESET_CATALOG_VERSION,
        claude_snippet,
        codex_snippet,
        gemini_snippet,
    )
}

const FAST_RESEARCHER_AGENT: &str = r#"[core]
name = "fast-researcher"
description = "Fast read-only investigator for dependency mapping and risk discovery."
provider = "gemini"
model = "flash"
instructions = "You are a read-only research specialist. Do not edit files. Return concise evidence-based summaries."
tags = ["research", "read-only", "fast"]

[runtime]
context_mode = "expanded_brief"
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "provider_default"
timeout_secs = 600
spawn_policy = "sync"

[workflow]
enabled = true
stages = ["research", "plan"]
max_runtime_depth = 1
"#;

const BACKEND_CODER_AGENT: &str = r#"[core]
name = "backend-coder"
description = "Implements backend and Rust changes from an approved plan."
provider = "codex"
model = "gpt-5.3-codex"
instructions = "Implement scoped changes from PLAN.md. Keep diffs minimal and reference plan steps in summary."
tags = ["build", "backend", "rust", "codex"]

[runtime]
context_mode = { selected_files = ["src/**", "Cargo.toml", "PLAN.md"] }
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 1200
spawn_policy = "async"

[provider_overrides.codex]
model_reasoning_effort = "medium"

[workflow]
enabled = true
stages = ["build", "review"]
max_runtime_depth = 1
"#;

const FRONTEND_BUILDER_AGENT: &str = r#"[core]
name = "frontend-builder"
description = "Implements frontend and UI changes from an approved plan."
provider = "gemini"
model = "pro"
instructions = "Implement frontend changes from PLAN.md with usable, reviewable diffs."
tags = ["build", "frontend", "ui", "gemini"]

[runtime]
context_mode = { selected_files = ["web/**", "src/**", "package.json", "PLAN.md"] }
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "provider_default"
timeout_secs = 1200
spawn_policy = "async"

[provider_overrides.gemini]
experimental_subagents = true

[workflow]
enabled = true
stages = ["build", "review"]
max_runtime_depth = 1
"#;

const CORRECTNESS_REVIEWER_AGENT: &str = r#"[core]
name = "correctness-reviewer"
description = "Reviews logic, regressions, edge cases, and verification claims."
provider = "codex"
model = "gpt-5.3-codex"
instructions = "Audit logic, regression risk, verification gaps, and plan compliance with explicit evidence."
tags = ["review", "correctness", "codex"]

[runtime]
context_mode = "summary_only"
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
spawn_policy = "sync"

[provider_overrides.codex]
model_reasoning_effort = "high"

[workflow]
enabled = true
stages = ["review"]
max_runtime_depth = 1
"#;

const STYLE_REVIEWER_AGENT: &str = r#"[core]
name = "style-reviewer"
description = "Reviews maintainability, naming, readability, and consistency."
provider = "claude"
model = "sonnet"
instructions = "Review maintainability and style. Do not claim certainty without evidence."
tags = ["review", "style", "claude"]

[runtime]
context_mode = "summary_only"
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
spawn_policy = "sync"

[workflow]
enabled = true
stages = ["review", "archive"]
max_runtime_depth = 1
"#;

const CODEX_STYLE_REVIEWER_AGENT: &str = r#"[core]
name = "codex-style-reviewer"
description = "Codex reviewer focused on style and maintainability."
provider = "codex"
model = "gpt-5.3-codex"
instructions = "Review maintainability, naming, readability, and consistency with concrete evidence."
tags = ["review", "style", "codex"]

[runtime]
context_mode = "summary_only"
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
spawn_policy = "sync"

[provider_overrides.codex]
model_reasoning_effort = "high"

[workflow]
enabled = true
stages = ["review"]
max_runtime_depth = 1
"#;

const GEMINI_STYLE_REVIEWER_AGENT: &str = r#"[core]
name = "gemini-style-reviewer"
description = "Gemini reviewer for frontend style and maintainability checks."
provider = "gemini"
model = "pro"
instructions = "Review style, readability, and maintainability with short actionable findings."
tags = ["review", "style", "gemini"]

[runtime]
context_mode = "summary_only"
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "provider_default"
timeout_secs = 900
spawn_policy = "sync"

[workflow]
enabled = true
stages = ["review"]
max_runtime_depth = 1
"#;

const LOCAL_FALLBACK_CODER_AGENT: &str = r#"[core]
name = "local-fallback-coder"
description = "Optional local fallback coding agent backed by Ollama."
provider = "ollama"
model = "qwen2.5-coder"
instructions = "Implement small scoped changes. If uncertain, return open questions."
tags = ["build", "local", "ollama", "fallback"]

[runtime]
context_mode = { selected_files = ["src/**", "PLAN.md"] }
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 1200
spawn_policy = "async"

[workflow]
enabled = true
stages = ["build"]
max_runtime_depth = 1
"#;

const SINGLE_PROVIDER_CODER_AGENT: &str = r#"[core]
name = "single-provider-coder"
description = "Minimal single-provider coder for small workflows."
provider = "codex"
model = "gpt-5.3-codex"
instructions = "Implement scoped changes and return concise structured summary."
tags = ["build", "codex", "minimal"]

[runtime]
context_mode = { selected_files = ["src/**", "PLAN.md"] }
memory_sources = ["auto_project_memory", "active_plan"]
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 900
spawn_policy = "sync"

[workflow]
enabled = true
stages = ["build", "review"]
max_runtime_depth = 1
"#;

#[cfg(test)]
mod tests {
    use std::{fs, io::ErrorKind};

    use tempfile::tempdir;

    use crate::connect::{build_connect_snippet, resolve_connect_snippet_paths, ConnectHost};

    use super::{init_workspace, InitPreset};

    #[test]
    fn init_creates_preset_files_and_valid_specs() {
        let dir = tempdir().expect("tempdir");
        let report = init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisor, false)
            .expect("init succeeds");

        assert_eq!(report.generated_agent_count, 6);
        assert_eq!(report.preset_catalog_version, "v0.8.1");
        assert!(dir.path().join("agents").exists());
        assert!(dir.path().join("PLAN.md").exists());
        assert!(dir.path().join(".mcp-subagent/config.toml").exists());
        assert!(dir.path().join("README.mcp-subagent.md").exists());
    }

    #[test]
    fn init_supports_all_presets_and_validates() {
        for preset in [
            InitPreset::ClaudeOpusSupervisor,
            InitPreset::CodexPrimaryBuilder,
            InitPreset::GeminiFrontendTeam,
            InitPreset::LocalOllamaFallback,
            InitPreset::MinimalSingleProvider,
        ] {
            let dir = tempdir().expect("tempdir");
            let report = init_workspace(dir.path(), preset, false).expect("init preset");
            assert!(
                report.generated_agent_count >= 1,
                "preset {} should generate at least one agent",
                preset.as_str()
            );
            assert_eq!(report.preset_catalog_version, "v0.8.1");
            assert!(dir.path().join("README.mcp-subagent.md").exists());
        }
    }

    #[test]
    fn init_refuses_to_overwrite_without_force() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("agents")).expect("create agents");
        fs::write(dir.path().join("agents/backend-coder.agent.toml"), "seed").expect("seed file");

        let err = init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisor, false)
            .expect_err("must fail on existing files");
        assert!(
            matches!(
                err,
                crate::error::McpSubagentError::Io(ref io) if io.kind() == ErrorKind::AlreadyExists
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn init_force_overwrites_existing_files() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("agents")).expect("create agents");
        fs::write(
            dir.path().join("agents/backend-coder.agent.toml"),
            "old-content",
        )
        .expect("seed file");

        let report = init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisor, true)
            .expect("init with force");
        let content =
            fs::read_to_string(dir.path().join("agents/backend-coder.agent.toml")).expect("read");
        assert!(content.contains("provider = \"codex\""));
        assert!(report
            .overwritten_files
            .iter()
            .any(|path| path.ends_with("agents/backend-coder.agent.toml")));
    }

    #[test]
    fn init_readme_contains_executable_connect_snippets() {
        let dir = tempdir().expect("tempdir");
        init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisor, false).expect("init succeeds");

        let readme = fs::read_to_string(dir.path().join("README.mcp-subagent.md"))
            .expect("read generated readme");
        assert!(!readme.contains("<ABSOLUTE_PATH_TO_"));
        assert!(readme.contains("claude mcp add --transport stdio mcp-subagent --"));
        assert!(readme.contains("codex mcp add mcp-subagent --"));
        assert!(readme.contains("gemini mcp add mcp-subagent"));

        let cwd = std::env::current_dir().expect("cwd");
        let binary = std::env::current_exe().expect("current exe");
        let paths = resolve_connect_snippet_paths(
            &cwd,
            binary,
            dir.path().join("agents"),
            dir.path().join(".mcp-subagent/state"),
        );
        assert!(readme.contains(&build_connect_snippet(ConnectHost::Claude, &paths)));
        assert!(readme.contains(&build_connect_snippet(ConnectHost::Codex, &paths)));
        assert!(readme.contains(&build_connect_snippet(ConnectHost::Gemini, &paths)));
    }
}
