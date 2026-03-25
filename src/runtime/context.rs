use std::fmt::Write;

use crate::{
    error::{McpSubagentError, Result},
    runtime::summary::{
        parse_summary_envelope, SummaryEnvelope, SUMMARY_END_SENTINEL, SUMMARY_START_SENTINEL,
    },
    spec::{
        core::{AgentSpecCore, Provider},
        runtime_policy::{ContextMode, RuntimePolicy},
        AgentSpec,
    },
    types::{
        CompiledContext, ContextSourceRef, InjectionMode, MemorySnippet, ResolvedMemory, RunMode,
        RunRequest, TaskSpec, WorkflowHints,
    },
};

const REQUIRED_TEMPLATE_SECTIONS: [&str; 8] = [
    "ROLE",
    "TASK",
    "OBJECTIVE",
    "CONSTRAINTS",
    "ACCEPTANCE CRITERIA",
    "SELECTED CONTEXT",
    "RESPONSE CONTRACT",
    "OUTPUT SENTINELS",
];

pub trait ContextCompiler: Send + Sync {
    fn compile_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext>;

    fn compile(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext> {
        let task_spec = request.to_task_spec();
        let hints = request.to_workflow_hints();
        self.compile_task(spec, &task_spec, &hints, memory)
    }

    fn parse_summary(&self, raw_stdout: &str, raw_stderr: &str) -> Result<SummaryEnvelope>;
}

#[derive(Debug, Default)]
pub struct DefaultContextCompiler;

impl ContextCompiler for DefaultContextCompiler {
    fn compile_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext> {
        let mut source_manifest = Vec::new();
        let mut selected_context = String::new();
        let injection = ContextInjectionPolicy::for_mode(&spec.runtime.context_mode);

        if injection.include_parent_summary {
            if let Some(parent_summary) = hints.parent_summary.as_deref() {
                if is_likely_raw_transcript(parent_summary) {
                    writeln!(
                        &mut selected_context,
                        "parent_summary: [suppressed because it appears to contain raw transcript]\n"
                    )
                    .expect("write to string");
                } else if injection.parent_summary_digest_only {
                    let digest = summarize_parent_summary(parent_summary);
                    writeln!(&mut selected_context, "parent_summary_digest:\n{digest}\n")
                        .expect("write to string");
                } else {
                    writeln!(&mut selected_context, "parent_summary:\n{parent_summary}\n")
                        .expect("write to string");
                }
            }
        }

        for selected in &task_spec.selected_files {
            if !injection.should_include_selected_file(selected.path.as_path()) {
                continue;
            }
            source_manifest.push(ContextSourceRef {
                label: format!("selected_file:{}", selected.path.display()),
                path: Some(selected.path.clone()),
                injection_mode: InjectionMode::InlineSummary,
            });

            writeln!(
                &mut selected_context,
                "selected_file: {}",
                selected.path.display()
            )
            .expect("write to string");
            if let Some(rationale) = selected.rationale.as_deref() {
                writeln!(&mut selected_context, "rationale: {rationale}").expect("write");
            }
            if let Some(content) = selected.content.as_deref() {
                writeln!(&mut selected_context, "content:\n{content}").expect("write");
            }
            selected_context.push('\n');
        }

        inject_memory_sources(
            "project_memory",
            &memory.project_memories,
            &mut selected_context,
            &mut source_manifest,
        );
        inject_memory_sources(
            "additional_memory",
            &memory.additional_memories,
            &mut selected_context,
            &mut source_manifest,
        );

        for native_path in &memory.native_passthrough_paths {
            source_manifest.push(ContextSourceRef {
                label: format!("native_passthrough:{}", native_path.display()),
                path: Some(native_path.clone()),
                injection_mode: InjectionMode::NativePassThrough,
            });
            writeln!(
                &mut selected_context,
                "native_passthrough_memory: {} (delegated to provider native loader)",
                native_path.display()
            )
            .expect("write");
        }

        if selected_context.trim().is_empty() {
            selected_context.push_str("none");
        }

        let task_line = task_spec
            .task_brief
            .as_deref()
            .unwrap_or(task_spec.task.as_str())
            .trim();
        let acceptance = if task_spec.acceptance_criteria.is_empty() {
            "1) Return structured summary JSON in sentinels.\n2) Keep response concise and actionable."
                .to_string()
        } else {
            task_spec
                .acceptance_criteria
                .iter()
                .enumerate()
                .map(|(idx, item)| format!("{} ) {item}", idx + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let response_contract = format!(
            "Return a machine-readable JSON block only inside sentinels. The JSON must match SummaryEnvelope.\n{}\n{{...valid json...}}\n{}",
            SUMMARY_START_SENTINEL, SUMMARY_END_SENTINEL
        );

        let injected_prompt = format!(
            "ROLE\nYou are agent `{name}` for provider `{provider}`.\n\nTASK\n{task}\n\nOBJECTIVE\n{objective}\n\nCONSTRAINTS\n{constraints}\n\nACCEPTANCE CRITERIA\n{acceptance}\n\nSELECTED CONTEXT\n{context}\n\nRESPONSE CONTRACT\n{contract}\n\nOUTPUT SENTINELS\n{start}\n{{...valid json...}}\n{end}\n",
            name = spec.core.name,
            provider = spec.core.provider.as_str(),
            task = task_line,
            objective = task_spec.task,
            constraints = compile_constraints(spec, hints),
            acceptance = acceptance,
            context = selected_context.trim(),
            contract = response_contract,
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL
        );

        Ok(CompiledContext {
            system_prefix: spec.core.instructions.clone(),
            injected_prompt,
            source_manifest,
        })
    }

    fn parse_summary(&self, raw_stdout: &str, raw_stderr: &str) -> Result<SummaryEnvelope> {
        Ok(parse_summary_envelope(raw_stdout, raw_stderr))
    }
}

pub fn validate_compiled_prompt_template(injected_prompt: &str) -> Result<()> {
    for section in REQUIRED_TEMPLATE_SECTIONS {
        if !injected_prompt.contains(section) {
            return Err(McpSubagentError::SpecValidation(format!(
                "summary contract template missing section: {section}"
            )));
        }
    }

    if !injected_prompt.contains(SUMMARY_START_SENTINEL)
        || !injected_prompt.contains(SUMMARY_END_SENTINEL)
    {
        return Err(McpSubagentError::SpecValidation(
            "summary contract template missing summary sentinels".to_string(),
        ));
    }

    if !injected_prompt.contains("SummaryEnvelope") {
        return Err(McpSubagentError::SpecValidation(
            "summary contract template must reference SummaryEnvelope JSON contract".to_string(),
        ));
    }

    Ok(())
}

pub fn validate_default_summary_contract_template() -> Result<()> {
    let compiler = DefaultContextCompiler;
    let sample_spec = AgentSpec {
        core: AgentSpecCore {
            name: "contract-validator".to_string(),
            description: "validate summary contract template".to_string(),
            provider: Provider::Codex,
            model: None,
            instructions: "Emit structured summary JSON in sentinels.".to_string(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            skills: Vec::new(),
            tags: Vec::new(),
            metadata: Default::default(),
        },
        runtime: RuntimePolicy::default(),
        provider_overrides: Default::default(),
        workflow: None,
    };
    let sample_request = RunRequest {
        task: "Validate context compiler output contract".to_string(),
        task_brief: Some("Validate summary contract".to_string()),
        parent_summary: None,
        selected_files: Vec::new(),
        stage: None,
        plan_ref: None,
        working_dir: ".".into(),
        run_mode: RunMode::Sync,
        acceptance_criteria: vec![
            "Keep fixed sections in compiled template".to_string(),
            "Keep sentinel-wrapped SummaryEnvelope JSON contract".to_string(),
        ],
    };

    let task_spec = sample_request.to_task_spec();
    let hints = sample_request.to_workflow_hints();
    let compiled =
        compiler.compile_task(&sample_spec, &task_spec, &hints, ResolvedMemory::default())?;
    validate_compiled_prompt_template(&compiled.injected_prompt)
}

fn inject_memory_sources(
    section: &str,
    snippets: &[MemorySnippet],
    selected_context: &mut String,
    source_manifest: &mut Vec<ContextSourceRef>,
) {
    for snippet in snippets {
        source_manifest.push(ContextSourceRef {
            label: format!("{section}:{}", snippet.label),
            path: snippet.source_path.clone(),
            injection_mode: InjectionMode::InlineSummary,
        });

        let source = snippet
            .source_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "inline".to_string());

        writeln!(
            selected_context,
            "{} [{} from {}]:\n{}\n",
            section, snippet.label, source, snippet.content
        )
        .expect("write");
    }
}

fn compile_constraints(spec: &AgentSpec, hints: &WorkflowHints) -> String {
    let mut constraints = format!(
        "Do not request or rely on parent raw transcript.\nFollow agent instructions:\n{}\nProvider: {}\nContextMode: {}\nDelegationContext: {:?}",
        spec.core.instructions,
        spec.core.provider.as_str(),
        spec.runtime.context_mode,
        spec.runtime.delegation_context
    );
    if let Some(selector) = spec.runtime.plan_section_selector.as_deref() {
        constraints.push('\n');
        constraints.push_str(&format!("PlanSectionSelector: {selector}"));
    }
    if let Some(stage) = hints.stage.as_deref() {
        constraints.push('\n');
        constraints.push_str(&format!("WorkflowStage: {stage}"));
        constraints.push('\n');
        constraints.push_str(&format!(
            "StageRolePriority: {}",
            stage_role_priority(stage)
        ));
    }
    constraints
}

fn stage_role_priority(stage: &str) -> &'static str {
    match stage.to_ascii_lowercase().as_str() {
        "research" => "Researcher -> Planner -> Builder -> Reviewer",
        "plan" => "Planner -> Researcher -> Builder -> Reviewer",
        "build" => "Builder -> Reviewer -> Planner",
        "review" => "CorrectnessReviewer -> StyleReviewer -> Builder",
        "archive" => "Archivist -> Planner -> Reviewer",
        _ => "Generalist",
    }
}

#[derive(Debug)]
struct ContextInjectionPolicy {
    include_parent_summary: bool,
    parent_summary_digest_only: bool,
    selected_files_allowlist: Option<Vec<String>>,
}

impl ContextInjectionPolicy {
    fn for_mode(mode: &ContextMode) -> Self {
        match mode {
            ContextMode::Isolated => Self {
                include_parent_summary: false,
                parent_summary_digest_only: false,
                selected_files_allowlist: None,
            },
            ContextMode::SummaryOnly => Self {
                include_parent_summary: true,
                parent_summary_digest_only: false,
                selected_files_allowlist: None,
            },
            ContextMode::SelectedFiles(paths) => Self {
                include_parent_summary: false,
                parent_summary_digest_only: false,
                selected_files_allowlist: Some(paths.clone()),
            },
            ContextMode::ExpandedBrief => Self {
                include_parent_summary: true,
                parent_summary_digest_only: true,
                selected_files_allowlist: None,
            },
        }
    }

    fn should_include_selected_file(&self, candidate_path: &std::path::Path) -> bool {
        let Some(allowlist) = &self.selected_files_allowlist else {
            return false;
        };
        if allowlist.is_empty() {
            return false;
        }

        allowlist
            .iter()
            .map(|entry| entry.trim())
            .filter(|entry| !entry.is_empty())
            .any(|entry| {
                let allow_path = std::path::Path::new(entry);
                candidate_path == allow_path || candidate_path.ends_with(allow_path)
            })
    }
}

fn summarize_parent_summary(parent_summary: &str) -> String {
    let compact = parent_summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= 320 {
        compact
    } else {
        let truncated = compact.chars().take(320).collect::<String>();
        format!("{truncated} ...[digest truncated]")
    }
}

fn is_likely_raw_transcript(parent_summary: &str) -> bool {
    let mut role_lines = 0_u32;
    for line in parent_summary.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.starts_with("user:")
            || lower.starts_with("assistant:")
            || lower.starts_with("system:")
        {
            role_lines += 1;
        }
    }
    role_lines >= 2
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        runtime::context::{
            validate_compiled_prompt_template, validate_default_summary_contract_template,
            ContextCompiler, DefaultContextCompiler,
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::{ContextMode, RuntimePolicy},
            AgentSpec,
        },
        types::{MemorySnippet, ResolvedMemory, RunMode, RunRequest, SelectedFile},
    };

