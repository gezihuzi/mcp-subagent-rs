use std::{
    fs,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    connect::{
        build_connect_snippet, resolve_connect_snippet_paths, ConnectHost, ConnectSnippetPaths,
    },
    error::Result,
    spec::registry::load_agent_specs_from_dirs,
};

const PRESET_CATALOG_VERSION: &str = "v0.9.0";
const GENERATED_ROOT_MANIFEST_RELATIVE: &str = ".mcp-subagent/generated-root.toml";
const GENERATED_ROOT_MANIFEST_KIND: &str = "mcp-subagent-generated-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitPreset {
    ClaudeOpusSupervisor,
    ClaudeOpusSupervisorMinimal,
    CodexPrimaryBuilder,
    GeminiFrontendTeam,
    LocalOllamaFallback,
    MinimalSingleProvider,
}

impl InitPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeOpusSupervisor => "claude-opus-supervisor",
            Self::ClaudeOpusSupervisorMinimal => "claude-opus-supervisor-minimal",
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GeneratedRootManifest {
    pub kind: String,
    pub catalog_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

pub(crate) fn generated_root_manifest_path(root: &Path) -> PathBuf {
    root.join(GENERATED_ROOT_MANIFEST_RELATIVE)
}

pub(crate) fn load_generated_root_manifest(root: &Path) -> Option<GeneratedRootManifest> {
    let raw = fs::read_to_string(generated_root_manifest_path(root)).ok()?;
    let manifest = toml::from_str::<GeneratedRootManifest>(&raw).ok()?;
    (manifest.kind == GENERATED_ROOT_MANIFEST_KIND).then_some(manifest)
}

fn generated_root_manifest_template(preset: Option<&str>) -> String {
    let mut raw = format!(
        "kind = \"{GENERATED_ROOT_MANIFEST_KIND}\"\ncatalog_version = \"{PRESET_CATALOG_VERSION}\"\n"
    );
    if let Some(preset) = preset {
        raw.push_str(&format!("preset = \"{preset}\"\n"));
    }
    raw
}

fn sync_generated_root_manifest(
    root: &Path,
    preset: Option<&str>,
) -> Result<(PathBuf, bool, bool)> {
    let path = generated_root_manifest_path(root);
    let content = generated_root_manifest_template(preset);
    let existed = path.exists();
    let changed = if existed {
        fs::read_to_string(&path)? != content
    } else {
        true
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if changed {
        fs::write(&path, content)?;
    }
    Ok((path, !existed, existed && changed))
}

pub fn init_workspace(root: &Path, preset: InitPreset, force: bool) -> Result<InitReport> {
    init_with_preset(root, preset, force)
}

pub fn sync_project_bridge_workspace(root: &Path) -> Result<InitReport> {
    let root = root.to_path_buf();
    let agents_dir = root.join("agents");
    if !agents_dir.is_dir() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "generated-root agents directory not found: {} (pass --root-dir <generated-root>)",
                agents_dir.display()
            ),
        )
        .into());
    }
    if !is_generated_root(&root) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "root does not look like a generated mcp-subagent workspace: {}",
                root.display()
            ),
        )
        .into());
    }

    let generated = load_agent_specs_from_dirs(std::slice::from_ref(&agents_dir))?;
    let state_dir = root.join(".mcp-subagent/state");
    Ok(InitReport {
        preset: "sync-project-config-only".to_string(),
        preset_catalog_version: PRESET_CATALOG_VERSION.to_string(),
        root: root.clone(),
        agents_dir: agents_dir.clone(),
        created_files: Vec::new(),
        overwritten_files: Vec::new(),
        generated_agent_count: generated.len(),
        notes: vec![
            "Validated existing generated root; bootstrap templates were not rewritten."
                .to_string(),
            format!(
                "Run `mcp-subagent validate --agents-dir {}` to verify existing specs.",
                agents_dir.display()
            ),
            format!(
                "Run `mcp-subagent doctor --agents-dir {} --state-dir {}` to inspect provider readiness.",
                agents_dir.display(),
                state_dir.display()
            ),
        ],
    })
}

