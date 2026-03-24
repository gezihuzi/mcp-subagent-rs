use std::{collections::HashMap, fs, path::Path};

use rmcp::ErrorData;

use crate::mcp::{
    artifacts::{run_artifacts_dir, run_dir, sanitize_relative_artifact_path},
    state::{PersistedRunRecord, RunRecord},
};

fn run_meta_path(state_dir: &Path, handle_id: &str) -> std::path::PathBuf {
    run_dir(state_dir, handle_id).join("run.json")
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
        workspace: persisted.workspace,
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
