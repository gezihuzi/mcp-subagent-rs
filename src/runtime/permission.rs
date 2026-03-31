use std::{
    env,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::spec::{
    runtime_policy::{SandboxPolicy, WorkingDirPolicy},
    AgentSpec,
};

const ALLOWED_PATHS_ENV: &str = "MCP_SUBAGENT_ALLOWED_PATHS";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperation {
    Read,
    Write,
}

impl std::fmt::Display for PermissionOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Read => "read",
            Self::Write => "write",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionDenied {
    pub operation: PermissionOperation,
    pub requested_path: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    pub reason: String,
}

impl PermissionDenied {
    pub fn to_error_message(&self) -> String {
        let allowed = if self.allowed_paths.is_empty() {
            "<none configured>".to_string()
        } else {
            self.allowed_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "permission required: direct workspace {} for `{}` is outside allowed paths [{}]. Set {} to include this directory, or switch working_dir_policy to in_place/temp_copy/git_worktree.",
            self.operation,
            self.requested_path.display(),
            allowed,
            ALLOWED_PATHS_ENV,
        )
    }
}

#[derive(Debug, Clone)]
pub struct PermissionBroker {
    allowed_paths: Vec<PathBuf>,
}

impl PermissionBroker {
    pub fn from_env() -> Self {
        let cwd = env::current_dir().ok();
        let allowed_paths =
            parse_allowed_paths(env::var(ALLOWED_PATHS_ENV).ok().as_deref(), cwd.as_deref());
        Self { allowed_paths }
    }

    pub fn allowed_paths(&self) -> &[PathBuf] {
        &self.allowed_paths
    }

    pub fn is_allowed(&self, path: &Path) -> bool {
        self.allowed_paths
            .iter()
            .any(|allowed| path.starts_with(allowed))
    }
}

pub fn check_direct_workspace_permission(
    spec: &AgentSpec,
    source_path: &Path,
) -> Option<PermissionDenied> {
    if !matches!(spec.runtime.working_dir_policy, WorkingDirPolicy::Direct) {
        return None;
    }

    let requested_path = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.to_path_buf());
    let operation = match spec.runtime.sandbox {
        SandboxPolicy::ReadOnly => PermissionOperation::Read,
        SandboxPolicy::WorkspaceWrite | SandboxPolicy::FullAccess => PermissionOperation::Write,
    };
    let broker = PermissionBroker::from_env();
    if broker.is_allowed(&requested_path) {
        return None;
    }

    let allowed_paths = broker.allowed_paths().to_vec();
    Some(PermissionDenied {
        operation: operation.clone(),
        requested_path,
        reason: format!("direct workspace {} path is outside allowlist", operation),
        allowed_paths,
    })
}

fn parse_allowed_paths(raw: Option<&str>, cwd: Option<&Path>) -> Vec<PathBuf> {
    let candidates = match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.contains(',') => value
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        Some(value) => env::split_paths(value).collect::<Vec<_>>(),
        None => cwd.into_iter().map(Path::to_path_buf).collect::<Vec<_>>(),
    };

    normalize_paths(candidates, cwd)
}

