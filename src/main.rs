use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use mcp_subagent::{
    config::{resolve_runtime_config, ConfigOverrides, RuntimeConfig},
    connect::{build_connect_snippet, resolve_connect_snippet_paths, ConnectHost},
    doctor::{build_doctor_report, render_doctor_report},
    init::{init_workspace, InitPreset, InitReport},
    logging::{init_logging, LoggingGuard},
    mcp::{
        dto::{
            ArtifactOutput, HandleInput, ReadAgentArtifactInput, RunAgentInput,
            RunAgentSelectedFileInput,
        },
        server::McpSubagentServer,
    },
    probe::SystemProviderProber,
    runtime::context::validate_default_summary_contract_template,
    spec::registry::load_agent_specs_from_dirs,
};
use rmcp::handler::server::wrapper::Parameters;
use serde::Serialize;
use tracing::info;

const DEFAULT_BOOTSTRAP_ROOT_RELATIVE: &str = ".mcp-subagent/bootstrap";
const PROJECT_BRIDGE_CONFIG_RELATIVE: &str = ".mcp-subagent/config.toml";
const PROJECT_GITIGNORE_RELATIVE: &str = ".gitignore";
const BRIDGE_AGENTS_DIR_RELATIVE: &str = "./.mcp-subagent/bootstrap/agents";
const BRIDGE_STATE_DIR_RELATIVE: &str = "./.mcp-subagent/bootstrap/.mcp-subagent/state";
const GITIGNORE_RUNTIME_HEADER: &str = "# mcp-subagent runtime artifacts";
const GITIGNORE_RUNTIME_RULES: [&str; 3] = [
    ".mcp-subagent/state/",
    ".mcp-subagent/logs/",
    ".mcp-subagent/bootstrap/",
];