    fn sample_spec(mode: ContextMode) -> AgentSpec {
        let runtime = RuntimePolicy {
            context_mode: mode,
            ..Default::default()
        };
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "review".to_string(),
                provider: Provider::Codex,
                model: None,
                instructions: "Review carefully.".to_string(),
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

    #[test]
    fn compile_contains_required_sections() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::SelectedFiles(
            vec!["src/parser.rs".to_string()],
        ));
        let req = RunRequest {
            task: "Review parser changes".to_string(),
            task_brief: Some("Review parser module".to_string()),
            parent_summary: Some("parent summary".to_string()),
            selected_files: vec![SelectedFile {
                path: PathBuf::from("src/parser.rs"),
                rationale: Some("target file".to_string()),
                content: Some("fn parse() {}".to_string()),
            }],
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: vec!["Provide key findings".to_string()],
        };
        let memory = ResolvedMemory {
            project_memories: vec![MemorySnippet {
                label: "PROJECT.md".to_string(),
                content: "project constraints".to_string(),
                source_path: Some(PathBuf::from("PROJECT.md")),
            }],
            additional_memories: Vec::new(),
            native_passthrough_paths: vec![PathBuf::from("AGENTS.md")],
        };

        let compiled = compiler.compile(&spec, &req, memory).expect("compile");
        for section in [
            "ROLE",
            "TASK",
            "OBJECTIVE",
            "CONSTRAINTS",
            "ACCEPTANCE CRITERIA",
            "SELECTED CONTEXT",
            "RESPONSE CONTRACT",
            "OUTPUT SENTINELS",
        ] {
            assert!(
                compiled.injected_prompt.contains(section),
                "missing section {section}"
            );
        }
        assert_eq!(compiled.source_manifest.len(), 3);
    }

