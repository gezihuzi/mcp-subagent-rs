use std::{
    fs,
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
        json: bool,
    },
    Watch {
        handle_id: String,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        timeout_secs: Option<u64>,
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
            read_logs(cfg, handle_id, stdout, stderr, json)
        }
        Commands::Watch {
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
            info!("starting command: watch");
            watch_run(cfg, handle_id, interval_ms, timeout_secs, json).await
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
struct StoredStructuredSummary {
    summary: String,
    key_findings: Vec<String>,
    open_questions: Vec<String>,
    next_steps: Vec<String>,
    exit_code: i32,
    verification_status: String,
    touched_files: Vec<String>,
    plan_refs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
struct StoredSummaryEnvelope {
    contract_version: String,
    parse_status: String,
    summary: StoredStructuredSummary,
    raw_fallback_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
struct StoredRunSpecSnapshot {
    name: String,
    provider: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
struct StoredExecutionPolicy {
    attempts_used: Option<u32>,
    retries_used: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
struct StoredRunRecord {
    status: String,
    created_at: Option<String>,
    updated_at: String,
    status_history: Vec<String>,
    summary: Option<StoredSummaryEnvelope>,
    artifact_index: Vec<ArtifactOutput>,
    error_message: Option<String>,
    task: String,
    spec_snapshot: Option<StoredRunSpecSnapshot>,
    execution_policy: Option<StoredExecutionPolicy>,
    compiled_context_markdown: Option<String>,
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
    normalized_result: Option<StoredSummaryEnvelope>,
    provider_exit_code: Option<i32>,
    retries: u32,
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

fn runs_root(state_dir: &Path) -> PathBuf {
    state_dir.join("runs")
}

fn run_json_path(state_dir: &Path, handle_id: &str) -> PathBuf {
    runs_root(state_dir).join(handle_id).join("run.json")
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
        let record = match load_run_record(state_dir, &handle_id) {
            Ok(record) => record,
            Err(_) => continue,
        };
        runs.push((handle_id, record));
    }
    runs.sort_by_key(|(_, record)| parse_rfc3339(record.updated_at.as_str()));
    runs.reverse();
    Ok(runs)
}

fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
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

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "timed_out" | "cancelled")
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
    if let Some(summary) = &record.summary {
        return Some(summary.summary.exit_code);
    }
    let message = record.error_message.as_deref()?;
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
    let started_at = record.created_at.clone();
    let finished_at = if is_terminal_status(record.status.as_str()) {
        Some(record.updated_at.clone())
    } else {
        None
    };
    let estimated_prompt_bytes = record
        .compiled_context_markdown
        .as_ref()
        .map(|value| value.as_bytes().len() as u64);
    let stdout_bytes = read_artifact_from_disk(state_dir, handle_id, "stdout.txt")
        .ok()
        .flatten()
        .map(|text| text.as_bytes().len() as u64);
    let stderr_bytes = read_artifact_from_disk(state_dir, handle_id, "stderr.txt")
        .ok()
        .flatten()
        .map(|text| text.as_bytes().len() as u64);
    let estimated_output_bytes = match (stdout_bytes, stderr_bytes) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
    };
    let input_tokens = estimate_tokens(estimated_prompt_bytes);
    let output_tokens = estimate_tokens(estimated_output_bytes);
    let total_tokens = match (input_tokens, output_tokens) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        _ => None,
    };

    UsageStatsOutput {
        started_at: started_at.clone(),
        finished_at,
        duration_ms: compute_duration_ms(started_at.as_deref(), &record.updated_at),
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
            .execution_policy
            .as_ref()
            .and_then(|policy| policy.retries_used)
            .unwrap_or(0),
        token_source: if input_tokens.is_some() || output_tokens.is_some() {
            "estimated".to_string()
        } else {
            "unknown".to_string()
        },
        input_tokens,
        output_tokens,
        total_tokens,
        estimated_prompt_bytes,
        estimated_output_bytes,
    }
}

fn list_runs(cfg: RuntimeConfig, limit: usize, json: bool) -> ExitCode {
    let entries = match list_run_records(&cfg.state_dir) {
        Ok(items) => items,
        Err(err) => {
            eprintln!("ps failed: {err}");
            return ExitCode::from(1);
        }
    };

    let rows = entries
        .into_iter()
        .take(limit)
        .map(|(handle_id, record)| RunListEntry {
            handle_id,
            status: record.status.clone(),
            updated_at: record.updated_at.clone(),
            provider: record
                .spec_snapshot
                .as_ref()
                .map(|spec| spec.provider.clone()),
            agent: record.spec_snapshot.as_ref().map(|spec| spec.name.clone()),
            task: record.task.clone(),
            duration_ms: compute_duration_ms(record.created_at.as_deref(), &record.updated_at),
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
                "{} [{}] {} provider={} agent={} duration_ms={}",
                row.handle_id,
                row.status,
                row.updated_at,
                row.provider.as_deref().unwrap_or("unknown"),
                row.agent.as_deref().unwrap_or("unknown"),
                row.duration_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
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
    let view = RunShowOutput {
        handle_id: handle_id.clone(),
        status: record.status.clone(),
        updated_at: record.updated_at.clone(),
        error_message: record.error_message.clone(),
        provider: record
            .spec_snapshot
            .as_ref()
            .map(|spec| spec.provider.clone()),
        model: record
            .spec_snapshot
            .as_ref()
            .and_then(|spec| spec.model.clone()),
        normalization_status: record
            .summary
            .as_ref()
            .map(|summary| summary.parse_status.clone()),
        summary: record
            .summary
            .as_ref()
            .map(|summary| summary.summary.summary.clone()),
        provider_exit_code: infer_provider_exit_code(&record),
        retries: usage.retries,
        usage,
        artifact_index: record.artifact_index.clone(),
    };

    if json {
        print_json(&view);
    } else {
        println!("handle_id: {}", view.handle_id);
        println!("status: {}", view.status);
        println!("updated_at: {}", view.updated_at);
        println!(
            "provider: {}",
            view.provider.as_deref().unwrap_or("unknown")
        );
        println!("model: {}", view.model.as_deref().unwrap_or("unknown"));
        println!(
            "normalization_status: {}",
            view.normalization_status
                .as_deref()
                .unwrap_or("not_available")
        );
        println!(
            "provider_exit_code: {}",
            view.provider_exit_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "duration_ms: {}",
            view.usage
                .duration_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!("retries: {}", view.retries);
        if let Some(summary) = view.summary.as_deref() {
            println!("summary: {summary}");
        }
        if let Some(error_message) = view.error_message.as_deref() {
            println!("error: {error_message}");
        }
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
                .summary
                .as_ref()
                .and_then(|summary| summary.raw_fallback_text.clone())
        });
    let normalized_result = record.summary.clone();
    let usage = build_usage_output(&cfg.state_dir, &handle_id, &record);
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
        status: record.status.clone(),
        view: view.to_string(),
        normalization_status: record
            .summary
            .as_ref()
            .map(|summary| summary.parse_status.clone())
            .unwrap_or_else(|| "NotAvailable".to_string()),
        summary: record
            .summary
            .as_ref()
            .map(|summary| summary.summary.summary.clone()),
        native_result: native_result.clone(),
        normalized_result: normalized_result.clone(),
        provider_exit_code: usage.provider_exit_code,
        retries: usage.retries,
        usage,
        error_message: record.error_message.clone(),
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
            None => println!(""),
        }
        return ExitCode::SUCCESS;
    }

    if let Some(summary) = record.summary.as_ref() {
        println!("{}", summary.summary.summary);
    } else if let Some(raw_text) = native_result {
        println!("{raw_text}");
    }
    ExitCode::SUCCESS
}

fn read_logs(
    cfg: RuntimeConfig,
    handle_id: String,
    stdout_only: bool,
    stderr_only: bool,
    json: bool,
) -> ExitCode {
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
        if !stdout_only {
            if !err.is_empty() {
                println!("{err}");
            }
        }
    }
    ExitCode::SUCCESS
}

