use std::{collections::HashMap, fs, path::Path};

use rmcp::ErrorData;
use serde::Serialize;
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::mcp::{
    artifacts::{run_artifacts_dir, run_dir, sanitize_relative_artifact_path},
    state::{PersistedRunRecord, RunRecord},
};

fn run_meta_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("run.json")
}

fn status_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("status.json")
}

fn request_snapshot_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("request.json")
}

fn resolved_spec_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("resolved-spec.json")
}

fn workspace_meta_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("workspace.meta.json")
}

fn summary_root_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("summary.json")
}

fn summary_raw_root_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("summary.raw.txt")
}

fn compiled_context_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("compiled-context.md")
}

fn events_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("events.ndjson")
}

fn artifact_index_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_artifacts_dir(state_dir, handle_id).join("index.json")
}

pub(crate) fn persist_run_record(
    state_dir: &Path,
    handle_id: &str,
    record: &RunRecord,
) -> std::result::Result<(), ErrorData> {
    let run_directory = run_dir(state_dir, handle_id);
    let temp_dir = run_directory.join("temp");
    let artifacts_dir = run_artifacts_dir(state_dir, handle_id);
    fs::create_dir_all(&artifacts_dir).map_err(|err| {
        ErrorData::internal_error(
            format!(
                "failed to create run directory {}: {err}",
                run_directory.display()
            ),
            None,
        )
    })?;
    fs::create_dir_all(&temp_dir).map_err(|err| {
        ErrorData::internal_error(
            format!(
                "failed to create run temp directory {}: {err}",
                temp_dir.display()
            ),
            None,
        )
    })?;

    let persisted = PersistedRunRecord::from(record);
    let meta_json = serde_json::to_string_pretty(&persisted)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    fs::write(run_meta_path(state_dir, handle_id), meta_json)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    write_json_file(
        &status_path(state_dir, handle_id),
        &json!({
            "status": record.status,
            "updated_at": format_time(record.updated_at),
            "status_history": record.status_history,
            "error_message": record.error_message,
        }),
    )?;
    write_optional_json_file(
        &request_snapshot_path(state_dir, handle_id),
        record.request_snapshot.as_ref(),
    )?;
    write_optional_json_file(
        &resolved_spec_path(state_dir, handle_id),
        record.spec_snapshot.as_ref(),
    )?;
    write_optional_json_file(
        &workspace_meta_path(state_dir, handle_id),
        record.workspace.as_ref(),
    )?;

    if let Some(summary) = &record.summary {
        write_json_file(&summary_root_path(state_dir, handle_id), summary)?;
        fs::write(
            summary_raw_root_path(state_dir, handle_id),
            summary.raw_fallback_text.clone().unwrap_or_default(),
        )
        .map_err(|err| {
            ErrorData::internal_error(
                format!("failed to write summary raw file for {handle_id}: {err}"),
                None,
            )
        })?;
    }

    fs::write(
        compiled_context_path(state_dir, handle_id),
        record
            .compiled_context_markdown
            .clone()
            .unwrap_or_else(|| "[compiled context unavailable]".to_string()),
    )
    .map_err(|err| {
        ErrorData::internal_error(
            format!("failed to write compiled context for {handle_id}: {err}"),
            None,
        )
    })?;

    for (artifact_path, content) in &record.artifacts {
        let rel_path = sanitize_relative_artifact_path(artifact_path).ok_or_else(|| {
            ErrorData::invalid_params(
                format!("invalid artifact path for persistence: {artifact_path}"),
                None,
            )
        })?;
        let full_path = artifacts_dir.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                ErrorData::internal_error(
                    format!(
                        "failed to create artifact parent directory {}: {err}",
                        parent.display()
                    ),
                    None,
                )
            })?;
        }
        fs::write(&full_path, content).map_err(|err| {
            ErrorData::internal_error(
                format!("failed to write artifact {}: {err}", full_path.display()),
                None,
            )
        })?;
    }

    write_json_file(
        &artifact_index_path(state_dir, handle_id),
        &record.artifact_index,
    )?;

    let stdout_log_content = record
        .artifacts
        .get("stdout.txt")
        .map(String::as_str)
        .unwrap_or("");
    let stderr_log_content = record
        .artifacts
        .get("stderr.txt")
        .map(String::as_str)
        .unwrap_or("");
    write_run_log_file(&run_directory.join("stdout.log"), stdout_log_content)?;
    write_run_log_file(&run_directory.join("stderr.log"), stderr_log_content)?;
    write_events_file(&events_path(state_dir, handle_id), build_run_events(record))?;

    Ok(())
}

