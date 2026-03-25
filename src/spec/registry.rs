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
        assert_eq!(loaded[0].spec.runtime.timeout_secs, 900);
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
}
