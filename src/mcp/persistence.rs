use std::{collections::HashMap, fs, io::Write, path::Path};

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

fn compiled_context_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("compiled-context.md")
}

fn events_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("events.jsonl")
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
    if matches!(
        record.status,
        crate::runtime::dispatcher::RunStatus::Succeeded
            | crate::runtime::dispatcher::RunStatus::Failed
            | crate::runtime::dispatcher::RunStatus::TimedOut
            | crate::runtime::dispatcher::RunStatus::Cancelled
    ) {
        let events = build_run_events(record);
        write_events_file_if_missing(&events_path(state_dir, handle_id), &events)?;
    }

    Ok(())
}

pub(crate) struct RuntimeEventInput<'a> {
    pub(crate) event: &'a str,
    pub(crate) state: &'a str,
    pub(crate) phase: &'a str,
    pub(crate) source: &'a str,
    pub(crate) message: &'a str,
    pub(crate) detail: Value,
}

pub(crate) fn append_run_event(
    state_dir: &Path,
    handle_id: &str,
    event: RuntimeEventInput<'_>,
) -> std::result::Result<(), ErrorData> {
    let run_directory = run_dir(state_dir, handle_id);
    fs::create_dir_all(&run_directory).map_err(|err| {
        ErrorData::internal_error(
            format!(
                "failed to create run directory {}: {err}",
                run_directory.display()
            ),
            None,
        )
    })?;

    let canonical_path = events_path(state_dir, handle_id);
    let seq = next_event_seq(&canonical_path)?;
    let timestamp = format_time(OffsetDateTime::now_utc());
    let line = RunEventRecord {
        seq: Some(seq),
        ts: Some(timestamp.clone()),
        level: Some("info".to_string()),
        state: Some(event.state.to_string()),
        phase: Some(event.phase.to_string()),
        source: Some(event.source.to_string()),
        message: Some(event.message.to_string()),
        event: event.event.to_string(),
        timestamp,
        detail: event.detail,
    };
    append_event_line(&canonical_path, &line)?;
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
        usage: persisted.usage,
        retry_classification: persisted.retry_classification,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    event: String,
    timestamp: String,
    detail: Value,
}

impl RunEventRecord {
    fn legacy(event: String, timestamp: String, detail: Value) -> Self {
        Self {
            seq: None,
            ts: None,
            level: None,
            state: None,
            phase: None,
            source: None,
            message: None,
            event,
            timestamp,
            detail,
        }
    }

}

fn build_run_events(record: &RunRecord) -> Vec<RunEventRecord> {
    let timestamp = format_time(record.updated_at);
    let mut events = Vec::new();

    if let Some(probe) = &record.probe_result {
        events.push(RunEventRecord::legacy(
            "probe".to_string(),
            timestamp.clone(),
            json!({
                "provider": probe.provider,
                "status": probe.status,
                "executable": probe.executable,
            }),
        ));
    }

    if let Some(request) = &record.request_snapshot {
        events.push(RunEventRecord::legacy(
            "gate".to_string(),
            timestamp.clone(),
            json!({
                "stage": request.stage,
                "plan_ref": request.plan_ref,
                "run_mode": request.run_mode,
            }),
        ));
    }

    if let Some(workspace) = &record.workspace {
        events.push(RunEventRecord::legacy(
            "workspace".to_string(),
            timestamp.clone(),
            json!({
                "mode": workspace.mode,
                "source_path": workspace.source_path,
                "workspace_path": workspace.workspace_path,
            }),
        ));
    }

    if let Some(memory) = &record.memory_resolution {
        events.push(RunEventRecord::legacy(
            "memory".to_string(),
            timestamp.clone(),
            json!({
                "project_memory_count": memory.project_memory_count,
                "additional_memory_count": memory.additional_memory_count,
                "native_passthrough_count": memory.native_passthrough_count,
                "project_memory_labels": memory.project_memory_labels,
                "additional_memory_labels": memory.additional_memory_labels,
                "native_passthrough_paths": memory.native_passthrough_paths,
            }),
        ));
    }

    if let Some(policy) = &record.execution_policy {
        events.push(RunEventRecord::legacy(
            "policy".to_string(),
            timestamp.clone(),
            json!({
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
        ));
    }

    if let Some(retry) = &record.retry_classification {
        events.push(RunEventRecord::legacy(
            "retry_classification".to_string(),
            timestamp.clone(),
            json!({
                "classification": retry.classification,
                "reason": retry.reason,
            }),
        ));
    }

    if let Some(summary) = &record.summary {
        events.push(RunEventRecord::legacy(
            "parse".to_string(),
            timestamp.clone(),
            json!({
                "parse_status": format!("{}", summary.parse_status),
                "verification_status": format!("{}", summary.summary.verification_status),
                "exit_code": summary.summary.exit_code,
            }),
        ));
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
        events.push(RunEventRecord::legacy(
            "cleanup".to_string(),
            timestamp,
            json!({
                "cleaned": cleaned,
                "status": record.status,
            }),
        ));
    }

    events
}

fn write_events_file_if_missing(
    path: &Path,
    events: &[RunEventRecord],
) -> std::result::Result<(), ErrorData> {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.len() > 0 {
            return Ok(());
        }
    }
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

fn append_event_line(path: &Path, event: &RunEventRecord) -> std::result::Result<(), ErrorData> {
    let line = serde_json::to_string(event)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| {
            ErrorData::internal_error(
                format!("failed to open events file {}: {err}", path.display()),
                None,
            )
        })?;
    file.write_all(line.as_bytes()).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to append events file {}: {err}", path.display()),
            None,
        )
    })?;
    file.write_all(b"\n").map_err(|err| {
        ErrorData::internal_error(
            format!(
                "failed to append newline in events file {}: {err}",
                path.display()
            ),
            None,
        )
    })?;
    Ok(())
}