pub fn refresh_bootstrap_workspace(root: &Path) -> Result<InitReport> {
    let root = root.to_path_buf();
    let agents_dir = root.join("agents");
    if !agents_dir.is_dir() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "bootstrap agents directory not found: {} (run `mcp-subagent init` first or pass --root-dir <bootstrap-root>)",
                agents_dir.display()
            ),
        )
        .into());
    }

    let mut builtin_template_paths = Vec::new();
    let mut created_files = Vec::new();
    let mut overwritten_files = Vec::new();
    let mut preserved_custom_agent_count = 0usize;
    let mut entries = fs::read_dir(&agents_dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(expected) = builtin_agent_template(file_name) else {
            if file_name.ends_with(".agent.toml") {
                preserved_custom_agent_count += 1;
            }
            continue;
        };

        builtin_template_paths.push(path.clone());
        let actual = fs::read_to_string(&path)?;
        if actual != expected {
            write(&path, expected)?;
            overwritten_files.push(path);
        }
    }

    if builtin_template_paths.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "no built-in bootstrap agent templates found under {}",
                agents_dir.display()
            ),
        )
        .into());
    }

    let manifest_preset = load_generated_root_manifest(&root)
        .and_then(|manifest| manifest.preset)
        .or_else(|| detect_preset_from_generated_agents(&builtin_template_paths));
    let (manifest_path, created_manifest, overwritten_manifest) =
        sync_generated_root_manifest(&root, manifest_preset.as_deref())?;
    if created_manifest {
        created_files.push(manifest_path);
    } else if overwritten_manifest {
        overwritten_files.push(manifest_path);
    }

    let state_dir = root.join(".mcp-subagent/state");
    let mut notes = if overwritten_files.is_empty() {
        vec![format!(
            "No drifted built-in bootstrap templates were found under `{}`; catalog files already match {}.",
            agents_dir.display(),
            PRESET_CATALOG_VERSION
        )]
    } else {
        vec![format!(
            "Refreshed {} drifted built-in bootstrap template(s) under `{}`.",
            overwritten_files.len(),
            agents_dir.display()
        )]
    };
    if preserved_custom_agent_count > 0 {
        notes.push(format!(
            "Preserved {} custom agent file(s) outside the built-in catalog.",
            preserved_custom_agent_count
        ));
    }
    notes.push(format!(
        "Run `mcp-subagent validate --agents-dir {}` to verify refreshed specs.",
        agents_dir.display()
    ));
    notes.push(format!(
        "Run `mcp-subagent doctor --agents-dir {} --state-dir {}` to confirm bootstrap drift is gone.",
        agents_dir.display(),
        state_dir.display()
    ));

    Ok(InitReport {
        preset: "refresh-bootstrap".to_string(),
        preset_catalog_version: PRESET_CATALOG_VERSION.to_string(),
        root,
        agents_dir,
        created_files,
        overwritten_files,
        generated_agent_count: builtin_template_paths.len(),
        notes,
    })
}

