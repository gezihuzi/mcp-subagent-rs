use std::{env, path::PathBuf, process::ExitCode};

use mcp_subagent::{mcp::server::McpSubagentServer, spec::registry::load_agent_specs_from_dirs};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--mcp") => run_mcp_server(args.get(2).map(PathBuf::from)).await,
        Some("validate") => validate_specs(args.get(2).map(PathBuf::from)),
        _ => {
            eprintln!("Usage:");
            eprintln!("  mcp-subagent --mcp [agents_dir]");
            eprintln!("  mcp-subagent validate [agents_dir]");
            ExitCode::from(2)
        }
    }
}

async fn run_mcp_server(dir: Option<PathBuf>) -> ExitCode {
    let dirs = vec![dir.unwrap_or_else(|| PathBuf::from("./agents"))];
    let server = McpSubagentServer::new(dirs);
    match server.serve_stdio().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("failed to run mcp server: {err}");
            ExitCode::from(1)
        }
    }
}

fn validate_specs(dir: Option<PathBuf>) -> ExitCode {
    let dirs = vec![dir.unwrap_or_else(|| PathBuf::from("./agents"))];
    match load_agent_specs_from_dirs(&dirs) {
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
