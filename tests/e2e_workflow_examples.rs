use std::{fs, path::PathBuf};

use mcp_subagent::mcp::{
    dto::{RunAgentInput, RunAgentSelectedFileInput},
    server::McpSubagentServer,
};
use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples_agents_dir() -> PathBuf {
    repo_root().join("examples/agents")
}

fn workflow_workspace_dir() -> PathBuf {
    repo_root().join("examples/workspaces/workflow_demo")
}

#[tokio::test]
async fn example_workflow_build_stage_with_plan_ref_succeeds() {
    let temp = tempdir().expect("tempdir");
    let server = McpSubagentServer::new_with_state_dir(
        vec![examples_agents_dir()],
        temp.path().join("state"),
    );

    let out = server
        .run_agent(Parameters(RunAgentInput {
            agent_name: "workflow_builder".to_string(),
            task: "build workflow demo".to_string(),
            task_brief: Some("execute build stage".to_string()),
            parent_summary: None,
            selected_files: vec![RunAgentSelectedFileInput {
                path: "src/lib.rs".to_string(),
                rationale: Some("core implementation target".to_string()),
                content: None,
            }],
            stage: Some("Build".to_string()),
            plan_ref: Some("PLAN.md".to_string()),
            working_dir: Some(workflow_workspace_dir().display().to_string()),
        }))
        .await
        .expect("run should succeed")
        .0;

    assert_eq!(out.status, "succeeded");
    assert_eq!(out.structured_summary.parse_status, "Validated");
}

#[tokio::test]
async fn example_workflow_build_stage_without_plan_fails_gate() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("no-plan-workspace");
    fs::create_dir_all(workspace.join("src")).expect("create workspace");
    fs::write(workspace.join("src/lib.rs"), "pub fn x() -> i32 { 1 }\n").expect("write source");
    let server = McpSubagentServer::new_with_state_dir(
        vec![examples_agents_dir()],
        temp.path().join("state"),
    );

    let err = match server
        .run_agent(Parameters(RunAgentInput {
            agent_name: "workflow_builder".to_string(),
            task: "build without plan".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: vec![RunAgentSelectedFileInput {
                path: "src/lib.rs".to_string(),
                rationale: None,
                content: None,
            }],
            stage: Some("Build".to_string()),
            plan_ref: None,
            working_dir: Some(workspace.display().to_string()),
        }))
        .await
    {
        Ok(_) => panic!("run should fail when plan gate is hit"),
        Err(err) => err,
    };

    assert!(
        err.message.as_ref().contains("plan required"),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn example_workflow_depth_limit_rejects_nested_runtime_run() {
    let temp = tempdir().expect("tempdir");
    let server = McpSubagentServer::new_with_state_dir(
        vec![examples_agents_dir()],
        temp.path().join("state"),
    );

    let err = match server
        .run_agent(Parameters(RunAgentInput {
            agent_name: "workflow_builder".to_string(),
            task: "nested build".to_string(),
            task_brief: None,
            parent_summary: Some("runtime_depth=2 upstream delegated execution".to_string()),
            selected_files: vec![RunAgentSelectedFileInput {
                path: "src/lib.rs".to_string(),
                rationale: None,
                content: None,
            }],
            stage: Some("Build".to_string()),
            plan_ref: Some("PLAN.md".to_string()),
            working_dir: Some(workflow_workspace_dir().display().to_string()),
        }))
        .await
    {
        Ok(_) => panic!("run should fail when max runtime depth is exceeded"),
        Err(err) => err,
    };

    assert!(
        err.message.as_ref().contains("runtime depth exceeded"),
        "unexpected error: {err:?}"
    );
}
