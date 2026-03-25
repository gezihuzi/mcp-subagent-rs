use std::collections::HashSet;
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

impl std::fmt::Display for SummaryParseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
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

fn parse_from_raw<'a>(
    raw: &'a str,
    source: &str,
    invalid_candidate: &mut Option<(String, String)>,
) -> Option<SummaryEnvelope> {
    let mut seen = HashSet::new();
    for json_block in extract_json_blocks(raw)
        .into_iter()
        .chain(extract_json_objects(raw))
    {
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
    if let Ok(parsed_envelope) = serde_json::from_str::<SummaryEnvelope>(json_block) {
        return Some(parsed_envelope);
    }

    if let Ok(parsed_summary) = serde_json::from_str::<StructuredSummary>(json_block) {
        return Some(SummaryEnvelope {
            contract_version: SUMMARY_CONTRACT_VERSION.to_string(),
            parse_status: SummaryParseStatus::Validated,
            summary: parsed_summary,
            raw_fallback_text: None,
        });
    }

    None
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

fn extract_json_objects(raw: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut start_idx = None;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start_idx = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start_idx.take() {
                        let end = idx + ch.len_utf8();
                        objects.push(raw[start..end].trim());
                    }
                }
            }
            _ => {}
        }
    }

    objects
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
    fn parses_valid_json_without_sentinels() {
        let parsed = parse_summary_envelope(&legacy_summary_json(), "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
    }

    #[test]
    fn parses_late_valid_json_after_placeholder_sentinel_block() {
        let stdout = format!(
            "OUTPUT SENTINELS\n{start}\n{{...valid json...}}\n{end}\nrunner logs\n{envelope}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL,
            envelope = envelope_json()
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
    }

    #[test]
    fn parses_second_sentinel_block_when_first_is_placeholder() {
        let stdout = format!(
            "{start}\n{{...valid json...}}\n{end}\n{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL,
            json = legacy_summary_json()
        );
        let parsed = parse_summary_envelope(&stdout, "");
        assert_eq!(parsed.parse_status, SummaryParseStatus::Validated);
        assert_eq!(parsed.summary.summary, "ok");
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
