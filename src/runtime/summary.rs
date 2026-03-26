use std::collections::HashSet;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runtime::outcome::{SuccessOutcome, UsageStats};

pub const SUMMARY_START_SENTINEL: &str = "<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>";
pub const SUMMARY_END_SENTINEL: &str = "<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>";
pub const SUMMARY_CONTRACT_VERSION: &str = "mcp-subagent.summary.v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    SummaryJson,
    ReportMarkdown,
    ReportJson,
    PatchDiff,
    StdoutText,
    StderrText,
    Other,
}

impl std::fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub description: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub enum VerificationStatus {
    NotRun,
    Passed,
    Failed,
    Partial,
    ParseFailed,
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StructuredSummary {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
    pub verification_status: VerificationStatus,
    pub touched_files: Vec<String>,
    #[serde(default)]
    pub plan_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub enum SummaryParseStatus {
    Validated,
    Degraded,
    Invalid,
}

impl std::fmt::Display for SummaryParseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryEnvelope {
    contract_version: String,
    parse_status: SummaryParseStatus,
    summary: StructuredSummary,
    #[serde(default)]
    raw_fallback_text: Option<String>,
}

impl StructuredSummary {
    pub fn into_success_outcome(
        self,
        parse_status: SummaryParseStatus,
        usage: UsageStats,
    ) -> SuccessOutcome {
        SuccessOutcome {
            summary: self.summary,
            key_findings: self.key_findings,
            touched_files: self.touched_files,
            next_steps: self.next_steps,
            open_questions: self.open_questions,
            artifacts: self.artifacts,
            verification: self.verification_status,
            usage,
            parse_status,
            plan_refs: self.plan_refs,
        }
    }

    pub fn to_success_outcome(
        &self,
        parse_status: SummaryParseStatus,
        usage: UsageStats,
    ) -> SuccessOutcome {
        self.clone().into_success_outcome(parse_status, usage)
    }
}

impl SummaryEnvelope {
    pub fn from_success_outcome(
        success: SuccessOutcome,
        exit_code: i32,
        raw_fallback_text: Option<String>,
    ) -> Self {
        let parse_status = success.parse_status.clone();
        Self {
            contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
            parse_status,
            summary: StructuredSummary {
                summary: success.summary,
                key_findings: success.key_findings,
                artifacts: success.artifacts,
                open_questions: success.open_questions,
                next_steps: success.next_steps,
                exit_code,
                verification_status: success.verification,
                touched_files: success.touched_files,
                plan_refs: success.plan_refs,
            },
            raw_fallback_text,
        }
    }

    pub fn parse_status(&self) -> &SummaryParseStatus {
        &self.parse_status
    }

    pub fn summary_text(&self) -> &str {
        &self.summary.summary
    }

    pub fn key_findings(&self) -> &[String] {
        &self.summary.key_findings
    }

    pub fn artifacts(&self) -> &[ArtifactRef] {
        &self.summary.artifacts
    }

    pub fn open_questions(&self) -> &[String] {
        &self.summary.open_questions
    }

    pub fn next_steps(&self) -> &[String] {
        &self.summary.next_steps
    }

    pub fn exit_code(&self) -> i32 {
        self.summary.exit_code
    }

    pub fn verification_status(&self) -> &VerificationStatus {
        &self.summary.verification_status
    }

    pub fn touched_files(&self) -> &[String] {
        &self.summary.touched_files
    }

    pub fn plan_refs(&self) -> &[String] {
        &self.summary.plan_refs
    }

    pub fn raw_fallback_text(&self) -> Option<&str> {
        self.raw_fallback_text.as_deref()
    }

    pub fn into_success_outcome(self, usage: UsageStats) -> SuccessOutcome {
        self.summary.into_success_outcome(self.parse_status, usage)
    }

    pub fn to_success_outcome(&self, usage: UsageStats) -> SuccessOutcome {
        self.summary
            .to_success_outcome(self.parse_status.clone(), usage)
    }
}

pub fn parse_summary_envelope(raw_stdout: &str, raw_stderr: &str) -> SummaryEnvelope {
    let mut invalid_candidate = None;
    if let Some(parsed) = parse_from_raw(raw_stdout, "stdout", &mut invalid_candidate) {
        return parsed;
    }
    if let Some(parsed) = parse_from_raw(raw_stderr, "stderr", &mut invalid_candidate) {
        return parsed;
    }
    if let Some((reason, raw_fallback_text)) = invalid_candidate {
        return invalid_envelope(&reason, raw_fallback_text);
    }

    degraded_envelope(
        "summary sentinels not found in stdout/stderr",
        fallback_raw_text(raw_stdout, raw_stderr),
    )
}

fn parse_from_raw(
    raw: &str,
    source: &str,
    invalid_candidate: &mut Option<(String, String)>,
) -> Option<SummaryEnvelope> {
    let mut seen = HashSet::new();
    for json_block in extract_json_blocks(raw) {
        let trimmed = json_block.trim();
        if trimmed.is_empty() || !seen.insert(trimmed) {
            continue;
        }

        if let Some(parsed) = parse_json_candidate(trimmed) {
            return Some(parsed);
        }

        *invalid_candidate = Some((
            format!("invalid summary json from {source}"),
            trimmed.to_string(),
        ));
    }

    None
}

fn parse_json_candidate(json_block: &str) -> Option<SummaryEnvelope> {
    serde_json::from_str::<SummaryEnvelope>(json_block).ok()
}

fn extract_json_blocks(raw: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut offset = 0usize;
    while let Some(start_rel) = raw[offset..].find(SUMMARY_START_SENTINEL) {
        let payload_start = offset + start_rel + SUMMARY_START_SENTINEL.len();
        let Some(end_rel) = raw[payload_start..].find(SUMMARY_END_SENTINEL) else {
            break;
        };
        let end = payload_start + end_rel;
        blocks.push(raw[payload_start..end].trim());
        offset = end + SUMMARY_END_SENTINEL.len();
    }
    blocks
}

fn degraded_envelope(reason: &str, raw_fallback_text: Option<String>) -> SummaryEnvelope {
    SummaryEnvelope {
        contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
        parse_status: SummaryParseStatus::Degraded,
        summary: fallback_summary(reason),
        raw_fallback_text,
    }
}

fn invalid_envelope(reason: &str, raw_fallback_text: String) -> SummaryEnvelope {
    SummaryEnvelope {
        contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
        parse_status: SummaryParseStatus::Invalid,
        summary: fallback_summary(reason),
        raw_fallback_text: Some(raw_fallback_text),
    }
}

fn fallback_summary(reason: &str) -> StructuredSummary {
    StructuredSummary {
        summary: "Structured summary parsing failed; generated degraded summary.".to_string(),
        key_findings: vec![reason.to_string()],
        artifacts: Vec::new(),
        open_questions: vec![
            "Check provider output and confirm sentinel-wrapped JSON exists.".to_string(),
        ],
        next_steps: vec![
            "Fix prompt contract or runner bridge to emit valid summary JSON.".to_string(),
        ],
        exit_code: 1,
        verification_status: VerificationStatus::NotRun,
        touched_files: Vec::new(),
        plan_refs: Vec::new(),
    }
}

fn fallback_raw_text(stdout: &str, stderr: &str) -> Option<String> {
    let merged = format!("stdout:\n{}\n\nstderr:\n{}", stdout.trim(), stderr.trim());
    let trimmed = merged.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_summary_envelope, SummaryEnvelope, SummaryParseStatus, VerificationStatus,
        SUMMARY_END_SENTINEL, SUMMARY_START_SENTINEL,
    };
    use crate::runtime::outcome::UsageStats;

    fn envelope_json() -> String {
        r#"{
  "contract_version": "mcp-subagent.summary.v2",
  "parse_status": "Validated",
  "summary": {
    "summary": "ok",
    "key_findings": ["a"],
    "artifacts": [],
    "open_questions": [],
    "next_steps": ["next"],
    "exit_code": 0,
    "verification_status": "Passed",
    "touched_files": ["src/main.rs"],
    "plan_refs": ["step-1"]
  },
  "raw_fallback_text": null
}"#
        .to_string()
    }

