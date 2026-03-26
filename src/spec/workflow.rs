use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStageKind {
    Research,
    Plan,
    Build,
    Review,
    Archive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowGatePolicy {
    pub require_plan_if_touched_files_ge: Option<u32>,
    #[serde(default)]
    pub require_plan_if_cross_module: bool,
    #[serde(default)]
    pub require_plan_if_parallel_agents: bool,
    #[serde(default)]
    pub require_plan_if_new_interface: bool,
    #[serde(default)]
    pub require_plan_if_migration: bool,
    #[serde(default)]
    pub require_plan_if_human_approval_point: bool,
    pub require_plan_if_estimated_runtime_minutes_ge: Option<u32>,
}

impl Default for WorkflowGatePolicy {
    fn default() -> Self {
        Self {
            require_plan_if_touched_files_ge: Some(5),
            require_plan_if_cross_module: true,
            require_plan_if_parallel_agents: true,
            require_plan_if_new_interface: true,
            require_plan_if_migration: true,
            require_plan_if_human_approval_point: true,
            require_plan_if_estimated_runtime_minutes_ge: Some(15),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePlanPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub prefer_root_plan: bool,
}

impl Default for ActivePlanPolicy {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            prefer_root_plan: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPolicy {
    #[serde(default = "default_true")]
    pub require_correctness_review: bool,
    #[serde(default)]
    pub require_style_review: bool,
    #[serde(default = "default_true")]
    pub allow_same_provider_dual_review: bool,
    #[serde(default = "default_true")]
    pub prefer_cross_provider_review: bool,
}

impl Default for ReviewPolicy {
    fn default() -> Self {
        Self {
            require_correctness_review: default_true(),
            require_style_review: false,
            allow_same_provider_dual_review: default_true(),
            prefer_cross_provider_review: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCapturePolicy {
    pub trigger_if_touched_files_gt: Option<u32>,
    #[serde(default)]
    pub trigger_if_new_config: bool,
    #[serde(default)]
    pub trigger_if_behavior_change: bool,
    #[serde(default)]
    pub trigger_if_non_obvious_bugfix: bool,
    #[serde(default = "default_true")]
    pub write_decision_note: bool,
    #[serde(default)]
    pub update_project_memory: bool,
}

impl Default for KnowledgeCapturePolicy {
    fn default() -> Self {
        Self {
            trigger_if_touched_files_gt: Some(3),
            trigger_if_new_config: default_true(),
            trigger_if_behavior_change: default_true(),
            trigger_if_non_obvious_bugfix: default_true(),
            write_decision_note: default_true(),
            update_project_memory: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_archive_dir")]
    pub archive_dir: String,
    #[serde(default = "default_true")]
    pub write_final_summary: bool,
    #[serde(default = "default_true")]
    pub write_metadata_index: bool,
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            archive_dir: default_archive_dir(),
            write_final_summary: default_true(),
            write_metadata_index: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub require_plan_when: WorkflowGatePolicy,
    #[serde(default = "default_workflow_stages")]
    pub stages: Vec<WorkflowStageKind>,
    #[serde(default)]
    pub active_plan: ActivePlanPolicy,
    #[serde(default)]
    pub review_policy: ReviewPolicy,
    #[serde(default)]
    pub knowledge_capture: KnowledgeCapturePolicy,
    #[serde(default)]
    pub archive_policy: ArchivePolicy,
    #[serde(default = "default_max_runtime_depth")]
    pub max_runtime_depth: u8,
    #[serde(default)]
    pub allowed_stages: Vec<WorkflowStageKind>,
}

impl Default for WorkflowSpec {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            require_plan_when: WorkflowGatePolicy::default(),
            stages: default_workflow_stages(),
            active_plan: ActivePlanPolicy::default(),
            review_policy: ReviewPolicy::default(),
            knowledge_capture: KnowledgeCapturePolicy::default(),
            archive_policy: ArchivePolicy::default(),
            max_runtime_depth: default_max_runtime_depth(),
            allowed_stages: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_archive_dir() -> String {
    "docs/plans".to_string()
}

fn default_workflow_stages() -> Vec<WorkflowStageKind> {
    vec![
        WorkflowStageKind::Research,
        WorkflowStageKind::Plan,
        WorkflowStageKind::Build,
        WorkflowStageKind::Review,
        WorkflowStageKind::Archive,
    ]
}

fn default_max_runtime_depth() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::{KnowledgeCapturePolicy, WorkflowGatePolicy};

    #[test]
    fn workflow_gate_policy_option_fields_deserialize_without_default_annotations() {
        let policy: WorkflowGatePolicy = toml::from_str(
            r#"
require_plan_if_cross_module = true
require_plan_if_parallel_agents = true
require_plan_if_new_interface = true
require_plan_if_migration = true
require_plan_if_human_approval_point = true
"#,
        )
        .expect("workflow gate policy should parse");

        assert!(policy.require_plan_if_touched_files_ge.is_none());
        assert!(policy
            .require_plan_if_estimated_runtime_minutes_ge
            .is_none());
    }

    #[test]
    fn knowledge_capture_option_fields_deserialize_without_default_annotations() {
        let policy: KnowledgeCapturePolicy = toml::from_str(
            r#"
trigger_if_new_config = true
trigger_if_behavior_change = true
trigger_if_non_obvious_bugfix = true
write_decision_note = true
update_project_memory = false
"#,
        )
        .expect("knowledge capture policy should parse");

        assert!(policy.trigger_if_touched_files_gt.is_none());
    }
}