#[derive(Debug, Parser)]
#[command(name = "mcp-subagent", version, about = "MCP subagent runtime")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long = "agents-dir", global = true)]
    agents_dirs: Vec<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    log_level: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Mcp {
        #[arg(value_name = "AGENTS_DIR")]
        agents_dir: Option<PathBuf>,
    },
    Doctor {
        #[arg(value_name = "AGENTS_DIR")]
        agents_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Validate {
        #[arg(value_name = "AGENTS_DIR")]
        agents_dir: Option<PathBuf>,
    },
    Init {
        #[arg(long, value_enum, default_value_t = InitPresetArg::ClaudeOpusSupervisor)]
        preset: InitPresetArg,
        #[arg(long, value_name = "ROOT_DIR")]
        root_dir: Option<PathBuf>,
        #[arg(long, conflicts_with = "root_dir")]
        in_place: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    ConnectSnippet {
        #[arg(long, value_enum)]
        host: ConnectHostArg,
    },
    ListAgents {
        #[arg(long)]
        json: bool,
    },
    Run {
        agent: String,
        #[arg(long)]
        task: String,
        #[arg(long)]
        task_brief: Option<String>,
        #[arg(long)]
        parent_summary: Option<String>,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long = "plan")]
        plan_ref: Option<String>,
        #[arg(long = "selected-file")]
        selected_files: Vec<PathBuf>,
        #[arg(long = "selected-file-inline")]
        selected_files_inline: Vec<PathBuf>,
        #[arg(long)]
        working_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Spawn {
        agent: String,
        #[arg(long)]
        task: String,
        #[arg(long)]
        task_brief: Option<String>,
        #[arg(long)]
        parent_summary: Option<String>,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long = "plan")]
        plan_ref: Option<String>,
        #[arg(long = "selected-file")]
        selected_files: Vec<PathBuf>,
        #[arg(long = "selected-file-inline")]
        selected_files_inline: Vec<PathBuf>,
        #[arg(long)]
        working_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Status {
        handle_id: String,
        #[arg(long)]
        json: bool,
    },
    Cancel {
        handle_id: String,
        #[arg(long)]
        json: bool,
    },
    Artifact {
        handle_id: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<ArtifactKindArg>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ArtifactKindArg {
    Summary,
    Log,
    Patch,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InitPresetArg {
    ClaudeOpusSupervisor,
    CodexPrimaryBuilder,
    GeminiFrontendTeam,
    LocalOllamaFallback,
    MinimalSingleProvider,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ConnectHostArg {
    Claude,
    Codex,
    Gemini,
}

impl From<InitPresetArg> for InitPreset {
    fn from(value: InitPresetArg) -> Self {
        match value {
            InitPresetArg::ClaudeOpusSupervisor => InitPreset::ClaudeOpusSupervisor,
            InitPresetArg::CodexPrimaryBuilder => InitPreset::CodexPrimaryBuilder,
            InitPresetArg::GeminiFrontendTeam => InitPreset::GeminiFrontendTeam,
            InitPresetArg::LocalOllamaFallback => InitPreset::LocalOllamaFallback,
            InitPresetArg::MinimalSingleProvider => InitPreset::MinimalSingleProvider,
        }
    }
}

impl From<ConnectHostArg> for ConnectHost {
    fn from(value: ConnectHostArg) -> Self {
        match value {
            ConnectHostArg::Claude => ConnectHost::Claude,
            ConnectHostArg::Codex => ConnectHost::Codex,
            ConnectHostArg::Gemini => ConnectHost::Gemini,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let config_path = cli.config.clone();
    let global_agents_dirs = cli.agents_dirs.clone();
    let state_dir = cli.state_dir.clone();
    let cli_log_level = cli.log_level.clone();

    match cli.command {
        Commands::Mcp { agents_dir } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                agents_dir,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: mcp");
            run_mcp_server(cfg).await
        }
        Commands::Doctor { agents_dir, json } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                agents_dir,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: doctor");
            doctor(cfg, json)
        }
        Commands::Validate { agents_dir } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                agents_dir,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: validate");
            validate_specs(cfg)
        }
        Commands::Init {
            preset,
            root_dir,
            in_place,
            force,
            json,
        } => {
            info!("starting command: init");
            init_command(preset, root_dir, in_place, force, json)
        }
        Commands::ConnectSnippet { host } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: connect-snippet");
            connect_snippet_command(cfg, host)
        }
        Commands::ListAgents { json } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: list-agents");
            list_agents(cfg, json).await
        }
        Commands::Run {
            agent,
            task,
            task_brief,
            parent_summary,
            stage,
            plan_ref,
            selected_files,
            selected_files_inline,
            working_dir,
            json,
        } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: run");
            run_agent(
                cfg,
                agent,
                task,
                task_brief,
                parent_summary,
                stage,
                plan_ref,
                selected_files,
                selected_files_inline,
                working_dir,
                json,
            )
            .await
        }
        Commands::Spawn {
            agent,
            task,
            task_brief,
            parent_summary,
            stage,
            plan_ref,
            selected_files,
            selected_files_inline,
            working_dir,
            json,
        } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: spawn");
            spawn_agent(
                cfg,
                agent,
                task,
                task_brief,
                parent_summary,
                stage,
                plan_ref,
                selected_files,
                selected_files_inline,
                working_dir,
                json,
            )
            .await
        }
        Commands::Status { handle_id, json } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: status");
            get_status(cfg, handle_id, json).await
        }
        Commands::Cancel { handle_id, json } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: cancel");
            cancel_agent(cfg, handle_id, json).await
        }
        Commands::Artifact {
            handle_id,
            path,
            kind,
            json,
        } => {
            let (cfg, _guard) = match resolve_cli_config_with_logging(
                config_path,
                state_dir,
                global_agents_dirs,
                None,
                cli_log_level,
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            info!("starting command: artifact");
            read_artifact(cfg, handle_id, path, kind, json).await
        }
    }
}