    #[test]
    fn isolated_mode_excludes_parent_summary_and_selected_files() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::Isolated);
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: Some("parent summary should be hidden".to_string()),
            selected_files: vec![SelectedFile {
                path: PathBuf::from("src/a.rs"),
                rationale: None,
                content: Some("fn a() {}".to_string()),
            }],
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(!compiled.injected_prompt.contains("parent_summary:"));
        assert!(!compiled.injected_prompt.contains("selected_file: src/a.rs"));
    }

    #[test]
    fn summary_only_mode_includes_parent_summary_but_excludes_selected_files() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::SummaryOnly);
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: Some("this is parent summary".to_string()),
            selected_files: vec![SelectedFile {
                path: PathBuf::from("src/a.rs"),
                rationale: None,
                content: Some("fn a() {}".to_string()),
            }],
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(compiled.injected_prompt.contains("parent_summary:"));
        assert!(!compiled.injected_prompt.contains("selected_file: src/a.rs"));
    }

    #[test]
    fn selected_files_mode_only_includes_allowlisted_files() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::SelectedFiles(vec!["src/keep.rs".to_string()]));
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: Some("ignored".to_string()),
            selected_files: vec![
                SelectedFile {
                    path: PathBuf::from("src/keep.rs"),
                    rationale: None,
                    content: Some("fn keep() {}".to_string()),
                },
                SelectedFile {
                    path: PathBuf::from("src/drop.rs"),
                    rationale: None,
                    content: Some("fn drop() {}".to_string()),
                },
            ],
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(compiled
            .injected_prompt
            .contains("selected_file: src/keep.rs"));
        assert!(!compiled
            .injected_prompt
            .contains("selected_file: src/drop.rs"));
        assert!(!compiled.injected_prompt.contains("parent_summary:"));
    }

    #[test]
    fn expanded_brief_mode_uses_parent_summary_digest() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::ExpandedBrief);
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: Some("word ".repeat(120)),
            selected_files: vec![SelectedFile {
                path: PathBuf::from("src/ignored.rs"),
                rationale: None,
                content: Some("fn ignored() {}".to_string()),
            }],
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(compiled.injected_prompt.contains("parent_summary_digest:"));
        assert!(!compiled
            .injected_prompt
            .contains("selected_file: src/ignored.rs"));
    }

    #[test]
    fn summary_only_blocks_raw_transcript_like_parent_summary() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::SummaryOnly);
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: Some("User: hi\nAssistant: hello".to_string()),
            selected_files: Vec::new(),
            stage: None,
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(compiled
            .injected_prompt
            .contains("suppressed because it appears to contain raw transcript"));
        assert!(!compiled.injected_prompt.contains("User: hi"));
    }

    #[test]
    fn validates_default_summary_contract_template() {
        validate_default_summary_contract_template().expect("default template should be valid");
    }

    #[test]
    fn rejects_template_missing_required_sections() {
        let err = validate_compiled_prompt_template(
            "ROLE\nonly role and no sentinels or required sections",
        )
        .expect_err("missing sections should fail");
        assert!(
            err.to_string().contains("missing section"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn includes_stage_role_priority_when_stage_present() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec(ContextMode::Isolated);
        let req = RunRequest {
            task: "task".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            stage: Some("build".to_string()),
            plan_ref: None,
            working_dir: PathBuf::from("."),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        };

        let compiled = compiler
            .compile(&spec, &req, ResolvedMemory::default())
            .expect("compile");
        assert!(compiled.injected_prompt.contains("WorkflowStage: build"));
        assert!(compiled
            .injected_prompt
            .contains("StageRolePriority: Builder -> Reviewer -> Planner"));
    }
}
