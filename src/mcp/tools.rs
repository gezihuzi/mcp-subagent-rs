use std::{collections::HashMap, fs, time::Duration};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_router, ErrorData, Json,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    mcp::{
        archive::apply_archive_hook,
        artifacts::{
            build_runtime_artifacts, read_artifact_from_disk, run_root_dir,
            sanitize_relative_artifact_path,
        },
        dto::{
            AgentListing, AgentStatusOutput, CancelAgentOutput, GetRunResultInput,
            GetRunResultOutput, HandleInput, ListAgentsOutput, ListRunsInput, ListRunsOutput,
            ReadAgentArtifactInput, ReadAgentArtifactOutput, ReadRunLogsInput, ReadRunLogsOutput,
            RunAgentInput, RunAgentOutput, RunListingOutput, RunUsageOutput, RuntimePolicySummary,
            SpawnAgentOutput, WatchRunInput, WatchRunOutput,
        },
        helpers::{
            build_capability_notes, cancelled_summary, failed_summary, format_time,
            map_summary_output,
        },
        persistence::{load_run_record_from_disk, persist_run_record},
        review::apply_review_evidence_hook,
        server::{acquire_serialize_locks_from_state, McpSubagentServer},
        service::run_dispatch,
        state::{
            append_status_if_terminal, apply_execution_policy_outcome, build_probe_result_snapshot,
            build_run_request_snapshot, build_run_spec_snapshot, RunRecord,
        },
    },
    runtime::dispatcher::RunStatus,
    types::RunMode,
};

pub(crate) fn build_tool_router() -> ToolRouter<McpSubagentServer> {
    McpSubagentServer::tool_router()
}

fn is_terminal_status(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::TimedOut | RunStatus::Cancelled
    )
}

fn compute_duration_ms(created_at: OffsetDateTime, updated_at: OffsetDateTime) -> Option<u64> {
    if updated_at < created_at {
        return None;
    }
    let millis = (updated_at - created_at).whole_milliseconds();
    if millis < 0 {
        None
    } else {
        Some(millis as u64)
    }
}

fn estimate_tokens(bytes: Option<u64>) -> Option<u64> {
    bytes.map(|value| value.saturating_add(3) / 4)
}