fn normalize_paths(paths: Vec<PathBuf>, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut normalized = Vec::new();
    for path in paths {
        let path = if path.is_absolute() {
            path
        } else if let Some(cwd) = cwd {
            cwd.join(path)
        } else {
            path
        };
        let canonical = path.canonicalize().unwrap_or(path);
        if normalized.iter().any(|existing| existing == &canonical) {
            continue;
        }
        normalized.push(canonical);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::{env, ffi::OsString, fs, path::PathBuf};

    use tempfile::tempdir;

    use crate::spec::{
        core::{AgentSpecCore, Provider},
        runtime_policy::{RuntimePolicy, SandboxPolicy, WorkingDirPolicy},
        AgentSpec,
    };

    use super::{check_direct_workspace_permission, parse_allowed_paths, PermissionOperation};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    struct EnvVarGuard {
        key: String,
        old: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: Option<&str>) -> Self {
            let old = env::var_os(key);
            match value {
                Some(value) => {
                    // Safety: test-only environment override.
                    unsafe { env::set_var(key, value) };
                }
                None => {
                    // Safety: test-only environment override.
                    unsafe { env::remove_var(key) };
                }
            }
            Self {
                key: key.to_string(),
                old,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.as_ref() {
                // Safety: restoring test-local environment snapshot.
                unsafe { env::set_var(&self.key, value) };
            } else {
                // Safety: restoring test-local environment snapshot.
                unsafe { env::remove_var(&self.key) };
            }
        }
    }

    fn sample_spec(policy: WorkingDirPolicy, sandbox: SandboxPolicy) -> AgentSpec {
        AgentSpec {
            core: AgentSpecCore {
                name: "writer".to_string(),
                description: "write code".to_string(),
                provider: Provider::Mock,
                model: None,
                instructions: "write".to_string(),
                allowed_tools: Vec::new(),
                disallowed_tools: Vec::new(),
                skills: Vec::new(),
                tags: Vec::new(),
                metadata: Default::default(),
            },
            runtime: RuntimePolicy {
                working_dir_policy: policy,
                sandbox,
                ..RuntimePolicy::default()
            },
            provider_overrides: Default::default(),
            workflow: None,
        }
    }

    #[test]
    fn parse_allowed_paths_defaults_to_cwd_when_unset() {
        let cwd = PathBuf::from("/tmp/workspace");
        let parsed = parse_allowed_paths(None, Some(&cwd));
        assert_eq!(parsed, vec![cwd]);
    }

    #[test]
    fn parse_allowed_paths_supports_comma_delimited_values() {
        let cwd = PathBuf::from("/tmp/workspace");
        let parsed = parse_allowed_paths(Some("./a,./b"), Some(&cwd));
        assert_eq!(parsed, vec![cwd.join("a"), cwd.join("b")]);
    }

    #[test]
    fn direct_policy_denies_write_when_outside_allowlist() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let dir = tempdir().expect("tempdir");
        let cwd = dir.path().join("cwd");
        let target = dir.path().join("outside");
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::create_dir_all(&target).expect("create target");
        let _allow = EnvVarGuard::set(
            "MCP_SUBAGENT_ALLOWED_PATHS",
            Some(cwd.to_string_lossy().as_ref()),
        );

        let spec = sample_spec(WorkingDirPolicy::Direct, SandboxPolicy::WorkspaceWrite);
        let denial = check_direct_workspace_permission(&spec, &target).expect("should deny");
        assert_eq!(denial.operation, PermissionOperation::Write);
        assert_eq!(
            denial.requested_path,
            target.canonicalize().expect("canonical target")
        );
    }

    #[test]
    fn direct_policy_allows_path_within_allowlist() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let target = root.join("nested");
        fs::create_dir_all(&target).expect("create target");
        let _allow = EnvVarGuard::set(
            "MCP_SUBAGENT_ALLOWED_PATHS",
            Some(root.to_string_lossy().as_ref()),
        );

        let spec = sample_spec(WorkingDirPolicy::Direct, SandboxPolicy::WorkspaceWrite);
        let denial = check_direct_workspace_permission(&spec, &target);
        assert!(denial.is_none());
    }

    #[test]
    fn in_place_policy_skips_permission_gate() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        fs::create_dir_all(&target).expect("create target");
        let _allow = EnvVarGuard::set("MCP_SUBAGENT_ALLOWED_PATHS", Some("/nonexistent"));

        let spec = sample_spec(WorkingDirPolicy::InPlace, SandboxPolicy::WorkspaceWrite);
        let denial = check_direct_workspace_permission(&spec, &target);
        assert!(denial.is_none());
    }

    #[test]
    fn direct_policy_uses_read_operation_for_readonly_sandbox() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let dir = tempdir().expect("tempdir");
        let cwd = dir.path().join("cwd");
        let target = dir.path().join("outside");
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::create_dir_all(&target).expect("create target");
        let _allow = EnvVarGuard::set(
            "MCP_SUBAGENT_ALLOWED_PATHS",
            Some(cwd.to_string_lossy().as_ref()),
        );

        let spec = sample_spec(WorkingDirPolicy::Direct, SandboxPolicy::ReadOnly);
        let denial = check_direct_workspace_permission(&spec, &target).expect("should deny");
        assert_eq!(denial.operation, PermissionOperation::Read);
    }
}