pub(crate) fn load_run_record_from_disk(
    state_dir: &Path,
    handle_id: &str,
) -> std::result::Result<Option<RunRecord>, ErrorData> {
    let meta_path = run_meta_path(state_dir, handle_id);
    if !meta_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&meta_path).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to read run metadata {}: {err}", meta_path.display()),
            None,
        )
    })?;
    let persisted: PersistedRunRecord = serde_json::from_str(&raw)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    let mut artifacts = HashMap::new();
    for artifact in &persisted.artifact_index {
        let Some(rel_path) = sanitize_relative_artifact_path(&artifact.path) else {
            return Err(ErrorData::invalid_params(
                format!(
                    "invalid artifact path in persisted metadata for {handle_id}: {}",
                    artifact.path
                ),
                None,
            ));
        };

        let path = run_artifacts_dir(state_dir, handle_id).join(rel_path);
        if !path.exists() {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|err| {
            ErrorData::internal_error(
                format!("failed to read artifact {}: {err}", path.display()),
                None,
            )
        })?;
        artifacts.insert(artifact.path.clone(), content);
    }

    let status = persisted.status.clone();
    let updated_at = persisted.updated_at;
    let status_history = if persisted.status_history.is_empty() {
        vec![status.clone()]
    } else {
        persisted.status_history
    };

    Ok(Some(RunRecord {
        status,
        created_at: persisted.created_at.unwrap_or(updated_at),
        updated_at,
        status_history,
        summary: persisted.summary,
        artifact_index: persisted.artifact_index,
        artifacts,
        error_message: persisted.error_message,
        task: persisted.task,
        request_snapshot: persisted.request_snapshot,
        spec_snapshot: persisted.spec_snapshot,
        probe_result: persisted.probe_result,
        memory_resolution: persisted.memory_resolution,
        workspace: persisted.workspace,
        compiled_context_markdown: persisted.compiled_context_markdown,
        execution_policy: persisted.execution_policy,
    }))
}

fn write_run_log_file(path: &Path, content: &str) -> std::result::Result<(), ErrorData> {
    fs::write(path, content).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to write run log {}: {err}", path.display()),
            None,
        )
    })
}

#[derive(Debug, Serialize)]
struct RunEventRecord {
    event: String,
    timestamp: String,
    detail: Value,
}

