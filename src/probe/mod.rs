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
        let executable = default_executable(provider);
        let capabilities = ProviderCapabilities::for_provider(provider);

        let mut notes = Vec::new();
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
                if output.status.success() {
                    ProviderProbe {
                        provider: provider.clone(),
                        executable,
                        version,
                        status: ProbeStatus::Ready,
                        capabilities,
                        notes,
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                    let status = if stderr.contains("auth") || stderr.contains("login") {
                        ProbeStatus::NeedsAuthentication
                    } else if capabilities.experimental && stderr.contains("experimental") {
                        ProbeStatus::ExperimentalUnavailable
                    } else {
                        ProbeStatus::ProbeFailed
                    };
                    if let Some(line) = extract_first_error_line(&output) {
                        notes.push(line);
                    }
                    ProviderProbe {
                        provider: provider.clone(),
                        executable,
                        version,
                        status,
                        capabilities,
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
                    notes,
                }
            }
        }
    }
}

fn default_executable(provider: &Provider) -> PathBuf {
    match provider {
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
