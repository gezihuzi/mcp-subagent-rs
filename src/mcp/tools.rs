use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_router, ErrorData, Json,
};
use serde::Deserialize;
use serde_json::json;
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
            AgentListing, CancelAgentOutput, GetAgentStatsInput, GetAgentStatsOutput,
            GetRunResultInput, HandleInput, ListAgentsOutput, ListRunsInput, ListRunsOutput,
            OutcomeView, ReadAgentArtifactInput, ReadAgentArtifactOutput, ReadRunLogsInput,
            ReadRunLogsOutput, RunAgentInput, RunEventOutput, RunUsageOutput, RunView,
            RuntimePolicySummary, WatchAgentEventsInput, WatchAgentEventsOutput, WatchRunInput,
            WatchRunOutput,
        },
        helpers::{build_capability_notes, cancelled_summary, failed_summary, format_time},
        persistence::{
            append_run_event, load_run_record_from_disk, persist_run_record, RuntimeEventInput,
        },
        review::apply_review_evidence_hook,
        server::{
            acquire_serialize_locks_from_state, provider_unavailable_message, McpSubagentServer,
        },
        service::run_dispatch,
        state::{
            append_status_if_terminal, apply_execution_policy_outcome, build_probe_result_snapshot,
            build_run_request_snapshot, build_run_spec_snapshot, RetryClassificationRecord,
            RunRecord,
        },
    },
    runtime::dispatcher::{RetryClassification, RunMetadata, RunStatus},
    types::RunMode,
};

pub(crate) fn build_tool_router() -> ToolRouter<McpSubagentServer> {
    McpSubagentServer::tool_router()
}

const FIRST_OUTPUT_WARN_AFTER_SECS: u64 = 8;

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
        .map(|value| value.len() as u64);
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
    let estimated_total_tokens = match (input_tokens, output_tokens) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        _ => None,
    };
    let native_usage = record.usage.as_ref();
    let mut used_native = false;
    let mut used_estimated = false;
    let input_tokens = if let Some(value) = native_usage.and_then(|usage| usage.input_tokens) {
        used_native = true;
        Some(value)
    } else {
        if input_tokens.is_some() {
            used_estimated = true;
        }
        input_tokens
    };
    let output_tokens = if let Some(value) = native_usage.and_then(|usage| usage.output_tokens) {
        used_native = true;
        Some(value)
    } else {
        if output_tokens.is_some() {
            used_estimated = true;
        }
        output_tokens
    };
    let total_tokens = if let Some(value) = native_usage.and_then(|usage| usage.total_tokens) {
        used_native = true;
        Some(value)
    } else if let (Some(input), Some(output)) = (input_tokens, output_tokens) {
        Some(input.saturating_add(output))
    } else {
        if estimated_total_tokens.is_some() {
            used_estimated = true;
        }
        estimated_total_tokens
    };
    let token_source = match (used_native, used_estimated) {
        (true, true) => "mixed",
        (true, false) => "native",
        (false, true) => "estimated",
        (false, false) => "unknown",
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
        token_source: token_source.to_string(),
        input_tokens,
        output_tokens,
        total_tokens,
        estimated_prompt_bytes,
        estimated_output_bytes,
    }
}

fn map_retry_classification(metadata: &RunMetadata) -> RetryClassificationRecord {
    RetryClassificationRecord {
        classification: format!("{}", metadata.retry_classification),
        reason: metadata.retry_classification_reason.clone(),
    }
}

fn resolve_retry_classification(record: &RunRecord) -> (String, Option<String>) {
    match &record.retry_classification {
        Some(value) => {
            let normalized = match value.classification.as_str() {
                "retryable" | "non_retryable" | "unknown" => value.classification.clone(),
                _ => "unknown".to_string(),
            };
            (normalized, value.reason.clone())
        }
        None => (format!("{}", RetryClassification::Unknown), None),
    }
}

fn map_record_outcome(record: &RunRecord, usage: RunUsageOutput) -> Option<OutcomeView> {
    match record.status {
        RunStatus::Succeeded => record
            .summary
            .as_ref()
            .map(|summary| OutcomeView::Succeeded {
                summary: summary.summary.summary.clone(),
                key_findings: summary.summary.key_findings.clone(),
                touched_files: summary.summary.touched_files.clone(),
                artifacts: record.artifact_index.clone(),
                usage,
            }),
        RunStatus::Failed => {
            let (retry_classification, _) = resolve_retry_classification(record);
            Some(OutcomeView::Failed {
                error: record
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "run failed".to_string()),
                retry_classification,
                partial_summary: record
                    .summary
                    .as_ref()
                    .map(|summary| summary.summary.summary.clone()),
                usage,
            })
        }
        RunStatus::Cancelled => Some(OutcomeView::Cancelled {
            reason: record
                .error_message
                .clone()
                .unwrap_or_else(|| "cancelled".to_string()),
        }),
        RunStatus::TimedOut => Some(OutcomeView::TimedOut {
            elapsed_secs: compute_duration_ms(record.created_at, record.updated_at).unwrap_or(0)
                / 1000,
        }),
        _ => None,
    }
}

