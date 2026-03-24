use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use glob::glob;

use crate::{
    error::{McpSubagentError, Result},
    spec::{runtime_policy::MemorySource, AgentSpec, Provider},
    types::{MemorySnippet, ResolvedMemory, RunRequest},
};

const MAX_MEMORY_SNIPPET_BYTES: usize = 32 * 1024;
const PROJECT_MEMORY_CANDIDATES: [&str; 2] = ["PROJECT.md", ".mcp-subagent/PROJECT.md"];
const ACTIVE_PLAN_CANDIDATES: [&str; 2] = ["PLAN.md", ".mcp-subagent/PLAN.md"];
const ARCHIVED_PLAN_GLOB_PATTERNS: [&str; 3] =
    ["docs/plans/*.md", "archive/*.md", "plans/archive/*.md"];

pub fn resolve_memory(spec: &AgentSpec, request: &RunRequest) -> Result<ResolvedMemory> {
    let mut resolver = MemoryResolver::new(&request.working_dir);
    for source in &spec.runtime.memory_sources {
        match source {
            MemorySource::AutoProjectMemory => {
                resolver.resolve_auto_project_memory(&spec.core.provider)?;
            }
            MemorySource::ActivePlan => {
                resolver.resolve_active_plan()?;
            }
            MemorySource::ArchivedPlans => {
                resolver.resolve_archived_plans()?;
            }
            MemorySource::File(path) => {
                resolver.resolve_file_source(path)?;
            }
            MemorySource::Glob(pattern) => {
                resolver.resolve_glob_source(pattern)?;
            }
            MemorySource::Inline(content) => {
                resolver.resolve_inline_source(content);
            }
        }
    }
    Ok(resolver.finish())
}

struct MemoryResolver<'a> {
    workspace_root: &'a Path,
    resolved: ResolvedMemory,
    seen_inline_paths: HashSet<PathBuf>,
    seen_native_paths: HashSet<PathBuf>,
    inline_counter: usize,
}

impl<'a> MemoryResolver<'a> {
    fn new(workspace_root: &'a Path) -> Self {
        Self {
            workspace_root,
            resolved: ResolvedMemory::default(),
            seen_inline_paths: HashSet::new(),
            seen_native_paths: HashSet::new(),
            inline_counter: 0,
        }
    }

    fn finish(self) -> ResolvedMemory {
        self.resolved
    }

    fn resolve_auto_project_memory(&mut self, provider: &Provider) -> Result<()> {
        for relative in PROJECT_MEMORY_CANDIDATES {
            let full_path = self.workspace_root.join(relative);
            if !full_path.is_file() {
                continue;
            }
            self.add_inline_file(&full_path, relative, true)?;
        }

        for relative in provider_native_memory_candidates(provider) {
            let full_path = self.workspace_root.join(relative);
            if !full_path.is_file() {
                continue;
            }
            self.add_native_passthrough(full_path);
        }
        Ok(())
    }

    fn resolve_active_plan(&mut self) -> Result<()> {
        for relative in ACTIVE_PLAN_CANDIDATES {
            let full_path = self.workspace_root.join(relative);
            if !full_path.is_file() {
                continue;
            }
            self.add_inline_file(&full_path, &format!("active_plan:{relative}"), false)?;
            return Ok(());
        }

        Ok(())
    }

    fn resolve_archived_plans(&mut self) -> Result<()> {
        for pattern in ARCHIVED_PLAN_GLOB_PATTERNS {
            let absolute_pattern = self.workspace_root.join(pattern);
            let pattern_text = absolute_pattern.to_string_lossy().to_string();
            let entries = glob(&pattern_text).map_err(|err| {
                McpSubagentError::SpecValidation(format!(
                    "invalid archived plan glob pattern `{pattern}`: {err}"
                ))
            })?;

            for entry in entries {
                let path = entry.map_err(|err| {
                    McpSubagentError::SpecValidation(format!(
                        "invalid path matched by archived plan glob `{pattern}`: {err}"
                    ))
                })?;
                if !path.is_file() {
                    continue;
                }
                self.add_inline_file(&path, &format!("archived_plan:{pattern}"), false)?;
            }
        }

        Ok(())
    }

