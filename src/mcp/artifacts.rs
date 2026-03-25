use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

use rmcp::ErrorData;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    mcp::dto::ArtifactOutput,
    runtime::summary::{ArtifactKind, ArtifactRef, SummaryEnvelope},
};

pub(crate) fn run_root_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("runs")
}

pub(crate) fn run_dir(state_dir: &Path, handle_id: &str) -> PathBuf {
    run_root_dir(state_dir).join(handle_id)
}

pub(crate) fn run_artifacts_dir(state_dir: &Path, handle_id: &str) -> PathBuf {
    run_dir(state_dir, handle_id).join("artifacts")
}

pub(crate) fn sanitize_relative_artifact_path(path: &str) -> Option<PathBuf> {
    let original = Path::new(path);
    if path.is_empty() || original.is_absolute() {
        return None;
    }

    let mut sanitized = PathBuf::new();
    for component in original.components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            _ => return None,
        }
    }
    if sanitized.as_os_str().is_empty() {
        return None;
    }
    Some(sanitized)
}

pub(crate) fn read_artifact_from_disk(
    state_dir: &Path,
    handle_id: &str,
    artifact_path: &str,
) -> std::result::Result<Option<String>, ErrorData> {
    let rel_path = sanitize_relative_artifact_path(artifact_path).ok_or_else(|| {
        ErrorData::invalid_params(format!("invalid artifact path: {artifact_path}"), None)
    })?;
    let full_path = run_artifacts_dir(state_dir, handle_id).join(rel_path);
    if !full_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&full_path).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to read artifact {}: {err}", full_path.display()),
            None,
        )
    })?;
    Ok(Some(content))
}

pub(crate) fn build_runtime_artifacts(
    summary: &SummaryEnvelope,
    stdout: &str,
    stderr: &str,
    workspace_root: Option<&Path>,
) -> (Vec<ArtifactOutput>, HashMap<String, String>) {
    let created_at = now_rfc3339();
    let mut index = Vec::new();
    let mut payloads = HashMap::new();

    if let Some(root) = workspace_root {
        for artifact in &summary.summary.artifacts {
            let Some(content) = read_declared_artifact_content(root, &artifact.path) else {
                continue;
            };
            let artifact_path = artifact.path.display().to_string();
            index.push(map_artifact_output(artifact, &created_at));
            payloads.insert(artifact_path, content);
        }
    }

    if let Ok(summary_json) = serde_json::to_string_pretty(summary) {
        index.push(ArtifactOutput {
            path: "summary.json".to_string(),
            kind: format!("{}", ArtifactKind::SummaryJson),
            description: "Structured summary JSON".to_string(),
            media_type: Some("application/json".to_string()),
            producer: Some("runtime".to_string()),
            created_at: Some(created_at.clone()),
        });
        payloads.insert("summary.json".to_string(), summary_json);
    }

    if let Some(raw_text) = &summary.raw_fallback_text {
        if !raw_text.trim().is_empty() {
            index.push(ArtifactOutput {
                path: "summary.raw.txt".to_string(),
                kind: format!("{}", ArtifactKind::StderrText),
                description: "Raw summary fallback text".to_string(),
                media_type: Some("text/plain".to_string()),
                producer: Some("runtime".to_string()),
                created_at: Some(created_at.clone()),
            });
            payloads.insert("summary.raw.txt".to_string(), raw_text.clone());
        }
    }

    if !stdout.is_empty() {
        index.push(ArtifactOutput {
            path: "stdout.txt".to_string(),
            kind: format!("{}", ArtifactKind::StdoutText),
            description: "Captured stdout".to_string(),
            media_type: Some("text/plain".to_string()),
            producer: Some("runtime".to_string()),
            created_at: Some(created_at.clone()),
        });
        payloads.insert("stdout.txt".to_string(), stdout.to_string());
    }

    if !stderr.is_empty() {
        index.push(ArtifactOutput {
            path: "stderr.txt".to_string(),
            kind: format!("{}", ArtifactKind::StderrText),
            description: "Captured stderr".to_string(),
            media_type: Some("text/plain".to_string()),
            producer: Some("runtime".to_string()),
            created_at: Some(created_at),
        });
        payloads.insert("stderr.txt".to_string(), stderr.to_string());
    }

    (index, payloads)
}

fn map_artifact_output(artifact: &ArtifactRef, created_at: &str) -> ArtifactOutput {
    ArtifactOutput {
        path: artifact.path.display().to_string(),
        kind: format!("{}", artifact.kind),
        description: artifact.description.clone(),
        media_type: artifact.media_type.clone(),
        producer: Some("agent".to_string()),
        created_at: Some(created_at.to_string()),
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_string())
}

fn read_declared_artifact_content(workspace_root: &Path, artifact_path: &Path) -> Option<String> {
    let resolved = resolve_artifact_path_in_workspace(workspace_root, artifact_path)?;
    if !resolved.is_file() {
        return None;
    }
    fs::read_to_string(&resolved).ok()
}

fn resolve_artifact_path_in_workspace(
    workspace_root: &Path,
    artifact_path: &Path,
) -> Option<PathBuf> {
    if artifact_path.as_os_str().is_empty() {
        return None;
    }

    let combined = if artifact_path.is_absolute() {
        artifact_path.to_path_buf()
    } else {
        workspace_root.join(artifact_path)
    };
    let canonical = combined.canonicalize().ok()?;
    let canonical_workspace = workspace_root.canonicalize().ok()?;
    if !canonical.starts_with(&canonical_workspace) {
        return None;
    }
    Some(canonical)
}
