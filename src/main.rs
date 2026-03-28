use std::{
    collections::HashMap,
    fs,
    io::{IsTerminal, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum};
use mcp_subagent::{
    config::{resolve_runtime_config, ConfigOverrides, RuntimeConfig},
    connect::{
        build_connect_invocation, build_connect_snippet, build_host_launch_invocation,
        resolve_connect_snippet_paths, ConnectHost, ConnectInvocation,
    },
    cwd::resolve_cli_cwd,
    doctor::{build_doctor_report, render_doctor_report},
    init::{
        init_workspace, refresh_bootstrap_workspace, sync_project_bridge_workspace, InitPreset,
        InitReport,
    },
    logging::{init_logging, LoggingGuard},
    mcp::{
        dto::{
            ArtifactOutput, GetAgentStatsInput, GetAgentStatsOutput, HandleInput, OutcomeView,
            ReadAgentArtifactInput, RunAgentInput, RunAgentSelectedFileInput, RunView,
        },
        server::McpSubagentServer,
    },
    probe::SystemProviderProber,
    runtime::context::validate_default_summary_contract_template,
    spec::registry::load_agent_specs_from_dirs,
};
use rmcp::handler::server::wrapper::Parameters;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::info;

const DEFAULT_BOOTSTRAP_ROOT_RELATIVE: &str = ".mcp-subagent/bootstrap";
const PROJECT_BRIDGE_CONFIG_RELATIVE: &str = ".mcp-subagent/config.toml";
const PROJECT_GITIGNORE_RELATIVE: &str = ".gitignore";
const BRIDGE_AGENTS_DIR_RELATIVE: &str = "./.mcp-subagent/bootstrap/agents";
const BRIDGE_STATE_DIR_RELATIVE: &str = "./.mcp-subagent/bootstrap/.mcp-subagent/state";
const GITIGNORE_RUNTIME_HEADER: &str = "# mcp-subagent runtime artifacts";
const RESULT_CONTRACT_VERSION: &str = "mcp-subagent.result.v1";
const STREAM_FOLLOW_INTERVAL_MS: u64 = 250;
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
        #[arg(long, value_enum, default_value_t = InitPresetArg::ClaudeOpusSupervisorMinimal)]
        preset: InitPresetArg,
        #[arg(long, value_name = "ROOT_DIR")]
        root_dir: Option<PathBuf>,
        #[arg(long, conflicts_with = "root_dir")]
        in_place: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        refresh_bootstrap: bool,
        #[arg(long)]
        sync_project_config: bool,
        #[arg(long, requires = "root_dir")]
        sync_project_config_only: bool,
        #[arg(long)]
        json: bool,
    },
    ConnectSnippet {
        #[arg(long, value_enum)]
        host: ConnectHostArg,
    },
    Connect {
        #[arg(long, value_enum)]
        host: ConnectHostArg,
        #[arg(long)]
        run_host: bool,
    },
    Clean {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    ListAgents {
        #[arg(long)]
        json: bool,
    },
    Ps {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    Show {
        handle_id: String,
        #[arg(long)]
        json: bool,
    },
    Result {
        handle_id: String,
        #[arg(long, conflicts_with_all = ["normalized", "summary"])]
        raw: bool,
        #[arg(long, conflicts_with_all = ["raw", "summary"])]
        normalized: bool,
        #[arg(long, conflicts_with_all = ["raw", "normalized"])]
        summary: bool,
        #[arg(long)]
        json: bool,
    },
    Logs {
        handle_id: String,
        #[arg(long, conflicts_with = "stderr")]
        stdout: bool,
        #[arg(long, conflicts_with = "stdout")]
        stderr: bool,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long)]
        follow: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[arg(long)]
        phase_timeout_secs: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Timeline {
        handle_id: String,
        #[arg(long)]
        event: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Events {
        #[arg(conflicts_with = "all")]
        handle_id: Option<String>,
        #[arg(long, conflicts_with = "handle_id")]
        all: bool,
        #[arg(long)]
        event: Option<String>,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long)]
        follow: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[arg(long)]
        phase_timeout_secs: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Watch {
        handle_id: String,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[arg(long)]
        phase_timeout_secs: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Wait {
        handle_id: String,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        timeout_secs: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Stats {
        handle_id: String,
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
        stream: bool,
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
        stream: bool,
        #[arg(long)]
        json: bool,
    },
    Submit {
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
        stream: bool,
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
    ClaudeOpusSupervisorMinimal,
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
            InitPresetArg::ClaudeOpusSupervisorMinimal => InitPreset::ClaudeOpusSupervisorMinimal,
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

impl ConnectHostArg {
    fn as_str(self) -> &'static str {
        match self {
            ConnectHostArg::Claude => "claude",
            ConnectHostArg::Codex => "codex",
            ConnectHostArg::Gemini => "gemini",
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
            refresh_bootstrap,
            sync_project_config,
            sync_project_config_only,
            json,
        } => {
            info!("starting command: init");
            init_command(InitCommandOptions {
                preset,
                root_dir,
                in_place,
                force,
                refresh_bootstrap,
                sync_project_config,
                sync_project_config_only,
                json,
            })
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
        Commands::Connect { host, run_host } => {
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
            info!("starting command: connect");
            connect_command(cfg, host, run_host)
        }
        Commands::Clean { all, dry_run, json } => {
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
            info!("starting command: clean");
            clean_command(cfg, all, dry_run, json)
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
        Commands::Ps { limit, json } => {
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
            info!("starting command: ps");
            list_runs(cfg, limit, json)
        }
        Commands::Show { handle_id, json } => {
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
            info!("starting command: show");
            show_run(cfg, handle_id, json)
        }
        Commands::Result {
            handle_id,
            raw,
            normalized,
            summary,
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
            info!("starting command: result");
            read_result(cfg, handle_id, raw, normalized, summary, json)
        }
        Commands::Logs {
            handle_id,
            stdout,
            stderr,
            phase,
            follow,
            interval_ms,
            timeout_secs,
            phase_timeout_secs,
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
            info!("starting command: logs");
            read_logs(
                cfg,
                handle_id,
                ReadLogsOptions {
                    stdout_only: stdout,
                    stderr_only: stderr,
                    phase,
                    follow,
                    interval_ms,
                    timeout_secs,
                    phase_timeout_secs,
                    json,
                },
            )
            .await
        }
        Commands::Timeline {
            handle_id,
            event,
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
            info!("starting command: timeline");
            read_timeline(cfg, handle_id, event, None, json)
        }
        Commands::Events {
            handle_id,
            all,
            event,
            phase,
            follow,
            interval_ms,
            timeout_secs,
            phase_timeout_secs,
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
            info!("starting command: events");
            read_events(
                cfg,
                ReadEventsOptions {
                    handle_id,
                    all,
                    event,
                    phase,
                    follow,
                    interval_ms,
                    timeout_secs,
                    phase_timeout_secs,
                    json,
                },
            )
            .await
        }
        Commands::Watch {
            handle_id,
            phase,
            interval_ms,
            timeout_secs,
            phase_timeout_secs,
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
            info!("starting command: watch");
            watch_run(
                cfg,
                handle_id,
                phase,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            )
            .await
        }
        Commands::Wait {
            handle_id,
            interval_ms,
            timeout_secs,
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
            info!("starting command: wait");
            wait_run(cfg, handle_id, interval_ms, timeout_secs, json).await
        }
        Commands::Stats { handle_id, json } => {
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
            info!("starting command: stats");
            read_stats(cfg, handle_id, json)
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
            stream,
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
                AgentRunCommand {
                    agent,
                    task,
                    task_brief,
                    parent_summary,
                    stage,
                    plan_ref,
                    selected_files,
                    selected_files_inline,
                    working_dir,
                    stream,
                    json,
                },
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
            stream,
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
                AgentRunCommand {
                    agent,
                    task,
                    task_brief,
                    parent_summary,
                    stage,
                    plan_ref,
                    selected_files,
                    selected_files_inline,
                    working_dir,
                    stream,
                    json,
                },
            )
            .await
        }
        Commands::Submit {
            agent,
            task,
            task_brief,
            parent_summary,
            stage,
            plan_ref,
            selected_files,
            selected_files_inline,
            working_dir,
            stream,
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
            info!("starting command: submit");
            submit_agent(
                cfg,
                AgentRunCommand {
                    agent,
                    task,
                    task_brief,
                    parent_summary,
                    stage,
                    plan_ref,
                    selected_files,
                    selected_files_inline,
                    working_dir,
                    stream,
                    json,
                },
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

#[derive(Debug, Clone, Serialize)]
struct CleanEntry {
    path: PathBuf,
    bytes: u64,
    action: String,
}

#[derive(Debug, Clone, Serialize)]
struct CleanReport {
    state_dir: PathBuf,
    mode: String,
    dry_run: bool,
    reclaimed_bytes: u64,
    cleaned: Vec<CleanEntry>,
    missing: Vec<PathBuf>,
    errors: Vec<String>,
}

fn clean_command(cfg: RuntimeConfig, all: bool, dry_run: bool, json: bool) -> ExitCode {
    let report = clean_state_dir(&cfg.state_dir, all, dry_run);
    if json {
        print_json(&report);
    } else {
        print_clean_report(&report);
    }
    if report.errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn clean_state_dir(state_dir: &Path, all: bool, dry_run: bool) -> CleanReport {
    let mut report = CleanReport {
        state_dir: state_dir.to_path_buf(),
        mode: if all {
            "all".to_string()
        } else {
            "runtime".to_string()
        },
        dry_run,
        reclaimed_bytes: 0,
        cleaned: Vec::new(),
        missing: Vec::new(),
        errors: Vec::new(),
    };
    let targets = if all {
        vec![state_dir.to_path_buf()]
    } else {
        vec![
            state_dir.join("runs"),
            state_dir.join("server.log"),
            state_dir.join("logs"),
        ]
    };

    for path in targets {
        if !path.exists() {
            report.missing.push(path);
            continue;
        }

        let bytes = match estimate_path_size(&path) {
            Ok(value) => value,
            Err(err) => {
                report.errors.push(format!(
                    "failed to calculate size for `{}`: {err}",
                    path.display()
                ));
                0
            }
        };

        if !dry_run {
            let removal_result = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(err) = removal_result {
                report
                    .errors
                    .push(format!("failed to remove `{}`: {err}", path.display()));
                continue;
            }
        }

        report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(bytes);
        report.cleaned.push(CleanEntry {
            path,
            bytes,
            action: if dry_run {
                "would_remove".to_string()
            } else {
                "removed".to_string()
            },
        });
    }

    report
}

fn estimate_path_size(path: &Path) -> std::result::Result<u64, std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(metadata.len());
    }
    if metadata.is_dir() {
        let mut total = 0u64;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            total = total.saturating_add(estimate_path_size(&entry.path())?);
        }
        return Ok(total);
    }
    Ok(0)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredRunSpecSnapshot {
    name: String,
    provider: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredExecutionPolicy {
    requested_run_mode: Option<String>,
    effective_run_mode: Option<String>,
    effective_run_mode_source: Option<String>,
    spawn_policy: Option<String>,
    spawn_policy_source: Option<String>,
    background_preference: Option<String>,
    background_preference_source: Option<String>,
    max_turns: Option<u32>,
    max_turns_source: Option<String>,
    retry_max_attempts: Option<u32>,
    retry_backoff_secs: Option<u64>,
    retry_policy_source: Option<String>,
    attempts_used: Option<u32>,
    retries_used: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredNativeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredRetryInfo {
    classification: String,
    reason: Option<String>,
    attempts_used: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredOutcomeUsage {
    duration_ms: u64,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    provider_exit_code: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredSuccessOutcome {
    summary: String,
    key_findings: Vec<String>,
    touched_files: Vec<String>,
    next_steps: Vec<String>,
    open_questions: Vec<String>,
    verification: String,
    usage: StoredOutcomeUsage,
    parse_status: String,
    plan_refs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredFailureOutcome {
    error: String,
    retry: StoredRetryInfo,
    partial_summary: Option<String>,
    usage: StoredOutcomeUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum StoredRunOutcome {
    Succeeded(StoredSuccessOutcome),
    Failed(StoredFailureOutcome),
    Cancelled { reason: String },
    TimedOut { elapsed_secs: u64 },
}

impl StoredRunOutcome {
    fn success_outcome(&self) -> Option<&StoredSuccessOutcome> {
        match self {
            Self::Succeeded(success) => Some(success),
            Self::Failed(_) | Self::Cancelled { .. } | Self::TimedOut { .. } => None,
        }
    }

    fn parse_status(&self) -> Option<&str> {
        self.success_outcome()
            .map(|success| success.parse_status.as_str())
    }

    fn usage(&self) -> Option<&StoredOutcomeUsage> {
        match self {
            Self::Succeeded(success) => Some(&success.usage),
            Self::Failed(failure) => Some(&failure.usage),
            Self::Cancelled { .. } | Self::TimedOut { .. } => None,
        }
    }

    fn summary_text(&self) -> Option<&str> {
        match self {
            Self::Succeeded(success) => Some(success.summary.as_str()),
            Self::Failed(failure) => failure.partial_summary.as_deref(),
            Self::Cancelled { .. } | Self::TimedOut { .. } => None,
        }
    }

    fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed(failure) => Some(failure.error.as_str()),
            Self::Cancelled { reason } => Some(reason.as_str()),
            Self::Succeeded(_) | Self::TimedOut { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredTaskSpec {
    task: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredRunState {
    status: String,
    created_at: String,
    updated_at: String,
    status_history: Vec<String>,
    error_message: Option<String>,
    policy: Option<StoredExecutionPolicy>,
    compiled_context_markdown: Option<String>,
    usage: Option<StoredNativeUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoredRunRecord {
    task_spec: StoredTaskSpec,
    state: StoredRunState,
    outcome: Option<StoredRunOutcome>,
    artifact_index: Vec<ArtifactOutput>,
    spec_snapshot: Option<StoredRunSpecSnapshot>,
}

impl StoredRunRecord {
    fn status(&self) -> &str {
        self.state.status.as_str()
    }

    fn created_at(&self) -> &str {
        self.state.created_at.as_str()
    }

    fn updated_at(&self) -> &str {
        self.state.updated_at.as_str()
    }

    fn error_message(&self) -> Option<&str> {
        self.state.error_message.as_deref().or_else(|| {
            self.outcome
                .as_ref()
                .and_then(StoredRunOutcome::error_message)
        })
    }

    fn task(&self) -> &str {
        self.task_spec.task.as_str()
    }
}

#[derive(Debug, Clone, Serialize)]
struct UsageStatsOutput {
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_ms: Option<u64>,
    provider: String,
    model: Option<String>,
    provider_exit_code: Option<i32>,
    retries: u32,
    token_source: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    estimated_prompt_bytes: Option<u64>,
    estimated_output_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct RunListEntry {
    handle_id: String,
    status: String,
    updated_at: String,
    state: Option<String>,
    phase: Option<String>,
    last_event_at: Option<String>,
    last_event_age_ms: Option<u64>,
    stalled: bool,
    elapsed_ms: Option<u64>,
    block_reason: Option<String>,
    provider: Option<String>,
    agent: Option<String>,
    task: String,
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct RunShowOutput {
    handle_id: String,
    status: String,
    updated_at: String,
    error_message: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    normalization_status: Option<String>,
    summary: Option<String>,
    provider_exit_code: Option<i32>,
    retries: u32,
    retry_classification: String,
    classification_reason: Option<String>,
    usage: UsageStatsOutput,
    artifact_index: Vec<ArtifactOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct RunResultOutput {
    contract_version: String,
    handle_id: String,
    status: String,
    view: String,
    normalization_status: String,
    summary: Option<String>,
    native_result: Option<String>,
    normalized_result: Option<StoredSuccessOutcome>,
    provider_exit_code: Option<i32>,
    retries: u32,
    retry_classification: String,
    classification_reason: Option<String>,
    usage: UsageStatsOutput,
    error_message: Option<String>,
    artifact_index: Vec<ArtifactOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct RunLogsOutput {
    handle_id: String,
    stdout: Option<String>,
    stderr: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RunTimelineEvent {
    event: String,
    timestamp: String,
    detail: serde_json::Value,
    seq: Option<u64>,
    level: Option<String>,
    state: Option<String>,
    phase: Option<String>,
    source: Option<String>,
    message: Option<String>,
}

impl RunTimelineEvent {
    fn display_timestamp(&self) -> &str {
        self.timestamp.as_str()
    }
}

#[derive(Debug, Clone, Serialize)]
struct RunTimelineOutput {
    handle_id: String,
    events: Vec<RunTimelineEvent>,
}

#[derive(Debug, Clone, Serialize)]
struct RunTimelineAllOutput {
    runs: Vec<RunTimelineOutput>,
}

#[derive(Debug, Clone)]
struct RunEventsSnapshot {
    handle_id: String,
    events: Vec<RunTimelineEvent>,
}

#[derive(Debug, Clone, Default)]
struct EventStreamCursor {
    path: Option<PathBuf>,
    offset: u64,
    pending: String,
}

#[derive(Debug, Clone, Default)]
struct FollowEventState {
    cursor: EventStreamCursor,
    events: Vec<RunTimelineEvent>,
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WaitRunOutput {
    handle_id: String,
    status: String,
    updated_at: String,
    error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RunStatusOutput {
    handle_id: String,
    agent_name: String,
    task_brief: Option<String>,
    status: String,
    state: Option<String>,
    phase: String,
    terminal: bool,
    created_at: String,
    updated_at: String,
    stalled: bool,
    block_reason: Option<String>,
    last_event_at: Option<String>,
    last_event_age_ms: Option<u64>,
    current_wait_reason: Option<String>,
    wait_reasons: Vec<String>,
    advice: Vec<String>,
    outcome: Option<OutcomeView>,
}

#[derive(Debug, Clone, Serialize)]
struct RunStatsOutput {
    handle_id: String,
    status: String,
    state: Option<String>,
    phase: Option<String>,
    last_event_at: Option<String>,
    last_event_age_ms: Option<u64>,
    stalled: bool,
    block_reason: Option<String>,
    queue_ms: Option<u64>,
    provider_probe_ms: Option<u64>,
    workspace_prepare_ms: Option<u64>,
    provider_boot_ms: Option<u64>,
    execution_ms: Option<u64>,
    first_output_ms: Option<u64>,
    first_output_warned: bool,
    first_output_warning_at: Option<String>,
    current_wait_reason: Option<String>,
    wait_reasons: Vec<String>,
    wall_ms: Option<u64>,
    usage: UsageStatsOutput,
}

fn should_use_color_output() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM")
        .ok()
        .is_some_and(|term| term.eq_ignore_ascii_case("dumb"))
    {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn ansi(text: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
        text.to_string()
    }
}

fn status_badge(status: &str, color: bool) -> String {
    let label = status.to_ascii_uppercase();
    let code = match status {
        "succeeded" => "1;32",
        "failed" => "1;31",
        "running" => "1;33",
        "timed_out" => "1;35",
        "cancelled" => "1;36",
        _ => "1;34",
    };
    ansi(&label, code, color)
}

fn render_show_run_text(view: &RunShowOutput, color: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{}  {}",
        status_badge(&view.status, color),
        view.handle_id
    ));
    lines.push(format!(
        "provider={} model={} normalization={}",
        view.provider.as_deref().unwrap_or("unknown"),
        view.model.as_deref().unwrap_or("unknown"),
        view.normalization_status
            .as_deref()
            .unwrap_or("not_available")
    ));
    lines.push(format!(
        "updated={} duration_ms={} exit_code={} retries={} retry_classification={}",
        view.updated_at,
        view.usage
            .duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        view.provider_exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        view.retries,
        view.retry_classification
    ));
    if let Some(reason) = view.classification_reason.as_deref() {
        lines.push(format!("retry_reason: {reason}"));
    }
    if let Some(summary) = view.summary.as_deref() {
        lines.push(format!("summary: {summary}"));
    }
    if let Some(error) = view.error_message.as_deref() {
        lines.push(format!("error: {}", ansi(error, "31", color)));
    }
    lines.join("\n")
}

fn build_run_status_output(view: RunView, stats: GetAgentStatsOutput) -> RunStatusOutput {
    RunStatusOutput {
        handle_id: view.handle_id,
        agent_name: view.agent_name,
        task_brief: view.task_brief,
        status: stats.status,
        state: stats.state,
        phase: stats.phase.unwrap_or(view.phase),
        terminal: view.terminal,
        created_at: view.created_at,
        updated_at: view.updated_at,
        stalled: stats.stalled,
        block_reason: stats.block_reason,
        last_event_at: stats.last_event_at,
        last_event_age_ms: stats.last_event_age_ms,
        current_wait_reason: stats.current_wait_reason,
        wait_reasons: stats.wait_reasons,
        advice: stats.advice,
        outcome: view.outcome,
    }
}

fn render_status_output_text(output: &RunStatusOutput) -> String {
    let mut lines = Vec::new();
    lines.push(format!("handle_id: {}", output.handle_id));
    lines.push(format!("agent: {}", output.agent_name));
    if let Some(task_brief) = output.task_brief.as_deref() {
        lines.push(format!("task_brief: {task_brief}"));
    }
    lines.push(format!("status: {}", output.status));
    lines.push(format!(
        "state: {}",
        output.state.as_deref().unwrap_or(output.status.as_str())
    ));
    lines.push(format!("phase: {}", output.phase));
    lines.push(format!(
        "terminal: {}",
        if output.terminal { "yes" } else { "no" }
    ));
    lines.push(format!("created_at: {}", output.created_at));
    lines.push(format!("updated_at: {}", output.updated_at));
    lines.push(format!(
        "last_event_at: {}",
        output.last_event_at.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "last_event_age_ms: {}",
        output
            .last_event_age_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));
    lines.push(format!(
        "stalled: {}",
        if output.stalled { "yes" } else { "no" }
    ));
    lines.push(format!(
        "block_reason: {}",
        output.block_reason.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "current_wait_reason: {}",
        output.current_wait_reason.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "wait_reasons: {}",
        if output.wait_reasons.is_empty() {
            "-".to_string()
        } else {
            output.wait_reasons.join(",")
        }
    ));
    if !output.advice.is_empty() {
        lines.push(format!("advice: {}", output.advice.join(" | ")));
    }
    if let Some(outcome) = output.outcome.as_ref() {
        match outcome {
            OutcomeView::Succeeded { summary, .. } => lines.push(format!("summary: {summary}")),
            OutcomeView::Failed { error, .. } => lines.push(format!("error: {error}")),
            OutcomeView::Cancelled { reason } => lines.push(format!("cancelled: {reason}")),
            OutcomeView::TimedOut { elapsed_secs } => {
                lines.push(format!("timed_out_after: {elapsed_secs}s"))
            }
        }
    }
    lines.join("\n")
}

fn runs_root(state_dir: &Path) -> PathBuf {
    state_dir.join("runs")
}

fn run_json_path(state_dir: &Path, handle_id: &str) -> PathBuf {
    runs_root(state_dir).join(handle_id).join("run.json")
}

fn run_events_path(state_dir: &Path, handle_id: &str) -> PathBuf {
    runs_root(state_dir).join(handle_id).join("events.jsonl")
}

fn resolve_events_file_path(state_dir: &Path, handle_id: &str) -> Option<PathBuf> {
    let canonical_path = run_events_path(state_dir, handle_id);
    if canonical_path.exists() {
        return Some(canonical_path);
    }
    None
}

fn run_artifacts_root(state_dir: &Path, handle_id: &str) -> PathBuf {
    runs_root(state_dir).join(handle_id).join("artifacts")
}

fn load_run_record(
    state_dir: &Path,
    handle_id: &str,
) -> std::result::Result<StoredRunRecord, String> {
    let path = run_json_path(state_dir, handle_id);
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str::<StoredRunRecord>(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn list_run_records(
    state_dir: &Path,
) -> std::result::Result<Vec<(String, StoredRunRecord)>, String> {
    let root = runs_root(state_dir);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&root)
        .map_err(|err| format!("failed to read run directory {}: {err}", root.display()))?;
    let mut runs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read run entry: {err}"))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read run entry type: {err}"))?;
        if !file_type.is_dir() {
            continue;
        }
        let handle_id = entry.file_name().to_string_lossy().to_string();
        let record = load_run_record(state_dir, &handle_id)
            .map_err(|err| format!("failed to load run `{handle_id}`: {err}"))?;
        runs.push((handle_id, record));
    }
    runs.sort_by_key(|(_, record)| parse_rfc3339(record.updated_at()));
    runs.reverse();
    Ok(runs)
}

fn parse_timeline_event_line(
    path: &Path,
    line_no: usize,
    line: &str,
) -> std::result::Result<RunTimelineEvent, String> {
    serde_json::from_str::<RunTimelineEvent>(line)
        .map_err(|err| format!("failed to parse {} line {}: {err}", path.display(), line_no))
}

fn load_run_events(
    state_dir: &Path,
    handle_id: &str,
) -> std::result::Result<Vec<RunTimelineEvent>, String> {
    let path = resolve_events_file_path(state_dir, handle_id)
        .ok_or_else(|| format!("events not found for handle `{handle_id}`"))?;
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut events = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = parse_timeline_event_line(&path, line_no + 1, line)?;
        events.push(event);
    }
    Ok(events)
}

fn load_run_events_incremental(
    state_dir: &Path,
    handle_id: &str,
    cursor: &mut EventStreamCursor,
) -> std::result::Result<Vec<RunTimelineEvent>, String> {
    let Some(path) = resolve_events_file_path(state_dir, handle_id) else {
        cursor.path = None;
        cursor.offset = 0;
        cursor.pending.clear();
        return Ok(Vec::new());
    };

    if cursor.path.as_ref() != Some(&path) {
        cursor.path = Some(path.clone());
        cursor.offset = 0;
        cursor.pending.clear();
    }

    let mut file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("failed to open {}: {err}", path.display())),
    };

    let metadata = file
        .metadata()
        .map_err(|err| format!("failed to stat {}: {err}", path.display()))?;
    if metadata.len() < cursor.offset {
        cursor.offset = 0;
        cursor.pending.clear();
    }

    file.seek(SeekFrom::Start(cursor.offset))
        .map_err(|err| format!("failed to seek {}: {err}", path.display()))?;
    let mut chunk = String::new();
    file.read_to_string(&mut chunk)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    cursor.offset = cursor.offset.saturating_add(chunk.len() as u64);

    if chunk.is_empty() && cursor.pending.is_empty() {
        return Ok(Vec::new());
    }

    let mut text = String::new();
    if !cursor.pending.is_empty() {
        text.push_str(&cursor.pending);
        cursor.pending.clear();
    }
    text.push_str(&chunk);

    if text.is_empty() {
        return Ok(Vec::new());
    }

    if !text.ends_with('\n') {
        if let Some(last_newline) = text.rfind('\n') {
            cursor.pending = text[last_newline + 1..].to_string();
            text.truncate(last_newline + 1);
        } else {
            cursor.pending = text;
            return Ok(Vec::new());
        }
    }

    let mut events = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = parse_timeline_event_line(&path, line_no + 1, line)?;
        events.push(event);
    }
    Ok(events)
}

fn collect_watch_events_incremental(
    state_dir: &Path,
    handle_id: &str,
    cursor: &mut EventStreamCursor,
    all_events: &mut Vec<RunTimelineEvent>,
) -> std::result::Result<Vec<RunTimelineEvent>, String> {
    let new_events = load_run_events_incremental(state_dir, handle_id, cursor)?;
    all_events.extend(new_events.iter().cloned());
    Ok(new_events)
}

fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

fn event_time(event: &RunTimelineEvent) -> Option<OffsetDateTime> {
    parse_rfc3339(&event.timestamp)
}

fn duration_between(start: Option<OffsetDateTime>, end: Option<OffsetDateTime>) -> Option<u64> {
    let start = start?;
    let end = end?;
    if end < start {
        return None;
    }
    Some((end - start).whole_milliseconds().max(0) as u64)
}

fn first_event_time(events: &[RunTimelineEvent], name: &str) -> Option<OffsetDateTime> {
    events
        .iter()
        .find(|event| event.event == name)
        .and_then(event_time)
}

fn first_event_timestamp(events: &[RunTimelineEvent], name: &str) -> Option<String> {
    events
        .iter()
        .find(|event| event.event == name)
        .map(|event| event.display_timestamp().to_string())
        .filter(|value| !value.is_empty())
}

fn latest_event(events: &[RunTimelineEvent]) -> Option<&RunTimelineEvent> {
    events.last()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn auth_is_ready_signal(lowered: &str) -> bool {
    contains_any(
        lowered,
        &[
            "loaded cached credentials",
            "credentials loaded",
            "already authenticated",
            "auth restored",
            "using filekeychain fallback for secure storage",
        ],
    )
}

fn auth_is_wait_signal(lowered: &str) -> bool {
    if auth_is_ready_signal(lowered) {
        return false;
    }
    contains_any(
        lowered,
        &[
            "auth required",
            "authentication required",
            "authentication failed",
            "unauthorized",
            "login required",
            "missing credentials",
            "credentials required",
            "api key missing",
            "invalid api key",
        ],
    )
}

fn classify_block_reason_from_text(text: &str) -> Option<&'static str> {
    let lowered = text.to_ascii_lowercase();
    if contains_any(
        &lowered,
        &[
            "trusted folder",
            "trust this folder",
            "waiting_for_trust",
            "waiting for trust",
            "trust required",
        ],
    ) {
        return Some("trust_required");
    }
    if auth_is_wait_signal(&lowered) {
        return Some("auth_required");
    }
    if contains_any(
        &lowered,
        &[
            "tool approval",
            "approval required",
            "permission denied",
            "consent required",
            "approval mode",
        ],
    ) {
        return Some("tool_approval_required");
    }
    if contains_any(
        &lowered,
        &[
            "skill conflict",
            "skills conflict",
            "skill discovery",
            "find-skills",
            ".agents/skills",
            ".gemini/skills",
        ],
    ) {
        return Some("skill_discovery");
    }
    if contains_any(
        &lowered,
        &[
            "workspace scan",
            "scanning workspace",
            "indexing workspace",
            "workspace settings",
        ],
    ) {
        return Some("workspace_scan");
    }
    if contains_any(
        &lowered,
        &[
            "provider `",
            "provider unavailable",
            "missingbinary",
            "binary `",
            "not found in path",
        ],
    ) {
        return Some("provider_unavailable");
    }
    if contains_any(
        &lowered,
        &[
            "structured summary parse status is invalid",
            "invalid summary json",
            "sentinel",
            "structured summary parsing failed",
        ],
    ) {
        return Some("normalization_failed");
    }
    if contains_any(
        &lowered,
        &[
            "tls handshake eof",
            "stream disconnected before completion",
            "connection refused",
            "network error",
        ],
    ) {
        return Some("network_error");
    }
    None
}

fn classify_block_reason_from_events(
    events: &[RunTimelineEvent],
    stalled: bool,
) -> Option<&'static str> {
    for event in events.iter().rev() {
        match event.event.as_str() {
            "provider.waiting_for_trust" => return Some("trust_required"),
            "provider.waiting_for_auth" => return Some("auth_required"),
            "provider.waiting_for_tool_approval" => return Some("tool_approval_required"),
            "provider.waiting_for_consent" => return Some("consent_required"),
            "provider.waiting_for_skill_discovery" => return Some("skill_discovery"),
            "provider.waiting_for_workspace_scan" => return Some("workspace_scan"),
            "provider.first_output.warning" if stalled => return Some("provider_output_wait"),
            "run.queued" if stalled => return Some("queueing"),
            "workspace.prepare.started" if stalled => return Some("workspace_prepare"),
            "provider.probe.started" if stalled => return Some("provider_probe"),
            "provider.boot.started" if stalled => return Some("provider_boot"),
            _ => {}
        }
        if let Some(message) = event.message.as_deref() {
            if let Some(reason) = classify_block_reason_from_text(message) {
                return Some(reason);
            }
        }
        if !event.detail.is_null() {
            let detail_text = event.detail.to_string();
            if let Some(reason) = classify_block_reason_from_text(&detail_text) {
                return Some(reason);
            }
        }
    }
    None
}

fn classify_block_reason(
    status: &str,
    phase: Option<&str>,
    stalled: bool,
    events: &[RunTimelineEvent],
    error_message: Option<&str>,
) -> Option<String> {
    if status == "succeeded" {
        return None;
    }
    if let Some(message) = error_message {
        if let Some(reason) = classify_block_reason_from_text(message) {
            return Some(reason.to_string());
        }
    }
    if let Some(reason) = classify_block_reason_from_events(events, stalled) {
        return Some(reason.to_string());
    }
    if !is_terminal_status(status) && stalled {
        let fallback = match phase.unwrap_or_default() {
            "queueing" => "queueing",
            "workspace_prepare" => "workspace_prepare",
            "provider_probe" => "provider_probe",
            "provider_boot" => "provider_boot",
            "running" => "provider_output_wait",
            _ => "unknown_startup_wait",
        };
        return Some(fallback.to_string());
    }
    None
}

fn wait_reason_from_event_name(name: &str) -> Option<&'static str> {
    match name {
        "provider.waiting_for_trust" => Some("trust_required"),
        "provider.waiting_for_auth" => Some("auth_required"),
        "provider.waiting_for_tool_approval" => Some("tool_approval_required"),
        "provider.waiting_for_consent" => Some("consent_required"),
        "provider.waiting_for_skill_discovery" => Some("skill_discovery"),
        "provider.waiting_for_workspace_scan" => Some("workspace_scan"),
        _ => None,
    }
}

fn collect_wait_reasons(events: &[RunTimelineEvent]) -> (Vec<String>, Option<String>) {
    let mut reasons = Vec::new();
    for event in events {
        let Some(reason) = wait_reason_from_event_name(&event.event) else {
            continue;
        };
        if reasons.iter().any(|existing| existing == reason) {
            continue;
        }
        reasons.push(reason.to_string());
    }
    let current = reasons.last().cloned();
    (reasons, current)
}

fn compute_duration_ms(started_at: Option<&str>, finished_at: &str) -> Option<u64> {
    let start = parse_rfc3339(started_at?)?;
    let finish = parse_rfc3339(finished_at)?;
    if finish < start {
        return None;
    }
    let millis = (finish - start).whole_milliseconds();
    if millis < 0 {
        None
    } else {
        Some(millis as u64)
    }
}

fn format_elapsed_short(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "unknown".to_string();
    };
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    if ms < 60_000 {
        return format!("{:.1}s", ms as f64 / 1_000.0);
    }
    format!("{:.1}m", ms as f64 / 60_000.0)
}

fn format_elapsed_short_raw(ms: u64) -> String {
    format_elapsed_short(Some(ms))
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "timed_out" | "cancelled")
}

fn phase_matches_filter(phase: Option<&str>, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(filter) => phase.is_some_and(|phase| phase == filter),
    }
}

fn is_synthetic_progress_event(event: &RunTimelineEvent) -> bool {
    event
        .detail
        .get("synthetic")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn build_phase_progress_line(
    events: &[RunTimelineEvent],
    terminal: bool,
    now: OffsetDateTime,
    phase_filter: Option<&str>,
) -> Option<String> {
    if events.is_empty() {
        return None;
    }

    let mut current_phase: Option<String> = None;
    let mut current_start: Option<OffsetDateTime> = None;
    let mut last_ts: Option<OffsetDateTime> = None;
    let mut first_ts: Option<OffsetDateTime> = None;
    let mut segments: Vec<(String, u64, bool)> = Vec::new();

    for event in events {
        if is_synthetic_progress_event(event) {
            continue;
        }
        let Some(ts) = event_time(event) else {
            continue;
        };
        if first_ts.is_none() {
            first_ts = Some(ts);
        }
        let phase = event
            .phase
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        match &current_phase {
            None => {
                current_phase = Some(phase);
                current_start = Some(ts);
            }
            Some(existing) if *existing == phase => {}
            Some(existing) => {
                let duration = duration_between(current_start, Some(ts)).unwrap_or(0);
                segments.push((existing.clone(), duration, false));
                current_phase = Some(phase);
                current_start = Some(ts);
            }
        }
        last_ts = Some(ts);
    }

    if let Some(phase) = current_phase {
        let end = if terminal {
            last_ts.or(Some(now))
        } else {
            Some(now)
        };
        let duration = duration_between(current_start, end).unwrap_or(0);
        segments.push((phase, duration, !terminal));
    }

    if segments.is_empty() {
        return None;
    }
    if let Some(filter) = phase_filter {
        let current = segments.last().map(|(phase, _, _)| phase.as_str());
        if !phase_matches_filter(current, Some(filter)) {
            return None;
        }
    }

    let span_parts = segments
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|(phase, ms, current)| {
            if *current {
                format!("{phase}*={}", format_elapsed_short_raw(*ms))
            } else {
                format!("{phase}={}", format_elapsed_short_raw(*ms))
            }
        })
        .collect::<Vec<_>>();

    let wall_end = if terminal {
        last_ts.or(Some(now))
    } else {
        Some(now)
    };
    let wall_ms = duration_between(first_ts, wall_end);
    Some(format!(
        "phase_progress: {} wall={}",
        span_parts.join(" -> "),
        format_elapsed_short(wall_ms)
    ))
}

fn sanitize_rel_path(path: &str) -> std::result::Result<PathBuf, String> {
    let rel = PathBuf::from(path);
    if rel.is_absolute() {
        return Err(format!("artifact path must be relative: {path}"));
    }
    if rel
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("artifact path traversal is not allowed: {path}"));
    }
    Ok(rel)
}

fn read_artifact_from_disk(
    state_dir: &Path,
    handle_id: &str,
    path: &str,
) -> std::result::Result<Option<String>, String> {
    let rel = sanitize_rel_path(path)?;
    let full = run_artifacts_root(state_dir, handle_id).join(rel);
    if !full.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&full)
        .map_err(|err| format!("failed to read {}: {err}", full.display()))?;
    Ok(Some(content))
}

fn estimate_tokens(bytes: Option<u64>) -> Option<u64> {
    bytes.map(|value| (value.saturating_add(3)) / 4)
}

fn infer_provider_exit_code(record: &StoredRunRecord) -> Option<i32> {
    if let Some(usage) = record.outcome.as_ref().and_then(StoredRunOutcome::usage) {
        if usage.provider_exit_code.is_some() {
            return usage.provider_exit_code;
        }
    }
    let message = record.error_message()?;
    let marker = "exited with code ";
    let idx = message.find(marker)?;
    let code_start = idx + marker.len();
    let tail = &message[code_start..];
    let digits = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .collect::<String>();
    digits.parse::<i32>().ok()
}

fn build_usage_output(
    state_dir: &Path,
    handle_id: &str,
    record: &StoredRunRecord,
) -> UsageStatsOutput {
    let started_at = Some(record.created_at().to_string());
    let finished_at = if is_terminal_status(record.status()) {
        Some(record.updated_at().to_string())
    } else {
        None
    };
    let estimated_prompt_bytes = record
        .state
        .compiled_context_markdown
        .as_ref()
        .map(|value| value.len() as u64);
    let stdout_bytes = read_artifact_from_disk(state_dir, handle_id, "stdout.txt")
        .ok()
        .flatten()
        .map(|text| text.len() as u64);
    let stderr_bytes = read_artifact_from_disk(state_dir, handle_id, "stderr.txt")
        .ok()
        .flatten()
        .map(|text| text.len() as u64);
    let estimated_output_bytes = match (stdout_bytes, stderr_bytes) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
    };
    let estimated_input_tokens = estimate_tokens(estimated_prompt_bytes);
    let estimated_output_tokens = estimate_tokens(estimated_output_bytes);
    let estimated_total_tokens = match (estimated_input_tokens, estimated_output_tokens) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        _ => None,
    };
    let state_usage = record.state.usage.as_ref();
    let outcome_usage = record.outcome.as_ref().and_then(StoredRunOutcome::usage);
    let mut used_native = false;
    let mut used_estimated = false;
    let input_tokens = if let Some(value) = outcome_usage
        .and_then(|usage| usage.input_tokens)
        .or_else(|| state_usage.and_then(|usage| usage.input_tokens))
    {
        used_native = true;
        Some(value)
    } else {
        if estimated_input_tokens.is_some() {
            used_estimated = true;
        }
        estimated_input_tokens
    };
    let output_tokens = if let Some(value) = outcome_usage
        .and_then(|usage| usage.output_tokens)
        .or_else(|| state_usage.and_then(|usage| usage.output_tokens))
    {
        used_native = true;
        Some(value)
    } else {
        if estimated_output_tokens.is_some() {
            used_estimated = true;
        }
        estimated_output_tokens
    };
    let total_tokens = if let Some(value) = outcome_usage
        .and_then(|usage| usage.total_tokens)
        .or_else(|| state_usage.and_then(|usage| usage.total_tokens))
    {
        used_native = true;
        Some(value)
    } else if let (Some(input), Some(output)) = (input_tokens, output_tokens) {
        Some(input.saturating_add(output))
    } else {
        if estimated_total_tokens.is_some() {
            used_estimated = true;
        }
        estimated_total_tokens
    };
    let token_source = match (used_native, used_estimated) {
        (true, true) => "mixed",
        (true, false) => "native",
        (false, true) => "estimated",
        (false, false) => "unknown",
    };

    UsageStatsOutput {
        started_at: started_at.clone(),
        finished_at,
        duration_ms: outcome_usage
            .and_then(|usage| (usage.duration_ms > 0).then_some(usage.duration_ms))
            .or_else(|| compute_duration_ms(started_at.as_deref(), record.updated_at())),
        provider: record
            .spec_snapshot
            .as_ref()
            .map(|spec| spec.provider.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        model: record
            .spec_snapshot
            .as_ref()
            .and_then(|spec| spec.model.clone()),
        provider_exit_code: infer_provider_exit_code(record),
        retries: record
            .state
            .policy
            .as_ref()
            .and_then(|policy| policy.retries_used)
            .or_else(|| {
                record.outcome.as_ref().and_then(|outcome| match outcome {
                    StoredRunOutcome::Failed(failure) => {
                        Some(failure.retry.attempts_used.saturating_sub(1))
                    }
                    StoredRunOutcome::Succeeded(_)
                    | StoredRunOutcome::Cancelled { .. }
                    | StoredRunOutcome::TimedOut { .. } => None,
                })
            })
            .unwrap_or(0),
        token_source: token_source.to_string(),
        input_tokens,
        output_tokens,
        total_tokens,
        estimated_prompt_bytes,
        estimated_output_bytes,
    }
}

fn resolve_retry_classification(record: &StoredRunRecord) -> (String, Option<String>) {
    if let Some(StoredRunOutcome::Failed(failure)) = record.outcome.as_ref() {
        let normalized = match failure.retry.classification.as_str() {
            "retryable" | "non_retryable" | "unknown" => failure.retry.classification.clone(),
            _ => "unknown".to_string(),
        };
        return (normalized, failure.retry.reason.clone());
    }
    ("unknown".to_string(), None)
}

fn list_runs(cfg: RuntimeConfig, limit: usize, json: bool) -> ExitCode {
    let entries = match list_run_records(&cfg.state_dir) {
        Ok(items) => items,
        Err(err) => {
            eprintln!("ps failed: {err}");
            return ExitCode::from(1);
        }
    };

    let now = OffsetDateTime::now_utc();
    let rows = entries
        .into_iter()
        .take(limit)
        .map(|(handle_id, record)| {
            let events = load_run_events(&cfg.state_dir, &handle_id).unwrap_or_default();
            let latest = latest_event(&events);
            let last_event_at = latest
                .map(|event| event.display_timestamp().to_string())
                .filter(|value| !value.is_empty());
            let last_event_age_ms = latest.and_then(event_time).and_then(|ts| {
                if now < ts {
                    None
                } else {
                    Some((now - ts).whole_milliseconds().max(0) as u64)
                }
            });
            let duration_ms = compute_duration_ms(Some(record.created_at()), record.updated_at());
            let elapsed_ms = if is_terminal_status(record.status()) {
                duration_ms
            } else {
                let started = parse_rfc3339(record.created_at());
                duration_between(started, Some(now))
            };
            let stalled = !is_terminal_status(record.status())
                && last_event_age_ms.is_some_and(|value| value >= 8_000);
            let phase = latest.and_then(|event| event.phase.clone());
            let block_reason = classify_block_reason(
                record.status(),
                phase.as_deref(),
                stalled,
                &events,
                record.error_message(),
            );

            RunListEntry {
                handle_id,
                status: record.status().to_string(),
                updated_at: record.updated_at().to_string(),
                state: latest.and_then(|event| event.state.clone()),
                phase,
                last_event_at,
                last_event_age_ms,
                stalled,
                elapsed_ms,
                block_reason,
                provider: record
                    .spec_snapshot
                    .as_ref()
                    .map(|spec| spec.provider.clone()),
                agent: record.spec_snapshot.as_ref().map(|spec| spec.name.clone()),
                task: record.task().to_string(),
                duration_ms,
            }
        })
        .collect::<Vec<_>>();

    if json {
        print_json(&rows);
    } else {
        if rows.is_empty() {
            println!("no runs found");
            return ExitCode::SUCCESS;
        }
        for row in rows {
            println!(
                "{} [{}] phase={} elapsed={} last_event={} stalled={} block_reason={} provider={} agent={}",
                row.handle_id,
                row.status,
                row.phase.as_deref().unwrap_or("unknown"),
                format_elapsed_short(row.elapsed_ms),
                format_elapsed_short(row.last_event_age_ms),
                if row.stalled { "yes" } else { "no" },
                row.block_reason.as_deref().unwrap_or("-"),
                row.provider.as_deref().unwrap_or("unknown"),
                row.agent.as_deref().unwrap_or("unknown"),
            );
            println!("task: {}", row.task);
        }
    }

    ExitCode::SUCCESS
}

fn show_run(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    let record = match load_run_record(&cfg.state_dir, &handle_id) {
        Ok(record) => record,
        Err(err) => {
            eprintln!("show failed: {err}");
            return ExitCode::from(1);
        }
    };
    let usage = build_usage_output(&cfg.state_dir, &handle_id, &record);
    let (retry_classification, classification_reason) = resolve_retry_classification(&record);
    let normalization_status = record
        .outcome
        .as_ref()
        .and_then(StoredRunOutcome::parse_status)
        .map(str::to_string);
    let summary_text = record
        .outcome
        .as_ref()
        .and_then(StoredRunOutcome::summary_text)
        .map(str::to_string);
    let view = RunShowOutput {
        handle_id: handle_id.clone(),
        status: record.status().to_string(),
        updated_at: record.updated_at().to_string(),
        error_message: record.error_message().map(str::to_string),
        provider: record
            .spec_snapshot
            .as_ref()
            .map(|spec| spec.provider.clone()),
        model: record
            .spec_snapshot
            .as_ref()
            .and_then(|spec| spec.model.clone()),
        normalization_status,
        summary: summary_text,
        provider_exit_code: infer_provider_exit_code(&record),
        retries: usage.retries,
        retry_classification,
        classification_reason,
        usage,
        artifact_index: record.artifact_index.clone(),
    };

    if json {
        print_json(&view);
    } else {
        println!("{}", render_show_run_text(&view, should_use_color_output()));
    }
    ExitCode::SUCCESS
}

fn read_result(
    cfg: RuntimeConfig,
    handle_id: String,
    raw: bool,
    normalized: bool,
    summary: bool,
    json: bool,
) -> ExitCode {
    let record = match load_run_record(&cfg.state_dir, &handle_id) {
        Ok(record) => record,
        Err(err) => {
            eprintln!("result failed: {err}");
            return ExitCode::from(1);
        }
    };

    let native_result = read_artifact_from_disk(&cfg.state_dir, &handle_id, "stdout.txt")
        .ok()
        .flatten()
        .or_else(|| {
            record
                .outcome
                .as_ref()
                .and_then(StoredRunOutcome::summary_text)
                .map(str::to_string)
        });
    let normalized_result = record
        .outcome
        .as_ref()
        .and_then(StoredRunOutcome::success_outcome)
        .cloned();
    let normalization_status = record
        .outcome
        .as_ref()
        .and_then(StoredRunOutcome::parse_status)
        .unwrap_or("NotAvailable")
        .to_string();
    let summary_text = record
        .outcome
        .as_ref()
        .and_then(StoredRunOutcome::summary_text)
        .map(str::to_string);
    let usage = build_usage_output(&cfg.state_dir, &handle_id, &record);
    let (retry_classification, classification_reason) = resolve_retry_classification(&record);
    let view = if raw {
        "raw"
    } else if normalized {
        "normalized"
    } else if summary {
        "summary"
    } else {
        "auto"
    };
    let output = RunResultOutput {
        contract_version: RESULT_CONTRACT_VERSION.to_string(),
        handle_id: handle_id.clone(),
        status: record.status().to_string(),
        view: view.to_string(),
        normalization_status,
        summary: summary_text.clone(),
        native_result: native_result.clone(),
        normalized_result: normalized_result.clone(),
        provider_exit_code: usage.provider_exit_code,
        retries: usage.retries,
        retry_classification,
        classification_reason,
        usage,
        error_message: record.error_message().map(str::to_string),
        artifact_index: record.artifact_index.clone(),
    };

    if json {
        print_json(&output);
        return ExitCode::SUCCESS;
    }

    if raw {
        println!("{}", native_result.unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    if normalized {
        match normalized_result {
            Some(value) => print_json(&value),
            None => println!(),
        }
        return ExitCode::SUCCESS;
    }

    if let Some(summary) = summary_text.as_deref() {
        println!("{summary}");
    } else if let Some(raw_text) = native_result {
        println!("{raw_text}");
    }
    ExitCode::SUCCESS
}

fn print_json_line(value: &serde_json::Value) -> std::result::Result<(), String> {
    let line = serde_json::to_string(value)
        .map_err(|err| format!("failed to serialize follow line: {err}"))?;
    println!("{line}");
    Ok(())
}

fn print_stream_delta_text(stream: &str, delta: &str) {
    for line in delta.lines() {
        if line.is_empty() {
            continue;
        }
        println!("[{stream}] {line}");
    }
}

fn print_stream_delta_json(
    handle_id: &str,
    stream: &str,
    delta: &str,
) -> std::result::Result<(), String> {
    if delta.is_empty() {
        return Ok(());
    }
    print_json_line(&serde_json::json!({
        "kind": "stream",
        "handle_id": handle_id,
        "stream": stream,
        "text": delta,
    }))
}

fn print_event_follow_line(
    handle_id: &str,
    event: &RunTimelineEvent,
    json: bool,
) -> std::result::Result<(), String> {
    if json {
        return print_json_line(&serde_json::json!({
            "kind": "event",
            "handle_id": handle_id,
            "seq": event.seq,
            "event": event.event,
            "timestamp": event.display_timestamp(),
            "state": event.state,
            "phase": event.phase,
            "source": event.source,
            "message": event.message,
            "detail": event.detail,
        }));
    }
    let detail = serde_json::to_string(&event.detail).unwrap_or_else(|_| "null".to_string());
    let message = event.message.as_deref().unwrap_or("");
    if message.is_empty() {
        println!("{} [{}] {}", event.display_timestamp(), event.event, detail);
    } else {
        println!(
            "{} [{}] {} {}",
            event.display_timestamp(),
            event.event,
            message,
            detail
        );
    }
    Ok(())
}

struct ReadLogsOptions {
    stdout_only: bool,
    stderr_only: bool,
    phase: Option<String>,
    follow: bool,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    phase_timeout_secs: Option<u64>,
    json: bool,
}

async fn read_logs(cfg: RuntimeConfig, handle_id: String, options: ReadLogsOptions) -> ExitCode {
    let ReadLogsOptions {
        stdout_only,
        stderr_only,
        phase,
        follow,
        interval_ms,
        timeout_secs,
        phase_timeout_secs,
        json,
    } = options;
    let stdout_enabled = !stderr_only;
    let stderr_enabled = !stdout_only;
    if follow {
        let started = Instant::now();
        let sleep_ms = interval_ms.max(50);
        let mut seen_event_count = 0usize;
        let mut seen_stdout_bytes = 0usize;
        let mut seen_stderr_bytes = 0usize;
        let mut last_phase_progress = String::new();
        let mut observed_phase: Option<String> = None;
        let mut observed_phase_started_at = Instant::now();
        loop {
            let record = match load_run_record(&cfg.state_dir, &handle_id) {
                Ok(record) => record,
                Err(err) => {
                    eprintln!("logs failed: {err}");
                    return ExitCode::from(1);
                }
            };

            let events = if run_events_path(&cfg.state_dir, &handle_id).exists() {
                match load_run_events(&cfg.state_dir, &handle_id) {
                    Ok(events) => events,
                    Err(err) => {
                        eprintln!("logs failed: {err}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                Vec::new()
            };
            if seen_event_count > events.len() {
                seen_event_count = events.len();
            }
            for event in events.iter().skip(seen_event_count) {
                if phase.as_deref().is_some_and(|needle| {
                    event.phase.as_deref().is_none_or(|value| value != needle)
                }) {
                    continue;
                }
                if let Err(err) = print_event_follow_line(&handle_id, event, json) {
                    eprintln!("logs failed: {err}");
                    return ExitCode::from(1);
                }
            }
            seen_event_count = events.len();
            let current_phase = latest_event(&events).and_then(|evt| evt.phase.clone());
            if current_phase != observed_phase {
                observed_phase = current_phase;
                observed_phase_started_at = Instant::now();
            }
            if !json {
                if let Some(line) = build_phase_progress_line(
                    &events,
                    is_terminal_status(record.status()),
                    OffsetDateTime::now_utc(),
                    phase.as_deref(),
                ) {
                    if line != last_phase_progress {
                        println!("{line}");
                        last_phase_progress = line;
                    }
                }
            }

            if stdout_enabled {
                if let Some(stdout) =
                    read_artifact_from_disk(&cfg.state_dir, &handle_id, "stdout.txt")
                        .ok()
                        .flatten()
                {
                    if seen_stdout_bytes > stdout.len() {
                        seen_stdout_bytes = 0;
                    }
                    let delta = &stdout[seen_stdout_bytes..];
                    if !delta.is_empty() {
                        if json {
                            if let Err(err) = print_stream_delta_json(&handle_id, "stdout", delta) {
                                eprintln!("logs failed: {err}");
                                return ExitCode::from(1);
                            }
                        } else {
                            print_stream_delta_text("stdout", delta);
                        }
                        seen_stdout_bytes = stdout.len();
                    }
                }
            }

            if stderr_enabled {
                if let Some(stderr) =
                    read_artifact_from_disk(&cfg.state_dir, &handle_id, "stderr.txt")
                        .ok()
                        .flatten()
                {
                    if seen_stderr_bytes > stderr.len() {
                        seen_stderr_bytes = 0;
                    }
                    let delta = &stderr[seen_stderr_bytes..];
                    if !delta.is_empty() {
                        if json {
                            if let Err(err) = print_stream_delta_json(&handle_id, "stderr", delta) {
                                eprintln!("logs failed: {err}");
                                return ExitCode::from(1);
                            }
                        } else {
                            print_stream_delta_text("stderr", delta);
                        }
                        seen_stderr_bytes = stderr.len();
                    }
                }
            }

            if is_terminal_status(record.status()) {
                return ExitCode::SUCCESS;
            }
            if phase_timeout_secs
                .is_some_and(|secs| observed_phase_started_at.elapsed().as_secs() >= secs)
            {
                eprintln!(
                    "logs follow phase timeout after {}s in phase `{}` for handle `{}`",
                    phase_timeout_secs.unwrap_or_default(),
                    observed_phase.as_deref().unwrap_or("unknown"),
                    handle_id
                );
                return ExitCode::from(124);
            }
            if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
                eprintln!(
                    "logs follow timed out after {}s for handle `{}`",
                    timeout_secs.unwrap_or_default(),
                    handle_id
                );
                return ExitCode::from(1);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
        }
    }

    let stdout = if stderr_only {
        None
    } else {
        read_artifact_from_disk(&cfg.state_dir, &handle_id, "stdout.txt")
            .ok()
            .flatten()
    };
    let stderr = if stdout_only {
        None
    } else {
        read_artifact_from_disk(&cfg.state_dir, &handle_id, "stderr.txt")
            .ok()
            .flatten()
    };

    if json {
        print_json(&RunLogsOutput {
            handle_id,
            stdout,
            stderr,
        });
        return ExitCode::SUCCESS;
    }

    if let Some(out) = stdout {
        println!("{out}");
    }
    if let Some(err) = stderr {
        if !stdout_only && !err.is_empty() {
            println!("{err}");
        }
    }
    ExitCode::SUCCESS
}

fn filter_timeline_events(
    mut events: Vec<RunTimelineEvent>,
    event: Option<&str>,
    phase: Option<&str>,
) -> Vec<RunTimelineEvent> {
    if let Some(name) = event {
        events.retain(|item| item.event == name);
    }
    if let Some(name) = phase {
        events.retain(|item| item.phase.as_deref().is_some_and(|phase| phase == name));
    }
    events
}

fn collect_run_event_snapshots(
    state_dir: &Path,
    event: Option<&str>,
    phase: Option<&str>,
) -> std::result::Result<Vec<RunEventsSnapshot>, String> {
    let entries = list_run_records(state_dir)?;
    let mut snapshots = Vec::new();
    for (handle_id, _record) in entries {
        let events = if run_events_path(state_dir, &handle_id).exists() {
            load_run_events(state_dir, &handle_id)?
        } else {
            Vec::new()
        };
        snapshots.push(RunEventsSnapshot {
            handle_id,
            events: filter_timeline_events(events, event, phase),
        });
    }
    Ok(snapshots)
}

struct ReadEventsAllOptions {
    event: Option<String>,
    phase: Option<String>,
    follow: bool,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    phase_timeout_secs: Option<u64>,
    json: bool,
}

async fn read_events_all(cfg: RuntimeConfig, options: ReadEventsAllOptions) -> ExitCode {
    let ReadEventsAllOptions {
        event,
        phase,
        follow,
        interval_ms,
        timeout_secs,
        phase_timeout_secs,
        json,
    } = options;
    if !follow {
        let snapshots =
            match collect_run_event_snapshots(&cfg.state_dir, event.as_deref(), phase.as_deref()) {
                Ok(items) => items,
                Err(err) => {
                    eprintln!("events failed: {err}");
                    return ExitCode::from(1);
                }
            };
        let runs = snapshots
            .into_iter()
            .filter(|snapshot| !snapshot.events.is_empty())
            .map(|snapshot| RunTimelineOutput {
                handle_id: snapshot.handle_id,
                events: snapshot.events,
            })
            .collect::<Vec<_>>();

        if json {
            print_json(&RunTimelineAllOutput { runs });
            return ExitCode::SUCCESS;
        }
        if runs.is_empty() {
            println!("no events found");
            return ExitCode::SUCCESS;
        }
        for run in runs {
            for item in run.events {
                let detail =
                    serde_json::to_string(&item.detail).unwrap_or_else(|_| "null".to_string());
                let message = item.message.as_deref().unwrap_or("");
                if message.is_empty() {
                    println!(
                        "[{}] {} [{}] {}",
                        run.handle_id,
                        item.display_timestamp(),
                        item.event,
                        detail
                    );
                } else {
                    println!(
                        "[{}] {} [{}] {} {}",
                        run.handle_id,
                        item.display_timestamp(),
                        item.event,
                        message,
                        detail
                    );
                }
            }
        }
        return ExitCode::SUCCESS;
    }

    let started = Instant::now();
    let sleep_ms = interval_ms.max(50);
    let mut follow_states: HashMap<String, FollowEventState> = HashMap::new();
    let mut phase_track: HashMap<String, (Option<String>, Instant)> = HashMap::new();
    let mut last_phase_progress: HashMap<String, String> = HashMap::new();
    loop {
        let entries = match list_run_records(&cfg.state_dir) {
            Ok(items) => items,
            Err(err) => {
                eprintln!("events failed: {err}");
                return ExitCode::from(1);
            }
        };
        let mut active_handles: HashMap<String, String> = HashMap::new();
        let now = OffsetDateTime::now_utc();

        for (handle_id, record) in entries {
            active_handles.insert(handle_id.clone(), record.status().to_string());
            let state = follow_states.entry(handle_id.clone()).or_default();
            state.status = Some(record.status().to_string());

            let new_events =
                match load_run_events_incremental(&cfg.state_dir, &handle_id, &mut state.cursor) {
                    Ok(items) => items,
                    Err(err) => {
                        eprintln!("events failed: {err}");
                        return ExitCode::from(1);
                    }
                };
            let mut had_new_events = false;
            for evt in new_events {
                let matches_event = event
                    .as_deref()
                    .is_none_or(|needle| evt.event.as_str() == needle);
                let matches_phase = phase
                    .as_deref()
                    .is_none_or(|needle| evt.phase.as_deref().is_some_and(|value| value == needle));
                if matches_event && matches_phase {
                    had_new_events = true;
                    if json {
                        if let Err(err) = print_json_line(&serde_json::json!({
                            "kind": "event",
                            "handle_id": handle_id,
                            "seq": evt.seq,
                            "event": evt.event,
                            "timestamp": evt.display_timestamp(),
                            "state": evt.state,
                            "phase": evt.phase,
                            "source": evt.source,
                            "message": evt.message,
                            "detail": evt.detail,
                        })) {
                            eprintln!("events failed: {err}");
                            return ExitCode::from(1);
                        }
                    } else {
                        let detail = serde_json::to_string(&evt.detail)
                            .unwrap_or_else(|_| "null".to_string());
                        let message = evt.message.as_deref().unwrap_or("");
                        if message.is_empty() {
                            println!(
                                "[{}] {} [{}] {}",
                                handle_id,
                                evt.display_timestamp(),
                                evt.event,
                                detail
                            );
                        } else {
                            println!(
                                "[{}] {} [{}] {} {}",
                                handle_id,
                                evt.display_timestamp(),
                                evt.event,
                                message,
                                detail
                            );
                        }
                    }
                }
                state.events.push(evt);
            }

            let filtered_events =
                filter_timeline_events(state.events.clone(), event.as_deref(), phase.as_deref());
            let current_phase = latest_event(&filtered_events).and_then(|evt| evt.phase.clone());
            let entry = phase_track
                .entry(handle_id.clone())
                .or_insert((current_phase.clone(), Instant::now()));
            if entry.0 != current_phase {
                *entry = (current_phase, Instant::now());
            }

            if !json {
                if let Some(line) = build_phase_progress_line(
                    &filtered_events,
                    is_terminal_status(record.status()),
                    now,
                    phase.as_deref(),
                ) {
                    let last = last_phase_progress.entry(handle_id.clone()).or_default();
                    if *last != line && (had_new_events || last.is_empty()) {
                        println!("[{}] {line}", handle_id);
                        *last = line;
                    }
                }
            }
        }

        follow_states.retain(|handle_id, _| active_handles.contains_key(handle_id));
        phase_track.retain(|handle_id, _| active_handles.contains_key(handle_id));
        last_phase_progress.retain(|handle_id, _| active_handles.contains_key(handle_id));

        if phase_timeout_secs.is_some() {
            let timeout = phase_timeout_secs.unwrap_or_default();
            for (handle_id, status) in &active_handles {
                if is_terminal_status(status.as_str()) {
                    continue;
                }
                if let Some((phase_name, phase_started)) = phase_track.get(handle_id) {
                    if phase_started.elapsed().as_secs() >= timeout {
                        eprintln!(
                            "events --all follow phase timeout after {}s in phase `{}` for handle `{}`",
                            timeout,
                            phase_name.as_deref().unwrap_or("unknown"),
                            handle_id
                        );
                        return ExitCode::from(124);
                    }
                }
            }
        }
        if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
            eprintln!(
                "events --all follow timed out after {}s",
                timeout_secs.unwrap_or_default()
            );
            return ExitCode::from(1);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
    }
}

fn read_timeline(
    cfg: RuntimeConfig,
    handle_id: String,
    event: Option<String>,
    phase: Option<String>,
    json: bool,
) -> ExitCode {
    let events = match load_run_events(&cfg.state_dir, &handle_id) {
        Ok(items) => items,
        Err(err) => {
            eprintln!("timeline failed: {err}");
            return ExitCode::from(1);
        }
    };
    let events = filter_timeline_events(events, event.as_deref(), phase.as_deref());

    if json {
        print_json(&RunTimelineOutput { handle_id, events });
        return ExitCode::SUCCESS;
    }

    if events.is_empty() {
        println!("no events found");
        return ExitCode::SUCCESS;
    }

    for item in events {
        let detail = serde_json::to_string(&item.detail).unwrap_or_else(|_| "null".to_string());
        println!("{} [{}] {}", item.display_timestamp(), item.event, detail);
    }
    ExitCode::SUCCESS
}

struct ReadEventsOptions {
    handle_id: Option<String>,
    all: bool,
    event: Option<String>,
    phase: Option<String>,
    follow: bool,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    phase_timeout_secs: Option<u64>,
    json: bool,
}

async fn read_events(cfg: RuntimeConfig, options: ReadEventsOptions) -> ExitCode {
    let ReadEventsOptions {
        handle_id,
        all,
        event,
        phase,
        follow,
        interval_ms,
        timeout_secs,
        phase_timeout_secs,
        json,
    } = options;
    if handle_id.is_none() && !all {
        eprintln!("events failed: missing handle_id (or pass --all)");
        return ExitCode::from(2);
    }
    if handle_id.is_some() && all {
        eprintln!("events failed: --all conflicts with <handle-id>");
        return ExitCode::from(2);
    }
    let Some(handle_id) = handle_id else {
        return read_events_all(
            cfg,
            ReadEventsAllOptions {
                event,
                phase,
                follow,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            },
        )
        .await;
    };

    if !follow {
        return read_timeline(cfg, handle_id, event, phase, json);
    }

    let started = Instant::now();
    let mut cursor = EventStreamCursor::default();
    let mut all_events = Vec::new();
    let mut last_phase_progress = String::new();
    let mut observed_phase: Option<String> = None;
    let mut observed_phase_started_at = Instant::now();
    let sleep_ms = interval_ms.max(50);
    loop {
        let record = match load_run_record(&cfg.state_dir, &handle_id) {
            Ok(record) => record,
            Err(err) => {
                eprintln!("events failed: {err}");
                return ExitCode::from(1);
            }
        };

        let new_events = match load_run_events_incremental(&cfg.state_dir, &handle_id, &mut cursor)
        {
            Ok(events) => events,
            Err(err) => {
                eprintln!("events failed: {err}");
                return ExitCode::from(1);
            }
        };

        for evt in &new_events {
            if event
                .as_deref()
                .is_some_and(|needle| evt.event.as_str() != needle)
            {
                continue;
            }
            if phase
                .as_deref()
                .is_some_and(|needle| evt.phase.as_deref().is_none_or(|value| value != needle))
            {
                continue;
            }
            if json {
                match serde_json::to_string(evt) {
                    Ok(line) => println!("{line}"),
                    Err(err) => {
                        eprintln!("events failed to serialize line: {err}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                let detail =
                    serde_json::to_string(&evt.detail).unwrap_or_else(|_| "null".to_string());
                let message = evt.message.as_deref().unwrap_or("");
                if message.is_empty() {
                    println!("{} [{}] {}", evt.display_timestamp(), evt.event, detail);
                } else {
                    println!(
                        "{} [{}] {} {}",
                        evt.display_timestamp(),
                        evt.event,
                        message,
                        detail
                    );
                }
            }
        }
        all_events.extend(new_events);
        let current_phase = latest_event(&all_events).and_then(|evt| evt.phase.clone());
        if current_phase != observed_phase {
            observed_phase = current_phase;
            observed_phase_started_at = Instant::now();
        }
        if !json {
            if let Some(line) = build_phase_progress_line(
                &all_events,
                is_terminal_status(record.status()),
                OffsetDateTime::now_utc(),
                phase.as_deref(),
            ) {
                if line != last_phase_progress {
                    println!("{line}");
                    last_phase_progress = line;
                }
            }
        }

        if is_terminal_status(record.status()) {
            return ExitCode::SUCCESS;
        }
        if phase_timeout_secs
            .is_some_and(|secs| observed_phase_started_at.elapsed().as_secs() >= secs)
        {
            eprintln!(
                "events follow phase timeout after {}s in phase `{}` for handle `{}`",
                phase_timeout_secs.unwrap_or_default(),
                observed_phase.as_deref().unwrap_or("unknown"),
                handle_id
            );
            return ExitCode::from(124);
        }
        if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
            eprintln!(
                "events follow timed out after {}s for handle `{}`",
                timeout_secs.unwrap_or_default(),
                handle_id
            );
            return ExitCode::from(1);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
    }
}

fn wait_exit_code(status: &str) -> ExitCode {
    match status {
        "succeeded" => ExitCode::SUCCESS,
        "timed_out" => ExitCode::from(124),
        "cancelled" => ExitCode::from(2),
        _ => ExitCode::from(1),
    }
}

async fn wait_run(
    cfg: RuntimeConfig,
    handle_id: String,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    json: bool,
) -> ExitCode {
    let started = Instant::now();
    let sleep_ms = interval_ms.max(50);
    loop {
        let record = match load_run_record(&cfg.state_dir, &handle_id) {
            Ok(record) => record,
            Err(err) => {
                eprintln!("wait failed: {err}");
                return ExitCode::from(1);
            }
        };

        if is_terminal_status(record.status()) {
            if json {
                print_json(&WaitRunOutput {
                    handle_id,
                    status: record.status().to_string(),
                    updated_at: record.updated_at().to_string(),
                    error_message: record.error_message().map(str::to_string),
                });
            } else if let Some(error) = record.error_message() {
                println!("{} {} ({})", record.status(), record.updated_at(), error);
            } else {
                println!("{} {}", record.status(), record.updated_at());
            }
            return wait_exit_code(record.status());
        }

        if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
            eprintln!(
                "wait timed out after {}s for handle `{}`",
                timeout_secs.unwrap_or_default(),
                handle_id
            );
            return ExitCode::from(124);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
    }
}

fn read_stats(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    let record = match load_run_record(&cfg.state_dir, &handle_id) {
        Ok(record) => record,
        Err(err) => {
            eprintln!("stats failed: {err}");
            return ExitCode::from(1);
        }
    };
    let output = build_run_stats_output(
        &cfg.state_dir,
        &handle_id,
        &record,
        OffsetDateTime::now_utc(),
    );

    if json {
        print_json(&output);
    } else {
        println!("handle_id: {}", output.handle_id);
        println!("status: {}", output.status);
        println!(
            "state: {}",
            output.state.as_deref().unwrap_or(output.status.as_str())
        );
        println!("phase: {}", output.phase.as_deref().unwrap_or("unknown"));
        println!(
            "wall_ms: {}",
            output
                .wall_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "queue_ms: {}",
            output
                .queue_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "provider_probe_ms: {}",
            output
                .provider_probe_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "workspace_prepare_ms: {}",
            output
                .workspace_prepare_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "provider_boot_ms: {}",
            output
                .provider_boot_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "execution_ms: {}",
            output
                .execution_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "first_output_ms: {}",
            output
                .first_output_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!("first_output_warned: {}", output.first_output_warned);
        println!(
            "first_output_warning_at: {}",
            output
                .first_output_warning_at
                .as_deref()
                .unwrap_or("unknown")
        );
        println!(
            "current_wait_reason: {}",
            output.current_wait_reason.as_deref().unwrap_or("-")
        );
        println!(
            "wait_reasons: {}",
            if output.wait_reasons.is_empty() {
                "-".to_string()
            } else {
                output.wait_reasons.join(",")
            }
        );
        println!(
            "last_event_at: {}",
            output.last_event_at.as_deref().unwrap_or("unknown")
        );
        println!(
            "last_event_age_ms: {}",
            output
                .last_event_age_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!("stalled: {}", output.stalled);
        println!(
            "block_reason: {}",
            output.block_reason.as_deref().unwrap_or("-")
        );
        println!(
            "tokens: input={:?} output={:?} total={:?} source={}",
            output.usage.input_tokens,
            output.usage.output_tokens,
            output.usage.total_tokens,
            output.usage.token_source
        );
    }
    ExitCode::SUCCESS
}

fn build_run_stats_output(
    state_dir: &Path,
    handle_id: &str,
    record: &StoredRunRecord,
    now: OffsetDateTime,
) -> RunStatsOutput {
    let usage = build_usage_output(state_dir, handle_id, record);
    let events = load_run_events(state_dir, handle_id).unwrap_or_default();

    let created_at = parse_rfc3339(record.created_at());
    let accepted_at = first_event_time(&events, "run.accepted").or(created_at);
    let probe_started = first_event_time(&events, "provider.probe.started");
    let probe_completed = first_event_time(&events, "provider.probe.completed");
    let workspace_started = first_event_time(&events, "workspace.prepare.started");
    let provider_boot_started = first_event_time(&events, "provider.boot.started");
    let first_output = first_event_time(&events, "provider.first_output");
    let first_output_warning_at = first_event_timestamp(&events, "provider.first_output.warning");
    let first_output_warned = first_output_warning_at.is_some();
    let terminal_at = if is_terminal_status(record.status()) {
        parse_rfc3339(record.updated_at())
    } else {
        None
    };
    let end_at = terminal_at.or(Some(now));

    let queue_ms = duration_between(accepted_at, probe_started.or(workspace_started));
    let provider_probe_ms = duration_between(probe_started, probe_completed);
    let workspace_prepare_ms = duration_between(
        workspace_started,
        provider_boot_started.or(first_output).or(end_at),
    );
    let provider_boot_ms = duration_between(provider_boot_started, first_output.or(end_at));
    let execution_start = workspace_started
        .or(probe_completed)
        .or(probe_started)
        .or(accepted_at);
    let execution_ms = duration_between(execution_start, end_at);
    let first_output_ms = duration_between(accepted_at, first_output);
    let wall_ms = duration_between(accepted_at.or(created_at), end_at);

    let latest = latest_event(&events);
    let last_event_at = latest
        .map(|event| event.display_timestamp().to_string())
        .filter(|value| !value.is_empty());
    let last_event_age_ms = latest.and_then(event_time).and_then(|ts| {
        if now < ts {
            None
        } else {
            Some((now - ts).whole_milliseconds().max(0) as u64)
        }
    });
    let state = latest.and_then(|event| event.state.clone());
    let phase = latest.and_then(|event| event.phase.clone());
    let stalled = !is_terminal_status(record.status())
        && last_event_age_ms.is_some_and(|value| value >= 8_000);
    let block_reason = classify_block_reason(
        record.status(),
        phase.as_deref(),
        stalled,
        &events,
        record.error_message(),
    );
    let (wait_reasons, current_wait_reason) = collect_wait_reasons(&events);

    RunStatsOutput {
        handle_id: handle_id.to_string(),
        status: record.status().to_string(),
        state,
        phase,
        last_event_at,
        last_event_age_ms,
        stalled,
        block_reason,
        queue_ms,
        provider_probe_ms,
        workspace_prepare_ms,
        provider_boot_ms,
        execution_ms,
        first_output_ms,
        first_output_warned,
        first_output_warning_at,
        current_wait_reason,
        wait_reasons,
        wall_ms,
        usage,
    }
}

async fn watch_run(
    cfg: RuntimeConfig,
    handle_id: String,
    phase: Option<String>,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    phase_timeout_secs: Option<u64>,
    json: bool,
) -> ExitCode {
    let started = Instant::now();
    let mut last_status = String::new();
    let mut cursor = EventStreamCursor::default();
    let mut all_events = Vec::new();
    let mut last_phase_progress = String::new();
    let mut observed_phase: Option<String> = None;
    let mut observed_phase_started_at = Instant::now();
    loop {
        let record = match load_run_record(&cfg.state_dir, &handle_id) {
            Ok(record) => record,
            Err(err) => {
                eprintln!("watch failed: {err}");
                return ExitCode::from(1);
            }
        };

        if !json {
            if let Ok(new_events) = collect_watch_events_incremental(
                &cfg.state_dir,
                &handle_id,
                &mut cursor,
                &mut all_events,
            ) {
                for event in &new_events {
                    if phase.as_deref().is_some_and(|needle| {
                        event.phase.as_deref().is_none_or(|value| value != needle)
                    }) {
                        continue;
                    }
                    let detail =
                        serde_json::to_string(&event.detail).unwrap_or_else(|_| "null".to_string());
                    let message = event.message.as_deref().unwrap_or("");
                    if message.is_empty() {
                        println!("{} [{}] {}", event.display_timestamp(), event.event, detail);
                    } else {
                        println!(
                            "{} [{}] {} {}",
                            event.display_timestamp(),
                            event.event,
                            message,
                            detail
                        );
                    }
                }
                let current_phase = latest_event(&all_events).and_then(|evt| evt.phase.clone());
                if current_phase != observed_phase {
                    observed_phase = current_phase;
                    observed_phase_started_at = Instant::now();
                }
                if let Some(line) = build_phase_progress_line(
                    &all_events,
                    is_terminal_status(record.status()),
                    OffsetDateTime::now_utc(),
                    phase.as_deref(),
                ) {
                    if line != last_phase_progress {
                        println!("{line}");
                        last_phase_progress = line;
                    }
                }
            }
            if record.status() != last_status {
                println!("{} {}", record.status(), record.updated_at());
                last_status = record.status().to_string();
            }
        }

        if is_terminal_status(record.status()) {
            if json {
                return show_run(cfg, handle_id, true);
            }
            return ExitCode::SUCCESS;
        }
        if phase_timeout_secs
            .is_some_and(|secs| observed_phase_started_at.elapsed().as_secs() >= secs)
        {
            eprintln!(
                "watch phase timeout after {}s in phase `{}` for handle `{}`",
                phase_timeout_secs.unwrap_or_default(),
                observed_phase.as_deref().unwrap_or("unknown"),
                handle_id
            );
            return ExitCode::from(124);
        }

        if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
            eprintln!(
                "watch timed out after {}s for handle `{}`",
                timeout_secs.unwrap_or_default(),
                handle_id
            );
            return ExitCode::from(1);
        }

        let sleep_ms = interval_ms.max(50);
        tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
    }
}

fn init_command(options: InitCommandOptions) -> ExitCode {
    let InitCommandOptions {
        preset,
        root_dir,
        in_place,
        force,
        refresh_bootstrap,
        sync_project_config,
        sync_project_config_only,
        json,
    } = options;
    if refresh_bootstrap && in_place {
        eprintln!(
            "init failed: --refresh-bootstrap cannot be combined with --in-place; run from project root or pass --root-dir <bootstrap-root>"
        );
        return ExitCode::from(1);
    }
    if sync_project_config_only && in_place {
        eprintln!(
            "init failed: --sync-project-config-only requires --root-dir <generated-root> and cannot be combined with --in-place"
        );
        return ExitCode::from(1);
    }
    if sync_project_config_only && refresh_bootstrap {
        eprintln!(
            "init failed: --sync-project-config-only cannot be combined with --refresh-bootstrap"
        );
        return ExitCode::from(1);
    }
    let use_default_bootstrap_root = root_dir.is_none() && !in_place;
    let sync_project_bridge =
        use_default_bootstrap_root || sync_project_config || sync_project_config_only;
    let cwd = match resolve_cli_cwd() {
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
    let result = if sync_project_config_only {
        sync_project_bridge_workspace(&root)
    } else if refresh_bootstrap {
        refresh_bootstrap_workspace(&root)
    } else {
        init_workspace(&root, preset.into(), force)
    };
    match result {
        Ok(mut report) => {
            if sync_project_bridge {
                let bridge_config_path = cwd.join(PROJECT_BRIDGE_CONFIG_RELATIVE);
                let bridge_config_existed_before = bridge_config_path.exists();
                let bridge_result = if use_default_bootstrap_root {
                    ensure_bootstrap_bridge_config(&cwd, force)
                } else {
                    ensure_project_bridge_config(
                        &cwd,
                        &root.join("agents"),
                        &root.join(".mcp-subagent/state"),
                        force,
                    )
                };
                match bridge_result {
                    Ok((path, true)) => {
                        record_init_report_file_write(
                            &mut report,
                            path.clone(),
                            bridge_config_existed_before,
                        );
                        report.notes.push(format!(
                            "Generated project bridge config at `{}`; you can run mcp-subagent commands from project root without extra --agents-dir/--state-dir flags.",
                            path.display()
                        ));
                    }
                    Ok((path, false)) => report.notes.push(format!(
                        "Using existing project config `{}` (preserved).",
                        path.display()
                    )),
                    Err(err) => {
                        eprintln!("init failed: {err}");
                        return ExitCode::from(1);
                    }
                }
                let extra_gitignore_rules = if use_default_bootstrap_root {
                    Vec::new()
                } else {
                    project_root_gitignore_rules(&cwd, &root)
                };
                if use_default_bootstrap_root || !extra_gitignore_rules.is_empty() {
                    let gitignore_path = cwd.join(PROJECT_GITIGNORE_RELATIVE);
                    let gitignore_existed_before = gitignore_path.exists();
                    let gitignore_result = if use_default_bootstrap_root {
                        ensure_project_gitignore(&cwd)
                    } else {
                        ensure_project_gitignore_with_rules(&cwd, &extra_gitignore_rules)
                    };
                    match gitignore_result {
                        Ok((path, true)) => {
                            record_init_report_file_write(
                                &mut report,
                                path.clone(),
                                gitignore_existed_before,
                            );
                            report.notes.push(format!(
                                "Updated `{}` with mcp-subagent runtime ignore rules.",
                                path.display()
                            ));
                        }
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

fn record_init_report_file_write(report: &mut InitReport, path: PathBuf, existed_before: bool) {
    report.created_files.retain(|existing| existing != &path);
    report
        .overwritten_files
        .retain(|existing| existing != &path);
    let target = if existed_before {
        &mut report.overwritten_files
    } else {
        &mut report.created_files
    };
    target.push(path);
}

struct InitCommandOptions {
    preset: InitPresetArg,
    root_dir: Option<PathBuf>,
    in_place: bool,
    force: bool,
    refresh_bootstrap: bool,
    sync_project_config: bool,
    sync_project_config_only: bool,
    json: bool,
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

fn ensure_project_bridge_config(
    cwd: &Path,
    agents_dir: &Path,
    state_dir: &Path,
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
    fs::write(
        &config_path,
        project_bridge_config_template(cwd, agents_dir, state_dir),
    )
    .map_err(|err| format!("failed to write project bridge config: {err}"))?;
    Ok((config_path, true))
}

fn ensure_bootstrap_bridge_config(
    cwd: &Path,
    force: bool,
) -> std::result::Result<(PathBuf, bool), String> {
    ensure_project_bridge_config(
        cwd,
        &cwd.join(BRIDGE_AGENTS_DIR_RELATIVE),
        &cwd.join(BRIDGE_STATE_DIR_RELATIVE),
        force,
    )
}

fn project_bridge_config_template(cwd: &Path, agents_dir: &Path, state_dir: &Path) -> String {
    let agents_dir = render_bridge_path_for_config(cwd, agents_dir);
    let state_dir = render_bridge_path_for_config(cwd, state_dir);
    format!(
        r#"[server]
transport = "stdio"
log_level = "info"

[paths]
agents_dirs = ["{agents_dir}"]
state_dir = "{state_dir}"
"#,
        agents_dir = toml_escape_string(&agents_dir),
        state_dir = toml_escape_string(&state_dir)
    )
}

fn render_bridge_path_for_config(cwd: &Path, target: &Path) -> String {
    let resolved = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    };
    if let Ok(relative) = resolved.strip_prefix(cwd) {
        return format!("./{}", relative.display());
    }
    resolved.display().to_string()
}

fn toml_escape_string(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

fn ensure_project_gitignore(cwd: &Path) -> std::result::Result<(PathBuf, bool), String> {
    ensure_project_gitignore_with_rules(cwd, &[])
}

fn ensure_project_gitignore_with_rules(
    cwd: &Path,
    extra_rules: &[String],
) -> std::result::Result<(PathBuf, bool), String> {
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
        .map(|rule| (*rule).to_string())
        .chain(extra_rules.iter().cloned())
        .filter(|target| {
            !existing_rules
                .iter()
                .any(|rule| gitignore_rule_matches_target(rule, target))
        })
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
        content.push_str(&rule);
        content.push('\n');
    }

    fs::write(&gitignore_path, content)
        .map_err(|err| format!("failed to write .gitignore: {err}"))?;
    Ok((gitignore_path, true))
}

fn project_root_gitignore_rules(cwd: &Path, root: &Path) -> Vec<String> {
    let resolved = fs::canonicalize(root).unwrap_or_else(|_| {
        if root.is_absolute() {
            root.to_path_buf()
        } else {
            cwd.join(root)
        }
    });
    let Ok(relative) = resolved.strip_prefix(cwd) else {
        return Vec::new();
    };
    if relative.as_os_str().is_empty() {
        return Vec::new();
    }
    vec![format!("{}/", relative.display())]
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
    let paths = match resolve_connect_paths(cfg, "connect-snippet") {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let snippet = build_connect_snippet(host.into(), &paths);
    println!("{snippet}");
    ExitCode::SUCCESS
}

fn connect_command(cfg: RuntimeConfig, host: ConnectHostArg, run_host: bool) -> ExitCode {
    let connect_host = host.into();
    let paths = match resolve_connect_paths(cfg, "connect") {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let invocation = build_connect_invocation(connect_host, &paths);
    if let Err(err) = run_connect_invocation(&invocation) {
        eprintln!("connect failed: {err}");
        return ExitCode::from(1);
    }
    println!(
        "registered mcp-subagent for host `{}` (agents_dir={}, state_dir={})",
        host.as_str(),
        paths.agents_dir.display(),
        paths.state_dir.display()
    );
    if run_host {
        let launch = build_host_launch_invocation(connect_host);
        if let Err(err) = run_host_invocation(&launch) {
            eprintln!("connect failed: {err}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

fn resolve_connect_paths(
    cfg: RuntimeConfig,
    command_label: &str,
) -> std::result::Result<mcp_subagent::connect::ConnectSnippetPaths, String> {
    let Some(first_agents_dir) = cfg.agents_dirs.first().cloned() else {
        return Err(format!(
            "{command_label} failed: no agents directory configured"
        ));
    };

    let cwd = match resolve_cli_cwd() {
        Ok(path) => path,
        Err(err) => {
            return Err(format!(
                "{command_label} failed: unable to resolve current directory: {err}"
            ));
        }
    };
    let binary = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            return Err(format!(
                "{command_label} failed: unable to resolve current executable: {err}"
            ));
        }
    };
    Ok(resolve_connect_snippet_paths(
        &cwd,
        binary,
        first_agents_dir,
        cfg.state_dir,
    ))
}

fn run_connect_invocation(invocation: &ConnectInvocation) -> std::result::Result<(), String> {
    run_invocation(invocation, false)
}

fn run_host_invocation(invocation: &ConnectInvocation) -> std::result::Result<(), String> {
    run_invocation(invocation, true)
}

fn run_invocation(
    invocation: &ConnectInvocation,
    interactive: bool,
) -> std::result::Result<(), String> {
    let mut command = Command::new(&invocation.executable);
    command.args(&invocation.args);
    if interactive {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }

    let status = command
        .status()
        .map_err(|err| format!("failed to execute `{}`: {err}", invocation.executable))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{}` exited with status {}",
            invocation.executable, status
        ))
    }
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

struct AgentRunCommand {
    agent: String,
    task: String,
    task_brief: Option<String>,
    parent_summary: Option<String>,
    stage: Option<String>,
    plan_ref: Option<String>,
    selected_files: Vec<PathBuf>,
    selected_files_inline: Vec<PathBuf>,
    working_dir: Option<PathBuf>,
    stream: bool,
    json: bool,
}

async fn run_agent(cfg: RuntimeConfig, options: AgentRunCommand) -> ExitCode {
    let AgentRunCommand {
        agent,
        task,
        task_brief,
        parent_summary,
        stage,
        plan_ref,
        selected_files,
        selected_files_inline,
        working_dir,
        stream,
        json,
    } = options;
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

    if stream {
        return spawn_and_optionally_stream(cfg, input, json, true, false, "run").await;
    }

    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);

    match server.run_agent(Parameters(input)).await {
        Ok(result) => {
            if json {
                print_json(&result.0);
            } else {
                let out = result.0;
                println!("handle_id: {}", out.handle_id);
                println!("phase: {}", out.phase);
                println!("terminal: {}", if out.terminal { "yes" } else { "no" });
                if let Some(OutcomeView::Succeeded {
                    summary,
                    key_findings,
                    ..
                }) = out.outcome
                {
                    println!("summary: {}", summary);
                    if !key_findings.is_empty() {
                        println!("key_findings:");
                        for finding in key_findings {
                            println!("- {finding}");
                        }
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

async fn spawn_agent(cfg: RuntimeConfig, options: AgentRunCommand) -> ExitCode {
    let AgentRunCommand {
        agent,
        task,
        task_brief,
        parent_summary,
        stage,
        plan_ref,
        selected_files,
        selected_files_inline,
        working_dir,
        stream,
        json,
    } = options;
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
    spawn_and_optionally_stream(cfg, input, json, stream, true, "spawn").await
}

async fn submit_agent(cfg: RuntimeConfig, options: AgentRunCommand) -> ExitCode {
    let AgentRunCommand {
        agent,
        task,
        task_brief,
        parent_summary,
        stage,
        plan_ref,
        selected_files,
        selected_files_inline,
        working_dir,
        stream,
        json,
    } = options;
    let selected_files = match build_selected_file_inputs(
        selected_files,
        selected_files_inline,
        working_dir.as_deref(),
    ) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("submit failed: {err}");
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
    spawn_and_optionally_stream(cfg, input, json, stream, true, "submit").await
}

fn cli_spawn_waits_for_completion() -> bool {
    !std::env::var("MCP_SUBAGENT_CLI_SPAWN_ACCEPT_ONLY")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
}

async fn fetch_status_output(
    server: &McpSubagentServer,
    handle_id: String,
) -> std::result::Result<RunStatusOutput, String> {
    let run_view = server
        .get_agent_status(Parameters(HandleInput {
            handle_id: handle_id.clone(),
        }))
        .await
        .map_err(|err| err.message.to_string())?
        .0;
    let stats = server
        .get_agent_stats(Parameters(GetAgentStatsInput { handle_id }))
        .await
        .map_err(|err| err.message.to_string())?
        .0;
    Ok(build_run_status_output(run_view, stats))
}

fn print_stream_accepted_output(output: &RunView, json: bool) -> std::result::Result<(), String> {
    if json {
        return print_json_line(&serde_json::json!({
            "kind": "accepted",
            "run": output,
        }));
    }
    println!("handle_id: {}", output.handle_id);
    println!("phase: {}", output.phase);
    println!("accepted_at: {}", output.updated_at);
    Ok(())
}

fn print_stream_terminal_output(
    output: &RunStatusOutput,
    json: bool,
) -> std::result::Result<(), String> {
    if json {
        return print_json_line(&serde_json::json!({
            "kind": "final_status",
            "run": output,
        }));
    }
    println!("{}", render_status_output_text(output));
    Ok(())
}

async fn follow_streamed_run(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    read_logs(
        cfg,
        handle_id,
        ReadLogsOptions {
            stdout_only: false,
            stderr_only: false,
            phase: None,
            follow: true,
            interval_ms: STREAM_FOLLOW_INTERVAL_MS,
            timeout_secs: None,
            phase_timeout_secs: None,
            json,
        },
    )
    .await
}

async fn spawn_and_optionally_stream(
    cfg: RuntimeConfig,
    input: RunAgentInput,
    json: bool,
    stream: bool,
    announce_keepalive: bool,
    error_label: &str,
) -> ExitCode {
    let server =
        McpSubagentServer::new_with_state_dir(cfg.agents_dirs.clone(), cfg.state_dir.clone());
    match server.spawn_agent(Parameters(input)).await {
        Ok(result) => {
            let output = result.0;
            if stream {
                if let Err(err) = print_stream_accepted_output(&output, json) {
                    eprintln!("stream failed: {err}");
                    return ExitCode::from(1);
                }
                let follow_code =
                    follow_streamed_run(cfg.clone(), output.handle_id.clone(), json).await;
                server.wait_for_run(&output.handle_id).await;
                if follow_code != ExitCode::SUCCESS {
                    return follow_code;
                }
                match fetch_status_output(&server, output.handle_id.clone()).await {
                    Ok(status) => {
                        if let Err(err) = print_stream_terminal_output(&status, json) {
                            eprintln!("stream failed: {err}");
                            return ExitCode::from(1);
                        }
                        return wait_exit_code(status.status.as_str());
                    }
                    Err(err) => {
                        eprintln!("stream failed: {err}");
                        return ExitCode::from(1);
                    }
                }
            }

            if announce_keepalive && cli_spawn_waits_for_completion() {
                eprintln!(
                    "spawn accepted; keeping CLI process alive until run settles (set MCP_SUBAGENT_CLI_SPAWN_ACCEPT_ONLY=1 for immediate return)"
                );
            }
            if json {
                print_json(&output);
            } else {
                println!("handle_id: {}", output.handle_id);
                println!("phase: {}", output.phase);
                println!("accepted_at: {}", output.updated_at);
            }
            if announce_keepalive && cli_spawn_waits_for_completion() {
                server.wait_for_run(&output.handle_id).await;
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{error_label} failed: {}", err.message);
            ExitCode::from(1)
        }
    }
}

async fn get_status(cfg: RuntimeConfig, handle_id: String, json: bool) -> ExitCode {
    let server = McpSubagentServer::new_with_state_dir(cfg.agents_dirs, cfg.state_dir);
    match fetch_status_output(&server, handle_id).await {
        Ok(output) => {
            if json {
                print_json(&output);
            } else {
                println!("{}", render_status_output_text(&output));
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("status failed: {err}");
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
            let index = artifacts_from_run_view(&status);
            match resolve_artifact_path(target_kind, &index) {
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

fn artifacts_from_run_view(run: &RunView) -> Vec<ArtifactOutput> {
    match run.outcome.as_ref() {
        Some(OutcomeView::Succeeded { artifacts, .. }) => artifacts.clone(),
        _ => Vec::new(),
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

fn print_clean_report(report: &CleanReport) {
    println!("# mcp-subagent clean");
    println!("state_dir: {}", report.state_dir.display());
    println!("mode: {}", report.mode);
    println!("dry_run: {}", report.dry_run);
    if !report.cleaned.is_empty() {
        println!("cleaned:");
        for entry in &report.cleaned {
            println!(
                "- [{}] {} ({} bytes)",
                entry.action,
                entry.path.display(),
                entry.bytes
            );
        }
    }
    if !report.missing.is_empty() {
        println!("missing:");
        for path in &report.missing {
            println!("- {}", path.display());
        }
    }
    println!("reclaimed_bytes: {}", report.reclaimed_bytes);
    if !report.errors.is_empty() {
        println!("errors:");
        for err in &report.errors {
            println!("- {err}");
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

    use mcp_subagent::config::RuntimeConfig;

    use crate::{
        build_selected_file_inputs, clean_state_dir, ensure_bootstrap_bridge_config,
        ensure_project_bridge_config, ensure_project_gitignore, project_bridge_config_template,
        project_root_gitignore_rules, record_init_report_file_write, resolve_init_root,
        ArtifactKindArg, Cli, Commands, ConnectHostArg, InitPresetArg, RunAgentSelectedFileInput,
        RunResultOutput, RunShowOutput, StoredNativeUsage, StoredRunRecord, StoredRunSpecSnapshot,
        UsageStatsOutput, BRIDGE_AGENTS_DIR_RELATIVE, BRIDGE_STATE_DIR_RELATIVE,
        DEFAULT_BOOTSTRAP_ROOT_RELATIVE, PROJECT_BRIDGE_CONFIG_RELATIVE,
        PROJECT_GITIGNORE_RELATIVE, RESULT_CONTRACT_VERSION,
    };

    fn sample_outcome_usage() -> super::StoredOutcomeUsage {
        super::StoredOutcomeUsage {
            duration_ms: 0,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            provider_exit_code: None,
        }
    }

    fn sample_run_state() -> super::StoredRunState {
        super::StoredRunState {
            status: "pending".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            updated_at: "2026-03-25T00:00:00Z".to_string(),
            status_history: Vec::new(),
            error_message: None,
            policy: None,
            compiled_context_markdown: None,
            usage: None,
        }
    }

    fn sample_run_record() -> StoredRunRecord {
        StoredRunRecord {
            task_spec: super::StoredTaskSpec {
                task: "review code".to_string(),
            },
            state: sample_run_state(),
            outcome: None,
            artifact_index: Vec::new(),
            spec_snapshot: None,
        }
    }

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
    fn parses_run_command_with_stream_flag() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "run",
            "reviewer",
            "--task",
            "review code",
            "--stream",
        ]);
        match cli.command {
            Commands::Run { stream, .. } => assert!(stream),
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
                refresh_bootstrap,
                sync_project_config,
                sync_project_config_only,
                json,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisor));
                assert!(in_place);
                assert!(force);
                assert!(!refresh_bootstrap);
                assert!(!sync_project_config);
                assert!(!sync_project_config_only);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn init_defaults_to_minimal_supervisor_preset() {
        let cli = Cli::parse_from(["mcp-subagent", "init"]);
        match cli.command {
            Commands::Init {
                preset,
                refresh_bootstrap,
                sync_project_config,
                sync_project_config_only,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisorMinimal));
                assert!(!refresh_bootstrap);
                assert!(!sync_project_config);
                assert!(!sync_project_config_only);
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
                preset,
                in_place,
                refresh_bootstrap,
                sync_project_config,
                sync_project_config_only,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::MinimalSingleProvider));
                assert!(!in_place);
                assert!(!refresh_bootstrap);
                assert!(!sync_project_config);
                assert!(!sync_project_config_only);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_init_refresh_bootstrap_flag() {
        let cli = Cli::parse_from(["mcp-subagent", "init", "--refresh-bootstrap"]);
        match cli.command {
            Commands::Init {
                preset,
                refresh_bootstrap,
                sync_project_config,
                sync_project_config_only,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisorMinimal));
                assert!(refresh_bootstrap);
                assert!(!sync_project_config);
                assert!(!sync_project_config_only);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_init_sync_project_config_flag() {
        let cli = Cli::parse_from(["mcp-subagent", "init", "--sync-project-config"]);
        match cli.command {
            Commands::Init {
                preset,
                sync_project_config,
                sync_project_config_only,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisorMinimal));
                assert!(sync_project_config);
                assert!(!sync_project_config_only);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_init_sync_project_config_only_flag() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "init",
            "--root-dir",
            "tmp/bootstrap",
            "--sync-project-config-only",
        ]);
        match cli.command {
            Commands::Init {
                preset,
                sync_project_config,
                sync_project_config_only,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisorMinimal));
                assert!(!sync_project_config);
                assert!(sync_project_config_only);
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
    fn init_rejects_sync_project_config_only_without_root_dir() {
        let result = Cli::try_parse_from(["mcp-subagent", "init", "--sync-project-config-only"]);
        assert!(
            result.is_err(),
            "init should require --root-dir with --sync-project-config-only"
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
        assert_eq!(
            content,
            project_bridge_config_template(
                dir.path(),
                &dir.path().join(BRIDGE_AGENTS_DIR_RELATIVE),
                &dir.path().join(BRIDGE_STATE_DIR_RELATIVE),
            )
        );
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
        assert_eq!(
            content,
            project_bridge_config_template(
                dir.path(),
                &dir.path().join(BRIDGE_AGENTS_DIR_RELATIVE),
                &dir.path().join(BRIDGE_STATE_DIR_RELATIVE),
            )
        );
    }

    #[test]
    fn writes_custom_root_bridge_config_when_requested() {
        let dir = tempdir().expect("tempdir");
        let custom_root = dir.path().join("custom-root");
        let agents_dir = custom_root.join("agents");
        let state_dir = custom_root.join(".mcp-subagent/state");

        let (path, written) =
            ensure_project_bridge_config(dir.path(), &agents_dir, &state_dir, false)
                .expect("write custom bridge");
        assert!(written);
        assert_eq!(path, dir.path().join(".mcp-subagent/config.toml"));
        let content = fs::read_to_string(path).expect("read");
        assert!(content.contains("agents_dirs = [\"./custom-root/agents\"]"));
        assert!(content.contains("state_dir = \"./custom-root/.mcp-subagent/state\""));
    }

    #[test]
    fn bridge_config_preserves_external_absolute_paths_without_canonicalizing() {
        let cwd = Path::new("/tmp/workspace");
        let agents_dir = Path::new("/var/folders/example/custom-root/agents");
        let state_dir = Path::new("/var/folders/example/custom-root/.mcp-subagent/state");

        let content = project_bridge_config_template(cwd, agents_dir, state_dir);

        assert!(content.contains("agents_dirs = [\"/var/folders/example/custom-root/agents\"]"));
        assert!(content
            .contains("state_dir = \"/var/folders/example/custom-root/.mcp-subagent/state\""));
    }

    #[test]
    fn custom_root_gitignore_rules_only_cover_project_local_roots() {
        let dir = tempdir().expect("tempdir");
        let inside = project_root_gitignore_rules(dir.path(), &dir.path().join("custom-root"));
        let outside = project_root_gitignore_rules(dir.path(), Path::new("/tmp/external-root"));

        assert_eq!(inside, vec!["custom-root/".to_string()]);
        assert!(outside.is_empty());
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
    fn init_report_records_created_bridge_and_gitignore_files() {
        let project_root = PathBuf::from("/tmp/project");
        let mut report = mcp_subagent::init::InitReport {
            preset: "test".to_string(),
            preset_catalog_version: "v0.9.0".to_string(),
            root: project_root.join(".mcp-subagent/bootstrap"),
            agents_dir: project_root.join(".mcp-subagent/bootstrap/agents"),
            created_files: Vec::new(),
            overwritten_files: Vec::new(),
            generated_agent_count: 0,
            notes: Vec::new(),
        };

        record_init_report_file_write(
            &mut report,
            project_root.join(PROJECT_BRIDGE_CONFIG_RELATIVE),
            false,
        );
        record_init_report_file_write(
            &mut report,
            project_root.join(PROJECT_GITIGNORE_RELATIVE),
            false,
        );

        assert_eq!(
            report.created_files,
            vec![
                project_root.join(PROJECT_BRIDGE_CONFIG_RELATIVE),
                project_root.join(PROJECT_GITIGNORE_RELATIVE)
            ]
        );
        assert!(report.overwritten_files.is_empty());
    }

    #[test]
    fn init_report_records_overwritten_bridge_config_for_synced_custom_root() {
        let project_root = PathBuf::from("/tmp/project");
        let bridge_path = project_root.join(PROJECT_BRIDGE_CONFIG_RELATIVE);
        let mut report = mcp_subagent::init::InitReport {
            preset: "test".to_string(),
            preset_catalog_version: "v0.9.0".to_string(),
            root: PathBuf::from("/tmp/custom-root"),
            agents_dir: PathBuf::from("/tmp/custom-root/agents"),
            created_files: vec![bridge_path.clone()],
            overwritten_files: Vec::new(),
            generated_agent_count: 0,
            notes: Vec::new(),
        };

        record_init_report_file_write(&mut report, bridge_path.clone(), true);

        assert!(report.created_files.is_empty());
        assert_eq!(report.overwritten_files, vec![bridge_path]);
    }

    #[test]
    fn init_report_does_not_record_preserved_paths_without_writes() {
        let project_root = PathBuf::from("/tmp/project");
        let report = mcp_subagent::init::InitReport {
            preset: "test".to_string(),
            preset_catalog_version: "v0.9.0".to_string(),
            root: project_root.join(".mcp-subagent/bootstrap"),
            agents_dir: project_root.join(".mcp-subagent/bootstrap/agents"),
            created_files: Vec::new(),
            overwritten_files: Vec::new(),
            generated_agent_count: 0,
            notes: vec![format!(
                "Using existing project config `{}` (preserved).",
                project_root.join(PROJECT_BRIDGE_CONFIG_RELATIVE).display()
            )],
        };

        assert!(report.created_files.is_empty());
        assert!(report.overwritten_files.is_empty());
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
    fn parses_connect_host() {
        let cli = Cli::parse_from(["mcp-subagent", "connect", "--host", "codex"]);
        match cli.command {
            Commands::Connect { host, run_host } => {
                assert!(matches!(host, ConnectHostArg::Codex));
                assert!(!run_host);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_connect_with_run_host_flag() {
        let cli = Cli::parse_from(["mcp-subagent", "connect", "--host", "gemini", "--run-host"]);
        match cli.command {
            Commands::Connect { host, run_host } => {
                assert!(matches!(host, ConnectHostArg::Gemini));
                assert!(run_host);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_clean_command_flags() {
        let cli = Cli::parse_from(["mcp-subagent", "clean", "--all", "--dry-run", "--json"]);
        match cli.command {
            Commands::Clean { all, dry_run, json } => {
                assert!(all);
                assert!(dry_run);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_ps_command_flags() {
        let cli = Cli::parse_from(["mcp-subagent", "ps", "--limit", "5", "--json"]);
        match cli.command {
            Commands::Ps { limit, json } => {
                assert_eq!(limit, 5);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_show_command() {
        let cli = Cli::parse_from(["mcp-subagent", "show", "handle-1", "--json"]);
        match cli.command {
            Commands::Show { handle_id, json } => {
                assert_eq!(handle_id, "handle-1");
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn build_run_status_output_carries_stall_and_block_reason() {
        let output = super::build_run_status_output(
            mcp_subagent::mcp::dto::RunView {
                handle_id: "handle-1".to_string(),
                agent_name: "backend-coder".to_string(),
                task_brief: Some("implement feature".to_string()),
                phase: "running".to_string(),
                terminal: false,
                outcome: None,
                created_at: "2026-03-28T00:00:00Z".to_string(),
                updated_at: "2026-03-28T00:00:05Z".to_string(),
            },
            mcp_subagent::mcp::dto::GetAgentStatsOutput {
                handle_id: "handle-1".to_string(),
                status: "running".to_string(),
                state: Some("running".to_string()),
                phase: Some("provider_boot".to_string()),
                last_event_at: Some("2026-03-28T00:00:05Z".to_string()),
                last_event_age_ms: Some(9000),
                stalled: true,
                block_reason: Some("waiting_for_trust".to_string()),
                advice: vec!["approve trust once".to_string()],
                queue_ms: Some(10),
                provider_probe_ms: Some(20),
                workspace_prepare_ms: Some(30),
                provider_boot_ms: Some(40),
                execution_ms: Some(50),
                first_output_ms: None,
                first_output_warned: true,
                first_output_warning_at: Some("2026-03-28T00:00:04Z".to_string()),
                current_wait_reason: Some("provider.first_output.warning".to_string()),
                wait_reasons: vec!["provider.first_output.warning".to_string()],
                wall_ms: Some(100),
                usage: mcp_subagent::mcp::dto::RunUsageOutput {
                    started_at: Some("2026-03-28T00:00:00Z".to_string()),
                    finished_at: None,
                    duration_ms: Some(100),
                    provider: "codex".to_string(),
                    model: Some("gpt-5.3-codex".to_string()),
                    provider_exit_code: None,
                    retries: 0,
                    token_source: "none".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    estimated_prompt_bytes: None,
                    estimated_output_bytes: None,
                },
            },
        );

        assert_eq!(output.phase, "provider_boot");
        assert!(output.stalled);
        assert_eq!(output.block_reason.as_deref(), Some("waiting_for_trust"));
        let rendered = super::render_status_output_text(&output);
        assert!(rendered.contains("stalled: yes"));
        assert!(rendered.contains("block_reason: waiting_for_trust"));
        assert!(rendered.contains("current_wait_reason: provider.first_output.warning"));
    }

    #[test]
    fn show_renderer_emits_color_badge_when_enabled() {
        let view = RunShowOutput {
            handle_id: "run-1".to_string(),
            status: "succeeded".to_string(),
            updated_at: "2026-03-25T00:00:00Z".to_string(),
            error_message: None,
            provider: Some("Codex".to_string()),
            model: Some("gpt-5.3-codex".to_string()),
            normalization_status: Some("Validated".to_string()),
            summary: Some("all good".to_string()),
            provider_exit_code: Some(0),
            retries: 0,
            retry_classification: "non_retryable".to_string(),
            classification_reason: Some("runner succeeded".to_string()),
            usage: UsageStatsOutput {
                started_at: Some("2026-03-25T00:00:00Z".to_string()),
                finished_at: Some("2026-03-25T00:00:01Z".to_string()),
                duration_ms: Some(1000),
                provider: "Codex".to_string(),
                model: Some("gpt-5.3-codex".to_string()),
                provider_exit_code: Some(0),
                retries: 0,
                token_source: "estimated".to_string(),
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
                estimated_prompt_bytes: Some(40),
                estimated_output_bytes: Some(80),
            },
            artifact_index: Vec::new(),
        };

        let rendered = super::render_show_run_text(&view, true);
        assert!(
            rendered.contains("\u{1b}[1;32mSUCCEEDED\u{1b}[0m"),
            "expected green succeeded badge: {rendered}"
        );
    }

    #[test]
    fn show_renderer_is_plain_when_color_disabled() {
        let view = RunShowOutput {
            handle_id: "run-2".to_string(),
            status: "failed".to_string(),
            updated_at: "2026-03-25T00:00:00Z".to_string(),
            error_message: Some("boom".to_string()),
            provider: Some("Codex".to_string()),
            model: None,
            normalization_status: Some("Invalid".to_string()),
            summary: None,
            provider_exit_code: Some(1),
            retries: 1,
            retry_classification: "retryable".to_string(),
            classification_reason: Some("matched retryable keyword `timeout`".to_string()),
            usage: UsageStatsOutput {
                started_at: Some("2026-03-25T00:00:00Z".to_string()),
                finished_at: Some("2026-03-25T00:00:01Z".to_string()),
                duration_ms: Some(1000),
                provider: "Codex".to_string(),
                model: None,
                provider_exit_code: Some(1),
                retries: 1,
                token_source: "estimated".to_string(),
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
                estimated_prompt_bytes: Some(40),
                estimated_output_bytes: Some(80),
            },
            artifact_index: Vec::new(),
        };

        let rendered = super::render_show_run_text(&view, false);
        assert!(rendered.starts_with("FAILED  run-2"), "{rendered}");
        assert!(
            !rendered.contains("\u{1b}["),
            "plain output must not contain ansi escapes: {rendered}"
        );
        assert!(rendered.contains("error: boom"), "{rendered}");
    }

    #[test]
    fn parses_result_command_raw_mode() {
        let cli = Cli::parse_from(["mcp-subagent", "result", "handle-1", "--raw"]);
        match cli.command {
            Commands::Result {
                handle_id,
                raw,
                normalized,
                summary,
                ..
            } => {
                assert_eq!(handle_id, "handle-1");
                assert!(raw);
                assert!(!normalized);
                assert!(!summary);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn result_json_schema_contains_stable_fields() {
        let output = RunResultOutput {
            contract_version: RESULT_CONTRACT_VERSION.to_string(),
            handle_id: "h-1".to_string(),
            status: "succeeded".to_string(),
            view: "summary".to_string(),
            normalization_status: "Validated".to_string(),
            summary: Some("done".to_string()),
            native_result: Some("native".to_string()),
            normalized_result: None,
            provider_exit_code: Some(0),
            retries: 0,
            retry_classification: "non_retryable".to_string(),
            classification_reason: Some("runner succeeded".to_string()),
            usage: UsageStatsOutput {
                started_at: Some("2026-03-25T00:00:00Z".to_string()),
                finished_at: Some("2026-03-25T00:00:01Z".to_string()),
                duration_ms: Some(1000),
                provider: "Codex".to_string(),
                model: Some("gpt-5.3-codex".to_string()),
                provider_exit_code: Some(0),
                retries: 0,
                token_source: "estimated".to_string(),
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
                estimated_prompt_bytes: Some(40),
                estimated_output_bytes: Some(80),
            },
            error_message: None,
            artifact_index: Vec::new(),
        };
        let value = serde_json::to_value(&output).expect("serialize output");

        for key in [
            "contract_version",
            "handle_id",
            "status",
            "view",
            "normalization_status",
            "native_result",
            "normalized_result",
            "usage",
            "provider_exit_code",
            "retries",
            "retry_classification",
            "classification_reason",
            "error_message",
            "artifact_index",
        ] {
            assert!(
                value.get(key).is_some(),
                "missing key `{key}` in result json: {value}"
            );
        }
        assert_eq!(
            value
                .get("contract_version")
                .and_then(serde_json::Value::as_str),
            Some(RESULT_CONTRACT_VERSION)
        );
    }

    #[test]
    fn status_json_schema_contains_stable_fields() {
        let output = super::RunStatusOutput {
            handle_id: "h-1".to_string(),
            agent_name: "backend-coder".to_string(),
            task_brief: Some("implement feature".to_string()),
            status: "running".to_string(),
            state: Some("running".to_string()),
            phase: "provider_boot".to_string(),
            terminal: false,
            created_at: "2026-03-28T00:00:00Z".to_string(),
            updated_at: "2026-03-28T00:00:05Z".to_string(),
            stalled: true,
            block_reason: Some("waiting_for_trust".to_string()),
            last_event_at: Some("2026-03-28T00:00:05Z".to_string()),
            last_event_age_ms: Some(9000),
            current_wait_reason: Some("provider.first_output.warning".to_string()),
            wait_reasons: vec!["provider.first_output.warning".to_string()],
            advice: vec!["approve trust once".to_string()],
            outcome: None,
        };
        let value = serde_json::to_value(&output).expect("serialize output");

        for key in [
            "handle_id",
            "agent_name",
            "task_brief",
            "status",
            "state",
            "phase",
            "terminal",
            "created_at",
            "updated_at",
            "stalled",
            "block_reason",
            "last_event_at",
            "last_event_age_ms",
            "current_wait_reason",
            "wait_reasons",
            "advice",
            "outcome",
        ] {
            assert!(
                value.get(key).is_some(),
                "missing key `{key}` in status json: {value}"
            );
        }
    }

    #[test]
    fn build_usage_output_prefers_native_tokens() {
        let dir = tempdir().expect("tempdir");
        let record = StoredRunRecord {
            state: super::StoredRunState {
                status: "succeeded".to_string(),
                created_at: "2026-03-25T00:00:00Z".to_string(),
                updated_at: "2026-03-25T00:00:01Z".to_string(),
                usage: Some(StoredNativeUsage {
                    input_tokens: Some(111),
                    output_tokens: Some(222),
                    total_tokens: Some(333),
                }),
                ..sample_run_state()
            },
            spec_snapshot: Some(StoredRunSpecSnapshot {
                name: "backend-coder".to_string(),
                provider: "Codex".to_string(),
                model: Some("gpt-5.3-codex".to_string()),
            }),
            ..sample_run_record()
        };

        let usage = super::build_usage_output(dir.path(), "run-native", &record);
        assert_eq!(usage.token_source, "native");
        assert_eq!(usage.input_tokens, Some(111));
        assert_eq!(usage.output_tokens, Some(222));
        assert_eq!(usage.total_tokens, Some(333));
    }

    #[test]
    fn build_usage_output_marks_mixed_when_partial_native_usage_exists() {
        let dir = tempdir().expect("tempdir");
        let run_artifacts = dir.path().join("runs").join("run-mixed").join("artifacts");
        fs::create_dir_all(&run_artifacts).expect("mkdir artifacts");
        fs::write(run_artifacts.join("stdout.txt"), "x".repeat(400)).expect("write stdout");

        let record = StoredRunRecord {
            state: super::StoredRunState {
                status: "succeeded".to_string(),
                created_at: "2026-03-25T00:00:00Z".to_string(),
                updated_at: "2026-03-25T00:00:01Z".to_string(),
                usage: Some(StoredNativeUsage {
                    input_tokens: Some(50),
                    output_tokens: None,
                    total_tokens: None,
                }),
                ..sample_run_state()
            },
            ..sample_run_record()
        };

        let usage = super::build_usage_output(dir.path(), "run-mixed", &record);
        assert_eq!(usage.token_source, "mixed");
        assert_eq!(usage.input_tokens, Some(50));
        assert!(usage.output_tokens.is_some(), "expected estimated fallback");
    }

    #[test]
    fn resolve_retry_classification_defaults_unknown_when_missing() {
        let record = sample_run_record();
        let (classification, reason) = super::resolve_retry_classification(&record);
        assert_eq!(classification, "unknown");
        assert_eq!(reason, None);
    }

    #[test]
    fn resolve_retry_classification_reads_failed_outcome_retry_info() {
        let record = StoredRunRecord {
            outcome: Some(super::StoredRunOutcome::Failed(
                super::StoredFailureOutcome {
                    error: "failed".to_string(),
                    retry: super::StoredRetryInfo {
                        classification: "retryable".to_string(),
                        reason: Some("matched retryable keyword `network`".to_string()),
                        attempts_used: 2,
                    },
                    partial_summary: None,
                    usage: sample_outcome_usage(),
                },
            )),
            ..sample_run_record()
        };
        let (classification, reason) = super::resolve_retry_classification(&record);
        assert_eq!(classification, "retryable");
        assert_eq!(
            reason.as_deref(),
            Some("matched retryable keyword `network`")
        );
    }

    #[test]
    fn parses_logs_command_stderr_mode() {
        let cli = Cli::parse_from(["mcp-subagent", "logs", "handle-1", "--stderr", "--json"]);
        match cli.command {
            Commands::Logs {
                handle_id,
                stdout,
                stderr,
                phase,
                follow,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert!(!stdout);
                assert!(stderr);
                assert_eq!(phase, None);
                assert!(!follow);
                assert_eq!(interval_ms, 1000);
                assert_eq!(timeout_secs, None);
                assert_eq!(phase_timeout_secs, None);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_logs_follow_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "logs",
            "handle-1",
            "--phase",
            "provider_boot",
            "--follow",
            "--interval-ms",
            "250",
            "--timeout-secs",
            "12",
            "--phase-timeout-secs",
            "7",
            "--stdout",
        ]);
        match cli.command {
            Commands::Logs {
                handle_id,
                stdout,
                stderr,
                phase,
                follow,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert!(stdout);
                assert!(!stderr);
                assert_eq!(phase.as_deref(), Some("provider_boot"));
                assert!(follow);
                assert_eq!(interval_ms, 250);
                assert_eq!(timeout_secs, Some(12));
                assert_eq!(phase_timeout_secs, Some(7));
                assert!(!json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_timeline_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "timeline",
            "handle-1",
            "--event",
            "parse",
            "--json",
        ]);
        match cli.command {
            Commands::Timeline {
                handle_id,
                event,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert_eq!(event.as_deref(), Some("parse"));
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_events_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "events",
            "handle-1",
            "--event",
            "provider.heartbeat",
            "--phase",
            "running",
            "--follow",
            "--interval-ms",
            "250",
            "--timeout-secs",
            "12",
            "--phase-timeout-secs",
            "8",
            "--json",
        ]);
        match cli.command {
            Commands::Events {
                handle_id,
                all,
                event,
                phase,
                follow,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            } => {
                assert_eq!(handle_id.as_deref(), Some("handle-1"));
                assert!(!all);
                assert_eq!(event.as_deref(), Some("provider.heartbeat"));
                assert_eq!(phase.as_deref(), Some("running"));
                assert!(follow);
                assert_eq!(interval_ms, 250);
                assert_eq!(timeout_secs, Some(12));
                assert_eq!(phase_timeout_secs, Some(8));
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_events_all_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "events",
            "--all",
            "--follow",
            "--event",
            "provider.first_output",
            "--phase",
            "provider_boot",
            "--interval-ms",
            "300",
            "--timeout-secs",
            "20",
            "--phase-timeout-secs",
            "10",
        ]);
        match cli.command {
            Commands::Events {
                handle_id,
                all,
                event,
                phase,
                follow,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            } => {
                assert!(handle_id.is_none());
                assert!(all);
                assert_eq!(event.as_deref(), Some("provider.first_output"));
                assert_eq!(phase.as_deref(), Some("provider_boot"));
                assert!(follow);
                assert_eq!(interval_ms, 300);
                assert_eq!(timeout_secs, Some(20));
                assert_eq!(phase_timeout_secs, Some(10));
                assert!(!json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn load_run_events_and_filter_by_event_name() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        let events_path = run_dir.join("events.jsonl");
        fs::write(
            &events_path,
            concat!(
                "{\"event\":\"probe\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{\"status\":\"ready\"},\"seq\":1}\n",
                "{\"event\":\"parse\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{\"parse_status\":\"Validated\"},\"seq\":2}\n"
            ),
        )
        .expect("write events");

        let events = super::load_run_events(dir.path(), "handle-1").expect("load events");
        assert_eq!(events.len(), 2);
        let filtered = super::filter_timeline_events(events, Some("parse"), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event, "parse");
    }

    #[test]
    fn load_run_events_reads_jsonl_only() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        fs::write(
            run_dir.join("events.jsonl"),
            "{\"event\":\"canonical\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{},\"seq\":1}\n",
        )
        .expect("write canonical events");

        let events = super::load_run_events(dir.path(), "handle-1").expect("load events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "canonical");
        assert_eq!(events[0].seq, Some(1));
    }

    #[test]
    fn load_run_events_incremental_only_returns_appended_events() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        let events_path = run_dir.join("events.jsonl");
        fs::write(
            &events_path,
            "{\"event\":\"run.accepted\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{},\"seq\":1}\n",
        )
        .expect("write initial events");

        let mut cursor = super::EventStreamCursor::default();
        let first = super::load_run_events_incremental(dir.path(), "handle-1", &mut cursor)
            .expect("load first chunk");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event, "run.accepted");

        let second = super::load_run_events_incremental(dir.path(), "handle-1", &mut cursor)
            .expect("load second chunk");
        assert!(second.is_empty());

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&events_path)
            .expect("open events append");
        writeln!(
            file,
            "{{\"event\":\"provider.probe.started\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{{}},\"seq\":2}}"
        )
        .expect("append event");

        let third = super::load_run_events_incremental(dir.path(), "handle-1", &mut cursor)
            .expect("load appended chunk");
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].event, "provider.probe.started");
    }

    #[test]
    fn load_run_events_incremental_handles_partial_trailing_line() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        let events_path = run_dir.join("events.jsonl");
        fs::write(
            &events_path,
            "{\"event\":\"run.accepted\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{},\"seq\":1",
        )
        .expect("write partial event");

        let mut cursor = super::EventStreamCursor::default();
        let first = super::load_run_events_incremental(dir.path(), "handle-1", &mut cursor)
            .expect("read partial");
        assert!(first.is_empty());

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&events_path)
            .expect("open events append");
        writeln!(file, "}}").expect("complete event");

        let second = super::load_run_events_incremental(dir.path(), "handle-1", &mut cursor)
            .expect("read completed");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].seq, Some(1));
    }

    #[test]
    fn collect_watch_events_incremental_only_returns_new_events() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        let events_path = run_dir.join("events.jsonl");
        fs::write(
            &events_path,
            "{\"event\":\"run.accepted\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{},\"seq\":1}\n",
        )
        .expect("write initial event");

        let mut cursor = super::EventStreamCursor::default();
        let mut all_events = Vec::new();
        let first = super::collect_watch_events_incremental(
            dir.path(),
            "handle-1",
            &mut cursor,
            &mut all_events,
        )
        .expect("first collect");
        assert_eq!(first.len(), 1);
        assert_eq!(all_events.len(), 1);

        let second = super::collect_watch_events_incremental(
            dir.path(),
            "handle-1",
            &mut cursor,
            &mut all_events,
        )
        .expect("second collect");
        assert!(second.is_empty());
        assert_eq!(all_events.len(), 1);

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&events_path)
            .expect("open append");
        writeln!(
            file,
            "{{\"event\":\"provider.boot.started\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{{}},\"seq\":2}}"
        )
        .expect("append event");

        let third = super::collect_watch_events_incremental(
            dir.path(),
            "handle-1",
            &mut cursor,
            &mut all_events,
        )
        .expect("third collect");
        assert_eq!(third.len(), 1);
        assert_eq!(all_events.len(), 2);
        assert_eq!(all_events[1].event, "provider.boot.started");
    }

    #[tokio::test]
    async fn watch_run_timeout_keeps_existing_timeout_semantics() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        fs::write(
            run_dir.join("run.json"),
            serde_json::json!({
                "task_spec": {
                    "task": "watch timeout fixture"
                },
                "state": {
                    "status": "running",
                    "created_at": "2026-03-25T00:00:00Z",
                    "updated_at": "2026-03-25T00:00:00Z",
                    "status_history": ["received", "running"],
                    "error_message": null,
                    "policy": null,
                    "compiled_context_markdown": null,
                    "usage": null
                },
                "outcome": null,
                "artifact_index": [],
                "spec_snapshot": null
            })
            .to_string(),
        )
        .expect("write run record");
        fs::write(
            run_dir.join("events.jsonl"),
            "{\"event\":\"run.accepted\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{},\"seq\":1,\"state\":\"accepted\",\"phase\":\"accepted\"}\n",
        )
        .expect("write events");

        let cfg = RuntimeConfig {
            agents_dirs: vec![PathBuf::from("./agents")],
            state_dir: dir.path().to_path_buf(),
            log_level: "info".to_string(),
        };

        let code =
            super::watch_run(cfg, "handle-1".to_string(), None, 50, Some(0), None, false).await;
        assert_eq!(code, std::process::ExitCode::from(1));
    }

    #[test]
    fn collect_run_event_snapshots_loads_all_handles_and_filters() {
        let dir = tempdir().expect("tempdir");
        for (handle, status) in [("handle-a", "running"), ("handle-b", "succeeded")] {
            let run_dir = dir.path().join("runs").join(handle);
            fs::create_dir_all(&run_dir).expect("mkdir run");
            fs::write(
                run_dir.join("run.json"),
                serde_json::json!({
                    "task_spec": {
                        "task": format!("snapshot fixture {handle}")
                    },
                    "state": {
                        "status": status,
                        "created_at": "2026-03-25T00:00:00Z",
                        "updated_at": "2026-03-25T00:00:00Z",
                        "status_history": ["received", status],
                        "error_message": null,
                        "policy": null,
                        "compiled_context_markdown": null,
                        "usage": null
                    },
                    "outcome": null,
                    "artifact_index": [],
                    "spec_snapshot": null
                })
                .to_string(),
            )
            .expect("write run.json");
            fs::write(
                run_dir.join("events.jsonl"),
                "{\"event\":\"provider.first_output\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{},\"seq\":1,\"phase\":\"running\"}\n",
            )
            .expect("write events");
        }

        let snapshots = super::collect_run_event_snapshots(
            dir.path(),
            Some("provider.first_output"),
            Some("running"),
        )
        .expect("collect snapshots");
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots.iter().all(|snapshot| snapshot.events.len() == 1
            && snapshot.events[0].event == "provider.first_output"));
    }

    #[test]
    fn list_run_records_fails_when_any_run_json_is_invalid() {
        let dir = tempdir().expect("tempdir");

        let valid_run_dir = dir.path().join("runs").join("handle-valid");
        fs::create_dir_all(&valid_run_dir).expect("mkdir valid run");
        fs::write(
            valid_run_dir.join("run.json"),
            serde_json::json!({
                "task_spec": {
                    "task": "valid fixture",
                    "task_brief": null,
                    "acceptance_criteria": [],
                    "selected_files": [],
                    "working_dir": "."
                },
                "state": {
                    "status": "running",
                    "created_at": "2026-03-25T00:00:00Z",
                    "updated_at": "2026-03-25T00:00:00Z",
                    "status_history": ["received", "running"],
                    "error_message": null,
                    "policy": null,
                    "compiled_context_markdown": null,
                    "usage": null
                },
                "outcome": null,
                "artifact_index": [],
                "spec_snapshot": null
            })
            .to_string(),
        )
        .expect("write valid run");

        let invalid_run_dir = dir.path().join("runs").join("handle-invalid");
        fs::create_dir_all(&invalid_run_dir).expect("mkdir invalid run");
        fs::write(
            invalid_run_dir.join("run.json"),
            serde_json::json!({
                "status": "running",
                "updated_at": "2026-03-25T00:00:00Z"
            })
            .to_string(),
        )
        .expect("write invalid run");

        let err = super::list_run_records(dir.path()).expect_err("should fail on invalid run json");
        assert!(err.contains("handle-invalid"), "unexpected error: {err}");
    }

    #[test]
    fn build_run_stats_output_derives_phase_and_durations_from_events() {
        let dir = tempdir().expect("tempdir");
        let run_dir = dir.path().join("runs").join("handle-1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        fs::write(
            run_dir.join("events.jsonl"),
            concat!(
                "{\"event\":\"run.accepted\",\"timestamp\":\"2026-03-25T00:00:00Z\",\"detail\":{},\"state\":\"accepted\",\"phase\":\"accepted\",\"seq\":1}\n",
                "{\"event\":\"provider.probe.started\",\"timestamp\":\"2026-03-25T00:00:01Z\",\"detail\":{},\"state\":\"preparing\",\"phase\":\"provider_probe\",\"seq\":2}\n",
                "{\"event\":\"provider.probe.completed\",\"timestamp\":\"2026-03-25T00:00:02Z\",\"detail\":{},\"state\":\"preparing\",\"phase\":\"provider_probe\",\"seq\":3}\n",
                "{\"event\":\"workspace.prepare.started\",\"timestamp\":\"2026-03-25T00:00:03Z\",\"detail\":{},\"state\":\"preparing\",\"phase\":\"workspace_prepare\",\"seq\":4}\n",
                "{\"event\":\"provider.boot.started\",\"timestamp\":\"2026-03-25T00:00:04Z\",\"detail\":{},\"state\":\"running\",\"phase\":\"provider_boot\",\"seq\":5}\n",
                "{\"event\":\"provider.waiting_for_auth\",\"timestamp\":\"2026-03-25T00:00:05Z\",\"detail\":{},\"state\":\"running\",\"phase\":\"waiting_for_auth\",\"seq\":6}\n",
                "{\"event\":\"provider.first_output.warning\",\"timestamp\":\"2026-03-25T00:00:06Z\",\"detail\":{},\"state\":\"running\",\"phase\":\"provider_boot\",\"seq\":7}\n",
                "{\"event\":\"provider.first_output\",\"timestamp\":\"2026-03-25T00:00:07Z\",\"detail\":{},\"state\":\"running\",\"phase\":\"running\",\"seq\":8}\n",
                "{\"event\":\"run.completed\",\"timestamp\":\"2026-03-25T00:00:08Z\",\"detail\":{},\"state\":\"succeeded\",\"phase\":\"completed\",\"seq\":9}\n"
            ),
        )
        .expect("write events");

        let record = StoredRunRecord {
            state: super::StoredRunState {
                status: "succeeded".to_string(),
                created_at: "2026-03-25T00:00:00Z".to_string(),
                updated_at: "2026-03-25T00:00:08Z".to_string(),
                ..sample_run_state()
            },
            ..sample_run_record()
        };
        let now = super::parse_rfc3339("2026-03-25T00:00:08Z").expect("parse now");
        let stats = super::build_run_stats_output(dir.path(), "handle-1", &record, now);
        assert_eq!(stats.queue_ms, Some(1_000));
        assert_eq!(stats.provider_probe_ms, Some(1_000));
        assert_eq!(stats.workspace_prepare_ms, Some(1_000));
        assert_eq!(stats.provider_boot_ms, Some(3_000));
        assert_eq!(stats.execution_ms, Some(5_000));
        assert_eq!(stats.first_output_ms, Some(7_000));
        assert_eq!(stats.wall_ms, Some(8_000));
        assert_eq!(stats.state.as_deref(), Some("succeeded"));
        assert_eq!(stats.phase.as_deref(), Some("completed"));
        assert!(stats.first_output_warned);
        assert_eq!(
            stats.first_output_warning_at.as_deref(),
            Some("2026-03-25T00:00:06Z")
        );
        assert_eq!(stats.current_wait_reason.as_deref(), Some("auth_required"));
        assert_eq!(stats.wait_reasons, vec!["auth_required".to_string()]);
        assert!(!stats.stalled);
    }

    #[test]
    fn classify_block_reason_detects_provider_unavailable_from_error_text() {
        let reason = super::classify_block_reason(
            "failed",
            Some("provider_probe"),
            false,
            &[],
            Some("provider `Codex` is unavailable (status=MissingBinary; binary `codex` not found in PATH)"),
        );
        assert_eq!(reason.as_deref(), Some("provider_unavailable"));
    }

    #[test]
    fn classify_block_reason_uses_stalled_phase_fallback() {
        let reason =
            super::classify_block_reason("running", Some("workspace_prepare"), true, &[], None);
        assert_eq!(reason.as_deref(), Some("workspace_prepare"));
    }

    #[test]
    fn classify_block_reason_uses_provider_wait_event() {
        let events = vec![super::RunTimelineEvent {
            event: "provider.waiting_for_auth".to_string(),
            timestamp: "2026-03-25T00:00:00Z".to_string(),
            detail: serde_json::json!({}),
            seq: Some(1),
            level: None,
            state: Some("running".to_string()),
            phase: Some("waiting_for_auth".to_string()),
            source: Some("provider".to_string()),
            message: Some("provider is waiting for authentication".to_string()),
        }];
        let reason = super::classify_block_reason("running", Some("running"), true, &events, None);
        assert_eq!(reason.as_deref(), Some("auth_required"));
    }

    #[test]
    fn classify_block_reason_ignores_cached_credentials_text() {
        let reason = super::classify_block_reason_from_text(
            "Loaded cached credentials. Using FileKeychain fallback for secure storage.",
        );
        assert!(reason.is_none());
    }

    #[test]
    fn classify_block_reason_is_none_for_succeeded_status() {
        let reason = super::classify_block_reason(
            "succeeded",
            Some("completed"),
            false,
            &[super::RunTimelineEvent {
                event: "provider.waiting_for_auth".to_string(),
                timestamp: "2026-03-25T00:00:00Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(1),
                level: None,
                state: Some("running".to_string()),
                phase: Some("waiting_for_auth".to_string()),
                source: Some("provider".to_string()),
                message: Some("provider is waiting for authentication".to_string()),
            }],
            None,
        );
        assert!(reason.is_none());
    }

    #[test]
    fn build_phase_progress_line_marks_current_phase() {
        let events = vec![
            super::RunTimelineEvent {
                event: "run.accepted".to_string(),
                timestamp: "2026-03-25T00:00:00Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(1),
                level: None,
                state: Some("accepted".to_string()),
                phase: Some("accepted".to_string()),
                source: Some("runtime".to_string()),
                message: None,
            },
            super::RunTimelineEvent {
                event: "provider.probe.started".to_string(),
                timestamp: "2026-03-25T00:00:01Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(2),
                level: None,
                state: Some("preparing".to_string()),
                phase: Some("provider_probe".to_string()),
                source: Some("provider".to_string()),
                message: None,
            },
            super::RunTimelineEvent {
                event: "provider.first_output".to_string(),
                timestamp: "2026-03-25T00:00:02Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(3),
                level: None,
                state: Some("running".to_string()),
                phase: Some("running".to_string()),
                source: Some("provider".to_string()),
                message: None,
            },
        ];
        let now = super::parse_rfc3339("2026-03-25T00:00:05Z").expect("parse now");
        let line =
            super::build_phase_progress_line(&events, false, now, None).expect("progress line");
        assert!(line.contains("accepted="), "{line}");
        assert!(line.contains("provider_probe="), "{line}");
        assert!(line.contains("running*="), "{line}");
        assert!(line.contains("wall=5.0s"), "{line}");
    }

    #[test]
    fn build_phase_progress_line_terminal_has_no_current_marker() {
        let events = vec![super::RunTimelineEvent {
            event: "run.completed".to_string(),
            timestamp: "2026-03-25T00:00:02Z".to_string(),
            detail: serde_json::json!({}),
            seq: Some(1),
            level: None,
            state: Some("succeeded".to_string()),
            phase: Some("completed".to_string()),
            source: Some("runtime".to_string()),
            message: None,
        }];
        let now = super::parse_rfc3339("2026-03-25T00:00:05Z").expect("parse now");
        let line =
            super::build_phase_progress_line(&events, true, now, None).expect("progress line");
        assert!(line.contains("completed="), "{line}");
        assert!(!line.contains("*="), "{line}");
    }

    #[test]
    fn build_phase_progress_line_respects_phase_filter() {
        let events = vec![super::RunTimelineEvent {
            event: "provider.first_output".to_string(),
            timestamp: "2026-03-25T00:00:02Z".to_string(),
            detail: serde_json::json!({}),
            seq: Some(1),
            level: None,
            state: Some("running".to_string()),
            phase: Some("running".to_string()),
            source: Some("provider".to_string()),
            message: None,
        }];
        let now = super::parse_rfc3339("2026-03-25T00:00:05Z").expect("parse now");
        let line = super::build_phase_progress_line(&events, false, now, Some("provider_boot"));
        assert!(line.is_none());
    }

    #[test]
    fn build_phase_progress_line_ignores_synthetic_events() {
        let events = vec![
            super::RunTimelineEvent {
                event: "provider.first_output".to_string(),
                timestamp: "2026-03-25T00:00:02Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(1),
                level: None,
                state: Some("running".to_string()),
                phase: Some("running".to_string()),
                source: Some("provider".to_string()),
                message: None,
            },
            super::RunTimelineEvent {
                event: "context.compile.started".to_string(),
                timestamp: "2026-03-25T00:00:03Z".to_string(),
                detail: serde_json::json!({
                    "synthetic": true,
                }),
                seq: Some(2),
                level: None,
                state: Some("preparing".to_string()),
                phase: Some("context_compile".to_string()),
                source: Some("context".to_string()),
                message: None,
            },
            super::RunTimelineEvent {
                event: "run.completed".to_string(),
                timestamp: "2026-03-25T00:00:04Z".to_string(),
                detail: serde_json::json!({}),
                seq: Some(3),
                level: None,
                state: Some("succeeded".to_string()),
                phase: Some("completed".to_string()),
                source: Some("runtime".to_string()),
                message: None,
            },
        ];
        let now = super::parse_rfc3339("2026-03-25T00:00:05Z").expect("parse now");
        let line =
            super::build_phase_progress_line(&events, true, now, None).expect("progress line");
        assert!(!line.contains("context_compile"), "{line}");
        assert!(line.contains("running="), "{line}");
        assert!(line.contains("completed="), "{line}");
    }

    #[test]
    fn parses_watch_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "watch",
            "handle-1",
            "--interval-ms",
            "250",
            "--timeout-secs",
            "15",
        ]);
        match cli.command {
            Commands::Watch {
                handle_id,
                phase,
                interval_ms,
                timeout_secs,
                phase_timeout_secs,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert_eq!(phase, None);
                assert_eq!(interval_ms, 250);
                assert_eq!(timeout_secs, Some(15));
                assert_eq!(phase_timeout_secs, None);
                assert!(!json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_watch_phase_timeout_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "watch",
            "handle-1",
            "--phase",
            "provider_boot",
            "--phase-timeout-secs",
            "10",
        ]);
        match cli.command {
            Commands::Watch {
                handle_id,
                phase,
                phase_timeout_secs,
                ..
            } => {
                assert_eq!(handle_id, "handle-1");
                assert_eq!(phase.as_deref(), Some("provider_boot"));
                assert_eq!(phase_timeout_secs, Some(10));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_wait_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "wait",
            "handle-1",
            "--interval-ms",
            "300",
            "--timeout-secs",
            "20",
            "--json",
        ]);
        match cli.command {
            Commands::Wait {
                handle_id,
                interval_ms,
                timeout_secs,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert_eq!(interval_ms, 300);
                assert_eq!(timeout_secs, Some(20));
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_stats_command_flags() {
        let cli = Cli::parse_from(["mcp-subagent", "stats", "handle-1", "--json"]);
        match cli.command {
            Commands::Stats { handle_id, json } => {
                assert_eq!(handle_id, "handle-1");
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_submit_command_flags() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "submit",
            "fast-researcher",
            "--task",
            "find docs",
            "--json",
        ]);
        match cli.command {
            Commands::Submit {
                agent, task, json, ..
            } => {
                assert_eq!(agent, "fast-researcher");
                assert_eq!(task, "find docs");
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_spawn_command_with_stream_flag() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "spawn",
            "backend-coder",
            "--task",
            "implement feature",
            "--stream",
        ]);
        match cli.command {
            Commands::Spawn { stream, .. } => assert!(stream),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_submit_command_with_stream_flag() {
        let cli = Cli::parse_from([
            "mcp-subagent",
            "submit",
            "fast-researcher",
            "--task",
            "find docs",
            "--stream",
        ]);
        match cli.command {
            Commands::Submit { stream, .. } => assert!(stream),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clean_runtime_targets_removes_runs_and_logs() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("state");
        let runs_dir = state_dir.join("runs/handle-1");
        let logs_dir = state_dir.join("logs");
        let server_log = state_dir.join("server.log");
        fs::create_dir_all(&runs_dir).expect("create runs");
        fs::create_dir_all(&logs_dir).expect("create logs");
        fs::write(runs_dir.join("stdout.log"), "out").expect("write run log");
        fs::write(logs_dir.join("app.log"), "log").expect("write app log");
        fs::write(&server_log, "server").expect("write server log");

        let report = clean_state_dir(&state_dir, false, false);
        assert!(
            report.errors.is_empty(),
            "unexpected errors: {:?}",
            report.errors
        );
        assert!(!state_dir.join("runs").exists());
        assert!(!state_dir.join("logs").exists());
        assert!(!server_log.exists());
        assert!(!report.cleaned.is_empty());
    }

    #[test]
    fn clean_dry_run_keeps_files() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("state");
        let runs_dir = state_dir.join("runs/handle-1");
        fs::create_dir_all(&runs_dir).expect("create runs");
        fs::write(runs_dir.join("stdout.log"), "out").expect("write run log");

        let report = clean_state_dir(&state_dir, false, true);
        assert!(
            report.errors.is_empty(),
            "unexpected errors: {:?}",
            report.errors
        );
        assert!(state_dir.join("runs").exists());
        assert!(report
            .cleaned
            .iter()
            .all(|entry| entry.action == "would_remove"));
    }

    #[test]
    fn clean_all_removes_state_dir() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("state");
        fs::create_dir_all(state_dir.join("runs/handle-1")).expect("create runs");
        fs::write(state_dir.join("server.log"), "server").expect("write server log");

        let report = clean_state_dir(&state_dir, true, false);
        assert!(
            report.errors.is_empty(),
            "unexpected errors: {:?}",
            report.errors
        );
        assert!(!state_dir.exists());
        assert_eq!(report.mode, "all");
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

    #[test]
    fn cli_spawn_waits_for_completion_by_default() {
        assert!(super::cli_spawn_waits_for_completion());
    }
}