fn resolve_cli_config_with_logging(
    config_path: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    global_agents_dirs: Vec<PathBuf>,
    command_agents_dir: Option<PathBuf>,
    cli_log_level: Option<String>,
) -> std::result::Result<(RuntimeConfig, LoggingGuard), String> {
    let cfg = resolve_cli_config(
        config_path,
        state_dir,
        global_agents_dirs,
        command_agents_dir,
        cli_log_level.clone(),
    )
    .map_err(|err| format!("failed to resolve config: {err}"))?;

    let guard = init_logging(
        &cfg.state_dir,
        cli_log_level.as_deref(),
        cfg.log_level.as_str(),
    )
    .map_err(|err| format!("failed to initialize logging: {err}"))?;

    Ok((cfg, guard))
}

fn resolve_cli_config(
    config_path: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    mut agents_dirs: Vec<PathBuf>,
    command_agents_dir: Option<PathBuf>,
    log_level: Option<String>,
) -> mcp_subagent::error::Result<RuntimeConfig> {
    if let Some(dir) = command_agents_dir {
        agents_dirs = vec![dir];
    }

    resolve_runtime_config(ConfigOverrides {
        config_path,
        agents_dirs,
        state_dir,
        log_level,
    })
}

async fn run_mcp_server(cfg: RuntimeConfig) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    match server.serve_stdio().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("failed to run mcp server: {err}");
            ExitCode::from(1)
        }
    }
}