async fn watch_run(
    cfg: RuntimeConfig,
    handle_id: String,
    interval_ms: u64,
    timeout_secs: Option<u64>,
    json: bool,
) -> ExitCode {
    let started = Instant::now();
    let mut last_status = String::new();
    loop {
        let record = match load_run_record(&cfg.state_dir, &handle_id) {
            Ok(record) => record,
            Err(err) => {
                eprintln!("watch failed: {err}");
                return ExitCode::from(1);
            }
        };

        if !json && record.status != last_status {
            println!("{} {}", record.status, record.updated_at);
            last_status = record.status.clone();
        }

        if is_terminal_status(record.status.as_str()) {
            if json {
                return show_run(cfg, handle_id, true);
            }
            return ExitCode::SUCCESS;
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

    let cwd = match std::env::current_dir() {
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
            let handle_id = result.0.handle_id.clone();
            // In CLI mode the process exits as soon as this function returns,
            // which would kill any background tokio task before it can persist
            // its results.  Wait for the task to finish before printing and
            // exiting.  True fire-and-forget spawning is only available in the
            // long-lived MCP server mode.
            server.wait_for_run(&handle_id).await;
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

    use crate::{
        bootstrap_bridge_config_template, build_selected_file_inputs, clean_state_dir,
        ensure_bootstrap_bridge_config, ensure_project_gitignore, resolve_init_root,
        ArtifactKindArg, Cli, Commands, ConnectHostArg, InitPresetArg, RunAgentSelectedFileInput,
        RunResultOutput, UsageStatsOutput, DEFAULT_BOOTSTRAP_ROOT_RELATIVE,
        RESULT_CONTRACT_VERSION,
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
    fn init_defaults_to_minimal_supervisor_preset() {
        let cli = Cli::parse_from(["mcp-subagent", "init"]);
        match cli.command {
            Commands::Init { preset, .. } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisorMinimal));
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
    fn parses_logs_command_stderr_mode() {
        let cli = Cli::parse_from(["mcp-subagent", "logs", "handle-1", "--stderr", "--json"]);
        match cli.command {
            Commands::Logs {
                handle_id,
                stdout,
                stderr,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert!(!stdout);
                assert!(stderr);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
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
                interval_ms,
                timeout_secs,
                json,
            } => {
                assert_eq!(handle_id, "handle-1");
                assert_eq!(interval_ms, 250);
                assert_eq!(timeout_secs, Some(15));
                assert!(!json);
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
}