    fn resolve_file_source(&mut self, relative: &str) -> Result<()> {
        let full_path = self.workspace_root.join(relative);
        if !full_path.exists() {
            return Err(McpSubagentError::SpecValidation(format!(
                "File memory source does not exist: {}",
                full_path.display()
            )));
        }
        if !full_path.is_file() {
            return Err(McpSubagentError::SpecValidation(format!(
                "File memory source must point to a regular file: {}",
                full_path.display()
            )));
        }
        self.add_inline_file(&full_path, relative, false)
    }

    fn resolve_glob_source(&mut self, pattern: &str) -> Result<()> {
        let absolute_pattern = self.workspace_root.join(pattern);
        let pattern_text = absolute_pattern.to_string_lossy().to_string();
        let entries = glob(&pattern_text).map_err(|err| {
            McpSubagentError::SpecValidation(format!(
                "invalid Glob memory source pattern `{pattern}`: {err}"
            ))
        })?;

        let mut matched_files = Vec::new();
        for entry in entries {
            let path = entry.map_err(|err| {
                McpSubagentError::SpecValidation(format!(
                    "invalid path matched by Glob memory source `{pattern}`: {err}"
                ))
            })?;
            if path.is_file() {
                matched_files.push(path);
            }
        }

        matched_files.sort();
        matched_files.dedup();
        if matched_files.is_empty() {
            return Err(McpSubagentError::SpecValidation(format!(
                "Glob memory source did not match any files: {pattern}"
            )));
        }

        for path in matched_files {
            self.add_inline_file(&path, pattern, false)?;
        }
        Ok(())
    }

    fn resolve_inline_source(&mut self, content: &str) {
        self.inline_counter += 1;
        self.resolved.additional_memories.push(MemorySnippet {
            label: format!("inline:{}", self.inline_counter),
            content: content.trim().to_string(),
            source_path: None,
        });
    }

    fn add_inline_file(&mut self, path: &Path, label: &str, project_memory: bool) -> Result<()> {
        let dedup_key = normalize_dedup_key(path);
        if self.seen_inline_paths.contains(&dedup_key) {
            return Ok(());
        }
        if self.seen_native_paths.remove(&dedup_key) {
            self.resolved
                .native_passthrough_paths
                .retain(|candidate| normalize_dedup_key(candidate) != dedup_key);
        }

        let content = read_memory_file(path)?;
        let snippet = MemorySnippet {
            label: label.to_string(),
            content,
            source_path: Some(path.to_path_buf()),
        };
        if project_memory {
            self.resolved.project_memories.push(snippet);
        } else {
            self.resolved.additional_memories.push(snippet);
        }
        self.seen_inline_paths.insert(dedup_key);
        Ok(())
    }

    fn add_native_passthrough(&mut self, path: PathBuf) {
        let dedup_key = normalize_dedup_key(&path);
        if self.seen_inline_paths.contains(&dedup_key)
            || self.seen_native_paths.contains(&dedup_key)
        {
            return;
        }
        self.resolved.native_passthrough_paths.push(path);
        self.seen_native_paths.insert(dedup_key);
    }
}

fn provider_native_memory_candidates(provider: &Provider) -> &'static [&'static str] {
    match provider {
        Provider::Mock => &[],
        Provider::Claude => &["CLAUDE.md", ".claude/CLAUDE.md"],
        Provider::Codex => &["AGENTS.md", "AGENTS.override.md"],
        Provider::Gemini => &["GEMINI.md"],
        Provider::Ollama => &[],
    }
}

fn read_memory_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    if bytes.len() <= MAX_MEMORY_SNIPPET_BYTES {
        return Ok(String::from_utf8_lossy(&bytes).to_string());
    }

    let truncated = String::from_utf8_lossy(&bytes[..MAX_MEMORY_SNIPPET_BYTES]).to_string();
    Ok(format!(
        "{truncated}\n\n[truncated by mcp-subagent: original={} bytes, kept={} bytes]",
        bytes.len(),
        MAX_MEMORY_SNIPPET_BYTES
    ))
}