fn next_event_seq(path: &Path) -> std::result::Result<u64, ErrorData> {
    if !path.exists() {
        return Ok(1);
    }
    let raw = fs::read_to_string(path).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to read events file {}: {err}", path.display()),
            None,
        )
    })?;
    let max_seq = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value.get("seq").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);
    Ok(max_seq + 1)
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::runtime::dispatcher::RunStatus;
    use serde_json::json;
    use tempfile::tempdir;
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};

    use super::{append_run_event, load_run_record_from_disk, RuntimeEventInput};

    #[test]
    fn loads_persisted_run_json_with_required_fields() {
        let temp = tempdir().expect("tempdir");
        let state_dir = temp.path().join("state");
        let handle_id = "run-record";
        let run_dir = state_dir.join("runs").join(handle_id);
        fs::create_dir_all(&run_dir).expect("create run dir");

        let created_at = OffsetDateTime::now_utc();
        let updated_at = created_at + time::Duration::seconds(1);
        let current = serde_json::json!({
            "status": "succeeded",
            "created_at": created_at.format(&Rfc3339).expect("created_at"),
            "updated_at": updated_at.format(&Rfc3339).expect("updated_at"),
            "status_history": ["received", "running", "succeeded"],
            "summary": null,
            "artifact_index": [],
            "request_snapshot": null,
            "spec_snapshot": null,
            "probe_result": null,
            "memory_resolution": null,
            "workspace": null,
            "compiled_context_markdown": null,
            "usage": null,
            "retry_classification": null,
            "execution_policy": null,
            "error_message": null,
            "task": "legacy task"
        });
        fs::write(
            run_dir.join("run.json"),
            serde_json::to_string_pretty(&current).expect("serialize run json"),
        )
        .expect("write run json");

        let loaded = load_run_record_from_disk(&state_dir, handle_id)
            .expect("load run")
            .expect("run exists");

        assert_eq!(loaded.status, RunStatus::Succeeded);
        assert_eq!(
            loaded.status_history,
            vec![
                RunStatus::Received,
                RunStatus::Running,
                RunStatus::Succeeded
            ]
        );
        assert_eq!(loaded.task, "legacy task");
        assert!(loaded.memory_resolution.is_none());
        assert!(loaded.compiled_context_markdown.is_none());
    }

    #[test]
    fn append_run_event_writes_jsonl_with_incrementing_seq() {
        let temp = tempdir().expect("tempdir");
        let state_dir = temp.path().join("state");
        let handle_id = "run-events";

        append_run_event(
            &state_dir,
            handle_id,
            RuntimeEventInput {
                event: "run.accepted",
                state: "accepted",
                phase: "accepted",
                source: "runtime",
                message: "accepted",
                detail: json!({"k":"v"}),
            },
        )
        .expect("append accepted");
        append_run_event(
            &state_dir,
            handle_id,
            RuntimeEventInput {
                event: "provider.heartbeat",
                state: "running",
                phase: "running",
                source: "runtime",
                message: "still alive",
                detail: json!({"elapsed_ms":100}),
            },
        )
        .expect("append heartbeat");

        let run_dir = state_dir.join("runs").join(handle_id);
        let jsonl = fs::read_to_string(run_dir.join("events.jsonl")).expect("read jsonl");
        assert_eq!(
            jsonl.lines().count(),
            2,
            "events.jsonl should contain appended lines"
        );
        assert!(!run_dir.join("events.ndjson").exists());

        let first: serde_json::Value =
            serde_json::from_str(jsonl.lines().next().expect("first line")).expect("parse first");
        let second: serde_json::Value =
            serde_json::from_str(jsonl.lines().nth(1).expect("second line")).expect("parse second");
        assert_eq!(first.get("seq").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(second.get("seq").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(
            second.get("event").and_then(|v| v.as_str()),
            Some("provider.heartbeat")
        );
    }
}
