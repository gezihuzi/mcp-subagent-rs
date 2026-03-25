use std::collections::HashMap;

use serde_json::json;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    mcp::dto::ArtifactOutput,
    runtime::summary::{ArtifactKind, SummaryEnvelope},
    spec::AgentSpec,
    types::RunRequest,
};

pub(crate) fn apply_review_evidence_hook(
    spec: &AgentSpec,
    request: &RunRequest,
    summary: &SummaryEnvelope,
    artifact_index: &mut Vec<ArtifactOutput>,
    artifacts: &mut HashMap<String, String>,
) {
    if !request
        .stage
        .as_deref()
        .is_some_and(|stage| stage.eq_ignore_ascii_case("review"))
    {
        return;
    }
    let Some(workflow) = spec.workflow.as_ref() else {
        return;
    };
    if !workflow.enabled {
        return;
    }

    let high_risk = is_high_risk_review(spec, request);
    let mut required_tracks = Vec::new();
    if workflow.review_policy.require_correctness_review {
        required_tracks.push("correctness".to_string());
    }
    if workflow.review_policy.require_style_review || high_risk {
        required_tracks.push("style".to_string());
    }

    let current_tracks = detect_review_tracks(&agent_profile(spec));
    let parent_tracks = request
        .parent_summary
        .as_deref()
        .map(|text| detect_review_tracks(&text.to_lowercase()))
        .unwrap_or_default();

    let mut available = Vec::new();
    if current_tracks.correctness || parent_tracks.correctness {
        available.push("correctness".to_string());
    }
    if current_tracks.style || parent_tracks.style {
        available.push("style".to_string());
    }
    let dual_review_satisfied = required_tracks
        .iter()
        .all(|required| available.contains(required));

    let content = serde_json::to_string_pretty(&json!({
        "agent": spec.core.name,
        "stage": request.stage,
        "high_risk": high_risk,
        "required_tracks": required_tracks,
        "current_agent_tracks": tracks_to_vec(current_tracks),
        "parent_summary_tracks": tracks_to_vec(parent_tracks),
        "available_tracks": available,
        "dual_review_satisfied": dual_review_satisfied,
        "review_policy": {
            "require_correctness_review": workflow.review_policy.require_correctness_review,
            "require_style_review": workflow.review_policy.require_style_review,
            "allow_same_provider_dual_review": workflow.review_policy.allow_same_provider_dual_review,
            "prefer_cross_provider_review": workflow.review_policy.prefer_cross_provider_review,
        },
        "summary": {
            "parse_status": format!("{}", summary.parse_status),
            "verification_status": format!("{}", summary.summary.verification_status),
            "touched_files": summary.summary.touched_files,
            "plan_refs": summary.summary.plan_refs,
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());

    upsert_artifact(
        artifact_index,
        artifacts,
        "review/evidence.json",
        ArtifactKind::ReportJson,
        "Review policy evidence snapshot",
        Some("application/json"),
        content,
    );
}

#[derive(Debug, Clone, Copy, Default)]
struct ReviewTracks {
    correctness: bool,
    style: bool,
}

fn tracks_to_vec(tracks: ReviewTracks) -> Vec<String> {
    let mut values = Vec::new();
    if tracks.correctness {
        values.push("correctness".to_string());
    }
    if tracks.style {
        values.push("style".to_string());
    }
    values
}

fn detect_review_tracks(text: &str) -> ReviewTracks {
    ReviewTracks {
        correctness: contains_any_keyword(
            text,
            &[
                "correctness",
                "logic",
                "regression",
                "bug",
                "safety",
                "security",
                "verify",
                "validation",
            ],
        ),
        style: contains_any_keyword(
            text,
            &[
                "style",
                "maintainability",
                "readability",
                "naming",
                "consistency",
            ],
        ),
    }
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn agent_profile(spec: &AgentSpec) -> String {
    let mut profile = String::new();
    profile.push_str(&spec.core.name);
    profile.push('\n');
    profile.push_str(&spec.core.description);
    profile.push('\n');
    profile.push_str(&spec.core.instructions);
    profile.push('\n');
    for tag in &spec.core.tags {
        profile.push_str(tag);
        profile.push('\n');
    }
    profile.to_lowercase()
}

fn is_high_risk_review(spec: &AgentSpec, request: &RunRequest) -> bool {
    let Some(workflow) = spec.workflow.as_ref() else {
        return false;
    };
    let gate = &workflow.require_plan_when;
    if gate
        .require_plan_if_touched_files_ge
        .is_some_and(|threshold| request.selected_files.len() as u32 >= threshold)
    {
        return true;
    }
    if gate.require_plan_if_parallel_agents
        && matches!(request.run_mode, crate::types::RunMode::Async)
    {
        return true;
    }
    if gate
        .require_plan_if_estimated_runtime_minutes_ge
        .is_some_and(|threshold| (spec.runtime.timeout_secs / 60) >= threshold as u64)
    {
        return true;
    }

    let lowered = format!(
        "{}\n{}",
        request.task,
        request.task_brief.clone().unwrap_or_default()
    )
    .to_lowercase();
    (gate.require_plan_if_cross_module
        && contains_any_keyword(
            &lowered,
            &["cross module", "cross-module", "multi-module", "跨模块"],
        ))
        || (gate.require_plan_if_new_interface
            && contains_any_keyword(
                &lowered,
                &[
                    "new interface",
                    "new api",
                    "new endpoint",
                    "新接口",
                    "新增接口",
                ],
            ))
        || (gate.require_plan_if_migration
            && contains_any_keyword(
                &lowered,
                &["migration", "migrate", "schema migration", "迁移", "升级"],
            ))
        || (gate.require_plan_if_human_approval_point
            && contains_any_keyword(
                &lowered,
                &[
                    "human approval",
                    "approval required",
                    "人工审批",
                    "需要审批",
                ],
            ))
}

fn upsert_artifact(
    artifact_index: &mut Vec<ArtifactOutput>,
    artifacts: &mut HashMap<String, String>,
    path: &str,
    kind: ArtifactKind,
    description: &str,
    media_type: Option<&str>,
    content: String,
) {
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_string());
    let output = ArtifactOutput {
        path: path.to_string(),
        kind: format!("{}", kind),
        description: description.to_string(),
        media_type: media_type.map(str::to_string),
        producer: Some("runtime".to_string()),
        created_at: Some(created_at),
    };
    if let Some(existing) = artifact_index.iter_mut().find(|item| item.path == path) {
        *existing = output;
    } else {
        artifact_index.push(output);
    }
    artifacts.insert(path.to_string(), content);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{
        mcp::{dto::ArtifactOutput, review::apply_review_evidence_hook},
        runtime::summary::{
            StructuredSummary, SummaryEnvelope, SummaryParseStatus, VerificationStatus,
        },
        spec::{
            core::{AgentSpecCore, Provider},
            runtime_policy::RuntimePolicy,
            workflow::WorkflowSpec,
            AgentSpec,
        },
        types::{RunMode, RunRequest},
    };

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "correctness-reviewer".to_string(),
                description: "review correctness".to_string(),
                provider: Provider::Codex,
                model: None,
                instructions: "review logic and regressions".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: vec!["review".to_string(), "correctness".to_string()],
                metadata: HashMap::new(),
            },
            runtime: RuntimePolicy::default(),
            provider_overrides: Default::default(),
            workflow: Some(WorkflowSpec::default()),
        }
    }

    fn sample_request() -> RunRequest {
        RunRequest {
            task: "review parser behavior changes".to_string(),
            task_brief: None,
            parent_summary: None,
            selected_files: Vec::new(),
            stage: Some("Review".to_string()),
            plan_ref: None,
            working_dir: ".".into(),
            run_mode: RunMode::Sync,
            acceptance_criteria: Vec::new(),
        }
    }

    fn sample_summary() -> SummaryEnvelope {
        SummaryEnvelope {
            contract_version: "mcp-subagent.summary.v2".to_string(),
            parse_status: SummaryParseStatus::Validated,
            summary: StructuredSummary {
                summary: "ok".to_string(),
                key_findings: vec!["a".to_string()],
                artifacts: Vec::new(),
                open_questions: Vec::new(),
                next_steps: Vec::new(),
                exit_code: 0,
                verification_status: VerificationStatus::Passed,
                touched_files: vec!["src/parser.rs".to_string()],
                plan_refs: vec!["step-1".to_string()],
            },
            raw_fallback_text: None,
        }
    }

    #[test]
    fn review_stage_emits_review_evidence_artifact() {
        let spec = sample_spec();
        let request = sample_request();
        let summary = sample_summary();
        let mut index: Vec<ArtifactOutput> = Vec::new();
        let mut artifacts = HashMap::new();

        apply_review_evidence_hook(&spec, &request, &summary, &mut index, &mut artifacts);

        assert!(index.iter().any(|item| item.path == "review/evidence.json"));
        assert!(artifacts
            .get("review/evidence.json")
            .is_some_and(|content| content.contains("\"dual_review_satisfied\"")));
    }
}
