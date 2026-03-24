use std::{collections::HashMap, time::Duration};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_router, ErrorData, Json,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    mcp::{
        artifacts::{
            build_runtime_artifacts, read_artifact_from_disk, sanitize_relative_artifact_path,
        },
        dto::{
            AgentListing, AgentStatusOutput, CancelAgentOutput, HandleInput, ListAgentsOutput,
            ReadAgentArtifactInput, ReadAgentArtifactOutput, RunAgentInput, RunAgentOutput,
            RuntimePolicySummary, SpawnAgentOutput,
        },
        persistence::persist_run_record,
        server::{
            acquire_serialize_lock_from_state, build_capability_notes, cancelled_summary,
            failed_summary, format_time, map_summary_output, McpSubagentServer,
        },
        service::run_dispatch,
        state::{
            append_status_if_terminal, build_probe_result_snapshot, build_run_request_snapshot,
            build_run_spec_snapshot, RunRecord,
        },
    },
    runtime::dispatcher::RunStatus,
};

pub(crate) fn build_tool_router() -> ToolRouter<McpSubagentServer> {
    McpSubagentServer::tool_router()
}

#[tool_router]
impl McpSubagentServer {
    #[tool(description = "List all available mcp-subagent agent specs.")]
    pub async fn list_agents(&self) -> std::result::Result<Json<ListAgentsOutput>, ErrorData> {
        let loaded = self.load_specs()?;
        let mut probe_cache = HashMap::new();
        let agents = loaded
            .into_iter()
            .map(|loaded| {
                let provider = loaded.spec.core.provider.clone();
                let runtime = loaded.spec.runtime;
                let probe = probe_cache
                    .entry(provider.clone())
                    .or_insert_with(|| self.probe_provider(&provider))
                    .clone();
                AgentListing {
                    name: loaded.spec.core.name,
                    description: loaded.spec.core.description,
                    provider: provider.as_str().to_string(),
                    available: probe.is_available(),
                    runtime_policy: RuntimePolicySummary {
                        context_mode: format!("{:?}", runtime.context_mode),
                        working_dir_policy: format!("{:?}", runtime.working_dir_policy),
                        sandbox: format!("{:?}", runtime.sandbox),
                        timeout_secs: runtime.timeout_secs,
                    },
                    capability_notes: build_capability_notes(&probe),
                }
            })
            .collect();

        Ok(Json(ListAgentsOutput { agents }))
    }

    #[tool(description = "Run an agent synchronously and return structured summary.")]
    pub async fn run_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<RunAgentOutput>, ErrorData> {
        let (loaded, request, probe_result) = self.prepare_run(input)?;
        let request_snapshot = build_run_request_snapshot(&request);
        let spec_snapshot = build_run_spec_snapshot(&loaded.spec);
        let probe_snapshot = build_probe_result_snapshot(&probe_result);
        let run_created_at = OffsetDateTime::now_utc();
        let handle_id = Uuid::now_v7().to_string();
        let lock_key = self.conflict_lock_key(&loaded.spec, &request)?;
        let _serialize_guard = self.acquire_serialize_lock(lock_key.clone()).await;
        let dispatch = run_dispatch(
            &loaded.spec,
            &request,
            &handle_id,
            self.state_dir(),
            lock_key.clone(),
        )
        .await?;
        let crate::mcp::service::DispatchEnvelope {
            result,
            workspace,
            _workspace_cleanup,
        } = dispatch;

        let (artifact_index, artifacts) = build_runtime_artifacts(
            &result.summary,
            &result.stdout,
            &result.stderr,
            Some(&workspace.workspace_path),
        );
        let output = RunAgentOutput {
            handle_id: handle_id.clone(),
            status: format!("{:?}", result.metadata.status),
            structured_summary: map_summary_output(&result.summary),
            artifact_index: artifact_index.clone(),
        };

        let record = RunRecord {
            status: result.metadata.status,
            created_at: run_created_at,
            updated_at: OffsetDateTime::now_utc(),
            status_history: result.metadata.status_history,
            summary: Some(result.summary),
            artifact_index,
            artifacts,
            error_message: result.metadata.error_message,
            task: request.task,
            request_snapshot: Some(request_snapshot),
            spec_snapshot: Some(spec_snapshot),
            probe_result: Some(probe_snapshot),
            workspace: Some(workspace),
        };
        self.upsert_and_persist_run(&handle_id, record).await?;

        Ok(Json(output))
    }

