use crate::{
    error::{McpSubagentError, Result},
    spec::{
        runtime_policy::{SandboxPolicy, WorkingDirPolicy},
        AgentSpec, Provider,
    },
};

pub fn validate_agent_spec(spec: &AgentSpec) -> Result<()> {
    validate_provider_overrides(spec)?;
    validate_runtime_policy(spec)?;
    Ok(())
}

fn validate_provider_overrides(spec: &AgentSpec) -> Result<()> {
    match spec.core.provider {
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

    if matches!(spec.runtime.sandbox, SandboxPolicy::ReadOnly)
        && matches!(
            spec.runtime.working_dir_policy,
            WorkingDirPolicy::GitWorktree
        )
    {
        return Err(McpSubagentError::SpecValidation(
            "ReadOnly sandbox cannot use working_dir_policy = GitWorktree".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::spec::{
        core::{AgentSpecCore, Provider},
        provider_overrides::{CodexOverrides, ProviderOverrides},
        runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
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
    fn rejects_invalid_readonly_write_policy_combo() {
        let mut spec = base_spec();
        spec.runtime.working_dir_policy = WorkingDirPolicy::GitWorktree;

        let err = validate_agent_spec(&spec).expect_err("readonly+tempcopy should fail");
        assert!(
            err.to_string().contains("ReadOnly sandbox cannot"),
            "unexpected error: {err}"
        );
    }
}
