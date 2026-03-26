use std::{
    fs,
    path::{Path, PathBuf},
};

use walkdir::WalkDir;

use crate::error::{McpSubagentError, Result};

use super::{validate::validate_agent_spec, AgentSpec};

#[derive(Debug, Clone)]
pub struct LoadedAgentSpec {
    pub path: PathBuf,
    pub spec: AgentSpec,
}

pub fn load_agent_spec(path: &Path) -> Result<AgentSpec> {
    let raw = fs::read_to_string(path)?;
    let spec: AgentSpec = toml::from_str(&raw).map_err(|source| McpSubagentError::Toml {
        path: path.to_path_buf(),
        source,
    })?;
    validate_agent_spec(&spec)?;
    Ok(spec)
}

pub fn load_agent_specs_from_dir(root: &Path) -> Result<Vec<LoadedAgentSpec>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut loaded = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_agent_toml(path) {
            continue;
        }
        let spec = load_agent_spec(path)?;
        loaded.push(LoadedAgentSpec {
            path: path.to_path_buf(),
            spec,
        });
    }

    loaded.sort_by(|a, b| a.spec.core.name.cmp(&b.spec.core.name));
    Ok(loaded)
}

pub fn load_agent_specs_from_dirs(dirs: &[PathBuf]) -> Result<Vec<LoadedAgentSpec>> {
    let mut all = Vec::new();
    for dir in dirs {
        let mut loaded = load_agent_specs_from_dir(dir)?;
        all.append(&mut loaded);
    }
    all.sort_by(|a, b| a.spec.core.name.cmp(&b.spec.core.name));
    Ok(all)
}

fn is_agent_toml(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".agent.toml"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::load_agent_specs_from_dir;

    #[test]
    fn loads_agent_specs_and_applies_defaults() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("reviewer.agent.toml");
        let raw = r#"
[core]
name = "reviewer"
description = "review code"
provider = "codex"
instructions = "review"
"#;
        fs::write(&file, raw).expect("write file");

        let loaded = load_agent_specs_from_dir(dir.path()).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].spec.core.name, "reviewer");
        assert!(loaded[0].spec.core.model.is_none());
        assert_eq!(loaded[0].spec.runtime.timeout_secs, 900);
        assert!(loaded[0].spec.provider_overrides.claude.is_none());
        assert!(loaded[0].spec.provider_overrides.codex.is_none());
        assert!(loaded[0].spec.provider_overrides.gemini.is_none());
        assert!(loaded[0].spec.workflow.is_none());
    }

    #[test]
    fn rejects_unknown_fields() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("broken.agent.toml");
        let raw = r#"
[core]
name = "broken"
description = "desc"
provider = "codex"
instructions = "review"
unknown_field = "boom"
"#;
        fs::write(&file, raw).expect("write file");

        let err = load_agent_specs_from_dir(dir.path()).expect_err("must fail");
        assert!(
            err.to_string().contains("unknown field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn loads_partial_workflow_subtables_with_consistent_defaults() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("workflow.agent.toml");
        let raw = r#"
[core]
name = "workflow-builder"
description = "workflow builder"
provider = "codex"
instructions = "build"

[workflow]
enabled = true

[workflow.require_plan_when]
require_plan_if_cross_module = false

[workflow.knowledge_capture]
update_project_memory = true
"#;
        fs::write(&file, raw).expect("write file");

        let loaded = load_agent_specs_from_dir(dir.path()).expect("load");
        let workflow = loaded[0]
            .spec
            .workflow
            .as_ref()
            .expect("workflow should exist");

        assert_eq!(
            workflow.require_plan_when.require_plan_if_touched_files_ge,
            Some(5)
        );
        assert!(!workflow.require_plan_when.require_plan_if_cross_module);
        assert!(workflow.require_plan_when.require_plan_if_parallel_agents);
        assert_eq!(
            workflow
                .require_plan_when
                .require_plan_if_estimated_runtime_minutes_ge,
            Some(15)
        );
        assert_eq!(
            workflow.knowledge_capture.trigger_if_touched_files_gt,
            Some(3)
        );
        assert!(workflow.knowledge_capture.trigger_if_new_config);
        assert!(workflow.knowledge_capture.trigger_if_behavior_change);
        assert!(workflow.knowledge_capture.trigger_if_non_obvious_bugfix);
        assert!(workflow.knowledge_capture.update_project_memory);
    }

    #[test]
    fn loads_partial_runtime_subtables_with_consistent_defaults() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("runtime.agent.toml");
        let raw = r#"
[core]
name = "runtime-builder"
description = "runtime builder"
provider = "codex"
instructions = "build"

[runtime]
timeout_secs = 30

[runtime.artifact_policy]

[runtime.retry_policy]
backoff_secs = 0
"#;
        fs::write(&file, raw).expect("write file");

        let loaded = load_agent_specs_from_dir(dir.path()).expect("load");
        let runtime = &loaded[0].spec.runtime;

        assert_eq!(runtime.timeout_secs, 30);
        assert!(runtime.artifact_policy.emit_summary_json);
        assert_eq!(runtime.retry_policy.max_attempts, 1);
        assert_eq!(runtime.retry_policy.backoff_secs, 0);
    }
}