fn init_with_preset(root: &Path, preset: InitPreset, force: bool) -> Result<InitReport> {
    let root = root.to_path_buf();
    let agents_dir = root.join("agents");
    let config_path = root.join(".mcp-subagent/config.toml");
    let readme_path = root.join("README.mcp-subagent.md");
    let plan_path = root.join("PLAN.md");
    let manifest_path = generated_root_manifest_path(&root);
    let agent_templates = preset_agent_templates(preset);

    let mut files = vec![
        plan_path.clone(),
        config_path.clone(),
        readme_path.clone(),
        manifest_path.clone(),
    ];
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
    sync_generated_root_manifest(&root, Some(preset.as_str()))?;
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
        InitPreset::ClaudeOpusSupervisor | InitPreset::ClaudeOpusSupervisorMinimal => vec![
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

pub(crate) fn preset_catalog_version() -> &'static str {
    PRESET_CATALOG_VERSION
}

pub fn is_generated_root(root: &Path) -> bool {
    load_generated_root_manifest(root).is_some() || is_legacy_generated_root(root)
}

pub(crate) fn builtin_agent_template(file_name: &str) -> Option<&'static str> {
    match file_name {
        "fast-researcher.agent.toml" => Some(FAST_RESEARCHER_AGENT),
        "backend-coder.agent.toml" => Some(BACKEND_CODER_AGENT),
        "frontend-builder.agent.toml" => Some(FRONTEND_BUILDER_AGENT),
        "correctness-reviewer.agent.toml" => Some(CORRECTNESS_REVIEWER_AGENT),
        "style-reviewer.agent.toml" => Some(STYLE_REVIEWER_AGENT),
        "codex-style-reviewer.agent.toml" => Some(CODEX_STYLE_REVIEWER_AGENT),
        "gemini-style-reviewer.agent.toml" => Some(GEMINI_STYLE_REVIEWER_AGENT),
        "local-fallback-coder.agent.toml" => Some(LOCAL_FALLBACK_CODER_AGENT),
        "single-provider-coder.agent.toml" => Some(SINGLE_PROVIDER_CODER_AGENT),
        _ => None,
    }
}

fn detect_preset_from_generated_agents(paths: &[PathBuf]) -> Option<String> {
    let mut file_names = paths
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    file_names.sort_unstable();

    for preset in [
        InitPreset::ClaudeOpusSupervisor,
        InitPreset::ClaudeOpusSupervisorMinimal,
        InitPreset::CodexPrimaryBuilder,
        InitPreset::GeminiFrontendTeam,
        InitPreset::LocalOllamaFallback,
        InitPreset::MinimalSingleProvider,
    ] {
        let mut expected = preset_agent_templates(preset)
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        if expected == file_names {
            return Some(preset.as_str().to_string());
        }
    }

    None
}

