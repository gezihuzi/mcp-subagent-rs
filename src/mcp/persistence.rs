use std::{collections::HashMap, fs, io::Write, path::Path};

use rmcp::ErrorData;
use serde::Serialize;
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::mcp::{
    artifacts::{run_artifacts_dir, run_dir, sanitize_relative_artifact_path},
    state::{PersistedRun, RunRecord},
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

    let persisted = PersistedRun::from(record);
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
    let line = RunEventRecord::runtime(event, seq, timestamp);
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
    let persisted: PersistedRun = serde_json::from_str(&raw)
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

    let status = persisted.state.status.clone();
    let updated_at = persisted.state.updated_at;

    Ok(Some(RunRecord {
        status,
        created_at: persisted.state.created_at,
        updated_at,
        status_history: persisted.state.status_history,
        outcome: persisted.outcome,
        artifact_index: persisted.artifact_index,
        artifacts,
        error_message: persisted.state.error_message,
        task_spec: persisted.task_spec,
        hints: persisted.hints,
        spec_snapshot: persisted.spec_snapshot,
        probe_result: persisted.state.probe_result,
        memory_resolution: persisted.state.memory_resolution,
        workspace: persisted.state.workspace,
        compiled_context_markdown: persisted.state.compiled_context_markdown,
        usage: persisted.state.usage,
        policy: persisted.state.policy,
        permission_request: persisted.state.permission_request,
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
    fn runtime(event: RuntimeEventInput<'_>, seq: u64, timestamp: String) -> Self {
        Self {
            seq: Some(seq),
            level: Some("info".to_string()),
            state: Some(event.state.to_string()),
            phase: Some(event.phase.to_string()),
            source: Some(event.source.to_string()),
            message: Some(event.message.to_string()),
            event: event.event.to_string(),
            timestamp,
            detail: event.detail,
        }
    }
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

    use crate::runtime::dispatcher::RunPhase;
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
            "task_spec": {
                "task": "sample task",
                "task_brief": null,
                "acceptance_criteria": [],
                "selected_files": [],
                "working_dir": "."
            },
            "hints": {
                "stage": null,
                "plan_ref": null,
                "parent_summary": null,
                "run_mode": "sync"
            },
            "state": {
                "status": "succeeded",
                "created_at": created_at.format(&Rfc3339).expect("created_at"),
                "updated_at": updated_at.format(&Rfc3339).expect("updated_at"),
                "status_history": ["received", "running", "succeeded"],
                "error_message": null,
                "probe_result": null,
                "memory_resolution": null,
                "workspace": null,
                "compiled_context_markdown": null,
                "usage": null,
                "policy": null
            },
            "outcome": null,
            "artifact_index": [],
            "spec_snapshot": null,
        });
        fs::write(
            run_dir.join("run.json"),
            serde_json::to_string_pretty(&current).expect("serialize run json"),
        )
        .expect("write run json");

        let loaded = load_run_record_from_disk(&state_dir, handle_id)
            .expect("load run")
            .expect("run exists");

        assert_eq!(loaded.status, RunPhase::Succeeded);
        assert_eq!(
            loaded.status_history,
            vec![RunPhase::Received, RunPhase::Running, RunPhase::Succeeded]
        );
        assert_eq!(loaded.task_spec.task, "sample task");
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