fn build_run_events(record: &RunRecord) -> Vec<RunEventRecord> {
    let timestamp = format_time(record.updated_at);
    let mut events = Vec::new();

    if let Some(probe) = &record.probe_result {
        events.push(RunEventRecord {
            event: "probe".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "provider": probe.provider,
                "status": probe.status,
                "executable": probe.executable,
            }),
        });
    }

    if let Some(request) = &record.request_snapshot {
        events.push(RunEventRecord {
            event: "gate".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "stage": request.stage,
                "plan_ref": request.plan_ref,
                "run_mode": request.run_mode,
            }),
        });
    }

    if let Some(workspace) = &record.workspace {
        events.push(RunEventRecord {
            event: "workspace".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "mode": workspace.mode,
                "source_path": workspace.source_path,
                "workspace_path": workspace.workspace_path,
            }),
        });
    }

    if let Some(memory) = &record.memory_resolution {
        events.push(RunEventRecord {
            event: "memory".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "project_memory_count": memory.project_memory_count,
                "additional_memory_count": memory.additional_memory_count,
                "native_passthrough_count": memory.native_passthrough_count,
                "project_memory_labels": memory.project_memory_labels,
                "additional_memory_labels": memory.additional_memory_labels,
                "native_passthrough_paths": memory.native_passthrough_paths,
            }),
        });
    }

    if let Some(policy) = &record.execution_policy {
        events.push(RunEventRecord {
            event: "policy".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "requested_run_mode": policy.requested_run_mode,
                "effective_run_mode": policy.effective_run_mode,
                "effective_run_mode_source": policy.effective_run_mode_source,
                "spawn_policy": policy.spawn_policy,
                "spawn_policy_source": policy.spawn_policy_source,
                "background_preference": policy.background_preference,
                "background_preference_source": policy.background_preference_source,
                "max_turns": policy.max_turns,
                "max_turns_source": policy.max_turns_source,
                "retry_max_attempts": policy.retry_max_attempts,
                "retry_backoff_secs": policy.retry_backoff_secs,
                "retry_policy_source": policy.retry_policy_source,
                "attempts_used": policy.attempts_used,
                "retries_used": policy.retries_used,
            }),
        });
    }

    if let Some(summary) = &record.summary {
        events.push(RunEventRecord {
            event: "parse".to_string(),
            timestamp: timestamp.clone(),
            detail: json!({
                "parse_status": format!("{:?}", summary.parse_status),
                "verification_status": format!("{:?}", summary.summary.verification_status),
                "exit_code": summary.summary.exit_code,
            }),
        });
    }

    if matches!(
        record.status,
        crate::runtime::dispatcher::RunStatus::Succeeded
            | crate::runtime::dispatcher::RunStatus::Failed
            | crate::runtime::dispatcher::RunStatus::TimedOut
            | crate::runtime::dispatcher::RunStatus::Cancelled
    ) {
        let cleaned = record
            .workspace
            .as_ref()
            .map(|workspace| !workspace.workspace_path.exists())
            .unwrap_or(true);
        events.push(RunEventRecord {
            event: "cleanup".to_string(),
            timestamp,
            detail: json!({
                "cleaned": cleaned,
                "status": record.status,
            }),
        });
    }

    events
}

fn write_events_file(
    path: &Path,
    events: Vec<RunEventRecord>,
) -> std::result::Result<(), ErrorData> {
    let mut lines = String::new();
    for event in events {
        let line = serde_json::to_string(&event)
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        lines.push_str(&line);
        lines.push('\n');
    }
    fs::write(path, lines).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to write events file {}: {err}", path.display()),
            None,
        )
    })
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> std::result::Result<(), ErrorData> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    fs::write(path, json).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to write file {}: {err}", path.display()),
            None,
        )
    })
}

fn write_optional_json_file<T: Serialize>(
    path: &Path,
    value: Option<&T>,
) -> std::result::Result<(), ErrorData> {
    if let Some(value) = value {
        write_json_file(path, value)
    } else {
        write_json_file(path, &json!(null))
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use time::OffsetDateTime;

    use crate::runtime::dispatcher::RunStatus;

    use super::load_run_record_from_disk;

    #[test]
    fn loads_legacy_run_json_without_new_fields() {
        let temp = tempdir().expect("tempdir");
        let state_dir = temp.path().join("state");
        let handle_id = "legacy-run";
        let run_dir = state_dir.join("runs").join(handle_id);
        fs::create_dir_all(&run_dir).expect("create run dir");

        let updated_at =
            serde_json::to_value(OffsetDateTime::now_utc()).expect("serialize timestamp");
        let legacy = serde_json::json!({
            "status": "SUCCEEDED",
            "updated_at": updated_at,
            "summary": null,
            "artifact_index": [],
            "error_message": null,
            "task": "legacy task"
        });
        fs::write(
            run_dir.join("run.json"),
            serde_json::to_string_pretty(&legacy).expect("serialize legacy json"),
        )
        .expect("write legacy run json");

        let loaded = load_run_record_from_disk(&state_dir, handle_id)
            .expect("load legacy run")
            .expect("legacy run exists");

        assert_eq!(loaded.status, RunStatus::Succeeded);
        assert_eq!(loaded.status_history, vec![RunStatus::Succeeded]);
        assert_eq!(loaded.task, "legacy task");
        assert!(loaded.memory_resolution.is_none());
        assert!(loaded.compiled_context_markdown.is_none());
    }
}