fn is_legacy_generated_root(root: &Path) -> bool {
    let components = root
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    components
        .windows(2)
        .any(|window| window == [".mcp-subagent", "bootstrap"])
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

- `claude-opus-supervisor-minimal`
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

## Current Defaults

Generated presets use the current runtime terms: `context_mode`, `delegation_context`,
`memory_sources`, and `working_dir_policy`.
Built-in templates keep `memory_sources = ["auto_project_memory"]` and do not inject `active_plan`
by default.
Gemini read-only research presets keep `working_dir_policy = "auto"`; on the stable scratch path,
runtime will keep the override visible by downgrading `native_discovery = "isolated"` to `minimal`.
If `doctor` reports bootstrap template drift, review those local edits first; if the drift is
accidental, run the exact `refresh_command` emitted by `doctor` (or use
`mcp-subagent init --refresh-bootstrap --root-dir <generated-root>`) to resync built-in
templates while preserving custom agents. Default `init` still will not overwrite files silently.

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
delegation_context = "minimal"
memory_sources = ["auto_project_memory"]
native_discovery = "isolated"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "provider_default"
timeout_secs = 600
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 1200
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "isolated"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "provider_default"
timeout_secs = 1200
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "plan_section"
plan_section_selector = "Acceptance Criteria"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "deny_by_default"
timeout_secs = 900
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "isolated"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "read_only"
approval = "provider_default"
timeout_secs = 900
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 1200
output_mode = "both"
parse_policy = "best_effort"
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
delegation_context = "selected_files"
memory_sources = ["auto_project_memory"]
native_discovery = "minimal"
working_dir_policy = "auto"
file_conflict_policy = "serialize"
sandbox = "workspace_write"
approval = "deny_by_default"
timeout_secs = 900
output_mode = "both"
parse_policy = "best_effort"
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

    use super::{
        generated_root_manifest_path, init_workspace, is_generated_root,
        load_generated_root_manifest, refresh_bootstrap_workspace, sync_project_bridge_workspace,
        InitPreset,
    };

    #[test]
    fn init_creates_preset_files_and_valid_specs() {
        let dir = tempdir().expect("tempdir");
        let report = init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisor, false)
            .expect("init succeeds");

        assert_eq!(report.generated_agent_count, 6);
        assert_eq!(report.preset_catalog_version, "v0.9.0");
        assert!(dir.path().join("agents").exists());
        assert!(dir.path().join("PLAN.md").exists());
        assert!(dir.path().join(".mcp-subagent/config.toml").exists());
        assert!(generated_root_manifest_path(dir.path()).exists());
        assert!(dir.path().join("README.mcp-subagent.md").exists());
        assert_eq!(
            load_generated_root_manifest(dir.path())
                .and_then(|manifest| manifest.preset)
                .as_deref(),
            Some("claude-opus-supervisor")
        );
    }

    #[test]
    fn init_supports_all_presets_and_validates() {
        for preset in [
            InitPreset::ClaudeOpusSupervisor,
            InitPreset::ClaudeOpusSupervisorMinimal,
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
            assert_eq!(report.preset_catalog_version, "v0.9.0");
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

    #[test]
    fn generated_presets_do_not_default_to_active_plan_memory() {
        for preset in [
            InitPreset::ClaudeOpusSupervisor,
            InitPreset::ClaudeOpusSupervisorMinimal,
            InitPreset::CodexPrimaryBuilder,
            InitPreset::GeminiFrontendTeam,
            InitPreset::LocalOllamaFallback,
            InitPreset::MinimalSingleProvider,
        ] {
            let dir = tempdir().expect("tempdir");
            init_workspace(dir.path(), preset, false).expect("init preset");

            for entry in fs::read_dir(dir.path().join("agents")).expect("read agents dir") {
                let path = entry.expect("dir entry").path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with(".agent.toml"))
                    .unwrap_or(false)
                {
                    continue;
                }
                let raw = fs::read_to_string(&path).expect("read agent");
                assert!(
                    raw.contains("memory_sources = [\"auto_project_memory\"]"),
                    "expected auto_project_memory default in {}",
                    path.display()
                );
                assert!(
                    !raw.contains("active_plan"),
                    "unexpected active_plan default in {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn init_readme_documents_current_runtime_terms_and_drift_guidance() {
        let dir = tempdir().expect("tempdir");
        init_workspace(dir.path(), InitPreset::ClaudeOpusSupervisorMinimal, false)
            .expect("init succeeds");

        let readme = fs::read_to_string(dir.path().join("README.mcp-subagent.md"))
            .expect("read generated readme");
        assert!(readme.contains("`context_mode`, `delegation_context`,"));
        assert!(readme.contains("`memory_sources`, and `working_dir_policy`."));
        assert!(readme.contains("memory_sources = [\"auto_project_memory\"]"));
        assert!(readme.contains("do not inject `active_plan`"));
        assert!(readme.contains("`doctor` reports bootstrap template drift"));
        assert!(readme.contains("exact `refresh_command` emitted by `doctor`"));
        assert!(
            readme.contains("`mcp-subagent init --refresh-bootstrap --root-dir <generated-root>`")
        );
        assert!(readme.contains("preserving custom agents"));
        assert!(readme.contains("Default `init` still will not overwrite files silently"));
    }

    #[test]
    fn refresh_bootstrap_overwrites_builtin_templates_and_preserves_custom_agents() {
        let dir = tempdir().expect("tempdir");
        init_workspace(dir.path(), InitPreset::CodexPrimaryBuilder, false).expect("init succeeds");

        let backend_path = dir.path().join("agents/backend-coder.agent.toml");
        let custom_path = dir.path().join("agents/custom.agent.toml");
        fs::write(&backend_path, "drifted = true\n").expect("write drifted builtin");
        fs::write(
            &custom_path,
            r#"[core]
name = "custom-agent"
description = "custom agent preserved during refresh"
provider = "mock"
instructions = "custom"
"#,
        )
        .expect("write custom agent");

        let report = refresh_bootstrap_workspace(dir.path()).expect("refresh succeeds");
        let backend = fs::read_to_string(&backend_path).expect("read refreshed builtin");
        let custom = fs::read_to_string(&custom_path).expect("read preserved custom");

        assert!(backend.contains("provider = \"codex\""));
        assert!(backend.contains("memory_sources = [\"auto_project_memory\"]"));
        assert!(custom.contains("name = \"custom-agent\""));
        assert!(report.created_files.is_empty());
        assert!(
            report
                .overwritten_files
                .iter()
                .any(|path| path == &backend_path),
            "expected drifted builtin to be refreshed"
        );
        assert_eq!(report.generated_agent_count, 3);
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("Preserved 1 custom agent file")),
            "expected preserved custom agent note: {:?}",
            report.notes
        );
        assert_eq!(
            load_generated_root_manifest(dir.path())
                .and_then(|manifest| manifest.preset)
                .as_deref(),
            Some("codex-primary-builder")
        );
    }

    #[test]
    fn refresh_bootstrap_backfills_manifest_for_legacy_root() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("agents")).expect("create agents");
        fs::write(
            dir.path().join("agents/backend-coder.agent.toml"),
            super::builtin_agent_template("backend-coder.agent.toml").expect("builtin template"),
        )
        .expect("write builtin template");
        assert!(
            !generated_root_manifest_path(dir.path()).exists(),
            "legacy root should start without manifest"
        );

        let report = refresh_bootstrap_workspace(dir.path()).expect("refresh succeeds");
        assert!(
            report
                .created_files
                .iter()
                .any(|path| path == &generated_root_manifest_path(dir.path())),
            "expected refresh to create generated-root manifest"
        );
        assert!(
            load_generated_root_manifest(dir.path()).is_some(),
            "expected generated-root manifest after refresh"
        );
    }

    #[test]
    fn refresh_bootstrap_fails_when_no_builtin_templates_exist() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("agents")).expect("create agents dir");
        fs::write(
            dir.path().join("agents/custom.agent.toml"),
            "custom = true\n",
        )
        .expect("write custom agent");

        let err = refresh_bootstrap_workspace(dir.path()).expect_err("refresh must fail");
        assert!(
            matches!(
                err,
                crate::error::McpSubagentError::Io(ref io) if io.kind() == ErrorKind::NotFound
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sync_project_bridge_validates_generated_root_without_rewriting_templates() {
        let dir = tempdir().expect("tempdir");
        init_workspace(dir.path(), InitPreset::CodexPrimaryBuilder, false).expect("init succeeds");

        let backend_path = dir.path().join("agents/backend-coder.agent.toml");
        let drifted = fs::read_to_string(&backend_path)
            .expect("read backend")
            .replacen(
                "memory_sources = [\"auto_project_memory\"]",
                "memory_sources = [\"auto_project_memory\", \"active_plan\"]",
                1,
            );
        fs::write(&backend_path, &drifted).expect("write drifted builtin");

        let report = sync_project_bridge_workspace(dir.path()).expect("sync-only succeeds");

        assert_eq!(report.preset, "sync-project-config-only");
        assert!(report.created_files.is_empty());
        assert!(report.overwritten_files.is_empty());
        assert_eq!(report.generated_agent_count, 3);
        assert_eq!(
            fs::read_to_string(&backend_path).expect("read backend after sync-only"),
            drifted
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("bootstrap templates were not rewritten")),
            "expected non-rewrite note: {:?}",
            report.notes
        );
    }

    #[test]
    fn sync_project_bridge_rejects_non_generated_root() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("agents")).expect("create agents");
        fs::write(
            dir.path().join("agents/custom.agent.toml"),
            r#"[core]
name = "custom-agent"
description = "custom"
provider = "mock"
instructions = "custom"
"#,
        )
        .expect("write custom agent");

        let err = sync_project_bridge_workspace(dir.path()).expect_err("must reject");
        assert!(
            matches!(
                err,
                crate::error::McpSubagentError::Io(ref io) if io.kind() == ErrorKind::InvalidInput
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn generated_root_detection_accepts_legacy_bootstrap_shape() {
        let dir = tempdir().expect("tempdir");
        let legacy_root = dir.path().join(".mcp-subagent/bootstrap");
        fs::create_dir_all(legacy_root.join("agents")).expect("create agents");

        assert!(is_generated_root(&legacy_root));
    }
}
