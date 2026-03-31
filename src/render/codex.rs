use std::collections::BTreeSet;

use crate::mcp::dto::OutcomeView;

const EMPTY_ITEM: &str = "- 无";

pub fn render_codex_outcome(outcome: Option<&OutcomeView>) -> String {
    match outcome {
        Some(OutcomeView::Succeeded {
            summary,
            key_findings,
            touched_files,
            ..
        }) => render_succeeded(summary, key_findings, touched_files),
        Some(OutcomeView::Failed { error, .. }) => format!("状态: failed\n错误: {error}"),
        Some(OutcomeView::Cancelled { reason }) => format!("状态: cancelled\n原因: {reason}"),
        Some(OutcomeView::TimedOut { elapsed_secs }) => {
            format!("状态: timed_out\n超时: {elapsed_secs}s")
        }
        None => "状态: unknown".to_string(),
    }
}

fn render_succeeded(summary: &str, key_findings: &[String], touched_files: &[String]) -> String {
    let mut p1 = Vec::new();
    let mut p2 = Vec::new();
    for finding in key_findings {
        if is_p1_finding(finding) {
            p1.push(finding.clone());
        } else {
            p2.push(finding.clone());
        }
    }

    let mut lines = vec![format!("Summary: {summary}"), String::new()];
    lines.push("P1 — 必须修复".to_string());
    if p1.is_empty() {
        lines.push(EMPTY_ITEM.to_string());
    } else {
        lines.extend(p1.into_iter().map(|item| format!("- {item}")));
    }

    lines.push(String::new());
    lines.push("P2 — 重要优化".to_string());
    if p2.is_empty() {
        lines.push(EMPTY_ITEM.to_string());
    } else {
        lines.extend(p2.into_iter().map(|item| format!("- {item}")));
    }

    lines.push(String::new());
    if touched_files.is_empty() {
        lines.push("Update(无文件变更)".to_string());
    } else {
        let unique = touched_files.iter().collect::<BTreeSet<_>>();
        for path in unique {
            lines.push(format!("Update({path})"));
        }
    }

    lines.push(String::new());
    lines.push("是否开始应用修改？".to_string());
    lines.join("\n")
}

fn is_p1_finding(finding: &str) -> bool {
    let lowered = finding.to_ascii_lowercase();
    lowered.starts_with("p1")
        || lowered.contains("critical")
        || lowered.contains("blocker")
        || lowered.contains("must fix")
        || lowered.contains("security")
        || lowered.contains("data loss")
        || finding.contains("必须")
        || finding.contains("阻塞")
        || finding.contains("严重")
}

#[cfg(test)]
mod tests {
    use crate::mcp::dto::{OutcomeView, RunUsageOutput};

    use super::render_codex_outcome;

    fn usage() -> RunUsageOutput {
        RunUsageOutput {
            started_at: None,
            finished_at: None,
            duration_ms: None,
            provider: "mock".to_string(),
            model: None,
            provider_exit_code: None,
            retries: 0,
            token_source: "unknown".to_string(),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            estimated_prompt_bytes: None,
            estimated_output_bytes: None,
        }
    }

    #[test]
    fn renders_success_with_p1_p2_and_updates() {
        let rendered = render_codex_outcome(Some(&OutcomeView::Succeeded {
            summary: "done".to_string(),
            key_findings: vec![
                "P1: must fix auth bug".to_string(),
                "Improve copywriting clarity".to_string(),
            ],
            touched_files: vec![
                "src/app.ts".to_string(),
                "src/app.ts".to_string(),
                "README.md".to_string(),
            ],
            artifacts: Vec::new(),
            usage: usage(),
        }));
        assert!(rendered.contains("P1 — 必须修复"));
        assert!(rendered.contains("- P1: must fix auth bug"));
        assert!(rendered.contains("P2 — 重要优化"));
        assert!(rendered.contains("- Improve copywriting clarity"));
        assert!(rendered.contains("Update(README.md)"));
        assert!(rendered.contains("Update(src/app.ts)"));
        assert!(rendered.contains("是否开始应用修改？"));
    }

    #[test]
    fn renders_failure_status() {
        let rendered = render_codex_outcome(Some(&OutcomeView::Failed {
            error: "provider unavailable".to_string(),
            retry_classification: "unknown".to_string(),
            partial_summary: None,
            usage: usage(),
        }));
        assert!(rendered.contains("状态: failed"));
        assert!(rendered.contains("provider unavailable"));
    }
}
