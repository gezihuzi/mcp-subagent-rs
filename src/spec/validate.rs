use std::collections::HashSet;
use std::path::{Component, Path};

use crate::{
    error::{McpSubagentError, Result},
    spec::{
        runtime_policy::{DelegationContextPolicy, MemorySource},
        workflow::WorkflowStageKind,
        AgentSpec, Provider,
    },
};

pub fn validate_agent_spec(spec: &AgentSpec) -> Result<()> {
    validate_provider_overrides(spec)?;
    validate_memory_sources(spec)?;
    validate_runtime_policy(spec)?;
    validate_workflow_spec(spec)?;
    Ok(())
}

fn validate_provider_overrides(spec: &AgentSpec) -> Result<()> {
    match spec.core.provider {
        Provider::Mock => {
            if spec.provider_overrides.claude.is_some()
                || spec.provider_overrides.codex.is_some()
                || spec.provider_overrides.gemini.is_some()
            {
                return Err(McpSubagentError::SpecValidation(
                    "provider overrides are not supported for Mock".to_string(),
                ));
            }
        }
        Provider::Claude => {
            if spec.provider_overrides.codex.is_some() || spec.provider_overrides.gemini.is_some() {
                return Err(McpSubagentError::SpecValidation(
                    "non-Claude override exists for Claude provider".to_string(),
                ));
            }
        }
        Provider::Codex => {
            if spec.provider_overrides.claude.is_some() || spec.provider_overrides.gemini.is_some()
            {
                return Err(McpSubagentError::SpecValidation(
                    "non-Codex override exists for Codex provider".to_string(),
                ));
            }
        }
        Provider::Gemini => {
            if spec.provider_overrides.claude.is_some() || spec.provider_overrides.codex.is_some() {
                return Err(McpSubagentError::SpecValidation(
                    "non-Gemini override exists for Gemini provider".to_string(),
                ));
            }
        }
        Provider::Ollama => {
            if spec.provider_overrides.claude.is_some()
                || spec.provider_overrides.codex.is_some()
                || spec.provider_overrides.gemini.is_some()
            {
                return Err(McpSubagentError::SpecValidation(
                    "provider overrides are not supported for Ollama".to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn validate_runtime_policy(spec: &AgentSpec) -> Result<()> {
    if spec.runtime.timeout_secs == 0 {
        return Err(McpSubagentError::SpecValidation(
            "timeout_secs must be greater than 0".to_string(),
        ));
    }

    if spec.runtime.max_turns == Some(0) {
        return Err(McpSubagentError::SpecValidation(
            "max_turns must be greater than 0 when provided".to_string(),
        ));
    }

    if matches!(
        spec.runtime.delegation_context,
        DelegationContextPolicy::PlanSection
    ) && spec
        .runtime
        .plan_section_selector
        .as_deref()
        .map(str::trim)
        .map(|value| value.is_empty())
        .unwrap_or(true)
    {
        return Err(McpSubagentError::SpecValidation(
            "delegation_context=plan_section requires runtime.plan_section_selector".to_string(),
        ));
    }

    if let Some(selector) = spec.runtime.plan_section_selector.as_deref() {
        if selector.trim().is_empty() {
            return Err(McpSubagentError::SpecValidation(
                "runtime.plan_section_selector must not be empty".to_string(),
            ));
        }
    }

    Ok(())
}

fn validate_memory_sources(spec: &AgentSpec) -> Result<()> {
    for source in &spec.runtime.memory_sources {
        match source {
            MemorySource::AutoProjectMemory
            | MemorySource::ActivePlan
            | MemorySource::ArchivedPlans => {}
            MemorySource::Inline(content) => {
                if content.trim().is_empty() {
                    return Err(McpSubagentError::SpecValidation(
                        "Inline memory source must not be empty".to_string(),
                    ));
                }
            }
            MemorySource::File(path) => {
                validate_relative_memory_path("File", path, false)?;
            }
            MemorySource::Glob(pattern) => {
                validate_relative_memory_path("Glob", pattern, true)?;
            }
        }
    }
    Ok(())
}

fn validate_workflow_spec(spec: &AgentSpec) -> Result<()> {
    let Some(workflow) = &spec.workflow else {
        return Ok(());
    };

    if workflow.max_runtime_depth == 0 {
        return Err(McpSubagentError::SpecValidation(
            "workflow.max_runtime_depth must be greater than 0".to_string(),
        ));
    }

    if workflow.enabled && workflow.stages.is_empty() {
        return Err(McpSubagentError::SpecValidation(
            "workflow.stages must not be empty when workflow is enabled".to_string(),
        ));
    }

    if workflow
        .require_plan_when
        .require_plan_if_touched_files_ge
        .is_some_and(|value| value == 0)
    {
        return Err(McpSubagentError::SpecValidation(
            "workflow.require_plan_when.require_plan_if_touched_files_ge must be greater than 0"
                .to_string(),
        ));
    }

    if workflow
        .require_plan_when
        .require_plan_if_estimated_runtime_minutes_ge
        .is_some_and(|value| value == 0)
    {
        return Err(McpSubagentError::SpecValidation(
            "workflow.require_plan_when.require_plan_if_estimated_runtime_minutes_ge must be greater than 0"
                .to_string(),
        ));
    }

    validate_unique_stages("workflow.stages", &workflow.stages)?;
    validate_unique_stages("workflow.allowed_stages", &workflow.allowed_stages)?;

    if !workflow.allowed_stages.is_empty() {
        let allowed = workflow
            .allowed_stages
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        for stage in &workflow.stages {
            if !allowed.contains(stage) {
                return Err(McpSubagentError::SpecValidation(format!(
                    "workflow.stages contains `{stage:?}` which is not in workflow.allowed_stages"
                )));
            }
        }
    }

    Ok(())
}

fn validate_unique_stages(field: &str, stages: &[WorkflowStageKind]) -> Result<()> {
    let mut seen = HashSet::new();
    for stage in stages {
        if !seen.insert(stage.clone()) {
            return Err(McpSubagentError::SpecValidation(format!(
                "{field} contains duplicated stage `{stage:?}`"
            )));
        }
    }
    Ok(())
}

fn validate_relative_memory_path(kind: &str, raw: &str, allow_glob: bool) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(McpSubagentError::SpecValidation(format!(
            "{kind} memory source path must not be empty"
        )));
    }

    if !allow_glob && contains_glob_meta(value) {
        return Err(McpSubagentError::SpecValidation(format!(
            "{kind} memory source path must not contain glob pattern: {value}"
        )));
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return Err(McpSubagentError::SpecValidation(format!(
            "{kind} memory source path must be relative: {value}"
        )));
    }

    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(McpSubagentError::SpecValidation(format!(
                    "{kind} memory source path cannot traverse parent directory: {value}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(McpSubagentError::SpecValidation(format!(
                    "{kind} memory source path must be relative: {value}"
                )));
            }
        }
    }

    Ok(())
}

fn contains_glob_meta(value: &str) -> bool {
    value
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

#[cfg(test)]
mod tests {
    use crate::spec::{
        core::{AgentSpecCore, Provider},
        provider_overrides::{CodexOverrides, ProviderOverrides},
        runtime_policy::{
            DelegationContextPolicy, MemorySource, RuntimePolicy, SandboxPolicy, WorkingDirPolicy,
        },
        workflow::{WorkflowSpec, WorkflowStageKind},
        AgentSpec,
    };

    use super::validate_agent_spec;

    fn base_spec() -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "reviewer".to_string(),
                description: "desc".to_string(),
                provider: Provider::Codex,
                model: None,
                instructions: "review".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: Default::default(),
            },
            runtime: RuntimePolicy {
                sandbox: SandboxPolicy::ReadOnly,
                working_dir_policy: WorkingDirPolicy::InPlace,
                ..RuntimePolicy::default()
            },
            provider_overrides: ProviderOverrides {
                codex: Some(CodexOverrides {
                    model_reasoning_effort: None,
                    sandbox_mode: None,
                }),
                ..ProviderOverrides::default()
            },
            workflow: None,
        }
    }

    #[test]
    fn rejects_override_mismatch() {
        let mut spec = base_spec();
        spec.provider_overrides.claude = Some(crate::spec::provider_overrides::ClaudeOverrides {
            permission_mode: None,
        });

        let err = validate_agent_spec(&spec).expect_err("mismatch should fail");
        assert!(
            err.to_string().contains("non-Codex override"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn allows_readonly_gitworktree_combo_in_spec_validation() {
        let mut spec = base_spec();
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;
        validate_agent_spec(&spec).expect("combo should be stage-gated at runtime");
    }

    #[test]
    fn rejects_absolute_file_memory_source_path() {
        let mut spec = base_spec();
        spec.runtime.memory_sources = vec![MemorySource::File("/tmp/PROJECT.md".to_string())];

        let err = validate_agent_spec(&spec).expect_err("absolute memory source must fail");
        assert!(
            err.to_string().contains("must be relative"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_parent_dir_glob_memory_source_path() {
        let mut spec = base_spec();
        spec.runtime.memory_sources = vec![MemorySource::Glob("../secrets/*.md".to_string())];

        let err = validate_agent_spec(&spec).expect_err("parent traversal must fail");
        assert!(
            err.to_string().contains("cannot traverse parent directory"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_glob_pattern_in_file_memory_source_path() {
        let mut spec = base_spec();
        spec.runtime.memory_sources = vec![MemorySource::File("docs/*.md".to_string())];

        let err = validate_agent_spec(&spec).expect_err("file path glob must fail");
        assert!(
            err.to_string().contains("must not contain glob pattern"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_empty_inline_memory_source() {
        let mut spec = base_spec();
        spec.runtime.memory_sources = vec![MemorySource::Inline("   ".to_string())];

        let err = validate_agent_spec(&spec).expect_err("empty inline content must fail");
        assert!(
            err.to_string().contains("must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_safe_memory_sources() {
        let mut spec = base_spec();
        spec.runtime.memory_sources = vec![
            MemorySource::AutoProjectMemory,
            MemorySource::ActivePlan,
            MemorySource::ArchivedPlans,
            MemorySource::File("PROJECT.md".to_string()),
            MemorySource::Glob("docs/**/*.md".to_string()),
            MemorySource::Inline("stable project memory".to_string()),
        ];

        validate_agent_spec(&spec).expect("safe memory source paths should pass");
    }

    #[test]
    fn rejects_plan_section_delegation_without_selector() {
        let mut spec = base_spec();
        spec.runtime.delegation_context = DelegationContextPolicy::PlanSection;
        spec.runtime.plan_section_selector = None;

        let err = validate_agent_spec(&spec).expect_err("missing plan section selector must fail");
        assert!(
            err.to_string().contains("plan_section_selector"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_plan_section_delegation_with_selector() {
        let mut spec = base_spec();
        spec.runtime.delegation_context = DelegationContextPolicy::PlanSection;
        spec.runtime.plan_section_selector = Some("Acceptance Criteria".to_string());

        validate_agent_spec(&spec).expect("selector should satisfy plan_section delegation");
    }

    #[test]
    fn rejects_zero_workflow_depth() {
        let mut spec = base_spec();
        spec.workflow = Some(WorkflowSpec {
            max_runtime_depth: 0,
            ..WorkflowSpec::default()
        });

        let err = validate_agent_spec(&spec).expect_err("zero workflow depth must fail");
        assert!(
            err.to_string().contains("max_runtime_depth"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_empty_stages_for_enabled_workflow() {
        let mut spec = base_spec();
        spec.workflow = Some(WorkflowSpec {
            enabled: true,
            stages: Vec::new(),
            ..WorkflowSpec::default()
        });

        let err = validate_agent_spec(&spec).expect_err("enabled workflow needs stages");
        assert!(
            err.to_string().contains("workflow.stages"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_duplicate_workflow_stages() {
        let mut spec = base_spec();
        spec.workflow = Some(WorkflowSpec {
            stages: vec![WorkflowStageKind::Plan, WorkflowStageKind::Plan],
            ..WorkflowSpec::default()
        });

        let err = validate_agent_spec(&spec).expect_err("duplicated stages must fail");
        assert!(
            err.to_string().contains("duplicated stage"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_stage_not_in_allowed_stages() {
        let mut spec = base_spec();
        spec.workflow = Some(WorkflowSpec {
            stages: vec![WorkflowStageKind::Build],
            allowed_stages: vec![WorkflowStageKind::Plan],
            ..WorkflowSpec::default()
        });

        let err = validate_agent_spec(&spec).expect_err("stage outside allowlist must fail");
        assert!(
            err.to_string().contains("not in workflow.allowed_stages"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accepts_workflow_with_consistent_stage_allowlist() {
        let mut spec = base_spec();
        spec.workflow = Some(WorkflowSpec {
            stages: vec![WorkflowStageKind::Plan, WorkflowStageKind::Build],
            allowed_stages: vec![WorkflowStageKind::Plan, WorkflowStageKind::Build],
            max_runtime_depth: 1,
            ..WorkflowSpec::default()
        });

        validate_agent_spec(&spec).expect("workflow should pass with consistent settings");
    }
}
