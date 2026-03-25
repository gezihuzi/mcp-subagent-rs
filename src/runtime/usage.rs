use serde::{Deserialize, Serialize};

use crate::spec::Provider;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

impl NativeUsage {
    pub fn has_any_tokens(&self) -> bool {
        self.input_tokens.is_some() || self.output_tokens.is_some() || self.total_tokens.is_some()
    }
}

pub fn parse_native_usage(provider: &Provider, stdout: &str, stderr: &str) -> Option<NativeUsage> {
    let mut usage = NativeUsage::default();
    parse_generic_usage(stdout, &mut usage);
    parse_generic_usage(stderr, &mut usage);

    if matches!(provider, Provider::Codex) {
        parse_codex_tokens_used(stdout, &mut usage);
        parse_codex_tokens_used(stderr, &mut usage);
    }

    if usage.total_tokens.is_none() {
        usage.total_tokens = match (usage.input_tokens, usage.output_tokens) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => usage.total_tokens,
        };
    }

    usage.has_any_tokens().then_some(usage)
}

fn parse_generic_usage(text: &str, usage: &mut NativeUsage) {
    for line in text.lines() {
        let lowered = line.to_ascii_lowercase();

        if usage.input_tokens.is_none() {
            usage.input_tokens = extract_number_after_keyword(&lowered, line, "input tokens")
                .or_else(|| extract_number_after_keyword(&lowered, line, "prompt tokens"));
        }
        if usage.output_tokens.is_none() {
            usage.output_tokens = extract_number_after_keyword(&lowered, line, "output tokens")
                .or_else(|| extract_number_after_keyword(&lowered, line, "completion tokens"));
        }
        if usage.total_tokens.is_none() {
            usage.total_tokens = extract_number_after_keyword(&lowered, line, "total tokens");
        }
    }
}

fn parse_codex_tokens_used(text: &str, usage: &mut NativeUsage) {
    if usage.total_tokens.is_some() {
        return;
    }
    let lines = text.lines().collect::<Vec<_>>();
    for (idx, line) in lines.iter().enumerate() {
        let lowered = line.trim().to_ascii_lowercase();
        if !lowered.contains("tokens used") {
            continue;
        }
        if let Some(value) =
            extract_number_after_keyword(&lowered, line, "tokens used").or_else(|| {
                lines
                    .iter()
                    .skip(idx + 1)
                    .find_map(|next| parse_token_number(next.trim()))
            })
        {
            usage.total_tokens = Some(value);
            return;
        }
    }
}

fn extract_number_after_keyword(
    lowered_line: &str,
    original_line: &str,
    keyword: &str,
) -> Option<u64> {
    let idx = lowered_line.find(keyword)?;
    let slice = &original_line[idx + keyword.len()..];
    parse_token_number(slice)
}

fn parse_token_number(text: &str) -> Option<u64> {
    let mut start = None;
    let mut end = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            start.get_or_insert(idx);
            end = idx + ch.len_utf8();
            continue;
        }
        if ch == ',' && start.is_some() {
            end = idx + ch.len_utf8();
            continue;
        }
        if start.is_some() {
            break;
        }
    }
    let start = start?;
    let candidate = text[start..end].replace(',', "");
    candidate.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use crate::{runtime::usage::parse_native_usage, spec::Provider};

    #[test]
    fn parses_codex_tokens_used_multiline() {
        let stderr = "warning: network fallback\ntokens used\n40,005\n";
        let usage = parse_native_usage(&Provider::Codex, "", stderr).expect("usage");
        assert_eq!(usage.total_tokens, Some(40005));
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
    }

    #[test]
    fn parses_prompt_completion_and_total_tokens() {
        let stdout = "prompt tokens: 120\ncompletion tokens: 80\ntotal tokens: 200";
        let usage = parse_native_usage(&Provider::Gemini, stdout, "").expect("usage");
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(80));
        assert_eq!(usage.total_tokens, Some(200));
    }

    #[test]
    fn returns_none_when_no_usage_detected() {
        let usage = parse_native_usage(&Provider::Claude, "plain output", "no tokens here");
        assert!(usage.is_none());
    }
}