fn build_run_view(
    handle_id: String,
    record: &RunRecord,
    runtime: Option<&EventRuntimeSnapshot>,
) -> RunView {
    let usage = build_usage_output(record);
    let phase = runtime
        .and_then(|snapshot| snapshot.phase.clone())
        .unwrap_or_else(|| format!("{}", record.status));
    RunView {
        handle_id,
        agent_name: record
            .spec_snapshot
            .as_ref()
            .map(|spec| spec.name.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        task_brief: record
            .request_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.task_brief.clone()),
        phase,
        terminal: is_terminal_status(&record.status),
        outcome: map_record_outcome(record, usage),
        created_at: format_time(record.created_at),
        updated_at: format_time(record.updated_at),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct StoredRunEventLine {
    event: String,
    timestamp: String,
    detail: serde_json::Value,
    seq: Option<u64>,
    ts: Option<String>,
    state: Option<String>,
    phase: Option<String>,
    source: Option<String>,
    message: Option<String>,
}

impl StoredRunEventLine {
    fn into_output(self) -> RunEventOutput {
        RunEventOutput {
            seq: self.seq,
            event: self.event,
            timestamp: if self.timestamp.is_empty() {
                self.ts.unwrap_or_default()
            } else {
                self.timestamp
            },
            state: self.state,
            phase: self.phase,
            source: self.source,
            message: self.message,
            detail: self.detail,
        }
    }
}

fn run_events_jsonl_path(state_dir: &std::path::Path, handle_id: &str) -> PathBuf {
    run_root_dir(state_dir).join(handle_id).join("events.jsonl")
}

fn load_run_events(
    state_dir: &std::path::Path,
    handle_id: &str,
) -> std::result::Result<Vec<RunEventOutput>, ErrorData> {
    let path = run_events_jsonl_path(state_dir, handle_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|err| {
        ErrorData::internal_error(
            format!("failed to read events file {}: {err}", path.display()),
            None,
        )
    })?;

    let mut events = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<StoredRunEventLine>(line).map_err(|err| {
            ErrorData::internal_error(
                format!(
                    "failed to parse events file {} line {}: {err}",
                    path.display(),
                    line_no + 1
                ),
                None,
            )
        })?;
        events.push(parsed.into_output());
    }
    Ok(events)
}

fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

fn event_time(event: &RunEventOutput) -> Option<OffsetDateTime> {
    parse_rfc3339(&event.timestamp)
}

fn duration_between(start: Option<OffsetDateTime>, end: Option<OffsetDateTime>) -> Option<u64> {
    let start = start?;
    let end = end?;
    if end < start {
        return None;
    }
    Some((end - start).whole_milliseconds().max(0) as u64)
}

fn first_event_time(events: &[RunEventOutput], name: &str) -> Option<OffsetDateTime> {
    events
        .iter()
        .find(|event| event.event == name)
        .and_then(event_time)
}

fn first_event_timestamp(events: &[RunEventOutput], name: &str) -> Option<String> {
    events
        .iter()
        .find(|event| event.event == name)
        .map(|event| event.timestamp.clone())
        .filter(|value| !value.is_empty())
}

fn latest_event(events: &[RunEventOutput]) -> Option<&RunEventOutput> {
    events.last()
}

fn current_phase_age_ms(
    events: &[RunEventOutput],
    now: OffsetDateTime,
) -> (Option<String>, Option<u64>) {
    let mut current_phase: Option<String> = None;
    let mut phase_started_at: Option<OffsetDateTime> = None;

    for event in events {
        let Some(ts) = event_time(event) else {
            continue;
        };
        let Some(phase) = event.phase.clone().filter(|value| !value.is_empty()) else {
            continue;
        };
        match current_phase.as_deref() {
            None => {
                current_phase = Some(phase);
                phase_started_at = Some(ts);
            }
            Some(existing) if existing == phase.as_str() => {}
            Some(_) => {
                current_phase = Some(phase);
                phase_started_at = Some(ts);
            }
        }
    }

    let age = phase_started_at.and_then(|started| {
        if now < started {
            None
        } else {
            Some((now - started).whole_milliseconds().max(0) as u64)
        }
    });
    (current_phase, age)
}

#[derive(Debug, Clone, Copy)]
struct ProviderWaitSignal {
    event: &'static str,
    phase: &'static str,
    reason: &'static str,
    message: &'static str,
}

fn detect_provider_wait_signal(text: &str) -> Option<ProviderWaitSignal> {
    let lowered = text.to_ascii_lowercase();
    if contains_any(
        &lowered,
        &[
            "trusted folder",
            "trust this folder",
            "waiting_for_trust",
            "waiting for trust",
            "trust required",
        ],
    ) {
        return Some(ProviderWaitSignal {
            event: "provider.waiting_for_trust",
            phase: "waiting_for_trust",
            reason: "trust_required",
            message: "provider is waiting for workspace trust",
        });
    }
    if auth_is_wait_signal(&lowered) {
        return Some(ProviderWaitSignal {
            event: "provider.waiting_for_auth",
            phase: "waiting_for_auth",
            reason: "auth_required",
            message: "provider is waiting for authentication",
        });
    }
    if contains_any(
        &lowered,
        &[
            "tool approval",
            "approval required",
            "permission denied",
            "consent required",
            "approval mode",
        ],
    ) {
        return Some(ProviderWaitSignal {
            event: "provider.waiting_for_tool_approval",
            phase: "waiting_for_tool_approval",
            reason: "tool_approval_required",
            message: "provider is waiting for tool approval",
        });
    }
    if contains_any(
        &lowered,
        &[
            "skill conflict",
            "skills conflict",
            "skill discovery",
            "find-skills",
            ".agents/skills",
            ".gemini/skills",
        ],
    ) {
        return Some(ProviderWaitSignal {
            event: "provider.waiting_for_skill_discovery",
            phase: "waiting_for_skill_discovery",
            reason: "skill_discovery",
            message: "provider is scanning skills/discovery context",
        });
    }
    if contains_any(
        &lowered,
        &[
            "workspace scan",
            "scanning workspace",
            "indexing workspace",
            "workspace settings",
        ],
    ) {
        return Some(ProviderWaitSignal {
            event: "provider.waiting_for_workspace_scan",
            phase: "waiting_for_workspace_scan",
            reason: "workspace_scan",
            message: "provider is scanning workspace context",
        });
    }
    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn auth_is_ready_signal(lowered: &str) -> bool {
    contains_any(
        lowered,
        &[
            "loaded cached credentials",
            "credentials loaded",
            "already authenticated",
            "auth restored",
            "using filekeychain fallback for secure storage",
        ],
    )
}

fn auth_is_wait_signal(lowered: &str) -> bool {
    if auth_is_ready_signal(lowered) {
        return false;
    }
    contains_any(
        lowered,
        &[
            "auth required",
            "authentication required",
            "authentication failed",
            "unauthorized",
            "login required",
            "missing credentials",
            "credentials required",
            "api key missing",
            "invalid api key",
        ],
    )
}

fn classify_block_reason_from_text(text: &str) -> Option<&'static str> {
    let lowered = text.to_ascii_lowercase();
    if contains_any(
        &lowered,
        &[
            "trusted folder",
            "trust this folder",
            "waiting_for_trust",
            "waiting for trust",
            "trust required",
        ],
    ) {
        return Some("trust_required");
    }
    if auth_is_wait_signal(&lowered) {
        return Some("auth_required");
    }
    if contains_any(
        &lowered,
        &[
            "tool approval",
            "approval required",
            "permission denied",
            "consent required",
            "approval mode",
        ],
    ) {
        return Some("tool_approval_required");
    }
    if contains_any(
        &lowered,
        &[
            "skill conflict",
            "skills conflict",
            "skill discovery",
            "find-skills",
            ".agents/skills",
            ".gemini/skills",
        ],
    ) {
        return Some("skill_discovery");
    }
    if contains_any(
        &lowered,
        &[
            "workspace scan",
            "scanning workspace",
            "indexing workspace",
            "workspace settings",
        ],
    ) {
        return Some("workspace_scan");
    }
    if contains_any(
        &lowered,
        &[
            "provider `",
            "provider unavailable",
            "missingbinary",
            "binary `",
            "not found in path",
        ],
    ) {
        return Some("provider_unavailable");
    }
    if contains_any(
        &lowered,
        &[
            "structured summary parse status is invalid",
            "invalid summary json",
            "sentinel",
            "structured summary parsing failed",
        ],
    ) {
        return Some("normalization_failed");
    }
    if contains_any(
        &lowered,
        &[
            "tls handshake eof",
            "stream disconnected before completion",
            "connection refused",
            "network error",
        ],
    ) {
        return Some("network_error");
    }
    None
}

fn classify_block_reason_from_events(
    events: &[RunEventOutput],
    stalled: bool,
) -> Option<&'static str> {
    for event in events.iter().rev() {
        match event.event.as_str() {
            "provider.waiting_for_trust" => return Some("trust_required"),
            "provider.waiting_for_auth" => return Some("auth_required"),
            "provider.waiting_for_tool_approval" => return Some("tool_approval_required"),
            "provider.waiting_for_consent" => return Some("consent_required"),
            "provider.waiting_for_skill_discovery" => return Some("skill_discovery"),
            "provider.waiting_for_workspace_scan" => return Some("workspace_scan"),
            "provider.first_output.warning" if stalled => return Some("provider_output_wait"),
            "run.queued" if stalled => return Some("queueing"),
            "workspace.prepare.started" if stalled => return Some("workspace_prepare"),
            "provider.probe.started" if stalled => return Some("provider_probe"),
            "provider.boot.started" if stalled => return Some("provider_boot"),
            _ => {}
        }
        if let Some(message) = event.message.as_deref() {
            if let Some(reason) = classify_block_reason_from_text(message) {
                return Some(reason);
            }
        }
        if !event.detail.is_null() {
            let detail_text = event.detail.to_string();
            if let Some(reason) = classify_block_reason_from_text(&detail_text) {
                return Some(reason);
            }
        }
    }
    None
}

