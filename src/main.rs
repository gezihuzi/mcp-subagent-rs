use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use mcp_subagent::{
    config::{resolve_runtime_config, ConfigOverrides, RuntimeConfig},
    doctor::{build_doctor_report, render_doctor_report},
    mcp::server::McpSubagentServer,
    probe::SystemProviderProber,
    spec::registry::load_agent_specs_from_dirs,
};

#[derive(Debug, Parser)]
#[command(name = "mcp-subagent", version, about = "MCP subagent runtime")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long = "agents-dir", global = true)]
    agents_dirs: Vec<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,

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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let config_path = cli.config.clone();
    let global_agents_dirs = cli.agents_dirs.clone();
    let state_dir = cli.state_dir.clone();

    match cli.command {
        Commands::Mcp { agents_dir } => {
            let cfg = match resolve_cli_config(
                config_path.clone(),
                state_dir.clone(),
                global_agents_dirs.clone(),
                agents_dir,
            ) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!("failed to resolve config: {err}");
                    return ExitCode::from(2);
                }
            };
            run_mcp_server(cfg).await
        }
        Commands::Doctor { agents_dir } => {
            let cfg = match resolve_cli_config(
                config_path.clone(),
                state_dir.clone(),
                global_agents_dirs.clone(),
                agents_dir,
            ) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!("failed to resolve config: {err}");
                    return ExitCode::from(2);
                }
            };
            doctor(cfg)
        }
        Commands::Validate { agents_dir } => {
            let cfg =
                match resolve_cli_config(config_path, state_dir, global_agents_dirs, agents_dir) {
                    Ok(cfg) => cfg,
                    Err(err) => {
                        eprintln!("failed to resolve config: {err}");
                        return ExitCode::from(2);
                    }
                };
            validate_specs(cfg)
        }
    }
}

fn resolve_cli_config(
    config_path: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    mut agents_dirs: Vec<PathBuf>,
    command_agents_dir: Option<PathBuf>,
) -> mcp_subagent::error::Result<RuntimeConfig> {
    if let Some(dir) = command_agents_dir {
        agents_dirs = vec![dir];
    }

    resolve_runtime_config(ConfigOverrides {
        config_path,
        agents_dirs,
        state_dir,
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
            println!("validated {} agent specs", loaded.len());
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