fn normalize_dedup_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::tempdir;

    use crate::{
        runtime::memory::resolve_memory,
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{MemorySource, RuntimePolicy},
            AgentSpec,
        },
        types::{RunMode, RunRequest},
    };

    fn sample_spec(provider: Provider, memory_sources: Vec<MemorySource>) -> AgentSpec {
        let runtime = RuntimePolicy {
            memory_sources,
            ..Default::default()
        };
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "desc".to_string(),
                provider,
                model: None,
                instructions: "review".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: Default::default(),
            },
            runtime,
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    fn sample_request(working_dir: PathBuf) -> RunRequest {
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
    fn auto_project_memory_resolves_project_and_native_paths() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("PROJECT.md"), "project memory").expect("write project");
        fs::write(temp.path().join("AGENTS.md"), "codex native").expect("write native");

        let spec = sample_spec(Provider::Codex, vec![MemorySource::AutoProjectMemory]);
        let request = sample_request(temp.path().to_path_buf());
        let resolved = resolve_memory(&spec, &request).expect("resolve");

        assert_eq!(resolved.project_memories.len(), 1);
        assert_eq!(resolved.additional_memories.len(), 0);
        assert_eq!(resolved.native_passthrough_paths.len(), 1);
        assert!(resolved.project_memories[0]
            .source_path
            .as_ref()
            .expect("source path")
            .ends_with("PROJECT.md"));
        assert!(resolved.native_passthrough_paths[0].ends_with("AGENTS.md"));
    }

    #[test]
    fn explicit_file_memory_dedups_native_passthrough() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("AGENTS.md"), "codex native").expect("write native");

        let spec = sample_spec(
            Provider::Codex,
            vec![
                MemorySource::AutoProjectMemory,
                MemorySource::File("AGENTS.md".to_string()),
            ],
        );
        let request = sample_request(temp.path().to_path_buf());
        let resolved = resolve_memory(&spec, &request).expect("resolve");

        assert_eq!(resolved.native_passthrough_paths.len(), 0);
        assert_eq!(resolved.additional_memories.len(), 1);
        assert!(resolved.additional_memories[0]
            .source_path
            .as_ref()
            .expect("source path")
            .ends_with("AGENTS.md"));
    }

    #[test]
    fn glob_memory_source_inlines_all_matches() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs/sub")).expect("mkdir");
        fs::write(temp.path().join("docs/a.md"), "A").expect("write A");
        fs::write(temp.path().join("docs/sub/b.md"), "B").expect("write B");

        let spec = sample_spec(
            Provider::Codex,
            vec![MemorySource::Glob("docs/**/*.md".to_string())],
        );
        let request = sample_request(temp.path().to_path_buf());
        let resolved = resolve_memory(&spec, &request).expect("resolve");

        assert_eq!(resolved.additional_memories.len(), 2);
    }

    #[test]
    fn glob_memory_source_requires_at_least_one_match() {
        let temp = tempdir().expect("tempdir");
        let spec = sample_spec(
            Provider::Codex,
            vec![MemorySource::Glob("missing/**/*.md".to_string())],
        );
        let request = sample_request(temp.path().to_path_buf());

        let err = resolve_memory(&spec, &request).expect_err("empty glob should fail");
        assert!(
            err.to_string().contains("did not match any files"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn active_plan_source_is_noop_when_plan_missing() {
        let temp = tempdir().expect("tempdir");
        let spec = sample_spec(Provider::Codex, vec![MemorySource::ActivePlan]);
        let request = sample_request(temp.path().to_path_buf());

        let resolved = resolve_memory(&spec, &request).expect("resolve");
        assert!(resolved.additional_memories.is_empty());
    }

    #[test]
    fn active_plan_source_inlines_plan_content() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("PLAN.md"), "# Goal\nship feature").expect("write plan");
        let spec = sample_spec(Provider::Codex, vec![MemorySource::ActivePlan]);
        let request = sample_request(temp.path().to_path_buf());

        let resolved = resolve_memory(&spec, &request).expect("resolve");
        assert_eq!(resolved.additional_memories.len(), 1);
        assert!(resolved.additional_memories[0]
            .label
            .contains("active_plan"));
        assert!(resolved.additional_memories[0]
            .content
            .contains("ship feature"));
    }

    #[test]
    fn archived_plans_source_inlines_existing_archives() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("docs/plans")).expect("create plans dir");
        fs::write(
            temp.path().join("docs/plans/2026-03-24-demo.md"),
            "archived plan",
        )
        .expect("write archived");
        let spec = sample_spec(Provider::Codex, vec![MemorySource::ArchivedPlans]);
        let request = sample_request(temp.path().to_path_buf());

        let resolved = resolve_memory(&spec, &request).expect("resolve");
        assert_eq!(resolved.additional_memories.len(), 1);
        assert!(resolved.additional_memories[0]
            .label
            .contains("archived_plan"));
    }
}
