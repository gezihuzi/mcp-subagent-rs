use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use glob::glob;
use serde::Serialize;

use crate::{
    probe::{ProviderProbe, ProviderProber},
    spec::{
        registry::{load_agent_specs_from_dirs, LoadedAgentSpec},
        runtime_policy::{NativeDiscoveryPolicy, WorkingDirPolicy},
        Provider,
    },
};

const PROJECT_MEMORY_CANDIDATES: [&str; 2] = ["PROJECT.md", ".mcp-subagent/PROJECT.md"];
const ACTIVE_PLAN_CANDIDATES: [&str; 2] = ["PLAN.md", ".mcp-subagent/PLAN.md"];
const ARCHIVED_PLAN_GLOB_PATTERNS: [&str; 3] =
    ["docs/plans/*.md", "archive/*.md", "plans/archive/*.md"];

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub cwd: PathBuf,
    pub agents_dirs: Vec<PathBuf>,
    pub state_dir: PathBuf,
    pub agents_loaded: Option<usize>,
    pub agents_error: Option<String>,
    pub probes: Vec<ProviderProbe>,
    pub workspace_policy_hints: Vec<WorkspacePolicyHint>,
    pub ambient_isolation: AmbientIsolationReport,
    pub knowledge_layout: KnowledgeLayoutHealth,
    pub version_pins: ProviderVersionPinReport,
    pub status: String,
    pub issues: Vec<DoctorIssue>,
    pub advice: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorIssue {
    pub level: String,
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderVersionPinReport {
    pub enabled: bool,
    pub source: Option<PathBuf>,
    pub entries: Vec<ProviderVersionPinEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderVersionPinEntry {
    pub provider: String,
    pub configured_pin: Option<String>,
    pub detected_version: Option<String>,
    pub compatibility: String,
    pub supported_policy: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspacePolicyHint {
    pub policy: String,
    pub usage_count: usize,
    pub cost_hint: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeLayoutHealth {
    pub root: PathBuf,
    pub active_plan_path: Option<PathBuf>,
    pub project_memory_paths: Vec<PathBuf>,
    pub archived_plan_paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmbientIsolationReport {
    pub provider_profiles: Vec<ProviderAmbientProfile>,
    pub skill_roots: Vec<SkillRootStatus>,
    pub skill_conflicts: Vec<SkillConflictRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAmbientProfile {
    pub provider: String,
    pub agent_count: usize,
    pub native_discovery_modes: Vec<NativeDiscoveryModeUsage>,
    pub ambient_risk: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeDiscoveryModeUsage {
    pub mode: String,
    pub usage_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillRootStatus {
    pub scope: String,
    pub path: PathBuf,
    pub exists: bool,
    pub skill_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillConflictRecord {
    pub skill: String,
    pub sources: Vec<String>,
}

pub fn build_doctor_report(
    agents_dirs: Vec<PathBuf>,
    state_dir: PathBuf,
    prober: &dyn ProviderProber,
) -> DoctorReport {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    build_doctor_report_for_cwd_with_home(cwd, agents_dirs, state_dir, prober, home_dir())
}

#[cfg(test)]
fn build_doctor_report_for_cwd(
    cwd: PathBuf,
    agents_dirs: Vec<PathBuf>,
    state_dir: PathBuf,
    prober: &dyn ProviderProber,
) -> DoctorReport {
    build_doctor_report_for_cwd_with_home(cwd, agents_dirs, state_dir, prober, home_dir())
}

fn build_doctor_report_for_cwd_with_home(
    cwd: PathBuf,
    agents_dirs: Vec<PathBuf>,
    state_dir: PathBuf,
    prober: &dyn ProviderProber,
    user_home: Option<PathBuf>,
) -> DoctorReport {
    let (agents_loaded, agents_error, loaded_specs) = match load_agent_specs_from_dirs(&agents_dirs)
    {
        Ok(loaded) => (Some(loaded.len()), None, loaded),
        Err(err) => (None, Some(err.to_string()), Vec::new()),
    };

    let workspace_policy_hints = build_workspace_policy_hints(&loaded_specs);
    let ambient_isolation =
        build_ambient_isolation_report(&cwd, &loaded_specs, user_home.as_deref());
    let knowledge_layout = build_knowledge_layout_health(&cwd);
    let probes: Vec<ProviderProbe> = all_providers()
        .into_iter()
        .map(|provider| prober.probe(&provider))
        .collect();
    let version_pins = build_provider_version_pin_report(&cwd, &probes);
    let (status, issues, advice) = build_doctor_health(
        &agents_error,
        &knowledge_layout,
        &probes,
        &version_pins,
        &ambient_isolation,
    );

    DoctorReport {
        cwd,
        agents_dirs,
        state_dir,
        agents_loaded,
        agents_error,
        probes,
        workspace_policy_hints,
        ambient_isolation,
        knowledge_layout,
        version_pins,
        status,
        issues,
        advice,
    }
}

fn build_doctor_health(
    agents_error: &Option<String>,
    knowledge_layout: &KnowledgeLayoutHealth,
    probes: &[ProviderProbe],
    version_pins: &ProviderVersionPinReport,
    ambient_isolation: &AmbientIsolationReport,
) -> (String, Vec<DoctorIssue>, Vec<String>) {
    let mut issues = Vec::new();
    let mut advice = Vec::new();

    if let Some(error) = agents_error {
        issues.push(DoctorIssue {
            level: "error".to_string(),
            code: "agents_load_failed".to_string(),
            message: error.clone(),
            suggestion: Some(
                "Run `mcp-subagent validate --agents-dir <dir>` and fix invalid specs.".to_string(),
            ),
        });
    }

    for warning in &knowledge_layout.warnings {
        issues.push(DoctorIssue {
            level: "warning".to_string(),
            code: "knowledge_layout".to_string(),
            message: warning.clone(),
            suggestion: Some(
                "Create PLAN.md / PROJECT.md and archive plans under docs/plans/.".to_string(),
            ),
        });
    }

    for probe in probes {
        if probe.is_available() {
            continue;
        }
        let suggestion = probe.notes.first().cloned();
        issues.push(DoctorIssue {
            level: "warning".to_string(),
            code: format!(
                "provider_{}_unavailable",
                probe.provider.as_str().to_lowercase()
            ),
            message: format!(
                "provider {} unavailable ({})",
                probe.provider.as_str(),
                probe.status
            ),
            suggestion,
        });
    }

    for entry in &version_pins.entries {
        if entry.compatibility == "drift" || entry.compatibility == "not_detected" {
            issues.push(DoctorIssue {
                level: "warning".to_string(),
                code: format!(
                    "provider_{}_version_{}",
                    entry.provider.to_lowercase(),
                    entry.compatibility
                ),
                message: format!(
                    "provider {} version compatibility is {}",
                    entry.provider, entry.compatibility
                ),
                suggestion: Some(entry.suggestion.clone()),
            });
        }
    }

    for profile in &ambient_isolation.provider_profiles {
        if profile.agent_count == 0 {
            continue;
        }
        if profile.ambient_risk == "medium" || profile.ambient_risk == "high" {
            issues.push(DoctorIssue {
                level: "warning".to_string(),
                code: format!("provider_{}_ambient_discovery", profile.provider),
                message: format!(
                    "provider {} ambient isolation risk is {}",
                    profile.provider, profile.ambient_risk
                ),
                suggestion: Some(profile.recommendation.clone()),
            });
        }
    }

    if !ambient_isolation.skill_conflicts.is_empty() {
        issues.push(DoctorIssue {
            level: "warning".to_string(),
            code: "ambient_skill_conflicts".to_string(),
            message: format!(
                "{} workspace-visible skill name conflicts detected across ambient roots",
                ambient_isolation.skill_conflicts.len()
            ),
            suggestion: Some(
                "Prefer runtime.native_discovery = \"isolated\" for Gemini agents and remove duplicated skill names between workspace/user roots.".to_string(),
            ),
        });
    }

    if issues.is_empty() {
        advice.push("Environment looks healthy.".to_string());
        return ("ok".to_string(), issues, advice);
    }

    for issue in &issues {
        if let Some(suggestion) = &issue.suggestion {
            if !advice.iter().any(|item| item == suggestion) {
                advice.push(suggestion.clone());
            }
        }
    }

    let status = if issues.iter().any(|issue| issue.level == "error") {
        "error"
    } else {
        "warning"
    };
    (status.to_string(), issues, advice)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn build_ambient_isolation_report(
    cwd: &Path,
    loaded_specs: &[LoadedAgentSpec],
    user_home: Option<&Path>,
) -> AmbientIsolationReport {
    let analyses = build_skill_root_analyses(cwd, user_home);
    let skill_roots = analyses
        .iter()
        .map(|analysis| SkillRootStatus {
            scope: analysis.scope.to_string(),
            path: analysis.path.clone(),
            exists: analysis.exists,
            skill_count: analysis.skills.len(),
        })
        .collect::<Vec<_>>();
    let skill_conflicts = build_skill_conflicts_from_analyses(&analyses);
    let provider_profiles = build_provider_ambient_profiles(loaded_specs, skill_conflicts.len());
    AmbientIsolationReport {
        provider_profiles,
        skill_roots,
        skill_conflicts,
    }
}

fn build_provider_ambient_profiles(
    loaded_specs: &[LoadedAgentSpec],
    workspace_conflict_count: usize,
) -> Vec<ProviderAmbientProfile> {
    let mut by_provider: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for loaded in loaded_specs {
        let provider = loaded.spec.core.provider.as_str().to_string();
        let mode = native_discovery_mode_name(&loaded.spec.runtime.native_discovery).to_string();
        *by_provider
            .entry(provider)
            .or_default()
            .entry(mode)
            .or_insert(0usize) += 1;
    }

    [Provider::Claude, Provider::Codex, Provider::Gemini]
        .into_iter()
        .map(|provider| {
            let provider_name = provider.as_str().to_string();
            let mode_counts = by_provider
                .get(provider_name.as_str())
                .cloned()
                .unwrap_or_default();
            let agent_count = mode_counts.values().copied().sum();
            let native_discovery_modes = mode_counts
                .into_iter()
                .map(|(mode, usage_count)| NativeDiscoveryModeUsage { mode, usage_count })
                .collect::<Vec<_>>();
            let (ambient_risk, recommendation) = classify_provider_ambient_risk(
                &provider,
                agent_count,
                &native_discovery_modes,
                workspace_conflict_count,
            );
            ProviderAmbientProfile {
                provider: provider_name,
                agent_count,
                native_discovery_modes,
                ambient_risk: ambient_risk.to_string(),
                recommendation: recommendation.to_string(),
            }
        })
        .collect()
}

fn classify_provider_ambient_risk(
    provider: &Provider,
    agent_count: usize,
    modes: &[NativeDiscoveryModeUsage],
    workspace_conflict_count: usize,
) -> (&'static str, &'static str) {
    if agent_count == 0 {
        return ("not_applicable", "no agents configured for this provider");
    }
    let has_loose_mode = modes
        .iter()
        .any(|mode| mode.mode == "inherit" || mode.mode == "allowlist");
    if has_loose_mode {
        if matches!(provider, Provider::Gemini) && workspace_conflict_count > 0 {
            return (
                "high",
                "Set runtime.native_discovery = \"isolated\" for Gemini agents to suppress ambient skill conflicts.",
            );
        }
        if matches!(provider, Provider::Gemini) {
            return (
                "medium",
                "Prefer runtime.native_discovery = \"minimal\" or \"isolated\" for Gemini agents unless ambient discovery is explicitly required.",
            );
        }
        return (
            "medium",
            "Prefer runtime.native_discovery = \"minimal\" for this provider unless ambient discovery is required.",
        );
    }
    (
        "low",
        "Current native_discovery profile is isolation-friendly.",
    )
}

fn native_discovery_mode_name(policy: &NativeDiscoveryPolicy) -> &'static str {
    match policy {
        NativeDiscoveryPolicy::Inherit => "inherit",
        NativeDiscoveryPolicy::Minimal => "minimal",
        NativeDiscoveryPolicy::Isolated => "isolated",
        NativeDiscoveryPolicy::Allowlist => "allowlist",
    }
}

#[derive(Debug, Clone)]
struct SkillRootAnalysis {
    scope: &'static str,
    path: PathBuf,
    exists: bool,
    workspace_root: bool,
    skills: Vec<String>,
}

fn build_skill_root_analyses(cwd: &Path, user_home: Option<&Path>) -> Vec<SkillRootAnalysis> {
    let mut roots = Vec::new();
    roots.push(analyze_skill_root(
        "workspace_agents",
        cwd.join(".agents").join("skills"),
        true,
    ));
    roots.push(analyze_skill_root(
        "workspace_gemini",
        cwd.join(".gemini").join("skills"),
        true,
    ));
    if let Some(home) = user_home {
        roots.push(analyze_skill_root(
            "user_agents",
            home.join(".agents").join("skills"),
            false,
        ));
        roots.push(analyze_skill_root(
            "user_gemini",
            home.join(".gemini").join("skills"),
            false,
        ));
    }
    roots
}

fn analyze_skill_root(
    scope: &'static str,
    path: PathBuf,
    workspace_root: bool,
) -> SkillRootAnalysis {
    let exists = path.is_dir();
    let skills = if exists {
        collect_skill_names(&path)
    } else {
        Vec::new()
    };
    SkillRootAnalysis {
        scope,
        path,
        exists,
        workspace_root,
        skills,
    }
}

fn collect_skill_names(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return names,
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        if !path.join("SKILL.md").is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

fn build_skill_conflicts_from_analyses(analyses: &[SkillRootAnalysis]) -> Vec<SkillConflictRecord> {
    let mut index: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for analysis in analyses {
        for skill in &analysis.skills {
            index.entry(skill.clone()).or_default().push((
                format!("{} ({})", analysis.scope, analysis.path.display()),
                analysis.workspace_root,
            ));
        }
    }
    let mut conflicts = Vec::new();
    for (skill, sources) in index {
        if sources.len() < 2 {
            continue;
        }
        if !sources.iter().any(|(_, workspace_root)| *workspace_root) {
            continue;
        }
        conflicts.push(SkillConflictRecord {
            skill,
            sources: sources.into_iter().map(|(label, _)| label).collect(),
        });
    }
    conflicts
}

fn build_provider_version_pin_report(
    root: &Path,
    probes: &[ProviderProbe],
) -> ProviderVersionPinReport {
    let config_path = root.join(".mcp-subagent/config.toml");
    let pin_config = load_provider_pin_config(&config_path);
    let enabled = pin_config.enabled;
    let entries = all_providers()
        .into_iter()
        .map(|provider| {
            let configured_pin = pin_config.pins.get(provider.as_str()).cloned();
            let detected_version = probes
                .iter()
                .find(|probe| probe.provider == provider)
                .and_then(|probe| probe.version.clone());
            let compatibility = if !enabled {
                "disabled".to_string()
            } else if configured_pin.is_none() {
                "unpinned".to_string()
            } else if detected_version.is_none() {
                "not_detected".to_string()
            } else if version_matches_pin(
                detected_version.as_deref().unwrap_or_default(),
                configured_pin.as_deref().unwrap_or_default(),
            ) {
                "matched".to_string()
            } else {
                "drift".to_string()
            };
            let suggestion = match compatibility.as_str() {
                "matched" => "Pinned version matches detected CLI version.".to_string(),
                "disabled" => "Enable [provider_version_pins] to enforce version drift checks."
                    .to_string(),
                "unpinned" => format!(
                    "Add {} = \"<version>\" under [provider_version_pins] to pin this provider.",
                    provider.as_str().to_lowercase()
                ),
                "not_detected" => {
                    "CLI version not detected; verify binary installation and --version output."
                        .to_string()
                }
                "drift" => format!(
                    "Detected version differs from pin; update pin or install a compatible CLI for {}.",
                    provider.as_str()
                ),
                _ => "unknown".to_string(),
            };

            ProviderVersionPinEntry {
                provider: provider.as_str().to_string(),
                configured_pin,
                detected_version,
                compatibility,
                supported_policy: "pin_exact_or_prefix_match".to_string(),
                suggestion,
            }
        })
        .collect::<Vec<_>>();

    ProviderVersionPinReport {
        enabled,
        source: pin_config.source,
        entries,
    }
}

#[derive(Debug, Default)]
struct ProviderPinConfig {
    enabled: bool,
    source: Option<PathBuf>,
    pins: BTreeMap<String, String>,
}

fn load_provider_pin_config(path: &Path) -> ProviderPinConfig {
    if !path.is_file() {
        return ProviderPinConfig::default();
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return ProviderPinConfig::default(),
    };
    let parsed = match raw.parse::<toml::Value>() {
        Ok(value) => value,
        Err(_) => return ProviderPinConfig::default(),
    };

    let Some(table) = parsed
        .get("provider_version_pins")
        .and_then(|value| value.as_table())
    else {
        return ProviderPinConfig::default();
    };

    let enabled = table
        .get("enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let mut pins = BTreeMap::new();
    for provider in all_providers() {
        let key = provider.as_str().to_lowercase();
        if let Some(pin) = table.get(&key).and_then(|value| value.as_str()) {
            pins.insert(provider.as_str().to_string(), pin.to_string());
        }
    }
    ProviderPinConfig {
        enabled,
        source: Some(path.to_path_buf()),
        pins,
    }
}

fn version_matches_pin(detected_version: &str, configured_pin: &str) -> bool {
    let detected = detected_version.to_lowercase();
    let pin = configured_pin.to_lowercase();
    detected.contains(&pin)
}

fn build_workspace_policy_hints(loaded_specs: &[LoadedAgentSpec]) -> Vec<WorkspacePolicyHint> {
    let mut usage = BTreeMap::new();
    for loaded in loaded_specs {
        let key = format!("{}", loaded.spec.runtime.working_dir_policy);
        *usage.entry(key).or_insert(0usize) += 1;
    }

    [
        WorkingDirPolicy::Auto,
        WorkingDirPolicy::InPlace,
        WorkingDirPolicy::GitWorktree,
        WorkingDirPolicy::TempCopy,
    ]
    .into_iter()
    .map(|policy| {
        let policy_name = format!("{policy}");
        WorkspacePolicyHint {
            policy: policy_name.clone(),
            usage_count: usage.get(&policy_name).copied().unwrap_or(0),
            cost_hint: workspace_policy_cost_hint(&policy).to_string(),
            recommendation: workspace_policy_recommendation(&policy).to_string(),
        }
    })
    .collect()
}

fn workspace_policy_cost_hint(policy: &WorkingDirPolicy) -> &'static str {
    match policy {
        WorkingDirPolicy::Auto => "balanced: read tasks stay in-place, write tasks prefer worktree",
        WorkingDirPolicy::InPlace => "lowest setup cost, highest repo pollution risk",
        WorkingDirPolicy::GitWorktree => "moderate setup cost, strong isolation for write tasks",
        WorkingDirPolicy::TempCopy => "highest I/O and disk cost, strongest isolation",
    }
}

fn workspace_policy_recommendation(policy: &WorkingDirPolicy) -> &'static str {
    match policy {
        WorkingDirPolicy::Auto => "recommended default for mixed read/write workloads",
        WorkingDirPolicy::InPlace => "use for read-only or very small safe edits",
        WorkingDirPolicy::GitWorktree => "use for parallel write-heavy task isolation",
        WorkingDirPolicy::TempCopy => "use when worktree is unavailable or full clone is required",
    }
}

fn build_knowledge_layout_health(root: &Path) -> KnowledgeLayoutHealth {
    let active_plan_path = ACTIVE_PLAN_CANDIDATES
        .iter()
        .map(|candidate| root.join(candidate))
        .find(|path| path.is_file());

    let project_memory_paths = PROJECT_MEMORY_CANDIDATES
        .iter()
        .map(|candidate| root.join(candidate))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();

    let mut archived_plan_paths = Vec::new();
    for pattern in ARCHIVED_PLAN_GLOB_PATTERNS {
        let absolute_pattern = root.join(pattern);
        let pattern_text = absolute_pattern.to_string_lossy().to_string();
        if let Ok(entries) = glob(&pattern_text) {
            for entry in entries.flatten() {
                if entry.is_file() {
                    archived_plan_paths.push(entry);
                }
            }
        }
    }
    archived_plan_paths.sort();
    archived_plan_paths.dedup();

    let mut warnings = Vec::new();
    if active_plan_path.is_none() {
        warnings.push("active plan missing: expected PLAN.md or .mcp-subagent/PLAN.md".to_string());
    }
    if project_memory_paths.is_empty() {
        warnings.push(
            "project memory missing: expected PROJECT.md or .mcp-subagent/PROJECT.md".to_string(),
        );
    }
    if archived_plan_paths.is_empty() {
        warnings.push(
            "archive plans missing: expected docs/plans/*.md, archive/*.md or plans/archive/*.md"
                .to_string(),
        );
    }

    KnowledgeLayoutHealth {
        root: root.to_path_buf(),
        active_plan_path,
        project_memory_paths,
        archived_plan_paths,
        warnings,
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
    out.push_str(&format!("status: {}\n", report.status));
    if !report.issues.is_empty() {
        out.push_str("issues:\n");
        for issue in &report.issues {
            out.push_str(&format!(
                "- [{}] {}: {}\n",
                issue.level, issue.code, issue.message
            ));
            if let Some(suggestion) = &issue.suggestion {
                out.push_str(&format!("  suggestion: {}\n", suggestion));
            }
        }
    }

    out.push_str("\nprovider_matrix:\n");
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
        out.push_str("  capabilities:\n");
        out.push_str(&format!(
            "    - supports_background_native: {}\n",
            probe.capabilities.supports_background_native
        ));
        out.push_str(&format!(
            "    - supports_native_project_memory: {}\n",
            probe.capabilities.supports_native_project_memory
        ));
        out.push_str(&format!(
            "    - experimental: {}\n",
            probe.capabilities.experimental
        ));
        if probe.validated_flags.is_empty() {
            out.push_str("  validated_flags: []\n");
        } else {
            out.push_str("  validated_flags:\n");
            for flag in &probe.validated_flags {
                out.push_str(&format!("    - {flag}\n"));
            }
        }
        if probe.notes.is_empty() {
            out.push_str("  notes: []\n");
        } else {
            out.push_str("  notes:\n");
            for note in &probe.notes {
                out.push_str(&format!("    - {note}\n"));
            }
        }
    }

    out.push_str("\nprovider_version_pins:\n");
    out.push_str(&format!("  enabled: {}\n", report.version_pins.enabled));
    out.push_str(&format!(
        "  source: {}\n",
        report
            .version_pins
            .source
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    for entry in &report.version_pins.entries {
        out.push_str(&format!(
            "  - provider: {}\n    configured_pin: {}\n    detected_version: {}\n    compatibility: {}\n    policy: {}\n    suggestion: {}\n",
            entry.provider,
            entry.configured_pin.as_deref().unwrap_or("none"),
            entry.detected_version.as_deref().unwrap_or("unknown"),
            entry.compatibility,
            entry.supported_policy,
            entry.suggestion,
        ));
    }

    out.push_str("\nworkspace_policy_hints:\n");
    for hint in &report.workspace_policy_hints {
        out.push_str(&format!(
            "- policy: {}\n  usage_count: {}\n  cost_hint: {}\n  recommendation: {}\n",
            hint.policy, hint.usage_count, hint.cost_hint, hint.recommendation
        ));
    }

    out.push_str("\nambient_isolation:\n");
    out.push_str("  provider_profiles:\n");
    for profile in &report.ambient_isolation.provider_profiles {
        out.push_str(&format!(
            "  - provider: {}\n    agent_count: {}\n    ambient_risk: {}\n    recommendation: {}\n",
            profile.provider, profile.agent_count, profile.ambient_risk, profile.recommendation
        ));
        if profile.native_discovery_modes.is_empty() {
            out.push_str("    native_discovery_modes: []\n");
        } else {
            out.push_str("    native_discovery_modes:\n");
            for mode in &profile.native_discovery_modes {
                out.push_str(&format!(
                    "      - mode: {}\n        usage_count: {}\n",
                    mode.mode, mode.usage_count
                ));
            }
        }
    }
    out.push_str("  skill_roots:\n");
    for root in &report.ambient_isolation.skill_roots {
        out.push_str(&format!(
            "  - scope: {}\n    path: {}\n    exists: {}\n    skill_count: {}\n",
            root.scope,
            root.path.display(),
            root.exists,
            root.skill_count
        ));
    }
    if report.ambient_isolation.skill_conflicts.is_empty() {
        out.push_str("  skill_conflicts: []\n");
    } else {
        out.push_str("  skill_conflicts:\n");
        for conflict in &report.ambient_isolation.skill_conflicts {
            out.push_str(&format!("    - skill: {}\n", conflict.skill));
            out.push_str("      sources:\n");
            for source in &conflict.sources {
                out.push_str(&format!("        - {}\n", source));
            }
        }
    }

    out.push_str("\nknowledge_layout_health:\n");
    out.push_str(&format!(
        "  root: {}\n",
        report.knowledge_layout.root.display()
    ));
    out.push_str(&format!(
        "  active_plan_path: {}\n",
        report
            .knowledge_layout
            .active_plan_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "missing".to_string())
    ));

    if report.knowledge_layout.project_memory_paths.is_empty() {
        out.push_str("  project_memory_paths: []\n");
    } else {
        out.push_str("  project_memory_paths:\n");
        for path in &report.knowledge_layout.project_memory_paths {
            out.push_str(&format!("    - {}\n", path.display()));
        }
    }

    if report.knowledge_layout.archived_plan_paths.is_empty() {
        out.push_str("  archived_plan_paths: []\n");
    } else {
        out.push_str("  archived_plan_paths:\n");
        for path in &report.knowledge_layout.archived_plan_paths {
            out.push_str(&format!("    - {}\n", path.display()));
        }
    }

    if report.knowledge_layout.warnings.is_empty() {
        out.push_str("  warnings: []\n");
    } else {
        out.push_str("  warnings:\n");
        for warning in &report.knowledge_layout.warnings {
            out.push_str(&format!("    - {warning}\n"));
        }
    }

    if !report.advice.is_empty() {
        out.push_str("\nadvice:\n");
        for advice in &report.advice {
            out.push_str(&format!("- {advice}\n"));
        }
    }

    out
}

fn all_providers() -> Vec<Provider> {
    vec![
        Provider::Mock,
        Provider::Claude,
        Provider::Codex,
        Provider::Gemini,
        Provider::Ollama,
    ]
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use tempfile::tempdir;

    use crate::probe::{ProbeStatus, ProviderCapabilities, ProviderProbe, ProviderProber};

    use super::{
        build_doctor_report, build_doctor_report_for_cwd, build_doctor_report_for_cwd_with_home,
        render_doctor_report,
    };

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
            validated_flags: Vec::new(),
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
provider = "mock"
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
            validated_flags: vec!["--sandbox".to_string(), "--ask-for-approval".to_string()],
            notes: vec!["binary missing".to_string()],
        });
        let report =
            build_doctor_report(vec![agents_dir.clone()], temp.path().join("state"), &prober);

        assert_eq!(report.agents_loaded, Some(1));
        assert_eq!(report.probes.len(), 5);
        let rendered = render_doctor_report(&report);
        assert!(rendered.contains("mcp-subagent doctor"));
        assert!(rendered.contains("agents_loaded: 1"));
        assert!(rendered.contains("provider: mock"));
        assert!(rendered.contains("provider: ollama"));
        assert!(rendered.contains("provider: codex"));
        assert!(rendered.contains("status: missing_binary"));
        assert!(rendered.contains("supports_native_project_memory: true"));
        assert!(rendered.contains("validated_flags"));
        assert!(rendered.contains("--ask-for-approval"));
        assert!(rendered.contains("binary missing"));
        assert!(rendered.contains("workspace_policy_hints"));
        assert!(rendered.contains("ambient_isolation"));
        assert!(rendered.contains("knowledge_layout_health"));
    }

    #[test]
    fn checks_knowledge_layout_and_policy_usage() {
        let temp = tempdir().expect("tempdir");
        let agents_dir = temp.path().join("agents");
        std::fs::create_dir_all(&agents_dir).expect("create agents");
        std::fs::create_dir_all(temp.path().join("docs").join("plans"))
            .expect("create archived plans dir");
        std::fs::write(temp.path().join("PLAN.md"), "# plan").expect("write plan");
        std::fs::write(temp.path().join("PROJECT.md"), "# project").expect("write project");
        std::fs::write(temp.path().join("docs").join("plans").join("p1.md"), "# p1")
            .expect("write archive");
        std::fs::write(
            agents_dir.join("writer.agent.toml"),
            r#"
[core]
name = "writer"
description = "write code"
provider = "mock"
instructions = "write"

[runtime]
working_dir_policy = "git_worktree"
sandbox = "workspace_write"
"#,
        )
        .expect("write agent");

        let report = build_doctor_report_for_cwd(
            temp.path().to_path_buf(),
            vec![agents_dir],
            temp.path().join("state"),
            &FakeProber::default(),
        );

        assert_eq!(report.agents_loaded, Some(1));
        assert!(report.knowledge_layout.active_plan_path.is_some());
        assert_eq!(report.knowledge_layout.project_memory_paths.len(), 1);
        assert_eq!(report.knowledge_layout.archived_plan_paths.len(), 1);
        assert!(report.knowledge_layout.warnings.is_empty());
        assert_eq!(
            report
                .workspace_policy_hints
                .iter()
                .find(|hint| hint.policy == "git_worktree")
                .map(|hint| hint.usage_count),
            Some(1)
        );
    }

    #[test]
    fn provider_pin_report_marks_matched_when_pin_hits() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join(".mcp-subagent")).expect("create config dir");
        fs::write(
            temp.path().join(".mcp-subagent/config.toml"),
            r#"
[provider_version_pins]
enabled = true
codex = "test-version"
"#,
        )
        .expect("write config");

        let report = build_doctor_report_for_cwd(
            temp.path().to_path_buf(),
            vec![temp.path().join("agents")],
            temp.path().join("state"),
            &FakeProber::default(),
        );
        let codex = report
            .version_pins
            .entries
            .iter()
            .find(|entry| entry.provider == "codex")
            .expect("codex entry");
        assert_eq!(codex.compatibility, "matched");
    }

    #[test]
    fn provider_pin_report_marks_drift_when_pin_mismatches() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join(".mcp-subagent")).expect("create config dir");
        fs::write(
            temp.path().join(".mcp-subagent/config.toml"),
            r#"
[provider_version_pins]
enabled = true
codex = "9.9.9"
"#,
        )
        .expect("write config");

        let report = build_doctor_report_for_cwd(
            temp.path().to_path_buf(),
            vec![temp.path().join("agents")],
            temp.path().join("state"),
            &FakeProber::default(),
        );
        let codex = report
            .version_pins
            .entries
            .iter()
            .find(|entry| entry.provider == "codex")
            .expect("codex entry");
        assert_eq!(codex.compatibility, "drift");
    }

    #[test]
    fn provider_pin_report_marks_disabled_when_config_disabled() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join(".mcp-subagent")).expect("create config dir");
        fs::write(
            temp.path().join(".mcp-subagent/config.toml"),
            r#"
[provider_version_pins]
enabled = false
codex = "test-version"
"#,
        )
        .expect("write config");

        let report = build_doctor_report_for_cwd(
            temp.path().to_path_buf(),
            vec![temp.path().join("agents")],
            temp.path().join("state"),
            &FakeProber::default(),
        );
        let codex = report
            .version_pins
            .entries
            .iter()
            .find(|entry| entry.provider == "codex")
            .expect("codex entry");
        assert_eq!(codex.compatibility, "disabled");
    }

    #[test]
    fn ambient_isolation_detects_workspace_visible_skill_conflict_for_gemini() {
        let temp = tempdir().expect("tempdir");
        let home = tempdir().expect("home tempdir");
        let agents_dir = temp.path().join("agents");
        fs::create_dir_all(&agents_dir).expect("create agents");
        fs::create_dir_all(temp.path().join(".agents/skills/find-skills"))
            .expect("create workspace skill");
        fs::create_dir_all(home.path().join(".agents/skills/find-skills"))
            .expect("create user skill");
        fs::write(
            temp.path().join(".agents/skills/find-skills/SKILL.md"),
            "# workspace skill",
        )
        .expect("write workspace skill");
        fs::write(
            home.path().join(".agents/skills/find-skills/SKILL.md"),
            "# user skill",
        )
        .expect("write user skill");
        fs::write(
            agents_dir.join("fast-researcher.agent.toml"),
            r#"
[core]
name = "fast-researcher"
description = "research"
provider = "gemini"
instructions = "research"

[runtime]
native_discovery = "inherit"
"#,
        )
        .expect("write gemini agent");

        let report = build_doctor_report_for_cwd_with_home(
            temp.path().to_path_buf(),
            vec![agents_dir],
            temp.path().join("state"),
            &FakeProber::default(),
            Some(home.path().to_path_buf()),
        );

        assert!(
            report
                .ambient_isolation
                .skill_conflicts
                .iter()
                .any(|conflict| conflict.skill == "find-skills"),
            "expected workspace-visible skill conflict"
        );
        let gemini_profile = report
            .ambient_isolation
            .provider_profiles
            .iter()
            .find(|profile| profile.provider == "gemini")
            .expect("gemini profile");
        assert_eq!(gemini_profile.ambient_risk, "high");
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "ambient_skill_conflicts"),
            "expected ambient conflict issue"
        );
    }
}
