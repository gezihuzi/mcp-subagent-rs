use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::error::{McpSubagentError, Result};

const DEFAULT_AGENTS_DIR: &str = "./agents";
const DEFAULT_STATE_DIR: &str = ".mcp-subagent/state";
const ENV_CONFIG_PATH: &str = "MCP_SUBAGENT_CONFIG";
const ENV_AGENTS_DIRS: &str = "MCP_SUBAGENT_AGENTS_DIRS";
const ENV_STATE_DIR: &str = "MCP_SUBAGENT_STATE_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    pub config_path: Option<PathBuf>,
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: Option<PathBuf>,
}

pub fn resolve_runtime_config(overrides: ConfigOverrides) -> Result<RuntimeConfig> {
    let file_cfg = load_file_config(resolve_config_path(overrides.config_path.as_ref()))?;
    let env_layer = env_layer();

    let defaults = ConfigLayer {
        agents_dirs: Some(vec![PathBuf::from(DEFAULT_AGENTS_DIR)]),
        state_dir: Some(PathBuf::from(DEFAULT_STATE_DIR)),
    };
    let file_layer = file_cfg
        .as_ref()
        .map(file_layer_from_cfg)
        .unwrap_or_default();
    let cli_layer = ConfigLayer {
        agents_dirs: non_empty_dirs(overrides.agents_dirs),
        state_dir: overrides.state_dir,
    };

    Ok(merge_layers(defaults, file_layer, env_layer, cli_layer))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConfigLayer {
    agents_dirs: Option<Vec<PathBuf>>,
    state_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    paths: FilePaths,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePaths {
    #[serde(default)]
    agents_dirs: Vec<PathBuf>,
    #[serde(default)]
    state_dir: Option<PathBuf>,
}

fn resolve_config_path(cli_path: Option<&PathBuf>) -> PathBuf {
    if let Some(path) = cli_path {
        return path.clone();
    }
    if let Ok(raw) = env::var(ENV_CONFIG_PATH) {
        let path = raw.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    default_config_path()
}

fn default_config_path() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        return Path::new(&home)
            .join(".config")
            .join("mcp-subagent")
            .join("config.toml");
    }
    PathBuf::from("./.mcp-subagent/config.toml")
}

fn load_file_config(path: PathBuf) -> Result<Option<FileConfig>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)?;
    let parsed = toml::from_str::<FileConfig>(&raw).map_err(|source| McpSubagentError::Toml {
        path: path.clone(),
        source,
    })?;
    Ok(Some(parsed))
}

fn file_layer_from_cfg(cfg: &FileConfig) -> ConfigLayer {
    ConfigLayer {
        agents_dirs: non_empty_dirs(cfg.paths.agents_dirs.clone()),
        state_dir: cfg.paths.state_dir.clone(),
    }
}

fn env_layer() -> ConfigLayer {
    ConfigLayer {
        agents_dirs: env::var(ENV_AGENTS_DIRS).ok().and_then(parse_dirs_env),
        state_dir: env::var(ENV_STATE_DIR)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
    }
}

fn parse_dirs_env(raw: String) -> Option<Vec<PathBuf>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = if trimmed.contains(',') {
        trimmed
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else {
        env::split_paths(trimmed).collect::<Vec<_>>()
    };

    non_empty_dirs(parsed)
}

fn non_empty_dirs(dirs: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
    let cleaned = dirs
        .into_iter()
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn merge_layers(
    defaults: ConfigLayer,
    file_layer: ConfigLayer,
    env_layer: ConfigLayer,
    cli_layer: ConfigLayer,
) -> RuntimeConfig {
    let mut agents_dirs = defaults.agents_dirs.unwrap_or_default();
    let mut state_dir = defaults
        .state_dir
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_DIR));

    if let Some(v) = file_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = file_layer.state_dir {
        state_dir = v;
    }

    if let Some(v) = env_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = env_layer.state_dir {
        state_dir = v;
    }

    if let Some(v) = cli_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = cli_layer.state_dir {
        state_dir = v;
    }

    RuntimeConfig {
        agents_dirs,
        state_dir,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{merge_layers, ConfigLayer};

    #[test]
    fn merge_uses_precedence_cli_env_file_defaults() {
        let defaults = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("default-agents")]),
            state_dir: Some(PathBuf::from("default-state")),
        };
        let file = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("file-agents")]),
            state_dir: Some(PathBuf::from("file-state")),
        };
        let env = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("env-agents")]),
            state_dir: Some(PathBuf::from("env-state")),
        };
        let cli = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("cli-agents")]),
            state_dir: Some(PathBuf::from("cli-state")),
        };

        let merged = merge_layers(defaults, file, env, cli);
        assert_eq!(merged.agents_dirs, vec![PathBuf::from("cli-agents")]);
        assert_eq!(merged.state_dir, PathBuf::from("cli-state"));
    }

    #[test]
    fn merge_falls_back_when_higher_layers_missing() {
        let defaults = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("default-agents")]),
            state_dir: Some(PathBuf::from("default-state")),
        };
        let file = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("file-agents")]),
            state_dir: Some(PathBuf::from("file-state")),
        };

        let merged = merge_layers(
            defaults,
            file,
            ConfigLayer::default(),
            ConfigLayer::default(),
        );
        assert_eq!(merged.agents_dirs, vec![PathBuf::from("file-agents")]);
        assert_eq!(merged.state_dir, PathBuf::from("file-state"));
    }
}
