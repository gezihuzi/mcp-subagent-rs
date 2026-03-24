use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use mcp_subagent::{
    config::{resolve_runtime_config, ConfigOverrides, RuntimeConfig},
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
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
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
}

impl From<InitPresetArg> for InitPreset {
    fn from(value: InitPresetArg) -> Self {
        match value {
            InitPresetArg::ClaudeOpusSupervisor => InitPreset::ClaudeOpusSupervisor,
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
        Commands::Doctor { agents_dir } => {
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
            doctor(cfg)
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
            force,
            json,
        } => {
            info!("starting command: init");
            init_command(preset, root_dir, force, json)
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

fn doctor(cfg: RuntimeConfig) -> ExitCode {
    let report = build_doctor_report(cfg.agents_dirs, cfg.state_dir, &SystemProviderProber);
    println!("{}", render_doctor_report(&report));
    ExitCode::SUCCESS
}

fn init_command(
    preset: InitPresetArg,
    root_dir: Option<PathBuf>,
    force: bool,
    json: bool,
) -> ExitCode {
    let root = match root_dir {
        Some(path) => path,
        None => match std::env::current_dir() {
            Ok(path) => path,
            Err(err) => {
                eprintln!("init failed: unable to resolve current directory: {err}");
                return ExitCode::from(1);
            }
        },
    };
    match init_workspace(&root, preset.into(), force) {
        Ok(report) => {
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
    use std::fs;

    use clap::Parser;
    use tempfile::tempdir;

    use crate::{
        build_selected_file_inputs, ArtifactKindArg, Cli, Commands, InitPresetArg,
        RunAgentSelectedFileInput,
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
            "--force",
            "--json",
        ]);
        match cli.command {
            Commands::Init {
                preset,
                force,
                json,
                ..
            } => {
                assert!(matches!(preset, InitPresetArg::ClaudeOpusSupervisor));
                assert!(force);
                assert!(json);
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
