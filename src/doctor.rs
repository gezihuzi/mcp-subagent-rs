use std::path::PathBuf;

use crate::{
    probe::{ProviderProbe, ProviderProber},
    spec::{registry::load_agent_specs_from_dirs, Provider},
};

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub cwd: PathBuf,
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: PathBuf,
    pub agents_loaded: Option<usize>,
    pub agents_error: Option<String>,
    pub probes: Vec<ProviderProbe>,
}

pub fn build_doctor_report(
    agents_dirs: Vec<PathBuf>,
    state_dir: PathBuf,
    prober: &dyn ProviderProber,
) -> DoctorReport {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (agents_loaded, agents_error) = match load_agent_specs_from_dirs(&agents_dirs) {
        Ok(loaded) => (Some(loaded.len()), None),
        Err(err) => (None, Some(err.to_string())),
    };

    let probes = all_providers()
        .into_iter()
        .map(|provider| prober.probe(&provider))
        .collect();

    DoctorReport {
        cwd,
        agents_dirs,
        state_dir,
        agents_loaded,
        agents_error,
        probes,
    }
}

pub fn render_doctor_report(report: &DoctorReport) -> String {
    let mut out = String::new();

    out.push_str("# mcp-subagent doctor\n");
    out.push_str(&format!("cwd: {}\n", report.cwd.display()));
    out.push_str("agents_dirs:\n");
    for dir in &report.agents_dirs {
        out.push_str(&format!("- {}\n", dir.display()));
    }
    out.push_str(&format!("state_dir: {}\n", report.state_dir.display()));
    match report.agents_loaded {
        Some(count) => out.push_str(&format!("agents_loaded: {count}\n")),
        None => out.push_str("agents_loaded: unknown\n"),
    }
    if let Some(error) = &report.agents_error {
        out.push_str(&format!("agents_error: {error}\n"));
    }

    out.push_str("\nprovider_probe:\n");
    for probe in &report.probes {
        out.push_str(&format!(
            "- provider: {}\n  status: {}\n  executable: {}\n",
            probe.provider.as_str(),
            probe.status,
            probe.executable.display()
        ));
        out.push_str(&format!(
            "  version: {}\n",
            probe.version.as_deref().unwrap_or("unknown")
        ));
        if probe.notes.is_empty() {
            out.push_str("  notes: []\n");
        } else {
            out.push_str("  notes:\n");
            for note in &probe.notes {
                out.push_str(&format!("    - {note}\n"));
            }
        }
    }

    out
}

fn all_providers() -> Vec<Provider> {
    vec![
        Provider::Claude,
        Provider::Codex,
        Provider::Gemini,
        Provider::Ollama,
    ]
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use tempfile::tempdir;

    use crate::probe::{ProbeStatus, ProviderCapabilities, ProviderProbe, ProviderProber};

    use super::{build_doctor_report, render_doctor_report};

    #[derive(Debug, Clone, Default)]
    struct FakeProber {
        map: HashMap<crate::spec::Provider, ProviderProbe>,
    }

    impl FakeProber {
        fn with_probe(mut self, probe: ProviderProbe) -> Self {
            self.map.insert(probe.provider.clone(), probe);
            self
        }
    }

    impl ProviderProber for FakeProber {
        fn probe(&self, provider: &crate::spec::Provider) -> ProviderProbe {
            self.map
                .get(provider)
                .cloned()
                .unwrap_or_else(|| default_probe(provider.clone()))
        }
    }

    fn default_probe(provider: crate::spec::Provider) -> ProviderProbe {
        ProviderProbe {
            provider: provider.clone(),
            executable: PathBuf::from(provider.as_str().to_lowercase()),
            version: Some("test-version".to_string()),
            status: ProbeStatus::Ready,
            capabilities: ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: false,
                experimental: matches!(provider, crate::spec::Provider::Gemini),
            },
            notes: Vec::new(),
        }
    }

    #[test]
    fn builds_report_and_renders_key_fields() {
        let temp = tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        std::fs::create_dir_all(&agents_dir).expect("create agents");
        std::fs::write(
            agents_dir.join("reviewer.agent.toml"),
            r#"
[core]
name = "reviewer"
description = "review code"
provider = "Ollama"
instructions = "review"
"#,
        )
        .expect("write agent");

        let prober = FakeProber::default().with_probe(ProviderProbe {
            provider: crate::spec::Provider::Codex,
            executable: PathBuf::from("codex"),
            version: None,
            status: ProbeStatus::MissingBinary,
            capabilities: ProviderCapabilities {
                supports_background_native: false,
                supports_native_project_memory: true,
                experimental: false,
            },
            notes: vec!["binary missing".to_string()],
        });
        let report =
            build_doctor_report(vec![agents_dir.clone()], temp.path().join("state"), &prober);

        assert_eq!(report.agents_loaded, Some(1));
        assert_eq!(report.probes.len(), 4);
        let rendered = render_doctor_report(&report);
        assert!(rendered.contains("mcp-subagent doctor"));
        assert!(rendered.contains("agents_loaded: 1"));
        assert!(rendered.contains("provider: Codex"));
        assert!(rendered.contains("status: MissingBinary"));
        assert!(rendered.contains("binary missing"));
    }
}