    #[test]
    fn parses_valid_envelope_from_stdout() {
        let stdout = format!(
            "prefix\n{start}\n{json}\n{end}\nsuffix",
            start = SUMMARY_START_SENTINEL,
            json = envelope_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope(&stdout, "");

        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
        assert_eq!(parsed.summary.exit_code, 0);
        assert_eq!(
            parsed.summary.verification_status,
            VerificationStatus::Passed
        );
    }

    #[test]
    fn falls_back_to_stderr_when_stdout_missing() {
        let stdout = format!(
            "{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            json = envelope_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope("no summary", &stdout);
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
    }

    #[test]
    fn marks_invalid_when_json_is_invalid() {
        let stdout = format!(
            "{start}\n{{ invalid json }}\n{end}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL
        );
        let parsed: SummaryEnvelope = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Invalid);
        assert!(parsed.raw_fallback_text.is_some());
        assert_eq!(
            parsed.summary.verification_status,
            VerificationStatus::NotRun
        );
    }

    #[test]
    fn marks_degraded_when_envelope_json_without_sentinels() {
        let parsed = parse_summary_envelope(&envelope_json(), "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Degraded);
    }

    #[test]
    fn marks_invalid_when_only_placeholder_sentinel_and_late_raw_json() {
        let stdout = format!(
            "OUTPUT SENTINELS\n{start}\n{{...valid json...}}\n{end}\nrunner logs\n{envelope}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL,
            envelope = envelope_json()
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Invalid);
        assert!(parsed.raw_fallback_text.is_some());
    }

    #[test]
    fn parses_second_sentinel_block_when_first_is_placeholder() {
        let stdout = format!(
            "{start}\n{{...valid json...}}\n{end}\n{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL,
            json = envelope_json()
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
    }

    #[test]
    fn marks_invalid_when_json_payload_inside_sentinel_is_not_envelope_contract() {
        let stdout = format!(
            "{start}\n{{\"name\":\"Octoclip\",\"url\":\"https://octoclip.app\"}}\n{end}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Invalid);
        assert!(parsed.raw_fallback_text.is_some());
        assert_eq!(
            parsed.summary.verification_status,
            VerificationStatus::NotRun
        );
    }

    #[test]
    fn converts_parsed_envelope_to_success_outcome() {
        let stdout = format!(
            "{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            json = envelope_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope(&stdout, "");
        let usage = UsageStats {
            duration_ms: 42,
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            provider_exit_code: Some(0),
        };

        let success = parsed.to_success_outcome(usage);
        assert_eq!(success.summary, "ok");
        assert_eq!(success.parse_status, SummaryParseStatus::Validated);
        assert_eq!(success.verification, VerificationStatus::Passed);
        assert_eq!(success.touched_files, vec!["src/main.rs".to_string()]);
        assert_eq!(success.usage.duration_ms, 42);
        assert_eq!(success.usage.provider_exit_code, Some(0));
    }

    #[test]
    fn marks_degraded_when_sentinel_missing() {
        let parsed = parse_summary_envelope("plain text", "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Degraded);
        assert_eq!(
            parsed.summary.verification_status,
            VerificationStatus::NotRun
        );
    }
}
