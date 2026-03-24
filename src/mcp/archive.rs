use std::{collections::HashMap, fs, path::Path};

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    mcp::{
        artifacts::sanitize_relative_artifact_path, dto::ArtifactOutput, state::WorkspaceRecord,
    },
    runtime::{
        dispatcher::RunStatus,
        summary::{ArtifactKind, SummaryEnvelope},
    },
    spec::AgentSpec,
    types::RunRequest,
};

const ARCHIVE_WARNING_ARTIFACT_PATH: &str = "archive/hook-warnings.txt";

pub(crate) fn apply_archive_hook(
    spec: &AgentSpec,
    request: &RunRequest,
    run_status: &RunStatus,
    handle_id: &str,
    workspace: &WorkspaceRecord,
    summary: &SummaryEnvelope,
    artifact_index: &mut Vec<ArtifactOutput>,
    artifacts: &mut HashMap<String, String>,
) {
    let mut warnings = Vec::new();
    if !should_run_archive_hook(spec, request, run_status) {
        return;
    }

    let Some(workflow) = spec.workflow.as_ref() else {
        return;
    };

    let archive_dir = match sanitize_relative_path_string(&workflow.archive_policy.archive_dir) {
        Some(path) => path,
        None => {
            warnings.push(format!(
                "invalid workflow.archive_policy.archive_dir: {}",
                workflow.archive_policy.archive_dir
            ));
            write_warning_artifact(artifact_index, artifacts, &warnings);
            return;
        }
    };

    let now = OffsetDateTime::now_utc();
    let created_at = format_time(now);
    let date = now.date().to_string();
    let slug = slugify_task(&request.task);
    let handle_short = short_handle_id(handle_id);

    let mut final_summary_path = None;
    let mut decision_note_path = None;

    if workflow.archive_policy.write_final_summary {
        let final_summary_rel = join_relative(
            &archive_dir,
            &format!("{date}-{slug}-{handle_short}-final-summary.md"),
        );
        let final_summary = build_final_summary_markdown(spec, request, summary, handle_id, &date);
        if let Err(err) =
            write_repo_text_file(&workspace.source_path, &final_summary_rel, &final_summary)
        {
            warnings.push(err);
        } else {
            upsert_artifact(
                artifact_index,
                artifacts,
                &final_summary_rel,
                ArtifactKind::ReportMarkdown,
                "Archived final summary",
                Some("text/markdown"),
                &created_at,
                final_summary,
            );
            final_summary_path = Some(final_summary_rel);
        }
    }

    let should_write_decision_note = workflow.knowledge_capture.write_decision_note
        && should_write_decision_note(&workflow.knowledge_capture, request, summary);
    if should_write_decision_note {
        let decision_rel = format!("docs/decisions/{date}-{slug}-{handle_short}-decision-note.md");
        let decision_note = build_decision_note_markdown(spec, request, summary, handle_id, &date);
        if let Err(err) =
            write_repo_text_file(&workspace.source_path, &decision_rel, &decision_note)
        {
            warnings.push(err);
        } else {
            upsert_artifact(
                artifact_index,
                artifacts,
                &decision_rel,
                ArtifactKind::ReportMarkdown,
                "Archived decision note",
                Some("text/markdown"),
                &created_at,
                decision_note,
            );
            decision_note_path = Some(decision_rel);
        }
    }

    if workflow.archive_policy.write_metadata_index {
        let metadata_index_rel = join_relative(&archive_dir, "index.json");
        let metadata_entry = ArchiveMetadataEntry {
            handle_id: handle_id.to_string(),
            created_at: created_at.clone(),
            agent_name: spec.core.name.clone(),
            provider: spec.core.provider.as_str().to_string(),
            stage: request
                .stage
                .clone()
                .unwrap_or_else(|| "Archive".to_string()),
            task: request.task.clone(),
            parse_status: format!("{:?}", summary.parse_status),
            verification_status: format!("{:?}", summary.summary.verification_status),
            touched_files: summary.summary.touched_files.clone(),
            plan_refs: summary.summary.plan_refs.clone(),
            final_summary_path: final_summary_path.clone(),
            decision_note_path: decision_note_path.clone(),
        };

        match append_metadata_index(&workspace.source_path, &metadata_index_rel, &metadata_entry) {
            Ok(content) => {
                upsert_artifact(
                    artifact_index,
                    artifacts,
                    &metadata_index_rel,
                    ArtifactKind::ReportJson,
                    "Archive metadata index",
                    Some("application/json"),
                    &created_at,
                    content,
                );
            }
            Err(err) => warnings.push(err),
        }
    }

    if !warnings.is_empty() {
        write_warning_artifact(artifact_index, artifacts, &warnings);
    }
}

