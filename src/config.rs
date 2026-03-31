use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    cwd::resolve_cli_cwd,
    error::{McpSubagentError, Result},
    render::RenderStyle,
    spec::runtime_policy::WorkingDirPolicy,
};

const DEFAULT_AGENTS_DIR: &str = "./agents";
const DEFAULT_STATE_DIR: &str = ".mcp-subagent/state";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_PROJECT_CONFIG_PATH: &str = "./.mcp-subagent/config.toml";
const ENV_CONFIG_PATH: &str = "MCP_SUBAGENT_CONFIG";
const ENV_AGENTS_DIRS: &str = "MCP_SUBAGENT_AGENTS_DIRS";
const ENV_STATE_DIR: &str = "MCP_SUBAGENT_STATE_DIR";
const ENV_LOG_LEVEL: &str = "MCP_SUBAGENT_LOG_LEVEL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: PathBuf,
    pub log_level: String,
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub agent: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub stream: Option<bool>,
    pub working_dir_policy: Option<WorkingDirPolicy>,
    pub render_style: Option<RenderStyle>,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    pub config_path: Option<PathBuf>,
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub log_level: Option<String>,
}

pub fn resolve_runtime_config(overrides: ConfigOverrides) -> Result<RuntimeConfig> {
    let file_cfg = load_file_config(resolve_config_path(overrides.config_path.as_ref()))?;
    let profiles = file_cfg
        .as_ref()
        .map(|cfg| cfg.profiles.clone())
        .unwrap_or_default();
    let env_layer = env_layer();

    let defaults = ConfigLayer {
        agents_dirs: Some(vec![PathBuf::from(DEFAULT_AGENTS_DIR)]),
        state_dir: Some(PathBuf::from(DEFAULT_STATE_DIR)),
        log_level: Some(DEFAULT_LOG_LEVEL.to_string()),
    };
    let file_layer = file_cfg
        .as_ref()
        .map(file_layer_from_cfg)
        .unwrap_or_default();
    let cli_layer = ConfigLayer {
        agents_dirs: non_empty_dirs(overrides.agents_dirs),
        state_dir: overrides.state_dir,
        log_level: non_empty_string(overrides.log_level),
    };

    let mut merged = merge_layers(defaults, file_layer, env_layer, cli_layer);
    merged.profiles = profiles;
    Ok(merged)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConfigLayer {
    agents_dirs: Option<Vec<PathBuf>>,
    state_dir: Option<PathBuf>,
    log_level: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default = "default_file_server")]
    server: FileServer,
    #[serde(default = "default_file_paths")]
    paths: FilePaths,
    #[serde(default = "default_profiles")]
    profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileServer {
    #[serde(rename = "transport")]
    _transport: Option<String>,
    log_level: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePaths {
    #[serde(default = "default_path_vec")]
    agents_dirs: Vec<PathBuf>,
    state_dir: Option<PathBuf>,
}

fn default_file_server() -> FileServer {
    FileServer::default()
}

fn default_file_paths() -> FilePaths {
    FilePaths::default()
}

fn default_path_vec() -> Vec<PathBuf> {
    Vec::new()
}

fn default_profiles() -> HashMap<String, ProfileConfig> {
    HashMap::new()
}

fn resolve_config_path(cli_path: Option<&PathBuf>) -> PathBuf {
    let env_config = env::var(ENV_CONFIG_PATH).ok();
    let cwd = resolve_cli_cwd().ok();
    let home = env::var("HOME").ok().map(PathBuf::from);
    resolve_config_path_with(
        cli_path,
        env_config.as_deref(),
        cwd.as_deref(),
        home.as_deref(),
    )
}

fn resolve_config_path_with(
    cli_path: Option<&PathBuf>,
    env_config_path: Option<&str>,
    cwd: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    if let Some(path) = cli_path {
        return path.clone();
    }
    if let Some(raw) = env_config_path {
        let path = raw.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    if let Some(project_config_path) = cwd.map(default_project_config_path) {
        if project_config_path.exists() {
            return project_config_path;
        }
    }

    default_config_path(home_dir)
}

fn default_project_config_path(cwd: &Path) -> PathBuf {
    cwd.join(".mcp-subagent").join("config.toml")
}

fn default_config_path(home_dir: Option<&Path>) -> PathBuf {
    if let Some(home) = home_dir {
        return home
            .join(".config")
            .join("mcp-subagent")
            .join("config.toml");
    }
    PathBuf::from(DEFAULT_PROJECT_CONFIG_PATH)
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
        log_level: non_empty_string(cfg.server.log_level.clone()),
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
        log_level: non_empty_string(env::var(ENV_LOG_LEVEL).ok()),
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

fn non_empty_string(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty())
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
    let mut log_level = defaults
        .log_level
        .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string());

    if let Some(v) = file_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = file_layer.state_dir {
        state_dir = v;
    }
    if let Some(v) = file_layer.log_level {
        log_level = v;
    }

    if let Some(v) = env_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = env_layer.state_dir {
        state_dir = v;
    }
    if let Some(v) = env_layer.log_level {
        log_level = v;
    }

    if let Some(v) = cli_layer.agents_dirs {
        agents_dirs = v;
    }
    if let Some(v) = cli_layer.state_dir {
        state_dir = v;
    }
    if let Some(v) = cli_layer.log_level {
        log_level = v;
    }

    RuntimeConfig {
        agents_dirs,
        state_dir,
        log_level,
        profiles: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::{render::RenderStyle, spec::runtime_policy::WorkingDirPolicy};
    use tempfile::tempdir;

    use super::{
        file_layer_from_cfg, load_file_config, merge_layers, resolve_config_path_with, ConfigLayer,
        FileConfig,
    };

    #[test]
    fn merge_uses_precedence_cli_env_file_defaults() {
        let defaults = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("default-agents")]),
            state_dir: Some(PathBuf::from("default-state")),
            log_level: Some("info".to_string()),
        };
        let file = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("file-agents")]),
            state_dir: Some(PathBuf::from("file-state")),
            log_level: Some("warn".to_string()),
        };
        let env = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("env-agents")]),
            state_dir: Some(PathBuf::from("env-state")),
            log_level: Some("debug".to_string()),
        };
        let cli = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("cli-agents")]),
            state_dir: Some(PathBuf::from("cli-state")),
            log_level: Some("trace".to_string()),
        };

        let merged = merge_layers(defaults, file, env, cli);
        assert_eq!(merged.agents_dirs, vec![PathBuf::from("cli-agents")]);
        assert_eq!(merged.state_dir, PathBuf::from("cli-state"));
        assert_eq!(merged.log_level, "trace");
    }

    #[test]
    fn merge_falls_back_when_higher_layers_missing() {
        let defaults = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("default-agents")]),
            state_dir: Some(PathBuf::from("default-state")),
            log_level: Some("info".to_string()),
        };
        let file = ConfigLayer {
            agents_dirs: Some(vec![PathBuf::from("file-agents")]),
            state_dir: Some(PathBuf::from("file-state")),
            log_level: Some("warn".to_string()),
        };

        let merged = merge_layers(
            defaults,
            file,
            ConfigLayer::default(),
            ConfigLayer::default(),
        );
        assert_eq!(merged.agents_dirs, vec![PathBuf::from("file-agents")]);
        assert_eq!(merged.state_dir, PathBuf::from("file-state"));
        assert_eq!(merged.log_level, "warn");
    }

    #[test]
    fn resolve_config_path_prefers_cli_override() {
        let resolved = resolve_config_path_with(
            Some(&PathBuf::from("/tmp/cli.toml")),
            Some("/tmp/env.toml"),
            None,
            None,
        );
        assert_eq!(resolved, PathBuf::from("/tmp/cli.toml"));
    }

    #[test]
    fn resolve_config_path_prefers_env_when_cli_missing() {
        let resolved = resolve_config_path_with(
            None,
            Some("/tmp/env.toml"),
            None,
            Some(PathBuf::from("/home/user").as_path()),
        );
        assert_eq!(resolved, PathBuf::from("/tmp/env.toml"));
    }

    #[test]
    fn resolve_config_path_prefers_project_config_when_present() {
        let cwd = tempdir().expect("tempdir");
        let config = cwd.path().join(".mcp-subagent/config.toml");
        fs::create_dir_all(config.parent().expect("parent")).expect("create dir");
        fs::write(&config, "[server]\nlog_level='info'\n").expect("write");

        let resolved = resolve_config_path_with(
            None,
            None,
            Some(cwd.path()),
            Some(PathBuf::from("/home/user").as_path()),
        );
        assert_eq!(resolved, config);
    }

    #[test]
    fn resolve_config_path_falls_back_to_home_when_project_config_missing() {
        let cwd = tempdir().expect("tempdir");
        let resolved = resolve_config_path_with(
            None,
            None,
            Some(cwd.path()),
            Some(PathBuf::from("/home/user").as_path()),
        );
        assert_eq!(
            resolved,
            PathBuf::from("/home/user/.config/mcp-subagent/config.toml")
        );
    }

    #[test]
    fn resolve_config_path_falls_back_to_project_relative_without_home() {
        let resolved = resolve_config_path_with(None, None, None, None);
        assert_eq!(resolved, PathBuf::from("./.mcp-subagent/config.toml"));
    }

    #[test]
    fn file_config_direct_deserialization_preserves_section_defaults() {
        let cfg: FileConfig = toml::from_str("").expect("file config should parse");

        assert!(cfg.server._transport.is_none());
        assert!(cfg.server.log_level.is_none());
        assert!(cfg.paths.agents_dirs.is_empty());
        assert!(cfg.paths.state_dir.is_none());
        assert!(cfg.profiles.is_empty());
    }

    #[test]
    fn file_config_optional_fields_deserialize_without_default_annotations() {
        let cfg: FileConfig = toml::from_str(
            r#"
[server]

[paths]
agents_dirs = ["agents"]
"#,
        )
        .expect("file config should parse");

        assert!(cfg.server._transport.is_none());
        assert!(cfg.server.log_level.is_none());
        assert_eq!(cfg.paths.agents_dirs, vec![PathBuf::from("agents")]);
        assert!(cfg.paths.state_dir.is_none());
        assert!(cfg.profiles.is_empty());
    }

    #[test]
    fn file_config_profiles_deserialize_without_extra_sections() {
        let cfg: FileConfig = toml::from_str(
            r#"
[profiles.codex]
agent = "backend-coder"
provider = "codex"
model = "gpt-5.3-codex"
stream = true
working_dir_policy = "direct"
render_style = "codex"
"#,
        )
        .expect("file config should parse profiles");

        assert_eq!(cfg.profiles.len(), 1);
        let profile = cfg.profiles.get("codex").expect("profile should exist");
        assert_eq!(profile.agent, "backend-coder");
        assert_eq!(profile.provider.as_deref(), Some("codex"));
        assert_eq!(profile.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(profile.stream, Some(true));
        assert_eq!(profile.working_dir_policy, Some(WorkingDirPolicy::Direct));
        assert_eq!(profile.render_style, Some(RenderStyle::Codex));
    }

    #[test]
    fn resolve_runtime_config_includes_profiles_from_file() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[paths]
agents_dirs = ["agents"]
state_dir = ".mcp-subagent/state"

[profiles.codex]
agent = "backend-coder"
provider = "codex"
stream = true
working_dir_policy = "direct"
render_style = "codex"
"#,
        )
        .expect("write config");

        let cfg = super::resolve_runtime_config(super::ConfigOverrides {
            config_path: Some(config_path),
            ..super::ConfigOverrides::default()
        })
        .expect("resolve config");

        let profile = cfg.profiles.get("codex").expect("profile should exist");
        assert_eq!(profile.agent, "backend-coder");
        assert_eq!(profile.provider.as_deref(), Some("codex"));
        assert_eq!(profile.stream, Some(true));
        assert_eq!(profile.working_dir_policy, Some(WorkingDirPolicy::Direct));
        assert_eq!(profile.render_style, Some(RenderStyle::Codex));
    }

    #[test]
    fn load_file_config_keeps_missing_optional_fields_as_none() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("config.toml");
        fs::write(
            &file,
            r#"
[server]

[paths]
agents_dirs = ["team-agents"]
"#,
        )
        .expect("write");

        let cfg = load_file_config(file)
            .expect("load config")
            .expect("config should exist");
        let layer = file_layer_from_cfg(&cfg);

        assert_eq!(layer.agents_dirs, Some(vec![PathBuf::from("team-agents")]));
        assert!(layer.state_dir.is_none());
        assert!(layer.log_level.is_none());
    }
}