fn classify_block_reason(
    status: &RunStatus,
    phase: Option<&str>,
    stalled: bool,
    events: &[RunEventOutput],
    error_message: Option<&str>,
) -> Option<String> {
    if matches!(status, RunStatus::Succeeded) {
        return None;
    }
    if let Some(message) = error_message {
        if let Some(reason) = classify_block_reason_from_text(message) {
            return Some(reason.to_string());
        }
    }
    if let Some(reason) = classify_block_reason_from_events(events, stalled) {
        return Some(reason.to_string());
    }
    if !is_terminal_status(status) && stalled {
        let fallback = match phase.unwrap_or_default() {
            "queueing" => "queueing",
            "workspace_prepare" => "workspace_prepare",
            "provider_probe" => "provider_probe",
            "provider_boot" => "provider_boot",
            "running" => "provider_output_wait",
            _ => "unknown_startup_wait",
        };
        return Some(fallback.to_string());
    }
    None
}

fn push_advice(advice: &mut Vec<String>, item: &str) {
    if advice.iter().any(|existing| existing == item) {
        return;
    }
    advice.push(item.to_string());
}

fn build_watch_advice(
    status: &RunStatus,
    phase: Option<&str>,
    block_reason: Option<&str>,
    phase_timeout_hit: bool,
) -> Vec<String> {
    let mut advice = Vec::new();
    if phase_timeout_hit {
        let phase_label = phase.unwrap_or("unknown");
        push_advice(
            &mut advice,
            &format!(
                "phase timeout hit in `{phase_label}`; inspect events/logs for this phase and consider cancel/retry"
            ),
        );
    }
    if let Some(reason) = block_reason {
        match reason {
            "trust_required" => push_advice(
                &mut advice,
                "provider is waiting for trust; mark the workspace as trusted and retry",
            ),
            "auth_required" => push_advice(
                &mut advice,
                "provider is waiting for authentication; refresh login/session credentials",
            ),
            "tool_approval_required" | "consent_required" => push_advice(
                &mut advice,
                "provider is waiting for approval; adjust approval mode or confirm prompt",
            ),
            "skill_discovery" => push_advice(
                &mut advice,
                "skill discovery is noisy; prefer isolated scratch workspace for simple research tasks",
            ),
            "workspace_scan" => push_advice(
                &mut advice,
                "workspace scan is slow; reduce include scope or switch to minimal delegation context",
            ),
            "provider_unavailable" => push_advice(
                &mut advice,
                "provider binary is unavailable; install it or fix PATH before retrying",
            ),
            "network_error" => push_advice(
                &mut advice,
                "network connectivity looks unstable; retry or switch transport/network",
            ),
            "normalization_failed" => push_advice(
                &mut advice,
                "normalization failed; consume native result/logs and tighten output contract if needed",
            ),
            "provider_output_wait" | "provider_boot" => push_advice(
                &mut advice,
                "provider has not produced output; check auth/trust/approval prompts and provider startup logs",
            ),
            _ => {}
        }
    }

    match status {
        RunStatus::Succeeded => push_advice(
            &mut advice,
            "run completed; use get_run_result/read_run_logs/read_agent_artifact for outputs",
        ),
        RunStatus::Failed => push_advice(
            &mut advice,
            "run failed; inspect summary.json and stderr.txt for root cause before retry",
        ),
        RunStatus::TimedOut => push_advice(
            &mut advice,
            "run timed out; increase timeout or narrow task/context scope",
        ),
        RunStatus::Cancelled => push_advice(
            &mut advice,
            "run was cancelled; restart with spawn/run if still needed",
        ),
        _ => {}
    }
    advice
}