fn infer_provider_exit_code(record: &RunRecord) -> Option<i32> {
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

fn read_text_artifact(record: &RunRecord, path: &str) -> Option<String> {
    record.artifacts.get(path).cloned()
}

fn build_usage_output(record: &RunRecord) -> RunUsageOutput {
    let started_at = Some(format_time(record.created_at));
    let finished_at = if is_terminal_status(&record.status) {
        Some(format_time(record.updated_at))
    } else {
        None
    };
    let estimated_prompt_bytes = record
        .compiled_context_markdown
        .as_ref()
        .map(|value| value.as_bytes().len() as u64);
    let stdout_bytes = read_text_artifact(record, "stdout.txt").map(|text| text.len() as u64);
    let stderr_bytes = read_text_artifact(record, "stderr.txt").map(|text| text.len() as u64);
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

    RunUsageOutput {
        started_at: started_at.clone(),
        finished_at,
        duration_ms: compute_duration_ms(record.created_at, record.updated_at),
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
                let capability_notes = build_capability_notes(&probe);
                let available = probe.is_available();
                AgentListing {
                    name: loaded.spec.core.name,
                    description: loaded.spec.core.description,
                    provider: provider.as_str().to_string(),
                    available,
                    runtime_policy: RuntimePolicySummary {
                        context_mode: format!("{}", runtime.context_mode),
                        working_dir_policy: format!("{}", runtime.working_dir_policy),
                        sandbox: format!("{}", runtime.sandbox),
                        timeout_secs: runtime.timeout_secs,
                    },
                    capability_notes,
                }
            })
            .collect();

        Ok(Json(ListAgentsOutput { agents }))
    }

    #[tool(description = "List run handles ordered by latest update time.")]
    pub async fn list_runs(
        &self,
        Parameters(input): Parameters<ListRunsInput>,
    ) -> std::result::Result<Json<ListRunsOutput>, ErrorData> {
        let mut run_map = {
            let runtime_state = self.runtime_state();
            let state = runtime_state.lock().await;
            state
                .runs
                .iter()
                .map(|(handle_id, record)| (handle_id.clone(), record.clone()))
                .collect::<HashMap<_, _>>()
        };

        let run_root = run_root_dir(self.state_dir());
        if run_root.exists() {
            let entries = fs::read_dir(&run_root).map_err(|err| {
                ErrorData::internal_error(
                    format!(
                        "failed to read runs directory {}: {err}",
                        run_root.display()
                    ),
                    None,
                )
            })?;
            for entry in entries {
                let entry = entry.map_err(|err| {
                    ErrorData::internal_error(
                        format!("failed to read run entry in {}: {err}", run_root.display()),
                        None,
                    )
                })?;
                let file_type = entry.file_type().map_err(|err| {
                    ErrorData::internal_error(
                        format!(
                            "failed to read run entry type {}: {err}",
                            entry.path().display()
                        ),
                        None,
                    )
                })?;
                if !file_type.is_dir() {
                    continue;
                }
                let handle_id = entry.file_name().to_string_lossy().to_string();
                if run_map.contains_key(&handle_id) {
                    continue;
                }
                if let Some(record) = load_run_record_from_disk(self.state_dir(), &handle_id)? {
                    run_map.insert(handle_id, record);
                }
            }
        }

        let mut rows = run_map.into_iter().collect::<Vec<_>>();
        rows.sort_by_key(|(_, record)| record.updated_at);
        rows.reverse();

        let limit = input.limit.unwrap_or(50).max(1);
        let runs = rows
            .into_iter()
            .take(limit)
            .map(|(handle_id, record)| RunListingOutput {
                handle_id,
                status: format!("{}", record.status),
                updated_at: format_time(record.updated_at),
                provider: record
                    .spec_snapshot
                    .as_ref()
                    .map(|spec| spec.provider.clone()),
                agent: record.spec_snapshot.as_ref().map(|spec| spec.name.clone()),
                task: record.task,
                duration_ms: compute_duration_ms(record.created_at, record.updated_at),
            })
            .collect();

        Ok(Json(ListRunsOutput { runs }))
    }

    #[tool(description = "Return normalized and native result for a run handle.")]
    pub async fn get_run_result(
        &self,
        Parameters(input): Parameters<GetRunResultInput>,
    ) -> std::result::Result<Json<GetRunResultOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let native_result = read_text_artifact(&record, "stdout.txt").or_else(|| {
            record
                .summary
                .as_ref()
                .and_then(|summary| summary.raw_fallback_text.clone())
        });
        let usage = build_usage_output(&record);

        Ok(Json(GetRunResultOutput {
            handle_id: input.handle_id,
            status: format!("{}", record.status),
            updated_at: format_time(record.updated_at),
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
                .map(|summary| format!("{}", summary.parse_status)),
            summary: record
                .summary
                .as_ref()
                .map(|summary| summary.summary.summary.clone()),
            native_result,
            normalized_result: record.summary.as_ref().map(map_summary_output),
            provider_exit_code: usage.provider_exit_code,
            retries: usage.retries,
            usage,
            artifact_index: record.artifact_index,
        }))
    }

    #[tool(description = "Read stdout/stderr logs for a run handle.")]
    pub async fn read_run_logs(
        &self,
        Parameters(input): Parameters<ReadRunLogsInput>,
    ) -> std::result::Result<Json<ReadRunLogsOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let stream = input.stream.unwrap_or_else(|| "both".to_string());
        let (stdout_enabled, stderr_enabled) = match stream.as_str() {
            "stdout" => (true, false),
            "stderr" => (false, true),
            "both" => (true, true),
            _ => {
                return Err(ErrorData::invalid_params(
                    format!("invalid stream `{}`; expected stdout|stderr|both", stream),
                    None,
                ))
            }
        };

        let stdout = if stdout_enabled {
            read_text_artifact(&record, "stdout.txt").or(read_artifact_from_disk(
                self.state_dir(),
                &input.handle_id,
                "stdout.txt",
            )?)
        } else {
            None
        };
        let stderr = if stderr_enabled {
            read_text_artifact(&record, "stderr.txt").or(read_artifact_from_disk(
                self.state_dir(),
                &input.handle_id,
                "stderr.txt",
            )?)
        } else {
            None
        };

        Ok(Json(ReadRunLogsOutput {
            handle_id: input.handle_id,
            stdout,
            stderr,
        }))
    }

    #[tool(description = "Wait for a run to finish and return final status.")]
    pub async fn watch_run(
        &self,
        Parameters(input): Parameters<WatchRunInput>,
    ) -> std::result::Result<Json<WatchRunOutput>, ErrorData> {
        let interval_ms = input.interval_ms.unwrap_or(500).max(50);
        let timeout_secs = input.timeout_secs;
        let started = std::time::Instant::now();
        loop {
            let record = self.get_or_load_run_record(&input.handle_id).await?;
            if is_terminal_status(&record.status) {
                return Ok(Json(WatchRunOutput {
                    handle_id: input.handle_id,
                    status: format!("{}", record.status),
                    updated_at: format_time(record.updated_at),
                    error_message: record.error_message,
                    terminal: true,
                    timed_out: false,
                }));
            }
            if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
                return Ok(Json(WatchRunOutput {
                    handle_id: input.handle_id,
                    status: format!("{}", record.status),
                    updated_at: format_time(record.updated_at),
                    error_message: record.error_message,
                    terminal: false,
                    timed_out: true,
                }));
            }
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    #[tool(description = "Run an agent synchronously and return structured summary.")]
    pub async fn run_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<RunAgentOutput>, ErrorData> {
        let (loaded, request, probe_result, execution_policy) =
            self.prepare_run(input, RunMode::Sync)?;
        let request_snapshot = build_run_request_snapshot(&request);
        let spec_snapshot = build_run_spec_snapshot(&loaded.spec);
        let probe_snapshot = build_probe_result_snapshot(&probe_result);
        let run_created_at = OffsetDateTime::now_utc();
        let handle_id = Uuid::now_v7().to_string();
        let lock_keys = self.conflict_lock_keys(&loaded.spec, &request)?;
        let _serialize_guards = self.acquire_serialize_locks(lock_keys.clone()).await;
        let dispatch = run_dispatch(
            &loaded.spec,
            &request,
            &handle_id,
            self.state_dir(),
            lock_keys.clone(),
        )
        .await?;
        let crate::mcp::service::DispatchEnvelope {
            result,
            workspace,
            memory_resolution,
            _workspace_cleanup: workspace_cleanup,
        } = dispatch;

        let (artifact_index, artifacts) = build_runtime_artifacts(
            &result.summary,
            &result.stdout,
            &result.stderr,
            Some(&workspace.workspace_path),
        );
        let mut execution_policy = Some(execution_policy);
        apply_execution_policy_outcome(
            &mut execution_policy,
            result.metadata.attempts_used,
            result.metadata.retry_attempts,
        );
        let mut artifact_index = artifact_index;
        let mut artifacts = artifacts;
        apply_review_evidence_hook(
            &loaded.spec,
            &request,
            &result.summary,
            &mut artifact_index,
            &mut artifacts,
        );
        apply_archive_hook(
            &loaded.spec,
            &request,
            &result.metadata.status,
            &handle_id,
            &workspace,
            &result.summary,
            &mut crate::mcp::archive::ArtifactCollector {
                index: &mut artifact_index,
                data: &mut artifacts,
            },
        );
        let output = RunAgentOutput {
            handle_id: handle_id.clone(),
            status: format!("{}", result.metadata.status),
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
            memory_resolution: Some(memory_resolution),
            workspace: Some(workspace),
            compiled_context_markdown: Some(result.compiled_context_markdown),
            execution_policy,
        };
        drop(workspace_cleanup);
        self.upsert_and_persist_run(&handle_id, record).await?;

        Ok(Json(output))
    }

    #[tool(description = "Spawn an agent asynchronously and return handle_id immediately.")]
    pub async fn spawn_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<SpawnAgentOutput>, ErrorData> {
        let (loaded, request, probe_result, execution_policy) =
            self.prepare_run(input, RunMode::Async)?;
        let handle_id = Uuid::now_v7().to_string();
        let running_record = RunRecord::running(
            request.task.clone(),
            Some(build_run_request_snapshot(&request)),
            Some(build_run_spec_snapshot(&loaded.spec)),
            Some(build_probe_result_snapshot(&probe_result)),
            Some(execution_policy),
        );
        let lock_keys = self.conflict_lock_keys(&loaded.spec, &request)?;

        self.upsert_and_persist_run(&handle_id, running_record)
            .await?;

        let state = self.runtime_state();
        let state_dir = self.state_dir().to_path_buf();
        let task_handle_id = handle_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let _serialize_guards =
                acquire_serialize_locks_from_state(&state, lock_keys.clone()).await;
            let dispatch = run_dispatch(
                &loaded.spec,
                &request,
                &task_handle_id,
                &state_dir,
                lock_keys.clone(),
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
                        memory_resolution,
                        _workspace_cleanup: workspace_cleanup,
                    } = dispatch;
                    let (artifact_index, artifacts) = build_runtime_artifacts(
                        &dispatch_result.summary,
                        &dispatch_result.stdout,
                        &dispatch_result.stderr,
                        Some(&workspace.workspace_path),
                    );
                    let mut artifact_index = artifact_index;
                    let mut artifacts = artifacts;
                    apply_execution_policy_outcome(
                        &mut record.execution_policy,
                        dispatch_result.metadata.attempts_used,
                        dispatch_result.metadata.retry_attempts,
                    );
                    apply_review_evidence_hook(
                        &loaded.spec,
                        &request,
                        &dispatch_result.summary,
                        &mut artifact_index,
                        &mut artifacts,
                    );
                    apply_archive_hook(
                        &loaded.spec,
                        &request,
                        &dispatch_result.metadata.status,
                        &task_handle_id,
                        &workspace,
                        &dispatch_result.summary,
                        &mut crate::mcp::archive::ArtifactCollector {
                            index: &mut artifact_index,
                            data: &mut artifacts,
                        },
                    );
                    record.status = dispatch_result.metadata.status;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.status_history = dispatch_result.metadata.status_history;
                    record.error_message = dispatch_result.metadata.error_message;
                    record.summary = Some(dispatch_result.summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.memory_resolution = Some(memory_resolution);
                    record.workspace = Some(workspace);
                    record.compiled_context_markdown =
                        Some(dispatch_result.compiled_context_markdown);
                    drop(workspace_cleanup);
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
                    record.memory_resolution = None;
                    record.workspace = None;
                    record.compiled_context_markdown = None;
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
            status: format!("{}", RunStatus::Running),
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
            status: format!("{}", record.status),
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
                status: format!("{}", existing_status),
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
            status: format!("{}", RunStatus::Cancelled),
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
