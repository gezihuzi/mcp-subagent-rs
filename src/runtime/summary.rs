use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const SUMMARY_START_SENTINEL: &str = "<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>";
pub const SUMMARY_END_SENTINEL: &str = "<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub description: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationStatus {
    NotRun,
    Passed,
    Failed,
    Partial,
    ParseFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
}

pub fn parse_structured_summary(raw_stdout: &str, raw_stderr: &str) -> StructuredSummary {
    if let Some(json_block) = extract_json_block(raw_stdout) {
        return parse_or_degrade(json_block, "stdout");
    }

    if let Some(json_block) = extract_json_block(raw_stderr) {
        return parse_or_degrade(json_block, "stderr");
    }

    degraded_summary(
        "summary sentinels not found in stdout/stderr",
        raw_stdout,
        raw_stderr,
    )
}

fn parse_or_degrade(json_block: &str, source: &str) -> StructuredSummary {
    match serde_json::from_str::<StructuredSummary>(json_block) {
        Ok(parsed) => parsed,
        Err(err) => degraded_summary(
            &format!("invalid summary json from {source}: {err}"),
            json_block,
            "",
        ),
    }
}

fn extract_json_block(raw: &str) -> Option<&str> {
    let start = raw.find(SUMMARY_START_SENTINEL)?;
    let payload_start = start + SUMMARY_START_SENTINEL.len();
    let end = raw[payload_start..].find(SUMMARY_END_SENTINEL)? + payload_start;
    Some(raw[payload_start..end].trim())
}

fn degraded_summary(reason: &str, stdout: &str, stderr: &str) -> StructuredSummary {
    let mut key_findings = vec![reason.to_string()];
    if !stdout.trim().is_empty() {
        key_findings.push("stdout contains fallback data for manual inspection".to_string());
    }
    if !stderr.trim().is_empty() {
        key_findings.push("stderr contains fallback data for manual inspection".to_string());
    }

    StructuredSummary {
        summary: "Structured summary parsing failed; generated degraded summary.".to_string(),
        key_findings,
        artifacts: Vec::new(),
        open_questions: vec![
            "Check provider output and confirm sentinel-wrapped JSON exists.".to_string(),
        ],
        next_steps: vec![
            "Fix prompt contract or runner bridge to emit valid summary JSON.".to_string(),
        ],
        exit_code: 1,
        verification_status: VerificationStatus::ParseFailed,
        touched_files: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_structured_summary, StructuredSummary, VerificationStatus, SUMMARY_END_SENTINEL,
        SUMMARY_START_SENTINEL,
    };

    fn summary_json() -> String {
        r#"{
  "summary": "ok",
  "key_findings": ["a"],
  "artifacts": [],
  "open_questions": [],
  "next_steps": ["next"],
  "exit_code": 0,
  "verification_status": "Passed",
  "touched_files": ["src/main.rs"]
}"#
        .to_string()
    }

    #[test]
    fn parses_valid_summary_from_stdout() {
        let stdout = format!(
            "prefix\n{start}\n{json}\n{end}\nsuffix",
            start = SUMMARY_START_SENTINEL,
            json = summary_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_structured_summary(&stdout, "");

        assert_eq!(parsed.summary, "ok");
        assert_eq!(parsed.exit_code, 0);
        assert_eq!(parsed.verification_status, VerificationStatus::Passed);
    }

    #[test]
    fn falls_back_to_stderr_when_stdout_missing() {
        let stderr = format!(
            "{start}\n{json}\n{end}",
            start = SUMMARY_START_SENTINEL,
            json = summary_json(),
            end = SUMMARY_END_SENTINEL
        );
        let parsed = parse_structured_summary("no summary", &stderr);
        assert_eq!(parsed.summary, "ok");
    }

    #[test]
    fn degrades_when_json_invalid() {
        let stdout = format!(
            "{start}\n{{ invalid json }}\n{end}",
            start = SUMMARY_START_SENTINEL,
            end = SUMMARY_END_SENTINEL
        );
        let parsed: StructuredSummary = parse_structured_summary(&stdout, "");
        assert_eq!(parsed.verification_status, VerificationStatus::ParseFailed);
        assert_eq!(parsed.exit_code, 1);
    }

    #[test]
    fn degrades_when_sentinel_missing() {
        let parsed = parse_structured_summary("plain text", "");
        assert_eq!(parsed.verification_status, VerificationStatus::ParseFailed);
    }
}