fn wait_reason_from_event_name(name: &str) -> Option<&'static str> {
    match name {
        "provider.waiting_for_trust" => Some("trust_required"),
        "provider.waiting_for_auth" => Some("auth_required"),
        "provider.waiting_for_tool_approval" => Some("tool_approval_required"),
        "provider.waiting_for_consent" => Some("consent_required"),
        "provider.waiting_for_skill_discovery" => Some("skill_discovery"),
        "provider.waiting_for_workspace_scan" => Some("workspace_scan"),
        _ => None,
    }
}

fn collect_wait_reasons(events: &[RunEventOutput]) -> (Vec<String>, Option<String>) {
    let mut reasons = Vec::new();
    for event in events {
        let Some(reason) = wait_reason_from_event_name(&event.event) else {
            continue;
        };
        if reasons.iter().any(|existing| existing == reason) {
            continue;
        }
        reasons.push(reason.to_string());
    }
    let current = reasons.last().cloned();
    (reasons, current)
}

fn has_event_named(events: &[RunEventOutput], name: &str) -> bool {
    events.iter().any(|event| event.event == name)
}

#[derive(Debug, Clone, Default)]
struct EventRuntimeSnapshot {
    phase: Option<String>,
    block_reason: Option<String>,
}

fn build_event_runtime_snapshot(
    status: &RunStatus,
    events: &[RunEventOutput],
    now: OffsetDateTime,
    error_message: Option<&str>,
) -> EventRuntimeSnapshot {
    let latest = latest_event(events);
    let last_event_age_ms = latest.and_then(event_time).and_then(|ts| {
        if now < ts {
            None
        } else {
            Some((now - ts).whole_milliseconds().max(0) as u64)
        }
    });
    let stalled = !is_terminal_status(status) && last_event_age_ms.is_some_and(|ms| ms >= 8_000);
    let phase = latest.and_then(|event| event.phase.clone());
    let block_reason =
        classify_block_reason(status, phase.as_deref(), stalled, events, error_message);
    EventRuntimeSnapshot {
        phase,
        block_reason,
    }
}