fn should_run_archive_hook(spec: &AgentSpec, request: &RunRequest, run_status: &RunStatus) -> bool {
    if !matches!(run_status, RunStatus::Succeeded) {
        return false;
    }

    let Some(workflow) = spec.workflow.as_ref() else {
        return false;
    };
    if !workflow.enabled || !workflow.archive_policy.enabled {
        return false;
    }

    request
        .stage
        .as_deref()
        .is_some_and(|stage| stage.eq_ignore_ascii_case("archive"))
}

fn should_write_decision_note(
    policy: &crate::spec::workflow::KnowledgeCapturePolicy,
    request: &RunRequest,
    summary: &SummaryEnvelope,
) -> bool {
    if policy
        .trigger_if_touched_files_gt
        .is_some_and(|threshold| summary.summary.touched_files.len() > threshold as usize)
    {
        return true;
    }

    let task_text = format!(
        "{}\n{}",
        request.task,
        request.task_brief.clone().unwrap_or_default()
    );
    let summary_text = format!(
        "{}\n{}\n{}",
        summary.summary.summary,
        summary.summary.key_findings.join("\n"),
        summary.summary.next_steps.join("\n")
    );
    let combined_text = format!("{task_text}\n{summary_text}").to_lowercase();

    if policy.trigger_if_new_config
        && summary
            .summary
            .touched_files
            .iter()
            .any(|file| is_config_or_schema_path(file))
    {
        return true;
    }

    if policy.trigger_if_behavior_change
        && contains_any_keyword(
            &combined_text,
            &[
                "behavior change",
                "semantic",
                "breaking",
                "regression risk",
                "行为变化",
                "语义变化",
                "不兼容",
            ],
        )
    {
        return true;
    }

    if policy.trigger_if_non_obvious_bugfix
        && contains_any_keyword(
            &combined_text,
            &[
                "bugfix",
                "race condition",
                "edge case",
                "non-obvious",
                "回归",
                "隐蔽",
                "疑难",
            ],
        )
    {
        return true;
    }

    false
}

fn is_config_or_schema_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    lowered.contains("config")
        || lowered.contains("settings")
        || lowered.contains("schema")
        || lowered.ends_with(".toml")
        || lowered.ends_with(".yaml")
        || lowered.ends_with(".yml")
        || lowered.ends_with(".json")
        || lowered.ends_with(".ini")
        || lowered.ends_with(".env")
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn build_final_summary_markdown(
    spec: &AgentSpec,
    request: &RunRequest,
    summary: &SummaryEnvelope,
    handle_id: &str,
    date: &str,
) -> String {
    let touched_files = markdown_list_or_empty(&summary.summary.touched_files);
    let plan_refs = markdown_list_or_empty(&summary.summary.plan_refs);
    let key_findings = markdown_list_or_empty(&summary.summary.key_findings);
    let open_questions = markdown_list_or_empty(&summary.summary.open_questions);
    let next_steps = markdown_list_or_empty(&summary.summary.next_steps);
    format!(
        "# Final Summary\n\nDate: {date}\n\nHandle: `{handle_id}`\n\nAgent: `{agent}` ({provider})\n\nStage: `{stage}`\n\nTask: {task}\n\nVerification: `{verification}`\n\nParse status: `{parse_status}`\n\nSummary:\n\n{summary_text}\n\n## Key findings\n\n{key_findings}\n\n## Touched files\n\n{touched_files}\n\n## Plan refs\n\n{plan_refs}\n\n## Open questions\n\n{open_questions}\n\n## Next steps\n\n{next_steps}\n",
        agent = spec.core.name,
        provider = spec.core.provider.as_str(),
        stage = request.stage.as_deref().unwrap_or("Archive"),
        task = request.task,
        verification = format!("{:?}", summary.summary.verification_status),
        parse_status = format!("{:?}", summary.parse_status),
        summary_text = summary.summary.summary,
        key_findings = key_findings,
        touched_files = touched_files,
        plan_refs = plan_refs,
        open_questions = open_questions,
        next_steps = next_steps,
    )
}