fn validate_specs(cfg: RuntimeConfig) -> ExitCode {
    match load_agent_specs_from_dirs(&cfg.agents_dirs) {
        Ok(loaded) => {
            if let Err(err) = validate_default_summary_contract_template() {
                eprintln!("validation failed: {err}");
                return ExitCode::from(1);
            }
            println!("validated {} agent specs", loaded.len());
            println!("summary contract template: ok");
            for agent in loaded {
                println!(
                    "- {} ({}) [{}]",
                    agent.spec.core.name,
                    agent.spec.core.provider.as_str(),
                    agent.path.display()
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("validation failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn doctor(cfg: RuntimeConfig, json: bool) -> ExitCode {
    let report = build_doctor_report(cfg.agents_dirs, cfg.state_dir, &SystemProviderProber);
    if json {
        print_json(&report);
    } else {
        println!("{}", render_doctor_report(&report));
    }
    if report.status == "error" {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn init_command(
    preset: InitPresetArg,
    root_dir: Option<PathBuf>,
    in_place: bool,
    force: bool,
    json: bool,
) -> ExitCode {
    let use_default_bootstrap_root = root_dir.is_none() && !in_place;
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("init failed: unable to resolve current directory: {err}");
            return ExitCode::from(1);
        }
    };
    let root = match resolve_init_root(&cwd, root_dir, in_place) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("init failed: {err}");
            return ExitCode::from(1);
        }
    };
    match init_workspace(&root, preset.into(), force) {
        Ok(mut report) => {
            if use_default_bootstrap_root {
                match ensure_bootstrap_bridge_config(&cwd, force) {
                    Ok((path, true)) => report.notes.push(format!(
                        "Generated project bridge config at `{}`; you can run mcp-subagent commands from project root without extra --agents-dir/--state-dir flags.",
                        path.display()
                    )),
                    Ok((path, false)) => report.notes.push(format!(
                        "Using existing project config `{}` (preserved).",
                        path.display()
                    )),
                    Err(err) => {
                        eprintln!("init failed: {err}");
                        return ExitCode::from(1);
                    }
                }
                match ensure_project_gitignore(&cwd) {
                    Ok((path, true)) => report.notes.push(format!(
                        "Updated `{}` with mcp-subagent runtime ignore rules.",
                        path.display()
                    )),
                    Ok((path, false)) => report.notes.push(format!(
                        "Using existing `.gitignore` rules in `{}` (no changes).",
                        path.display()
                    )),
                    Err(err) => {
                        eprintln!("init failed: {err}");
                        return ExitCode::from(1);
                    }
                }
            }
            if json {
                print_json(&report);
            } else {
                print_init_report(&report);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("init failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn resolve_init_root(
    cwd: &Path,
    root_dir: Option<PathBuf>,
    in_place: bool,
) -> std::result::Result<PathBuf, String> {
    if let Some(root) = root_dir {
        return Ok(root);
    }
    if in_place {
        return Ok(cwd.to_path_buf());
    }
    Ok(cwd.join(DEFAULT_BOOTSTRAP_ROOT_RELATIVE))
}

fn ensure_bootstrap_bridge_config(
    cwd: &Path,
    force: bool,
) -> std::result::Result<(PathBuf, bool), String> {
    let config_path = cwd.join(PROJECT_BRIDGE_CONFIG_RELATIVE);
    if config_path.exists() && !force {
        if !config_path.is_file() {
            return Err(format!(
                "project bridge config path is not a file: {}",
                config_path.display()
            ));
        }
        return Ok((config_path, false));
    }
    if config_path.is_dir() {
        return Err(format!(
            "project bridge config path is a directory: {}",
            config_path.display()
        ));
    }
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create project config directory: {err}"))?;
    }
    fs::write(&config_path, bootstrap_bridge_config_template())
        .map_err(|err| format!("failed to write project bridge config: {err}"))?;
    Ok((config_path, true))
}

fn bootstrap_bridge_config_template() -> String {
    format!(
        r#"[server]
transport = "stdio"
log_level = "info"

[paths]
agents_dirs = ["{agents_dir}"]
state_dir = "{state_dir}"
"#,
        agents_dir = BRIDGE_AGENTS_DIR_RELATIVE,
        state_dir = BRIDGE_STATE_DIR_RELATIVE
    )
}

fn ensure_project_gitignore(cwd: &Path) -> std::result::Result<(PathBuf, bool), String> {
    let gitignore_path = cwd.join(PROJECT_GITIGNORE_RELATIVE);
    if gitignore_path.is_dir() {
        return Err(format!(
            "project .gitignore path is a directory: {}",
            gitignore_path.display()
        ));
    }

    let mut content = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)
            .map_err(|err| format!("failed to read .gitignore: {err}"))?
    } else {
        String::new()
    };

    let existing_rules = content
        .lines()
        .filter_map(normalize_gitignore_rule)
        .collect::<Vec<_>>();
    if existing_rules
        .iter()
        .any(|rule| is_mcp_subagent_catch_all(rule))
    {
        return Ok((gitignore_path, false));
    }

    let missing_rules = GITIGNORE_RUNTIME_RULES
        .iter()
        .filter(|target| {
            !existing_rules
                .iter()
                .any(|rule| gitignore_rule_matches_target(rule, target))
        })
        .copied()
        .collect::<Vec<_>>();

    if missing_rules.is_empty() {
        return Ok((gitignore_path, false));
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(GITIGNORE_RUNTIME_HEADER);
    content.push('\n');
    for rule in missing_rules {
        content.push_str(rule);
        content.push('\n');
    }

    fs::write(&gitignore_path, content)
        .map_err(|err| format!("failed to write .gitignore: {err}"))?;
    Ok((gitignore_path, true))
}

fn normalize_gitignore_rule(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let without_dot = trimmed.strip_prefix("./").unwrap_or(trimmed);
    let normalized = without_dot.strip_prefix('/').unwrap_or(without_dot);
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.to_string())
}

fn is_mcp_subagent_catch_all(rule: &str) -> bool {
    matches!(rule.trim_end_matches('/'), ".mcp-subagent") || matches!(rule, ".mcp-subagent/**")
}

fn gitignore_rule_matches_target(rule: &str, target: &str) -> bool {
    if is_mcp_subagent_catch_all(rule) {
        return true;
    }
    let normalized_target = target
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/');
    rule.trim_end_matches('/') == normalized_target
}

fn connect_snippet_command(cfg: RuntimeConfig, host: ConnectHostArg) -> ExitCode {
    let Some(first_agents_dir) = cfg.agents_dirs.first().cloned() else {
        eprintln!("connect-snippet failed: no agents directory configured");
        return ExitCode::from(1);
    };

    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("connect-snippet failed: unable to resolve current directory: {err}");
            return ExitCode::from(1);
        }
    };
    let binary = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("connect-snippet failed: unable to resolve current executable: {err}");
            return ExitCode::from(1);
        }
    };

    let paths = resolve_connect_snippet_paths(&cwd, binary, first_agents_dir, cfg.state_dir);
    let snippet = build_connect_snippet(host.into(), &paths);
    println!("{snippet}");
    ExitCode::SUCCESS
}