fn build_agent_stats_output(
    handle_id: &str,
    record: &RunRecord,
    events: &[RunEventOutput],
    now: OffsetDateTime,
) -> GetAgentStatsOutput {
    let created_at = Some(record.created_at);
    let accepted_at = first_event_time(events, "run.accepted").or(created_at);
    let probe_started = first_event_time(events, "provider.probe.started");
    let probe_completed = first_event_time(events, "provider.probe.completed");
    let workspace_started = first_event_time(events, "workspace.prepare.started");
    let provider_boot_started = first_event_time(events, "provider.boot.started");
    let first_output = first_event_time(events, "provider.first_output");
    let first_output_warning_at = first_event_timestamp(events, "provider.first_output.warning");
    let first_output_warned = first_output_warning_at.is_some();
    let terminal_at = if is_terminal_status(&record.status) {
        Some(record.updated_at)
    } else {
        None
    };
    let end_at = terminal_at.or(Some(now));

    let queue_ms = duration_between(accepted_at, probe_started.or(workspace_started));
    let provider_probe_ms = duration_between(probe_started, probe_completed);
    let workspace_prepare_ms = duration_between(
        workspace_started,
        provider_boot_started.or(first_output).or(end_at),
    );
    let provider_boot_ms = duration_between(provider_boot_started, first_output.or(end_at));
    let execution_start = workspace_started
        .or(probe_completed)
        .or(probe_started)
        .or(accepted_at);
    let execution_ms = duration_between(execution_start, end_at);
    let first_output_ms = duration_between(accepted_at, first_output);
    let wall_ms = duration_between(accepted_at.or(created_at), end_at);

    let latest = latest_event(events);
    let last_event_at = latest
        .map(|event| event.timestamp.clone())
        .filter(|v| !v.is_empty());
    let last_event_age_ms = latest.and_then(event_time).and_then(|ts| {
        if now < ts {
            None
        } else {
            Some((now - ts).whole_milliseconds().max(0) as u64)
        }
    });
    let state = latest.and_then(|event| event.state.clone());
    let phase = latest.and_then(|event| event.phase.clone());
    let stalled =
        !is_terminal_status(&record.status) && last_event_age_ms.is_some_and(|ms| ms >= 8_000);
    let block_reason = classify_block_reason(
        &record.status,
        phase.as_deref(),
        stalled,
        events,
        record.error_message.as_deref(),
    );
    let advice = build_watch_advice(
        &record.status,
        phase.as_deref(),
        block_reason.as_deref(),
        false,
    );
    let (wait_reasons, current_wait_reason) = collect_wait_reasons(events);

    GetAgentStatsOutput {
        handle_id: handle_id.to_string(),
        status: format!("{}", record.status),
        state,
        phase,
        last_event_at,
        last_event_age_ms,
        stalled,
        block_reason,
        advice,
        queue_ms,
        provider_probe_ms,
        workspace_prepare_ms,
        provider_boot_ms,
        execution_ms,
        first_output_ms,
        first_output_warned,
        first_output_warning_at,
        current_wait_reason,
        wait_reasons,
        wall_ms,
        usage: build_usage_output(record),
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
        let now = OffsetDateTime::now_utc();
        let runs = rows
            .into_iter()
            .take(limit)
            .map(|(handle_id, record)| {
                let events = load_run_events(self.state_dir(), &handle_id).unwrap_or_default();
                let runtime = build_event_runtime_snapshot(
                    &record.status,
                    &events,
                    now,
                    record.error_message.as_deref(),
                );
                build_run_view(handle_id, &record, Some(&runtime))
            })
            .collect();

        Ok(Json(ListRunsOutput { runs }))
    }

    #[tool(description = "Return normalized and native result for a run handle.")]
    pub async fn get_run_result(
        &self,
        Parameters(input): Parameters<GetRunResultInput>,
    ) -> std::result::Result<Json<RunView>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let events = load_run_events(self.state_dir(), &input.handle_id)?;
        let runtime = build_event_runtime_snapshot(
            &record.status,
            &events,
            OffsetDateTime::now_utc(),
            record.error_message.as_deref(),
        );
        Ok(Json(build_run_view(
            input.handle_id,
            &record,
            Some(&runtime),
        )))
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
            let events = load_run_events(self.state_dir(), &input.handle_id)?;
            let now = OffsetDateTime::now_utc();
            let runtime = build_event_runtime_snapshot(
                &record.status,
                &events,
                now,
                record.error_message.as_deref(),
            );
            let (current_phase, phase_age_ms) = current_phase_age_ms(&events, now);
            let block_reason = runtime.block_reason.clone();
            let phase_timeout_hit = input.phase_timeout_secs.is_some_and(|timeout_secs| {
                let matches_phase = input.phase.as_deref().is_none_or(|phase| {
                    current_phase
                        .as_deref()
                        .is_some_and(|current| current == phase)
                });
                matches_phase
                    && phase_age_ms
                        .is_some_and(|age_ms| age_ms >= timeout_secs.saturating_mul(1000))
            });
            if is_terminal_status(&record.status) {
                let advice = build_watch_advice(
                    &record.status,
                    current_phase.as_deref(),
                    block_reason.as_deref(),
                    false,
                );
                return Ok(Json(WatchRunOutput {
                    run: build_run_view(input.handle_id.clone(), &record, Some(&runtime)),
                    timed_out: false,
                    phase_timeout_hit: false,
                    block_reason,
                    advice,
                }));
            }
            if phase_timeout_hit {
                let advice = build_watch_advice(
                    &record.status,
                    current_phase.as_deref(),
                    block_reason.as_deref(),
                    true,
                );
                return Ok(Json(WatchRunOutput {
                    run: build_run_view(input.handle_id.clone(), &record, Some(&runtime)),
                    timed_out: true,
                    phase_timeout_hit: true,
                    block_reason,
                    advice,
                }));
            }
            if timeout_secs.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
                let advice = build_watch_advice(
                    &record.status,
                    current_phase.as_deref(),
                    block_reason.as_deref(),
                    false,
                );
                return Ok(Json(WatchRunOutput {
                    run: build_run_view(input.handle_id.clone(), &record, Some(&runtime)),
                    timed_out: true,
                    phase_timeout_hit: false,
                    block_reason,
                    advice,
                }));
            }
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    #[tool(description = "Read run events with incremental cursor support.")]
    pub async fn watch_agent_events(
        &self,
        Parameters(input): Parameters<WatchAgentEventsInput>,
    ) -> std::result::Result<Json<WatchAgentEventsOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let all_events = load_run_events(self.state_dir(), &input.handle_id)?;
        let now = OffsetDateTime::now_utc();
        let runtime = build_event_runtime_snapshot(
            &record.status,
            &all_events,
            now,
            record.error_message.as_deref(),
        );
        let (current_phase, current_phase_age_ms) = current_phase_age_ms(&all_events, now);
        let block_reason = runtime.block_reason.clone();
        let phase_timeout_hit = input.phase_timeout_secs.is_some_and(|timeout_secs| {
            let matches_phase = input.phase.as_deref().is_none_or(|phase| {
                current_phase
                    .as_deref()
                    .is_some_and(|current| current == phase)
            });
            matches_phase
                && current_phase_age_ms
                    .is_some_and(|age_ms| age_ms >= timeout_secs.saturating_mul(1000))
        });

        let mut events = all_events;

        if let Some(since_seq) = input.since_seq {
            events.retain(|event| event.seq.is_some_and(|seq| seq > since_seq));
        }
        if let Some(phase) = input.phase.as_deref() {
            events.retain(|event| event.phase.as_deref().is_some_and(|value| value == phase));
        }

        let limit = input.limit.unwrap_or(200).max(1);
        if events.len() > limit {
            let start = events.len() - limit;
            events = events.split_off(start);
        }

        let next_seq = events
            .iter()
            .filter_map(|event| event.seq)
            .max()
            .map(|seq| seq + 1)
            .or(input.since_seq);
        let advice = build_watch_advice(
            &record.status,
            current_phase.as_deref(),
            block_reason.as_deref(),
            phase_timeout_hit,
        );

        Ok(Json(WatchAgentEventsOutput {
            handle_id: input.handle_id,
            status: format!("{}", record.status),
            updated_at: format_time(record.updated_at),
            terminal: is_terminal_status(&record.status),
            events,
            next_seq,
            current_phase,
            current_phase_age_ms,
            phase_timeout_hit,
            block_reason,
            advice,
        }))
    }

    #[tool(description = "Return run stats summary including phase timings and stall signal.")]
    pub async fn get_agent_stats(
        &self,
        Parameters(input): Parameters<GetAgentStatsInput>,
    ) -> std::result::Result<Json<GetAgentStatsOutput>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let events = load_run_events(self.state_dir(), &input.handle_id)?;
        let output = build_agent_stats_output(
            &input.handle_id,
            &record,
            &events,
            OffsetDateTime::now_utc(),
        );
        Ok(Json(output))
    }

    #[tool(description = "Run an agent synchronously and return structured summary.")]
    pub async fn run_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<RunView>, ErrorData> {
        let (loaded, request, execution_policy) = self.prepare_run(input, RunMode::Sync)?;
        let probe_result = self.ensure_provider_ready(&loaded.spec.core.provider)?;
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
        let native_usage = result.native_usage;
        let retry_classification = map_retry_classification(&result.metadata);

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
            usage: native_usage,
            retry_classification: Some(retry_classification),
            execution_policy,
        };
        let output = build_run_view(handle_id.clone(), &record, None);
        drop(workspace_cleanup);
        self.upsert_and_persist_run(&handle_id, record).await?;

        Ok(Json(output))
    }

    #[tool(description = "Spawn an agent asynchronously and return handle_id immediately.")]
    pub async fn spawn_agent(
        &self,
        Parameters(input): Parameters<RunAgentInput>,
    ) -> std::result::Result<Json<RunView>, ErrorData> {
        let (loaded, request, execution_policy) = self.prepare_run(input, RunMode::Async)?;
        let accepted_agent_name = loaded.spec.core.name.clone();
        let accepted_task_brief = request.task_brief.clone();
        let handle_id = Uuid::now_v7().to_string();
        let queued_at = format_time(OffsetDateTime::now_utc());
        let running_record = RunRecord::running(
            request.task.clone(),
            Some(build_run_request_snapshot(&request)),
            Some(build_run_spec_snapshot(&loaded.spec)),
            None,
            Some(execution_policy),
        );
        let lock_keys = self.conflict_lock_keys(&loaded.spec, &request)?;
        let provider_prober = self.provider_prober();

        self.upsert_and_persist_run(&handle_id, running_record)
            .await?;
        append_run_event(
            self.state_dir(),
            &handle_id,
            RuntimeEventInput {
                event: "run.accepted",
                state: "accepted",
                phase: "accepted",
                source: "runtime",
                message: "run accepted",
                detail: json!({
                    "agent": loaded.spec.core.name.clone(),
                    "provider": loaded.spec.core.provider.as_str(),
                }),
            },
        )?;
        append_run_event(
            self.state_dir(),
            &handle_id,
            RuntimeEventInput {
                event: "run.queued",
                state: "queued",
                phase: "queueing",
                source: "runtime",
                message: "run queued for async execution",
                detail: json!({
                    "queued_at": queued_at.clone(),
                }),
            },
        )?;

        let state = self.runtime_state();
        let state_dir = self.state_dir().to_path_buf();
        let task_handle_id = handle_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let _ = append_run_event(
                &state_dir,
                &task_handle_id,
                RuntimeEventInput {
                    event: "provider.probe.started",
                    state: "preparing",
                    phase: "provider_probe",
                    source: "provider",
                    message: "provider probe started",
                    detail: json!({
                        "provider": loaded.spec.core.provider.as_str(),
                    }),
                },
            );
            let probe_result = provider_prober.probe(&loaded.spec.core.provider);
            let probe_snapshot = build_probe_result_snapshot(&probe_result);
            let _ = append_run_event(
                &state_dir,
                &task_handle_id,
                RuntimeEventInput {
                    event: "provider.probe.completed",
                    state: "preparing",
                    phase: "provider_probe",
                    source: "provider",
                    message: "provider probe completed",
                    detail: json!({
                        "provider": probe_result.provider.as_str(),
                        "status": format!("{}", probe_result.status),
                        "version": probe_result.version.clone(),
                    }),
                },
            );
            if !probe_result.is_available() {
                let mut probe_details = vec![format!("status={}", probe_result.status)];
                if let Some(version) = &probe_result.version {
                    probe_details.push(format!("version={version}"));
                }
                probe_details.extend(probe_result.notes.clone());
                let error_message =
                    provider_unavailable_message(&loaded.spec.core.provider, &probe_details);
                let summary = failed_summary(error_message.clone());
                let (artifact_index, artifacts) = build_runtime_artifacts(&summary, "", "", None);

                let mut guard = state.lock().await;
                guard.tasks.remove(&task_handle_id);
                let Some(record) = guard.runs.get_mut(&task_handle_id) else {
                    return;
                };
                if matches!(record.status, RunStatus::Cancelled) {
                    return;
                }
                record.status = RunStatus::Failed;
                record.updated_at = OffsetDateTime::now_utc();
                append_status_if_terminal(&mut record.status_history, RunStatus::Failed);
                record.error_message = Some(error_message);
                record.summary = Some(summary);
                record.artifact_index = artifact_index;
                record.artifacts = artifacts;
                record.probe_result = Some(probe_snapshot);
                record.memory_resolution = None;
                record.workspace = None;
                record.compiled_context_markdown = None;
                record.usage = None;
                record.retry_classification = None;
                let _ = append_run_event(
                    &state_dir,
                    &task_handle_id,
                    RuntimeEventInput {
                        event: "run.failed",
                        state: "failed",
                        phase: "provider_probe",
                        source: "runtime",
                        message: "provider unavailable",
                        detail: json!({
                            "error": record.error_message.clone(),
                        }),
                    },
                );
                if let Err(err) = persist_run_record(&state_dir, &task_handle_id, record) {
                    record.error_message = Some(format!("failed to persist run state: {err}"));
                }
                return;
            }

            let _ = append_run_event(
                &state_dir,
                &task_handle_id,
                RuntimeEventInput {
                    event: "workspace.prepare.started",
                    state: "preparing",
                    phase: "workspace_prepare",
                    source: "workspace",
                    message: "workspace/context preparation started",
                    detail: json!({}),
                },
            );
            let _serialize_guards =
                acquire_serialize_locks_from_state(&state, lock_keys.clone()).await;
            let dispatch_started_at = Instant::now();
            let mut first_output_warned = false;
            let mut first_output_seen = false;
            let mut dispatch_future = Box::pin(run_dispatch(
                &loaded.spec,
                &request,
                &task_handle_id,
                &state_dir,
                lock_keys.clone(),
            ));
            let dispatch = loop {
                tokio::select! {
                    output = &mut dispatch_future => break output,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {
                        let elapsed_ms = dispatch_started_at.elapsed().as_millis() as u64;
                        if !first_output_seen {
                            if let Ok(events) = load_run_events(&state_dir, &task_handle_id) {
                                first_output_seen = has_event_named(&events, "provider.first_output");
                            }
                        }
                        let heartbeat_phase = if first_output_seen {
                            "running"
                        } else {
                            "provider_boot"
                        };
                        let _ = append_run_event(
                            &state_dir,
                            &task_handle_id,
                            RuntimeEventInput {
                                event: "provider.heartbeat",
                                state: "running",
                                phase: heartbeat_phase,
                                source: "runtime",
                                message: "still alive",
                                detail: json!({
                                    "elapsed_ms": elapsed_ms,
                                }),
                            },
                        );
                        if !first_output_warned
                            && !first_output_seen
                            && elapsed_ms >= FIRST_OUTPUT_WARN_AFTER_SECS.saturating_mul(1000)
                        {
                            first_output_warned = true;
                            let _ = append_run_event(
                                &state_dir,
                                &task_handle_id,
                                RuntimeEventInput {
                                    event: "provider.first_output.warning",
                                    state: "running",
                                    phase: "provider_boot",
                                    source: "runtime",
                                    message: "provider has not produced output yet",
                                    detail: json!({
                                        "elapsed_ms": elapsed_ms,
                                        "warn_after_secs": FIRST_OUTPUT_WARN_AFTER_SECS,
                                    }),
                                },
                            );
                        }
                    }
                }
            };

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
                    if !(dispatch_result.stdout.trim().is_empty()
                        && dispatch_result.stderr.trim().is_empty())
                    {
                        let existing_events =
                            load_run_events(&state_dir, &task_handle_id).unwrap_or_default();
                        let has_first_output =
                            has_event_named(&existing_events, "provider.first_output");
                        let has_stdout_delta =
                            has_event_named(&existing_events, "provider.stdout.delta");
                        let has_stderr_delta =
                            has_event_named(&existing_events, "provider.stderr.delta");

                        if !dispatch_result.stdout.trim().is_empty() && !has_stdout_delta {
                            let _ = append_run_event(
                                &state_dir,
                                &task_handle_id,
                                RuntimeEventInput {
                                    event: "provider.stdout.delta",
                                    state: "running",
                                    phase: "running",
                                    source: "provider",
                                    message: "provider stdout received",
                                    detail: json!({
                                        "bytes": dispatch_result.stdout.len(),
                                        "lines": dispatch_result.stdout.lines().count(),
                                    }),
                                },
                            );
                        }
                        if !dispatch_result.stderr.trim().is_empty() && !has_stderr_delta {
                            let _ = append_run_event(
                                &state_dir,
                                &task_handle_id,
                                RuntimeEventInput {
                                    event: "provider.stderr.delta",
                                    state: "running",
                                    phase: "running",
                                    source: "provider",
                                    message: "provider stderr received",
                                    detail: json!({
                                        "bytes": dispatch_result.stderr.len(),
                                        "lines": dispatch_result.stderr.lines().count(),
                                    }),
                                },
                            );
                        }
                        if !has_first_output {
                            let _ = append_run_event(
                                &state_dir,
                                &task_handle_id,
                                RuntimeEventInput {
                                    event: "provider.first_output",
                                    state: "running",
                                    phase: "running",
                                    source: "provider",
                                    message: "provider produced output",
                                    detail: json!({
                                        "stdout_bytes": dispatch_result.stdout.len(),
                                        "stderr_bytes": dispatch_result.stderr.len(),
                                    }),
                                },
                            );
                        }
                    }
                    let wait_text =
                        format!("{}\n{}", dispatch_result.stdout, dispatch_result.stderr);
                    if let Some(signal) = detect_provider_wait_signal(&wait_text) {
                        let _ = append_run_event(
                            &state_dir,
                            &task_handle_id,
                            RuntimeEventInput {
                                event: signal.event,
                                state: "running",
                                phase: signal.phase,
                                source: "provider",
                                message: signal.message,
                                detail: json!({
                                    "reason": signal.reason,
                                }),
                            },
                        );
                    }
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
                    let retry_classification = map_retry_classification(&dispatch_result.metadata);
                    record.status = dispatch_result.metadata.status;
                    record.updated_at = OffsetDateTime::now_utc();
                    record.status_history = dispatch_result.metadata.status_history;
                    record.error_message = dispatch_result.metadata.error_message;
                    record.summary = Some(dispatch_result.summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.probe_result = Some(probe_snapshot.clone());
                    record.memory_resolution = Some(memory_resolution);
                    record.workspace = Some(workspace);
                    record.compiled_context_markdown =
                        Some(dispatch_result.compiled_context_markdown);
                    record.usage = dispatch_result.native_usage;
                    record.retry_classification = Some(retry_classification);
                    let final_status = format!("{}", record.status);
                    let final_event = match record.status {
                        RunStatus::Succeeded => "run.completed",
                        RunStatus::Failed => "run.failed",
                        RunStatus::TimedOut => "run.timed_out",
                        RunStatus::Cancelled => "run.cancelled",
                        _ => "run.updated",
                    };
                    let _ = append_run_event(
                        &state_dir,
                        &task_handle_id,
                        RuntimeEventInput {
                            event: final_event,
                            state: final_status.as_str(),
                            phase: "completed",
                            source: "runtime",
                            message: "run finished",
                            detail: json!({
                                "status": final_status.clone(),
                                "verification_status": format!("{}", record.summary.as_ref().map(|v| v.summary.verification_status.clone()).unwrap_or(crate::runtime::summary::VerificationStatus::NotRun)),
                            }),
                        },
                    );
                    drop(workspace_cleanup);
                }
                Err(err) => {
                    let err_text = err.to_string();
                    if let Some(signal) = detect_provider_wait_signal(&err_text) {
                        let _ = append_run_event(
                            &state_dir,
                            &task_handle_id,
                            RuntimeEventInput {
                                event: signal.event,
                                state: "running",
                                phase: signal.phase,
                                source: "provider",
                                message: signal.message,
                                detail: json!({
                                    "reason": signal.reason,
                                }),
                            },
                        );
                    }
                    let summary = failed_summary(err.message.clone().into_owned());
                    let (artifact_index, artifacts) =
                        build_runtime_artifacts(&summary, "", "", None);
                    record.status = RunStatus::Failed;
                    record.updated_at = OffsetDateTime::now_utc();
                    append_status_if_terminal(&mut record.status_history, RunStatus::Failed);
                    record.error_message = Some(err_text);
                    record.summary = Some(summary);
                    record.artifact_index = artifact_index;
                    record.artifacts = artifacts;
                    record.probe_result = Some(probe_snapshot);
                    record.memory_resolution = None;
                    record.workspace = None;
                    record.compiled_context_markdown = None;
                    record.usage = None;
                    record.retry_classification = None;
                    let _ = append_run_event(
                        &state_dir,
                        &task_handle_id,
                        RuntimeEventInput {
                            event: "run.failed",
                            state: "failed",
                            phase: "execution",
                            source: "runtime",
                            message: "run failed",
                            detail: json!({
                                "error": record.error_message.clone(),
                            }),
                        },
                    );
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

        Ok(Json(RunView {
            handle_id,
            agent_name: accepted_agent_name,
            task_brief: accepted_task_brief,
            phase: "accepted".to_string(),
            terminal: false,
            outcome: None,
            created_at: queued_at.clone(),
            updated_at: queued_at,
        }))
    }

    #[tool(description = "Get current status for an async agent run.")]
    pub async fn get_agent_status(
        &self,
        Parameters(input): Parameters<HandleInput>,
    ) -> std::result::Result<Json<RunView>, ErrorData> {
        let record = self.get_or_load_run_record(&input.handle_id).await?;
        let events = load_run_events(self.state_dir(), &input.handle_id)?;
        let runtime = build_event_runtime_snapshot(
            &record.status,
            &events,
            OffsetDateTime::now_utc(),
            record.error_message.as_deref(),
        );

        Ok(Json(build_run_view(
            input.handle_id,
            &record,
            Some(&runtime),
        )))
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
            append_run_event(
                self.state_dir(),
                &input.handle_id,
                RuntimeEventInput {
                    event: "run.cancelled",
                    state: "cancelled",
                    phase: "completed",
                    source: "runtime",
                    message: "run cancelled by user request",
                    detail: json!({}),
                },
            )?;
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

#[cfg(test)]
mod tests {
    use super::{
        build_watch_advice, classify_block_reason, classify_block_reason_from_events,
        classify_block_reason_from_text, collect_wait_reasons, current_phase_age_ms,
        detect_provider_wait_signal, parse_rfc3339, RunEventOutput,
    };
    use crate::runtime::dispatcher::RunStatus;

    #[test]
    fn detect_provider_wait_signal_matches_trust_prompt() {
        let signal = detect_provider_wait_signal("This folder is not trusted folder yet")
            .expect("expected trust signal");
        assert_eq!(signal.event, "provider.waiting_for_trust");
        assert_eq!(signal.reason, "trust_required");
    }

    #[test]
    fn classify_block_reason_from_events_recognizes_first_output_warning() {
        let events = vec![RunEventOutput {
            seq: Some(1),
            event: "provider.first_output.warning".to_string(),
            timestamp: "2026-03-25T00:00:00Z".to_string(),
            state: Some("running".to_string()),
            phase: Some("provider_boot".to_string()),
            source: Some("runtime".to_string()),
            message: Some("provider has not produced output yet".to_string()),
            detail: serde_json::json!({}),
        }];
        let reason = classify_block_reason_from_events(&events, true);
        assert_eq!(reason, Some("provider_output_wait"));
    }

    #[test]
    fn detect_provider_wait_signal_ignores_cached_credentials_log() {
        let signal = detect_provider_wait_signal(
            "Using FileKeychain fallback for secure storage. Loaded cached credentials.",
        );
        assert!(signal.is_none());
    }

    #[test]
    fn classify_block_reason_is_none_for_succeeded_status() {
        let events = vec![RunEventOutput {
            seq: Some(1),
            event: "provider.waiting_for_auth".to_string(),
            timestamp: "2026-03-25T00:00:00Z".to_string(),
            state: Some("running".to_string()),
            phase: Some("waiting_for_auth".to_string()),
            source: Some("provider".to_string()),
            message: Some("provider is waiting for authentication".to_string()),
            detail: serde_json::json!({}),
        }];
        let reason = classify_block_reason(
            &RunStatus::Succeeded,
            Some("completed"),
            false,
            &events,
            None,
        );
        assert!(reason.is_none());
    }

    #[test]
    fn classify_block_reason_from_text_ignores_cached_credentials_log() {
        let reason = classify_block_reason_from_text(
            "Loaded cached credentials. Using FileKeychain fallback for secure storage.",
        );
        assert!(reason.is_none());
    }

    #[test]
    fn collect_wait_reasons_deduplicates_and_tracks_latest() {
        let events = vec![
            RunEventOutput {
                seq: Some(1),
                event: "provider.waiting_for_auth".to_string(),
                timestamp: "2026-03-25T00:00:00Z".to_string(),
                state: None,
                phase: None,
                source: None,
                message: None,
                detail: serde_json::json!({}),
            },
            RunEventOutput {
                seq: Some(2),
                event: "provider.waiting_for_auth".to_string(),
                timestamp: "2026-03-25T00:00:01Z".to_string(),
                state: None,
                phase: None,
                source: None,
                message: None,
                detail: serde_json::json!({}),
            },
            RunEventOutput {
                seq: Some(3),
                event: "provider.waiting_for_trust".to_string(),
                timestamp: "2026-03-25T00:00:02Z".to_string(),
                state: None,
                phase: None,
                source: None,
                message: None,
                detail: serde_json::json!({}),
            },
        ];
        let (reasons, current) = collect_wait_reasons(&events);
        assert_eq!(
            reasons,
            vec!["auth_required".to_string(), "trust_required".to_string()]
        );
        assert_eq!(current.as_deref(), Some("trust_required"));
    }

    #[test]
    fn current_phase_age_ms_tracks_latest_phase_window() {
        let events = vec![
            RunEventOutput {
                seq: Some(1),
                event: "provider.probe.started".to_string(),
                timestamp: "2026-03-25T00:00:01Z".to_string(),
                state: Some("preparing".to_string()),
                phase: Some("provider_probe".to_string()),
                source: Some("provider".to_string()),
                message: None,
                detail: serde_json::json!({}),
            },
            RunEventOutput {
                seq: Some(2),
                event: "provider.boot.started".to_string(),
                timestamp: "2026-03-25T00:00:03Z".to_string(),
                state: Some("running".to_string()),
                phase: Some("provider_boot".to_string()),
                source: Some("provider".to_string()),
                message: None,
                detail: serde_json::json!({}),
            },
        ];
        let now = parse_rfc3339("2026-03-25T00:00:08Z").expect("parse now");
        let (phase, age_ms) = current_phase_age_ms(&events, now);
        assert_eq!(phase.as_deref(), Some("provider_boot"));
        assert_eq!(age_ms, Some(5_000));
    }

    #[test]
    fn build_watch_advice_includes_timeout_and_reason_guidance() {
        let advice = build_watch_advice(
            &RunStatus::Running,
            Some("provider_boot"),
            Some("auth_required"),
            true,
        );
        assert!(
            advice
                .iter()
                .any(|item| item.contains("phase timeout hit in `provider_boot`")),
            "{advice:?}"
        );
        assert!(
            advice.iter().any(|item| item.contains("authentication")),
            "{advice:?}"
        );
    }

    #[test]
    fn build_watch_advice_includes_terminal_next_step() {
        let advice = build_watch_advice(&RunStatus::Succeeded, Some("completed"), None, false);
        assert!(
            advice.iter().any(|item| item.contains("get_run_result")),
            "{advice:?}"
        );
    }
}
