use std::{
    io::ErrorKind,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use serde::{Deserialize, Serialize};

use crate::spec::Provider;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ProbeStatus {
    Ready,
    MissingBinary,
    PermissionDenied,
    UnsupportedVersion,
    NeedsAuthentication,
    ExperimentalUnavailable,
    ProbeFailed,
}

impl ProbeStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

impl std::fmt::Display for ProbeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "Ready"),
            Self::MissingBinary => write!(f, "MissingBinary"),
            Self::PermissionDenied => write!(f, "PermissionDenied"),
            Self::UnsupportedVersion => write!(f, "UnsupportedVersion"),
            Self::NeedsAuthentication => write!(f, "NeedsAuthentication"),
            Self::ExperimentalUnavailable => write!(f, "ExperimentalUnavailable"),
            Self::ProbeFailed => write!(f, "ProbeFailed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub supports_background_native: bool,
    pub supports_native_project_memory: bool,
    pub experimental: bool,
}

impl ProviderCapabilities {
    fn for_provider(provider: &Provider) -> Self {
        match provider {
            Provider::Mock => Self {
                supports_background_native: false,
                supports_native_project_memory: false,
                experimental: false,
            },
            Provider::Claude => Self {
                supports_background_native: true,
                supports_native_project_memory: true,
                experimental: false,
            },
            Provider::Codex => Self {
                supports_background_native: false,
                supports_native_project_memory: true,
                experimental: false,
            },
            Provider::Gemini => Self {
                supports_background_native: false,
                supports_native_project_memory: true,
                experimental: true,
            },
            Provider::Ollama => Self {
                supports_background_native: false,
                supports_native_project_memory: false,
                experimental: false,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbe {
    pub provider: Provider,
    pub executable: PathBuf,
    #[serde(default)]
    pub version: Option<String>,
    pub status: ProbeStatus,
    pub capabilities: ProviderCapabilities,
    #[serde(default)]
    pub validated_flags: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl ProviderProbe {
    pub fn is_available(&self) -> bool {
        self.status.is_ready()
    }
}

pub trait ProviderProber: Send + Sync + std::fmt::Debug {
    fn probe(&self, provider: &Provider) -> ProviderProbe;
}

#[derive(Debug, Clone, Default)]
pub struct SystemProviderProber;

impl ProviderProber for SystemProviderProber {
    fn probe(&self, provider: &Provider) -> ProviderProbe {
        if matches!(provider, Provider::Mock) {
            return ProviderProbe {
                provider: provider.clone(),
                executable: PathBuf::from("<builtin:mock>"),
                version: Some("builtin".to_string()),
                status: ProbeStatus::Ready,
                capabilities: ProviderCapabilities::for_provider(provider),
                validated_flags: Vec::new(),
                notes: vec![
                    provider_tier_note(provider).to_string(),
                    "built-in mock runner".to_string(),
                ],
            };
        }

        let executable = default_executable(provider);
        let capabilities = ProviderCapabilities::for_provider(provider);
        let validated_flags = validated_flags_for_provider(provider);

        let mut notes = vec![provider_tier_note(provider).to_string()];
        for mapping_note in provider_cli_mapping_notes(provider) {
            notes.push((*mapping_note).to_string());
        }
        if capabilities.experimental {
            notes
                .push("provider support is experimental and may change across CLI versions".into());
        }

        let output = Command::new(&executable)
            .arg("--version")
            .stdin(Stdio::null())
            .output();

        match output {
            Ok(output) => {
                let version = extract_version_line(&output);
                let mut combined = String::new();
                combined.push_str(&String::from_utf8_lossy(&output.stdout));
                combined.push('\n');
                combined.push_str(&String::from_utf8_lossy(&output.stderr));
                let mut inferred = infer_probe_status(
                    &combined,
                    capabilities.experimental,
                    output.status.success(),
                );
                if output.status.success()
                    && matches!(inferred, ProbeStatus::PermissionDenied)
                    && version
                        .as_deref()
                        .map(is_likely_version_line)
                        .unwrap_or(false)
                {
                    inferred = ProbeStatus::Ready;
                    notes.push(
                        "probe emitted non-fatal permission warning; treated as ready".to_string(),
                    );
                }

                if output.status.success() && matches!(inferred, ProbeStatus::Ready) {
                    ProviderProbe {
                        provider: provider.clone(),
                        executable,
                        version,
                        status: ProbeStatus::Ready,
                        capabilities,
                        validated_flags,
                        notes,
                    }
                } else {
                    let status = inferred;
                    if let Some(line) = extract_first_error_line(&output) {
                        notes.push(line);
                    }
                    ProviderProbe {
                        provider: provider.clone(),
                        executable,
                        version,
                        status,
                        capabilities,
                        validated_flags,
                        notes,
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                notes.push(format!(
                    "binary `{}` not found in PATH",
                    executable.display()
                ));
                ProviderProbe {
                    provider: provider.clone(),
                    executable,
                    version: None,
                    status: ProbeStatus::MissingBinary,
                    capabilities,
                    validated_flags,
                    notes,
                }
            }
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                notes.push(format!(
                    "permission denied when executing `{}`: {err}",
                    executable.display()
                ));
                ProviderProbe {
                    provider: provider.clone(),
                    executable,
                    version: None,
                    status: ProbeStatus::PermissionDenied,
                    capabilities,
                    validated_flags,
                    notes,
                }
            }
            Err(err) => {
                notes.push(format!("failed to execute probe command: {err}"));
                ProviderProbe {
                    provider: provider.clone(),
                    executable,
                    version: None,
                    status: ProbeStatus::ProbeFailed,
                    capabilities,
                    validated_flags,
                    notes,
                }
            }
        }
    }
}

fn validated_flags_for_provider(provider: &Provider) -> Vec<String> {
    let flags: &[&str] = match provider {
        Provider::Mock => &[],
        Provider::Claude => &[
            "--permission-mode",
            "--add-dir",
            "--output-format",
            "--json-schema",
        ],
        Provider::Codex => &[
            "--sandbox",
            "--ask-for-approval",
            "--output-last-message",
            "--output-schema",
        ],
        Provider::Gemini => &[
            "--approval-mode",
            "--include-directories",
            "--output-format",
        ],
        Provider::Ollama => &[],
    };

    flags.iter().map(|flag| flag.to_string()).collect()
}

fn provider_cli_mapping_notes(provider: &Provider) -> &'static [&'static str] {
    match provider {
        Provider::Mock => &[],
        Provider::Claude => &[
            "permission mapping: ReadOnly->plan, WorkspaceWrite->acceptEdits, FullAccess->bypassPermissions",
            "permission_mode override allowlist: default|acceptEdits|plan|dontAsk|bypassPermissions",
        ],
        Provider::Codex => &[],
        Provider::Gemini => &[
            "approval mapping: ReadOnly->default, WorkspaceWrite->auto_edit, FullAccess->yolo",
        ],
        Provider::Ollama => &[],
    }
}

fn provider_tier_note(provider: &Provider) -> &'static str {
    match provider {
        Provider::Mock => "provider_tier: mock (stable local debug path)",
        Provider::Claude => "provider_tier: beta",
        Provider::Codex => "provider_tier: primary",
        Provider::Gemini => "provider_tier: experimental",
        Provider::Ollama => "provider_tier: local (community runner path)",
    }
}

fn default_executable(provider: &Provider) -> PathBuf {
    match provider {
        Provider::Mock => PathBuf::from("<builtin:mock>"),
        Provider::Claude => PathBuf::from("claude"),
        Provider::Codex => PathBuf::from("codex"),
        Provider::Gemini => PathBuf::from("gemini"),
        Provider::Ollama => PathBuf::from("ollama"),
    }
}

fn extract_version_line(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn extract_first_error_line(output: &Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn infer_probe_status(
    combined_output: &str,
    experimental_provider: bool,
    command_succeeded: bool,
) -> ProbeStatus {
    let text = combined_output.to_lowercase();
    if text.contains("permission denied")
        || text.contains("operation not permitted")
        || text.contains(" eperm")
        || text.contains(" eacces")
        || text.contains("errno: -1")
    {
        return ProbeStatus::PermissionDenied;
    }
    if text.contains("auth")
        || text.contains("login")
        || text.contains("api key")
        || text.contains("unauthorized")
    {
        return ProbeStatus::NeedsAuthentication;
    }
    if text.contains("unsupported version")
        || text.contains("not supported")
        || text.contains("requires version")
    {
        return ProbeStatus::UnsupportedVersion;
    }
    if experimental_provider && text.contains("experimental") {
        return ProbeStatus::ExperimentalUnavailable;
    }
    if command_succeeded {
        return ProbeStatus::Ready;
    }
    ProbeStatus::ProbeFailed
}

fn is_likely_version_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    if lower.contains("error") || lower.contains("failed") || lower.contains("exception") {
        return false;
    }
    line.chars().any(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use crate::spec::Provider;

    use super::{
        infer_probe_status, is_likely_version_line, provider_cli_mapping_notes, ProbeStatus,
    };

    #[test]
    fn classify_permission_denied() {
        let status = infer_probe_status(
            "Error: EPERM: operation not permitted, open '/Users/a/.gemini/projects.json'",
            true,
            false,
        );
        assert_eq!(status, ProbeStatus::PermissionDenied);
    }

    #[test]
    fn classify_auth_issue() {
        let status = infer_probe_status("login required: please authenticate first", false, false);
        assert_eq!(status, ProbeStatus::NeedsAuthentication);
    }

    #[test]
    fn classify_experimental_unavailable() {
        let status = infer_probe_status("this experimental feature is unavailable", true, false);
        assert_eq!(status, ProbeStatus::ExperimentalUnavailable);
    }

    #[test]
    fn classify_ready_when_command_succeeded_without_error_keywords() {
        let status = infer_probe_status("codex-cli 0.114.0", false, true);
        assert_eq!(status, ProbeStatus::Ready);
    }

    #[test]
    fn version_line_heuristic_rejects_error_text() {
        assert!(is_likely_version_line("codex-cli 0.114.0"));
        assert!(!is_likely_version_line("Error: failed to open file"));
    }

    #[test]
    fn gemini_mapping_notes_reflect_default_auto_edit_yolo() {
        let joined = provider_cli_mapping_notes(&Provider::Gemini).join(" ");
        assert!(joined.contains("ReadOnly->default"));
        assert!(joined.contains("WorkspaceWrite->auto_edit"));
        assert!(joined.contains("FullAccess->yolo"));
    }

    #[test]
    fn claude_mapping_notes_include_public_permission_modes() {
        let joined = provider_cli_mapping_notes(&Provider::Claude).join(" ");
        assert!(joined.contains("bypassPermissions"));
        assert!(joined.contains("dontAsk"));
        assert!(joined.contains("acceptEdits"));
    }
}