fn build_decision_note_markdown(
    spec: &AgentSpec,
    request: &RunRequest,
    summary: &SummaryEnvelope,
    handle_id: &str,
    date: &str,
) -> String {
    let key_findings = markdown_list_or_empty(&summary.summary.key_findings);
    let touched_files = markdown_list_or_empty(&summary.summary.touched_files);
    format!(
        "# Decision Note\n\nDate: {date}\n\nHandle: `{handle_id}`\n\nAgent: `{agent}` ({provider})\n\nContext:\n\n{task}\n\nDecision:\n\n{summary_text}\n\nEvidence:\n\n{key_findings}\n\nChanged surface:\n\n{touched_files}\n\nFollow-up:\n\n{next_steps}\n",
        agent = spec.core.name,
        provider = spec.core.provider.as_str(),
        task = request.task,
        summary_text = summary.summary.summary,
        key_findings = key_findings,
        touched_files = touched_files,
        next_steps = markdown_list_or_empty(&summary.summary.next_steps),
    )
}

fn markdown_list_or_empty(items: &[String]) -> String {
    if items.is_empty() {
        return "- (none)".to_string();
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_metadata_index(
    workspace_root: &Path,
    metadata_index_rel: &str,
    entry: &ArchiveMetadataEntry,
) -> std::result::Result<String, String> {
    let rel_path = sanitize_relative_artifact_path(metadata_index_rel)
        .ok_or_else(|| format!("invalid archive metadata index path: {metadata_index_rel}"))?;
    let target = workspace_root.join(rel_path);

    let mut existing = if target.exists() {
        let raw = fs::read_to_string(&target).map_err(|err| {
            format!(
                "failed to read archive metadata index {}: {err}",
                target.display()
            )
        })?;
        serde_json::from_str::<Vec<ArchiveMetadataEntry>>(&raw).map_err(|err| {
            format!(
                "failed to parse archive metadata index {}: {err}",
                target.display()
            )
        })?
    } else {
        Vec::new()
    };
    existing.push(entry.clone());
    let content = serde_json::to_string_pretty(&existing)
        .map_err(|err| format!("failed to serialize archive metadata index: {err}"))?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create archive metadata directory {}: {err}",
                parent.display()
            )
        })?;
    }
    fs::write(&target, &content).map_err(|err| {
        format!(
            "failed to write archive metadata index {}: {err}",
            target.display()
        )
    })?;
    Ok(content)
}

fn write_repo_text_file(
    workspace_root: &Path,
    relative_path: &str,
    content: &str,
) -> std::result::Result<(), String> {
    let rel_path = sanitize_relative_artifact_path(relative_path)
        .ok_or_else(|| format!("invalid archive output path: {relative_path}"))?;
    let target = workspace_root.join(rel_path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create archive output directory {}: {err}",
                parent.display()
            )
        })?;
    }
    fs::write(&target, content)
        .map_err(|err| format!("failed to write archive file {}: {err}", target.display()))
}

fn write_warning_artifact(
    artifact_index: &mut Vec<ArtifactOutput>,
    artifacts: &mut HashMap<String, String>,
    warnings: &[String],
) {
    let created_at = format_time(OffsetDateTime::now_utc());
    let warning_text = warnings.join("\n");
    upsert_artifact(
        artifact_index,
        artifacts,
        ARCHIVE_WARNING_ARTIFACT_PATH,
        ArtifactKind::StderrText,
        "Archive hook warnings",
        Some("text/plain"),
        &created_at,
        warning_text,
    );
}

fn upsert_artifact(
    artifact_index: &mut Vec<ArtifactOutput>,
    artifacts: &mut HashMap<String, String>,
    path: &str,
    kind: ArtifactKind,
    description: &str,
    media_type: Option<&str>,
    created_at: &str,
    content: String,
) {
    let output = ArtifactOutput {
        path: path.to_string(),
        kind: format!("{:?}", kind),
        description: description.to_string(),
        media_type: media_type.map(str::to_string),
        producer: Some("runtime".to_string()),
        created_at: Some(created_at.to_string()),
    };

    if let Some(existing) = artifact_index.iter_mut().find(|item| item.path == path) {
        *existing = output;
    } else {
        artifact_index.push(output);
    }

    artifacts.insert(path.to_string(), content);
}