    #[tool(description = "Spawn an agent asynchronously and return handle_id immediately.")]
    pub async fn spawn_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<SpawnAgentOutput>, ErrorData> {
        let (loaded, request, probe_result) = self.prepare_run(input)?;
        let handle_id = Uuid::now_v7().to_string();
        let running_record = RunRecord::running(
            request.task.clone(),
            Some(build_run_request_snapshot(&request)),
            Some(build_run_spec_snapshot(&loaded.spec)),
            Some(build_probe_result_snapshot(&probe_result)),
        );
        let lock_key = self.conflict_lock_key(&loaded.spec, &request)?;

        self.upsert_and_persist_run(&handle_id, running_record)
            .await?;

        let state = self.runtime_state();
        let state_dir = self.state_dir().to_path_buf();
        let task_handle_id = handle_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let _serialize_guard =
                acquire_serialize_lock_from_state(&state, lock_key.clone()).await;
            let dispatch = run_dispatch(
                &loaded.spec,
                &request,
                &task_handle_id,
                &state_dir,
                lock_key.clone(),
            )
            .await;

            let mut guard = state.lock().await;
            guard.tasks.remove(&task_handle_id);
            let Some(record) = guard.runs.get_mut(&task_handle_id) else {
                return;
            };

            if matches!(record.status, RunStatus::Cancelled) {
                return;
            }

            match dispatch {
                Ok(dispatch) => {
                    let crate::mcp::service::DispatchEnvelope {
                        result: dispatch_result,
                        workspace,
                        _workspace_cleanup,
                    } = dispatch;
                    let (artifact_index, artifacts) = build_runtime_artifacts(
                        &dispatch_result.summary,
                        &dispatch_result.stdout,
                        &dispatch_result.stderr,
                        Some(&workspace.workspace_path),
                    );
                    record.status = dispatch_result.metadata.status;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.status_history = dispatch_result.metadata.status_history;
                    record.error_message = dispatch_result.metadata.error_message;
                    record.summary = Some(dispatch_result.summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.workspace = Some(workspace);
                }
                Err(err) => {
                    let summary = failed_summary(err.message.clone().into_owned());
                    let (artifact_index, artifacts) =
                        build_runtime_artifacts(&summary, "", "", None);
                    record.status = RunStatus::Failed;
                    record.updated_at = OffsetDateTime::now_utc();
                    append_status_if_terminal(&mut record.status_history, RunStatus::Failed);
                    record.error_message = Some(err.to_string());
                    record.summary = Some(summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.workspace = None;
                }
            }

            if let Err(err) = persist_run_record(&state_dir, &task_handle_id, record) {
                record.error_message = Some(format!("failed to persist run state: {err}"));
            }
        });

        {
            let runtime_state = self.runtime_state();
            let mut state = runtime_state.lock().await;
            state.tasks.insert(handle_id.clone(), task);
        }

        Ok(Json(SpawnAgentOutput {
            handle_id,
            status: format!("{:?}", RunStatus::Running),
        }))
    }

    #[tool(description = "Get current status for an async agent run.")]
    pub async fn get_agent_status(
        &self,
        Parameters(input): Parameters<HandleInput>,
    ) -> std::result::Result<Json<AgentStatusOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let structured_summary = record.summary.as_ref().map(map_summary_output);

        Ok(Json(AgentStatusOutput {
            handle_id: input.handle_id,
            status: format!("{:?}", record.status),
            updated_at: format_time(record.updated_at),
            error_message: record.error_message,
            structured_summary,
            artifact_index: record.artifact_index,
        }))
    }

    #[tool(description = "Cancel an async agent run if still in progress.")]
    pub async fn cancel_agent(
        &self,
        Parameters(input): Parameters<HandleInput>,
    ) -> std::result::Result<Json<CancelAgentOutput>, ErrorData> {
        let runtime_state = self.runtime_state();
        let mut state = runtime_state.lock().await;

        let existing_status = state
            .runs
            .get(&input.handle_id)
            .map(|run| run.status.clone())
            .ok_or_else(|| {
                ErrorData::resource_not_found(
                    format!("handle not found: {}", input.handle_id),
                    None,
                )
            })?;

        if matches!(
            existing_status,
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled | RunStatus::TimedOut
        ) {
            return Ok(Json(CancelAgentOutput {
                handle_id: input.handle_id,
                status: format!("{:?}", existing_status),
            }));
        }

        if let Some(task) = state.tasks.remove(&input.handle_id) {
            task.abort();
        }

        if let Some(record) = state.runs.get_mut(&input.handle_id) {
            record.status = RunStatus::Cancelled;
            record.updated_at = OffsetDateTime::now_utc();
            append_status_if_terminal(&mut record.status_history, RunStatus::Cancelled);
            record.error_message = Some("cancelled by user request".to_string());
            if record.summary.is_none() {
                let summary = cancelled_summary(record.task.clone());
                let (artifact_index, artifacts) = build_runtime_artifacts(&summary, "", "", None);
                record.summary = Some(summary);
                record.artifact_index = artifact_index;
                record.artifacts = artifacts;
            }
            persist_run_record(self.state_dir(), &input.handle_id, record)?;
        }

        Ok(Json(CancelAgentOutput {
            handle_id: input.handle_id,
            status: format!("{:?}", RunStatus::Cancelled),
        }))
    }

    #[tool(description = "Read a UTF-8 text artifact by run handle and path.")]
    pub async fn read_agent_artifact(
        &self,
        Parameters(input): Parameters<ReadAgentArtifactInput>,
    ) -> std::result::Result<Json<ReadAgentArtifactOutput>, ErrorData> {
        if sanitize_relative_artifact_path(&input.path).is_none() {
            return Err(ErrorData::invalid_params(
                format!("invalid artifact path: {}", input.path),
                None,
            ));
        }

        let mut run = self.get_or_load_run_record(&input.handle_id).await?;
        let content = if let Some(content) = run.artifacts.get(&input.path) {
            content.clone()
        } else {
            let content = read_artifact_from_disk(self.state_dir(), &input.handle_id, &input.path)?
                .ok_or_else(|| {
                    ErrorData::resource_not_found(
                        format!(
                            "artifact not found for handle {}: {}",
                            input.handle_id, input.path
                        ),
                        None,
                    )
                })?;
            run.artifacts.insert(input.path.clone(), content.clone());
            let runtime_state = self.runtime_state();
            let mut state = runtime_state.lock().await;
            if let Some(existing) = state.runs.get_mut(&input.handle_id) {
                existing
                    .artifacts
                    .insert(input.path.clone(), content.clone());
            }
            content
        };

        Ok(Json(ReadAgentArtifactOutput {
            handle_id: input.handle_id,
            path: input.path,
            content,
        }))
    }
}
