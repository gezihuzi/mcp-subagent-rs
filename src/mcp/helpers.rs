use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    mcp::{dto::SummaryOutput, state::PolicyValueSource},
    probe::{ProbeStatus, ProviderProbe},
    runtime::summary::{
        StructuredSummary, SummaryEnvelope, SummaryParseStatus, VerificationStatus,
        SUMMARY_CONTRACT_VERSION,
    },
    spec::{
        runtime_policy::{BackgroundPreference, SpawnPolicy},
        AgentSpec, Provider,
    },
    types::RunMode,
};

pub(crate) fn resolve_preferred_run_mode(spec: &AgentSpec) -> (RunMode, PolicyValueSource) {
    match (
        &spec.runtime.spawn_policy,
        &spec.runtime.background_preference,
    ) {
        (SpawnPolicy::Async, _) => (RunMode::Async, PolicyValueSource::Spec),
        (SpawnPolicy::Sync, BackgroundPreference::PreferBackground) => {
            (RunMode::Async, PolicyValueSource::Spec)
        }
        (SpawnPolicy::Sync, BackgroundPreference::PreferForeground) => {
            (RunMode::Sync, PolicyValueSource::Spec)
        }
    }
}

pub(crate) fn resolve_effective_run_mode(
    requested_run_mode: RunMode,
    preferred_run_mode: RunMode,
    preferred_source: PolicyValueSource,
) -> (RunMode, PolicyValueSource, bool) {
    match (requested_run_mode, preferred_run_mode) {
        (RunMode::Sync, RunMode::Async) => (RunMode::Async, preferred_source, true),
        (RunMode::Async, RunMode::Sync) => (RunMode::Async, PolicyValueSource::Override, false),
        (RunMode::Sync, RunMode::Sync) => (RunMode::Sync, preferred_source, false),
        (RunMode::Async, RunMode::Async) => (RunMode::Async, preferred_source, false),
    }
}

pub(crate) fn run_mode_label(mode: &RunMode) -> &'static str {
    match mode {
        RunMode::Sync => "sync",
        RunMode::Async => "async",
    }
}

pub(crate) fn provider_tier_note(provider: &Provider) -> &'static str {
    match provider {
        Provider::Mock => "provider_tier: mock (stable local debug path)",
        Provider::Claude => "provider_tier: beta",
        Provider::Codex => "provider_tier: primary",
        Provider::Gemini => "provider_tier: experimental",
        Provider::Ollama => "provider_tier: local (community runner path)",
    }
}

pub(crate) fn build_capability_notes(probe: &ProviderProbe) -> Vec<String> {
    let mut notes = Vec::new();
    notes.push(provider_tier_note(&probe.provider).to_string());
    notes.push(format!("probe_status: {}", probe.status));
    if let Some(version) = &probe.version {
        notes.push(format!("detected_version: {version}"));
    }
    if matches!(probe.status, ProbeStatus::MissingBinary) {
        notes.push(format!(
            "install `{}` and ensure it is in PATH",
            probe.executable.display()
        ));
    }
    for note in &probe.notes {
        if !notes.iter().any(|existing| existing == note) {
            notes.push(note.clone());
        }
    }
    notes
}

pub(crate) fn map_summary_output(summary: &SummaryEnvelope) -> SummaryOutput {
    SummaryOutput {
        contract_version: summary.contract_version.clone(),
        parse_status: format!("{}", summary.parse_status),
        summary: summary.summary.summary.clone(),
        key_findings: summary.summary.key_findings.clone(),
        open_questions: summary.summary.open_questions.clone(),
        next_steps: summary.summary.next_steps.clone(),
        exit_code: summary.summary.exit_code,
        verification_status: format!("{}", summary.summary.verification_status),
        touched_files: summary.summary.touched_files.clone(),
        plan_refs: summary.summary.plan_refs.clone(),
    }
}

pub(crate) fn failed_summary(message: String) -> SummaryEnvelope {
    SummaryEnvelope {
        contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
        parse_status: SummaryParseStatus::Invalid,
        summary: StructuredSummary {
            summary: "Run failed before structured output was collected.".to_string(),
            key_findings: vec![message.clone()],
            artifacts: Vec::new(),
            open_questions: vec!["Inspect server logs for failure details.".to_string()],
            next_steps: vec!["Retry the run with corrected configuration.".to_string()],
            exit_code: 1,
            verification_status: VerificationStatus::NotRun,
            touched_files: Vec::new(),
            plan_refs: Vec::new(),
        },
        raw_fallback_text: Some(message),
    }
}

pub(crate) fn cancelled_summary(task: String) -> SummaryEnvelope {
    SummaryEnvelope {
        contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
        parse_status: SummaryParseStatus::Degraded,
        summary: StructuredSummary {
            summary: format!("Run cancelled before completion for task: {task}"),
            key_findings: vec!["User requested cancellation".to_string()],
            artifacts: Vec::new(),
            open_questions: Vec::new(),
            next_steps: vec!["Re-run the task if cancellation was accidental.".to_string()],
            exit_code: 130,
            verification_status: VerificationStatus::NotRun,
            touched_files: Vec::new(),
            plan_refs: Vec::new(),
        },
        raw_fallback_text: None,
    }
}

pub(crate) fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
