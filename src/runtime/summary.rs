use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuredSummary {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryEnvelope {
    pub contract_version: String,
    pub parse_status: SummaryParseStatus,
    pub summary: StructuredSummary,
    #[serde(default)]
    pub raw_fallback_text: Option<String>,
}

pub fn parse_summary_envelope(raw_stdout: &str, raw_stderr: &str) -> SummaryEnvelope {
    if let Some(json_block) = extract_json_block(raw_stdout) {
        return parse_or_degrade(json_block, "stdout");
    }

    if let Some(json_block) = extract_json_block(raw_stderr) {
        return parse_or_degrade(json_block, "stderr");
    }

    degraded_envelope(
        "summary sentinels not found in stdout/stderr",
        fallback_raw_text(raw_stdout, raw_stderr),
    )
}

fn parse_or_degrade(json_block: &str, source: &str) -> SummaryEnvelope {
    if let Ok(parsed_envelope) = serde_json::from_str::<SummaryEnvelope>(json_block) {
        return parsed_envelope;
    }

    if let Ok(parsed_summary) = serde_json::from_str::<StructuredSummary>(json_block) {
        return SummaryEnvelope {
            contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
            parse_status: SummaryParseStatus::Validated,
            summary: parsed_summary,
            raw_fallback_text: None,
        };
    }

    invalid_envelope(
        &format!("invalid summary json from {source}"),
        json_block.to_string(),
    )
}

fn extract_json_block(raw: &str) -> Option<&str> {
    let start = raw.find(SUMMARY_START_SENTINEL)?;
    let payload_start = start + SUMMARY_START_SENTINEL.len();
    let end = raw[payload_start..].find(SUMMARY_END_SENTINEL)? + payload_start;
    Some(raw[payload_start..end].trim())
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

    fn legacy_summary_json() -> String {
        r#"{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["next"],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": ["src/main.rs"],
  "plan_refs": ["step-1"]
}"#
        .to_string()
    }

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
    fn parses_valid_legacy_summary_from_stdout() {
        let stdout = format!(
            "prefix\n{start}\n{json}\n{end}\nsuffix",
            start = SUMMARY_START_SENTINEL,
            json = legacy_summary_json(),
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
    fn parses_valid_envelope_from_stdout() {
        let stdout = format!(
            "{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            json = envelope_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
    }

    #[test]
    fn falls_back_to_stderr_when_stdout_missing() {
        let stderr = format!(
            "{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            json = legacy_summary_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_summary_envelope("no summary", &stderr);
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
    fn marks_degraded_when_sentinel_missing() {
        let parsed = parse_summary_envelope("plain text", "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Degraded);
        assert_eq!(
            parsed.summary.verification_status,
            VerificationStatus::NotRun
        );
    }
}