fn join_relative(base: &str, leaf: &str) -> String {
    normalize_relative_path(&Path::new(base).join(leaf))
}

fn sanitize_relative_path_string(path: &str) -> Option<String> {
    sanitize_relative_artifact_path(path).map(|value| normalize_relative_path(&value))
}

fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn short_handle_id(handle_id: &str) -> &str {
    handle_id.split('-').next().unwrap_or(handle_id)
}

fn slugify_task(task: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in task.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "task".to_string()
    } else {
        trimmed.to_string()
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveMetadataEntry {
    handle_id: String,
    created_at: String,
    agent_name: String,
    provider: String,
    stage: String,
    task: String,
    parse_status: String,
    verification_status: String,
    touched_files: Vec<String>,
    plan_refs: Vec<String>,
    final_summary_path: Option<String>,
    decision_note_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use tempfile::tempdir;
    use uuid::Uuid;

    use crate::{
        mcp::{archive::apply_archive_hook, dto::ArtifactOutput, state::WorkspaceRecord},
        runtime::{
            dispatcher::RunStatus,
            summary::{StructuredSummary, SummaryEnvelope, SummaryParseStatus, VerificationStatus},
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::RuntimePolicy,
            workflow::{ArchivePolicy, KnowledgeCapturePolicy, WorkflowSpec},
            AgentSpec,
        },
        types::{RunMode, RunRequest},
    };

    fn sample_spec(archive_dir: &str) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "style-reviewer".to_string(),
                description: "review".to_string(),
                provider: Provider::Claude,
                model: Some("sonnet".to_string()),
                instructions: "review and archive".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: vec!["review".to_string(), "archive".to_string()],
                metadata: HashMap::new(),
            },
            runtime: RuntimePolicy::default(),
            provider_overrides: Default::default(),
            workflow: Some(WorkflowSpec {
                enabled: true,
                archive_policy: ArchivePolicy {
                    enabled: true,
                    archive_dir: archive_dir.to_string(),
                    write_final_summary: true,
                    write_metadata_index: true,
                },
                knowledge_capture: KnowledgeCapturePolicy {
                    trigger_if_touched_files_gt: Some(3),
                    trigger_if_new_config: true,
                    trigger_if_behavior_change: true,
                    trigger_if_non_obvious_bugfix: true,
                    write_decision_note: true,
                    update_project_memory: false,
                },
                ..WorkflowSpec::default()
            }),
        }
    }

    fn sample_request(working_dir: PathBuf, stage: &str) -> RunRequest {
        RunRequest {
            task: "Stabilize parser behavior with config migration".to_string(),
            task_brief: Some("behavior change for parser config and bugfix".to_string()),
            parent_summary: None,
            selected_files: Vec::new(),
            stage: Some(stage.to_string()),
            plan_ref: Some("PLAN.md".to_string()),
            working_dir,
            run_mode: RunMode::Sync,
            acceptance_criteria: vec!["pass tests".to_string()],
        }
    }

    fn sample_summary() -> SummaryEnvelope {
        SummaryEnvelope {
            contract_version: "mcp-subagent.summary.v2".to_string(),
            parse_status: SummaryParseStatus::Validated,
            summary: StructuredSummary {
                summary: "Implemented parser fix and config migration".to_string(),
                key_findings: vec![
                    "Added migration for config key rename".to_string(),
                    "Fixed parser edge case for empty token".to_string(),
                ],
                artifacts: Vec::new(),
                open_questions: Vec::new(),
                next_steps: vec!["run full integration tests".to_string()],
                exit_code: 0,
                verification_status: VerificationStatus::Passed,
                touched_files: vec![
                    "src/parser.rs".to_string(),
                    "src/config.rs".to_string(),
                    "config/default.toml".to_string(),
                    "docs/migration.md".to_string(),
                ],
                plan_refs: vec!["step-1".to_string(), "step-2".to_string()],
            },
            raw_fallback_text: None,
        }
    }

    #[test]
    fn archive_stage_generates_final_summary_decision_and_metadata_index() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).expect("create root");
        let spec = sample_spec("docs/plans");
        let request = sample_request(root.clone(), "Archive");
        let handle_id = Uuid::now_v7().to_string();
        let workspace = WorkspaceRecord {
            mode: "InPlace".to_string(),
            source_path: root.clone(),
            workspace_path: root.clone(),
            notes: Vec::new(),
            lock_key: None,
        };
        let summary = sample_summary();
        let mut artifact_index: Vec<ArtifactOutput> = Vec::new();
        let mut artifacts = HashMap::new();

        apply_archive_hook(
            &spec,
            &request,
            &RunStatus::Succeeded,
            &handle_id,
            &workspace,
            &summary,
            &mut artifact_index,
            &mut artifacts,
        );

        let final_summary_path = artifact_index
            .iter()
            .find(|item| item.description == "Archived final summary")
            .map(|item| item.path.clone())
            .expect("final summary artifact");
        let decision_note_path = artifact_index
            .iter()
            .find(|item| item.description == "Archived decision note")
            .map(|item| item.path.clone())
            .expect("decision note artifact");
        let metadata_index_path = artifact_index
            .iter()
            .find(|item| item.description == "Archive metadata index")
            .map(|item| item.path.clone())
            .expect("metadata index artifact");

        assert!(final_summary_path.starts_with("docs/plans/"));
        assert!(final_summary_path.ends_with("-final-summary.md"));
        assert!(decision_note_path.starts_with("docs/decisions/"));
        assert!(decision_note_path.ends_with("-decision-note.md"));
        assert_eq!(metadata_index_path, "docs/plans/index.json");

        assert!(root.join(&final_summary_path).exists());
        assert!(root.join(&decision_note_path).exists());
        assert!(root.join(&metadata_index_path).exists());

        let metadata_raw =
            std::fs::read_to_string(root.join(&metadata_index_path)).expect("read metadata index");
        assert!(metadata_raw.contains(&handle_id));
        assert!(metadata_raw.contains("style-reviewer"));
        assert!(metadata_raw.contains("step-1"));

        assert!(artifacts.contains_key(&final_summary_path));
        assert!(artifacts.contains_key(&decision_note_path));
        assert!(artifacts.contains_key(&metadata_index_path));
    }

    #[test]
    fn non_archive_stage_skips_archive_hook() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).expect("create root");
        let spec = sample_spec("docs/plans");
        let request = sample_request(root.clone(), "Build");
        let workspace = WorkspaceRecord {
            mode: "InPlace".to_string(),
            source_path: root.clone(),
            workspace_path: root,
            notes: Vec::new(),
            lock_key: None,
        };
        let summary = sample_summary();
        let mut artifact_index: Vec<ArtifactOutput> = Vec::new();
        let mut artifacts = HashMap::new();

        apply_archive_hook(
            &spec,
            &request,
            &RunStatus::Succeeded,
            "run-1",
            &workspace,
            &summary,
            &mut artifact_index,
            &mut artifacts,
        );

        assert!(artifact_index.is_empty());
        assert!(artifacts.is_empty());
    }

    #[test]
    fn invalid_archive_dir_creates_warning_artifact() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).expect("create root");
        let spec = sample_spec("/absolute/path/not-allowed");
        let request = sample_request(root.clone(), "Archive");
        let workspace = WorkspaceRecord {
            mode: "InPlace".to_string(),
            source_path: root.clone(),
            workspace_path: root,
            notes: Vec::new(),
            lock_key: None,
        };
        let summary = sample_summary();
        let mut artifact_index: Vec<ArtifactOutput> = Vec::new();
        let mut artifacts = HashMap::new();

        apply_archive_hook(
            &spec,
            &request,
            &RunStatus::Succeeded,
            "run-1",
            &workspace,
            &summary,
            &mut artifact_index,
            &mut artifacts,
        );

        let warning = artifact_index
            .iter()
            .find(|item| item.path == "archive/hook-warnings.txt")
            .expect("warning artifact exists");
        assert_eq!(warning.kind, "StderrText");
        assert!(artifacts
            .get("archive/hook-warnings.txt")
            .expect("warning payload")
            .contains("invalid workflow.archive_policy.archive_dir"));
    }
}
