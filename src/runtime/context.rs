use std::fmt::Write;

use crate::{
    error::Result,
    runtime::summary::{
        parse_structured_summary, StructuredSummary, SUMMARY_END_SENTINEL, SUMMARY_START_SENTINEL,
    },
    spec::AgentSpec,
    types::{
        CompiledContext, ContextSourceRef, InjectionMode, MemorySnippet, ResolvedMemory, RunRequest,
    },
};

pub trait ContextCompiler: Send + Sync {
    fn compile(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext>;

    fn parse_summary(&self, raw_stdout: &str, raw_stderr: &str) -> Result<StructuredSummary>;
}

#[derive(Debug, Default)]
pub struct DefaultContextCompiler;

impl ContextCompiler for DefaultContextCompiler {
    fn compile(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext> {
        let mut source_manifest = Vec::new();
        let mut selected_context = String::new();

        if let Some(parent_summary) = request.parent_summary.as_deref() {
            writeln!(&mut selected_context, "parent_summary:\n{parent_summary}\n")
                .expect("write to string");
        }

        for selected in &request.selected_files {
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

        let task_line = request
            .task_brief
            .as_deref()
            .unwrap_or(request.task.as_str())
            .trim();
        let acceptance = if request.acceptance_criteria.is_empty() {
            "1) Return structured summary JSON in sentinels.\n2) Keep response concise and actionable."
                .to_string()
        } else {
            request
                .acceptance_criteria
                .iter()
                .enumerate()
                .map(|(idx, item)| format!("{} ) {item}", idx + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let response_contract = format!(
            "Return a machine-readable JSON block only inside sentinels. The JSON must match StructuredSummary.\n{}\n{{...valid json...}}\n{}",
            SUMMARY_START_SENTINEL, SUMMARY_END_SENTINEL
        );

        let injected_prompt = format!(
            "ROLE\nYou are agent `{name}` for provider `{provider}`.\n\nTASK\n{task}\n\nOBJECTIVE\n{objective}\n\nCONSTRAINTS\n{constraints}\n\nACCEPTANCE CRITERIA\n{acceptance}\n\nSELECTED CONTEXT\n{context}\n\nRESPONSE CONTRACT\n{contract}\n\nOUTPUT SENTINELS\n{start}\n{{...valid json...}}\n{end}\n",
            name = spec.core.name,
            provider = spec.core.provider.as_str(),
            task = task_line,
            objective = request.task,
            constraints = compile_constraints(spec),
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

    fn parse_summary(&self, raw_stdout: &str, raw_stderr: &str) -> Result<StructuredSummary> {
        Ok(parse_structured_summary(raw_stdout, raw_stderr))
    }
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

fn compile_constraints(spec: &AgentSpec) -> String {
    format!(
        "Do not request or rely on parent raw transcript.\nFollow agent instructions:\n{}\nProvider: {}\nContextMode: {:?}",
        spec.core.instructions,
        spec.core.provider.as_str(),
        spec.runtime.context_mode
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        runtime::context::{ContextCompiler, DefaultContextCompiler},
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::RuntimePolicy,
            AgentSpec,
        },
        types::{MemorySnippet, ResolvedMemory, RunMode, RunRequest, SelectedFile},
    };

    fn sample_spec() -> AgentSpec {
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
            runtime: RuntimePolicy::default(),
            provider_overrides: Default::default(),
        }
    }

    #[test]
    fn compile_contains_required_sections() {
        let compiler = DefaultContextCompiler;
        let spec = sample_spec();
        let req = RunRequest {
            task: "Review parser changes".to_string(),
            task_brief: Some("Review parser module".to_string()),
            parent_summary: Some("parent summary".to_string()),
            selected_files: vec![SelectedFile {
                path: PathBuf::from("src/parser.rs"),
                rationale: Some("target file".to_string()),
                content: Some("fn parse() {}".to_string()),
            }],
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
}
