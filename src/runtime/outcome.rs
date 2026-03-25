use serde::{Deserialize, Serialize};

use crate::runtime::summary::{ArtifactRef, SummaryParseStatus, VerificationStatus};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetryClassification {
    Retryable,
    NonRetryable,
    #[default]
    Unknown,
}

impl std::fmt::Display for RetryClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Retryable => "retryable",
            Self::NonRetryable => "non_retryable",
            Self::Unknown => "unknown",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// RunOutcome — 终态不可变，一旦写入只读
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunOutcome {
    Succeeded(SuccessOutcome),
    Failed(FailureOutcome),
    Cancelled { reason: String },
    TimedOut { elapsed_secs: u64 },
}

impl RunOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded(_))
    }

    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed(f) => Some(&f.error),
            Self::Cancelled { reason } => Some(reason),
            Self::TimedOut { .. } => Some("runner exceeded timeout"),
            Self::Succeeded(_) => None,
        }
    }

    pub fn summary_text(&self) -> Option<&str> {
        match self {
            Self::Succeeded(s) => Some(&s.summary),
            Self::Failed(f) => f.partial_summary.as_deref(),
            _ => None,
        }
    }

    pub fn usage(&self) -> &UsageStats {
        match self {
            Self::Succeeded(s) => &s.usage,
            Self::Failed(f) => &f.usage,
            Self::Cancelled { .. } | Self::TimedOut { .. } => &UsageStats::ZERO,
        }
    }
}

// ---------------------------------------------------------------------------
// SuccessOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SuccessOutcome {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub touched_files: Vec<String>,
    pub next_steps: Vec<String>,
    pub open_questions: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
    pub verification: VerificationStatus,
    pub usage: UsageStats,
    pub parse_status: SummaryParseStatus,
    #[serde(default)]
    pub plan_refs: Vec<String>,
}

// ---------------------------------------------------------------------------
// FailureOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FailureOutcome {
    pub error: String,
    pub retry: RetryInfo,
    pub partial_summary: Option<String>,
    pub usage: UsageStats,
}

// ---------------------------------------------------------------------------
// RetryInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetryInfo {
    pub classification: RetryClassification,
    #[serde(default)]
    pub reason: Option<String>,
    pub attempts_used: u32,
}

// ---------------------------------------------------------------------------
// UsageStats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct UsageStats {
    pub duration_ms: u64,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub provider_exit_code: Option<i32>,
}

impl UsageStats {
    pub const ZERO: UsageStats = UsageStats {
        duration_ms: 0,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        provider_exit_code: None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_outcome_success_accessors() {
        let outcome = RunOutcome::Succeeded(SuccessOutcome {
            summary: "done".to_string(),
            key_findings: vec!["a".to_string()],
            touched_files: vec![],
            next_steps: vec![],
            open_questions: vec![],
            artifacts: vec![],
            verification: VerificationStatus::Passed,
            usage: UsageStats::ZERO,
            parse_status: SummaryParseStatus::Validated,
            plan_refs: vec![],
        });
        assert!(outcome.is_success());
        assert_eq!(outcome.summary_text(), Some("done"));
        assert!(outcome.error_message().is_none());
    }

    #[test]
    fn run_outcome_failure_accessors() {
        let outcome = RunOutcome::Failed(FailureOutcome {
            error: "boom".to_string(),
            retry: RetryInfo {
                classification: RetryClassification::NonRetryable,
                reason: Some("bad request".to_string()),
                attempts_used: 1,
            },
            partial_summary: Some("partial".to_string()),
            usage: UsageStats::ZERO,
        });
        assert!(!outcome.is_success());
        assert_eq!(outcome.error_message(), Some("boom"));
        assert_eq!(outcome.summary_text(), Some("partial"));
    }

    #[test]
    fn run_outcome_serialization_roundtrip() {
        let outcome = RunOutcome::Succeeded(SuccessOutcome {
            summary: "ok".to_string(),
            key_findings: vec![],
            touched_files: vec![],
            next_steps: vec![],
            open_questions: vec![],
            artifacts: vec![],
            verification: VerificationStatus::Passed,
            usage: UsageStats {
                duration_ms: 1234,
                input_tokens: Some(100),
                output_tokens: Some(200),
                total_tokens: Some(300),
                provider_exit_code: Some(0),
            },
            parse_status: SummaryParseStatus::Validated,
            plan_refs: vec![],
        });
        let json = serde_json::to_string(&outcome).expect("serialize");
        let deserialized: RunOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, deserialized);
    }
}