async fn list_agents(cfg: RuntimeConfig, json: bool) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    match server.list_agents().await {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                for agent in result.0.agents {
                    println!(
                        "{} [{}] available={} context_mode={} working_dir_policy={} sandbox={} timeout={}s",
                        agent.name,
                        agent.provider,
                        agent.available,
                        agent.runtime_policy.context_mode,
                        agent.runtime_policy.working_dir_policy,
                        agent.runtime_policy.sandbox,
                        agent.runtime_policy.timeout_secs
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("list-agents failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_agent(
    cfg: RuntimeConfig,
    agent: String,
    task: String,
    task_brief: Option<String>,
    parent_summary: Option<String>,
    stage: Option<String>,
    plan_ref: Option<String>,
    selected_files: Vec<PathBuf>,
    selected_files_inline: Vec<PathBuf>,
    working_dir: Option<PathBuf>,
    json: bool,
) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    let selected_files = match build_selected_file_inputs(
        selected_files,
        selected_files_inline,
        working_dir.as_deref(),
    ) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("run failed: {err}");
            return ExitCode::from(1);
        }
    };
    let input = RunAgentInput {
        agent_name: agent,
        task,
        task_brief,
        parent_summary,
        selected_files,
        stage,
        plan_ref,
        working_dir: working_dir.map(|path| path.display().to_string()),
    };

    match server.run_agent(Parameters(input)).await {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                let out = result.0;
                println!("handle_id: {}", out.handle_id);
                println!("status: {}", out.status);
                println!("summary: {}", out.structured_summary.summary);
                if !out.structured_summary.key_findings.is_empty() {
                    println!("key_findings:");
                    for finding in out.structured_summary.key_findings {
                        println!("- {finding}");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("run failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent(
    cfg: RuntimeConfig,
    agent: String,
    task: String,
    task_brief: Option<String>,
    parent_summary: Option<String>,
    stage: Option<String>,
    plan_ref: Option<String>,
    selected_files: Vec<PathBuf>,
    selected_files_inline: Vec<PathBuf>,
    working_dir: Option<PathBuf>,
    json: bool,
) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    let selected_files = match build_selected_file_inputs(
        selected_files,
        selected_files_inline,
        working_dir.as_deref(),
    ) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("spawn failed: {err}");
            return ExitCode::from(1);
        }
    };
    let input = RunAgentInput {
        agent_name: agent,
        task,
        task_brief,
        parent_summary,
        selected_files,
        stage,
        plan_ref,
        working_dir: working_dir.map(|path| path.display().to_string()),
    };

    match server.spawn_agent(Parameters(input)).await {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                println!("handle_id: {}", result.0.handle_id);
                println!("status: {}", result.0.status);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("spawn failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

async fn get_status(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    match server
        .get_agent_status(Parameters(HandleInput { handle_id }))
        .await
    {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                println!("handle_id: {}", result.0.handle_id);
                println!("status: {}", result.0.status);
                println!("updated_at: {}", result.0.updated_at);
                if let Some(err) = result.0.error_message {
                    println!("error: {err}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("status failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

async fn cancel_agent(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    match server
        .cancel_agent(Parameters(HandleInput { handle_id }))
        .await
    {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                println!("handle_id: {}", result.0.handle_id);
                println!("status: {}", result.0.status);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cancel failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

async fn read_artifact(
    cfg: RuntimeConfig,
    handle_id: String,
    explicit_path: Option<String>,
    kind: Option<ArtifactKindArg>,
    json: bool,
) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);

    let path = match explicit_path {
        Some(path) => path,
        None => {
            let status = match server
                .get_agent_status(Parameters(HandleInput {
                    handle_id: handle_id.clone(),
                }))
                .await
            {
                Ok(status) => status.0,
                Err(err) => {
                    eprintln!("artifact failed to resolve status: {}", err.message);
                    return ExitCode::from(1);
                }
            };
            let target_kind = kind.unwrap_or(ArtifactKindArg::Summary);
            match resolve_artifact_path(target_kind, &status.artifact_index) {
                Some(path) => path,
                None => {
                    eprintln!("artifact path not found for selected kind");
                    return ExitCode::from(1);
                }
            }
        }
    };

    match server
        .read_agent_artifact(Parameters(ReadAgentArtifactInput { handle_id, path }))
        .await
    {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                println!("{}", result.0.content);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("artifact failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

fn build_selected_file_inputs(
    selected_files: Vec<PathBuf>,
    selected_files_inline: Vec<PathBuf>,
    working_dir: Option<&Path>,
) -> std::result::Result<Vec<RunAgentSelectedFileInput>, String> {
    let mut merged = selected_files
        .into_iter()
        .map(|path| RunAgentSelectedFileInput {
            path: path.display().to_string(),
            rationale: None,
            content: None,
        })
        .collect::<Vec<_>>();

    for inline_path in selected_files_inline {
        let display = inline_path.display().to_string();
        let resolved = resolve_inline_read_path(&inline_path, working_dir);
        let content = fs::read_to_string(&resolved).map_err(|err| {
            format!(
                "failed to read --selected-file-inline `{display}` from `{}`: {err}",
                resolved.display()
            )
        })?;

        if let Some(existing) = merged.iter_mut().find(|item| item.path == display) {
            existing.content = Some(content);
            continue;
        }

        merged.push(RunAgentSelectedFileInput {
            path: display,
            rationale: Some("inline content provided by CLI --selected-file-inline".to_string()),
            content: Some(content),
        });
    }

    Ok(merged)
}

fn resolve_inline_read_path(path: &Path, working_dir: Option<&Path>) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }

    if let Some(base) = working_dir {
        let candidate = base.join(path);
        if candidate.exists() {
            return candidate;
        }
        return candidate;
    }

    path.to_path_buf()
}

fn resolve_artifact_path(kind: ArtifactKindArg, index: &[ArtifactOutput]) -> Option<String> {
    let by_path = |path: &str| index.iter().find(|item| item.path == path);
    let by_kind = |name: &str| index.iter().find(|item| item.kind == name);

    match kind {
        ArtifactKindArg::Summary => by_path("summary.json")
            .or_else(|| by_kind("SummaryJson"))
            .map(|item| item.path.clone()),
        ArtifactKindArg::Log => by_path("stdout.txt")
            .or_else(|| by_path("stderr.txt"))
            .or_else(|| by_kind("StdoutText"))
            .or_else(|| by_kind("StderrText"))
            .map(|item| item.path.clone()),
        ArtifactKindArg::Patch => index
            .iter()
            .find(|item| item.kind == "PatchDiff" || item.path.ends_with(".patch"))
            .map(|item| item.path.clone()),
        ArtifactKindArg::Json => index
            .iter()
            .find(|item| {
                item.path.ends_with(".json")
                    || item
                        .media_type
                        .as_deref()
                        .is_some_and(|media| media == "application/json")
            })
            .map(|item| item.path.clone()),
    }
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(err) => eprintln!("failed to render json output: {err}"),
    }
}

fn print_init_report(report: &InitReport) {
    println!("preset: {}", report.preset);
    println!("preset_catalog_version: {}", report.preset_catalog_version);
    println!("root: {}", report.root.display());
    println!("agents_dir: {}", report.agents_dir.display());
    println!("generated_agents: {}", report.generated_agent_count);
    println!("created_files:");
    for path in &report.created_files {
        println!("- {}", path.display());
    }
    if !report.overwritten_files.is_empty() {
        println!("overwritten_files:");
        for path in &report.overwritten_files {
            println!("- {}", path.display());
        }
    }
    if !report.notes.is_empty() {
        println!("next_steps:");
        for note in &report.notes {
            println!("- {note}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use clap::Parser;
    use tempfile::tempdir;

    use crate::{
        bootstrap_bridge_config_template, build_selected_file_inputs,
        ensure_bootstrap_bridge_config, ensure_project_gitignore, resolve_init_root,
        ArtifactKindArg, Cli, Commands, ConnectHostArg, InitPresetArg, RunAgentSelectedFileInput,
        DEFAULT_BOOTSTRAP_ROOT_RELATIVE,
    };

    #[test]
    fn parses_list_agents_json_flag() {
        let cli = Cli::parse_from(["mcp-subagent", "list-agents", "--json"]);
        match cli.command {
            Commands::ListAgents { json } => assert!(json),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_doctor_json_flag() {
        let cli = Cli::parse_from(["mcp-subagent", "doctor", "--json"]);
        match cli.command {
            Commands::Doctor { json, .. } => assert!(json),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_run_command_with_required_args() {
        let cli = Cli::parse_from(["mcp-subagent", "run", "reviewer", "--task", "review code"]);
        match cli.command {
            Commands::Run { agent, task, .. } => {
                assert_eq!(agent, "reviewer");
                assert_eq!(task, "review code");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_run_command_with_selected_file_inline() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "run",
            "reviewer",
            "--task",
            "review code",
            "--selected-file",
            "src/lib.rs",
            "--selected-file-inline",
            "src/main.rs",
        ]);
        match cli.command {
            Commands::Run {
                selected_files,
                selected_files_inline,
                ..
            } => {
                assert_eq!(selected_files.len(), 1);
                assert_eq!(selected_files_inline.len(), 1);
                assert_eq!(selected_files_inline[0].to_string_lossy(), "src/main.rs");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_artifact_kind_enum() {
        let cli = Cli::parse_from(["mcp-subagent", "artifact", "handle-1", "--kind", "summary"]);
        match cli.command {
            Commands::Artifact { kind, .. } => {
                assert!(matches!(kind, Some(ArtifactKindArg::Summary)));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_init_command() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "init",
            "--preset",
            "claude-opus-supervisor",
            "--in-place",
            "--force",
            "--json",
        ]);
        match cli.command {
            Commands::Init {
                preset,
                in_place,
                force,
                json,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisor));
                assert!(in_place);
                assert!(force);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_init_command_with_new_preset() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "init",
            "--preset",
            "minimal-single-provider",
        ]);
        match cli.command {
            Commands::Init {
                preset, in_place, ..
            } => {
                assert!(matches!(preset, InitPresetArg::MinimalSingleProvider));
                assert!(!in_place);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn init_rejects_in_place_with_root_dir() {
        let result =
            Cli::try_parse_from(["mcp-subagent", "init", "--in-place", "--root-dir", "tmp"]);
        assert!(
            result.is_err(),
            "init should reject --in-place with --root-dir"
        );
    }

    #[test]
    fn init_defaults_to_bootstrap_root_when_not_in_place() {
        let cwd = Path::new("/tmp/workspace");
        let root = resolve_init_root(cwd, None, false).expect("resolve");
        assert_eq!(
            root,
            PathBuf::from(format!("/tmp/workspace/{DEFAULT_BOOTSTRAP_ROOT_RELATIVE}"))
        );
    }

    #[test]
    fn init_in_place_uses_current_directory() {
        let cwd = Path::new("/tmp/workspace");
        let root = resolve_init_root(cwd, None, true).expect("resolve");
        assert_eq!(root, PathBuf::from("/tmp/workspace"));
    }

    #[test]
    fn writes_bootstrap_bridge_config_when_missing() {
        let dir = tempdir().expect("tempdir");
        let (path, written) = ensure_bootstrap_bridge_config(dir.path(), false).expect("write");
        assert!(written);
        let content = fs::read_to_string(path).expect("read");
        assert_eq!(content, bootstrap_bridge_config_template());
    }

    #[test]
    fn preserves_existing_bridge_config_without_force() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".mcp-subagent/config.toml");
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(&path, "custom = true\n").expect("write custom");

        let (resolved, written) =
            ensure_bootstrap_bridge_config(dir.path(), false).expect("ensure bridge");
        assert_eq!(resolved, path);
        assert!(!written);
        let content = fs::read_to_string(&resolved).expect("read");
        assert_eq!(content, "custom = true\n");
    }

    #[test]
    fn overwrites_existing_bridge_config_with_force() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".mcp-subagent/config.toml");
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(&path, "custom = true\n").expect("write custom");

        let (resolved, written) =
            ensure_bootstrap_bridge_config(dir.path(), true).expect("ensure bridge");
        assert_eq!(resolved, path);
        assert!(written);
        let content = fs::read_to_string(&resolved).expect("read");
        assert_eq!(content, bootstrap_bridge_config_template());
    }

    #[test]
    fn creates_gitignore_when_missing() {
        let dir = tempdir().expect("tempdir");
        let (path, updated) = ensure_project_gitignore(dir.path()).expect("ensure gitignore");
        assert!(updated);
        assert_eq!(path, dir.path().join(".gitignore"));
        let content = fs::read_to_string(path).expect("read");
        assert!(content.contains("# mcp-subagent runtime artifacts"));
        assert!(content.contains(".mcp-subagent/state/"));
        assert!(content.contains(".mcp-subagent/logs/"));
        assert!(content.contains(".mcp-subagent/bootstrap/"));
    }

    #[test]
    fn appends_only_missing_gitignore_rules() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".gitignore");
        fs::write(
            &path,
            "\
target/
.mcp-subagent/state/
",
        )
        .expect("write gitignore");

        let (resolved, updated) = ensure_project_gitignore(dir.path()).expect("ensure gitignore");
        assert_eq!(resolved, path);
        assert!(updated);
        let content = fs::read_to_string(resolved).expect("read");
        assert!(content.contains("target/"));
        assert!(content.contains(".mcp-subagent/state/"));
        assert!(content.contains(".mcp-subagent/logs/"));
        assert!(content.contains(".mcp-subagent/bootstrap/"));
    }

    #[test]
    fn preserves_gitignore_when_catch_all_rule_exists() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".gitignore");
        fs::write(&path, ".mcp-subagent/\n").expect("write gitignore");

        let (resolved, updated) = ensure_project_gitignore(dir.path()).expect("ensure gitignore");
        assert_eq!(resolved, path);
        assert!(!updated);
        let content = fs::read_to_string(resolved).expect("read");
        assert_eq!(content, ".mcp-subagent/\n");
    }

    #[test]
    fn parses_connect_snippet_host() {
        let cli = Cli::parse_from(["mcp-subagent", "connect-snippet", "--host", "claude"]);
        match cli.command {
            Commands::ConnectSnippet { host } => {
                assert!(matches!(host, ConnectHostArg::Claude));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn inline_selected_files_include_file_content() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("inline.txt");
        fs::write(&file, "inline body").expect("write inline file");

        let out = build_selected_file_inputs(Vec::new(), vec![file.clone()], None)
            .expect("build selected files");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, file.display().to_string());
        assert_eq!(out[0].content.as_deref(), Some("inline body"));
    }

    #[test]
    fn inline_selected_file_overrides_non_inline_entry() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("inline-override.txt");
        fs::write(&file, "override body").expect("write inline file");

        let out = build_selected_file_inputs(vec![file.clone()], vec![file.clone()], None)
            .expect("build selected files");
        let only = out
            .iter()
            .find(|item: &&RunAgentSelectedFileInput| item.path == file.display().to_string())
            .expect("entry exists");
        assert_eq!(only.content.as_deref(), Some("override body"));
    }
}
