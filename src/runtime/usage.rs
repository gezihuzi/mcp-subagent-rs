use serde::{Deserialize, Serialize};

use crate::spec::Provider;

const INPUT_TOKEN_KEYS: &[&str] = &[
    "input_tokens",
    "input tokens",
    "prompt_tokens",
    "prompt tokens",
    "inputtokencount",
    "prompttokencount",
    "tokens in",
];
const OUTPUT_TOKEN_KEYS: &[&str] = &[
    "output_tokens",
    "output tokens",
    "completion_tokens",
    "completion tokens",
    "outputtokencount",
    "completiontokencount",
    "candidatestokencount",
    "tokens out",
];
const TOTAL_TOKEN_KEYS: &[&str] = &[
    "total_tokens",
    "total tokens",
    "totaltokencount",
    "total token count",
];
const CODEX_TOTAL_TOKEN_KEYS: &[&str] = &["tokens used", "tokens_used"];

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
    if usage.input_tokens.is_none() {
        usage.input_tokens = find_usage_value(text, INPUT_TOKEN_KEYS);
    }
    if usage.output_tokens.is_none() {
        usage.output_tokens = find_usage_value(text, OUTPUT_TOKEN_KEYS);
    }
    if usage.total_tokens.is_none() {
        usage.total_tokens = find_usage_value(text, TOTAL_TOKEN_KEYS);
    }
}

fn parse_codex_tokens_used(text: &str, usage: &mut NativeUsage) {
    if usage.total_tokens.is_some() {
        return;
    }
    if let Some(value) = find_usage_value(text, CODEX_TOTAL_TOKEN_KEYS) {
        usage.total_tokens = Some(value);
    }
}

fn find_usage_value(text: &str, keys: &[&str]) -> Option<u64> {
    let lowered = text.to_ascii_lowercase();
    keys.iter()
        .find_map(|key| extract_number_after_key(&lowered, text, key))
}

fn extract_number_after_key(lowered_text: &str, original_text: &str, key: &str) -> Option<u64> {
    let key = key.to_ascii_lowercase();
    let mut offset = 0usize;
    while let Some(relative_idx) = lowered_text[offset..].find(&key) {
        let idx = offset + relative_idx;
        if !is_key_boundary(lowered_text.as_bytes(), idx, idx + key.len()) {
            offset = idx.saturating_add(key.len());
            continue;
        }
        let suffix = &original_text[idx + key.len()..];
        if let Some(value) = parse_usage_value_suffix(suffix) {
            return Some(value);
        }
        offset = idx.saturating_add(key.len());
    }
    None
}

fn is_key_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
    let before_ok = match start.checked_sub(1).and_then(|idx| bytes.get(idx).copied()) {
        Some(ch) => !is_key_char(ch),
        None => true,
    };
    let after_ok = match bytes.get(end).copied() {
        Some(ch) => !is_key_char(ch),
        None => true,
    };
    before_ok && after_ok
}

fn is_key_char(ch: u8) -> bool {
    (ch as char).is_ascii_alphanumeric() || ch == b'_'
}

fn parse_usage_value_suffix(text: &str) -> Option<u64> {
    let mut value_start = None;
    for (idx, ch) in text.char_indices() {
        if idx >= 96 {
            return None;
        }
        if ch.is_ascii_whitespace() || matches!(ch, ':' | '=' | '"' | '\'' | '`' | '-' | '>') {
            continue;
        }
        if ch.eq_ignore_ascii_case(&'n') {
            return None;
        }
        if ch.is_ascii_digit() {
            value_start = Some(idx);
            break;
        }
        return None;
    }
    let start = value_start?;
    parse_token_number(&text[start..])
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
        if matches!(ch, ',' | '_') && start.is_some() {
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
    fn parses_usage_from_json_keys() {
        let stdout = r#"{"usage":{"input_tokens":1234,"output_tokens":56,"total_tokens":1290}}"#;
        let usage = parse_native_usage(&Provider::Claude, stdout, "").expect("usage");
        assert_eq!(usage.input_tokens, Some(1234));
        assert_eq!(usage.output_tokens, Some(56));
        assert_eq!(usage.total_tokens, Some(1290));
    }

    #[test]
    fn parses_usage_from_camel_case_token_counts() {
        let stderr = r#"{"promptTokenCount":100,"candidatesTokenCount":25,"totalTokenCount":125}"#;
        let usage = parse_native_usage(&Provider::Gemini, "", stderr).expect("usage");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(25));
        assert_eq!(usage.total_tokens, Some(125));
    }

    #[test]
    fn does_not_treat_null_as_numeric_usage() {
        let stdout = r#"{"usage":{"input_tokens":null,"output_tokens":8,"total_tokens":8}}"#;
        let usage = parse_native_usage(&Provider::Claude, stdout, "").expect("usage");
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, Some(8));
        assert_eq!(usage.total_tokens, Some(8));
    }

    #[test]
    fn returns_none_when_no_usage_detected() {
        let usage = parse_native_usage(&Provider::Claude, "plain output", "no tokens here");
        assert!(usage.is_none());
    }
}
