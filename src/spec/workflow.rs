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
    #[serde(default = "default_require_plan_if_touched_files_ge")]
    pub require_plan_if_touched_files_ge: Option<u32>,
    #[serde(default = "default_true")]
    pub require_plan_if_cross_module: bool,
    #[serde(default = "default_true")]
    pub require_plan_if_parallel_agents: bool,
    #[serde(default = "default_true")]
    pub require_plan_if_new_interface: bool,
    #[serde(default = "default_true")]
    pub require_plan_if_migration: bool,
    #[serde(default = "default_true")]
    pub require_plan_if_human_approval_point: bool,
    #[serde(default = "default_require_plan_if_estimated_runtime_minutes_ge")]
    pub require_plan_if_estimated_runtime_minutes_ge: Option<u32>,
}

impl Default for WorkflowGatePolicy {
    fn default() -> Self {
        Self {
            require_plan_if_touched_files_ge: default_require_plan_if_touched_files_ge(),
            require_plan_if_cross_module: true,
            require_plan_if_parallel_agents: true,
            require_plan_if_new_interface: true,
            require_plan_if_migration: true,
            require_plan_if_human_approval_point: true,
            require_plan_if_estimated_runtime_minutes_ge:
                default_require_plan_if_estimated_runtime_minutes_ge(),
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
    #[serde(default = "default_trigger_if_touched_files_gt")]
    pub trigger_if_touched_files_gt: Option<u32>,
    #[serde(default = "default_true")]
    pub trigger_if_new_config: bool,
    #[serde(default = "default_true")]
    pub trigger_if_behavior_change: bool,
    #[serde(default = "default_true")]
    pub trigger_if_non_obvious_bugfix: bool,
    #[serde(default = "default_true")]
    pub write_decision_note: bool,
    #[serde(default)]
    pub update_project_memory: bool,
}

impl Default for KnowledgeCapturePolicy {
    fn default() -> Self {
        Self {
            trigger_if_touched_files_gt: default_trigger_if_touched_files_gt(),
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

fn default_require_plan_if_touched_files_ge() -> Option<u32> {
    Some(5)
}

fn default_require_plan_if_estimated_runtime_minutes_ge() -> Option<u32> {
    Some(15)
}

fn default_trigger_if_touched_files_gt() -> Option<u32> {
    Some(3)
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
    use super::{KnowledgeCapturePolicy, WorkflowGatePolicy, WorkflowSpec};

    #[test]
    fn workflow_gate_policy_partial_deserialization_preserves_business_defaults() {
        let policy: WorkflowGatePolicy = toml::from_str(
            r#"
require_plan_if_cross_module = false
"#,
        )
        .expect("workflow gate policy should parse");

        assert_eq!(policy.require_plan_if_touched_files_ge, Some(5));
        assert!(!policy.require_plan_if_cross_module);
        assert!(policy.require_plan_if_parallel_agents);
        assert!(policy.require_plan_if_new_interface);
        assert!(policy.require_plan_if_migration);
        assert!(policy.require_plan_if_human_approval_point);
        assert_eq!(
            policy.require_plan_if_estimated_runtime_minutes_ge,
            Some(15)
        );
    }

    #[test]
    fn knowledge_capture_partial_deserialization_preserves_business_defaults() {
        let policy: KnowledgeCapturePolicy = toml::from_str(
            r#"
update_project_memory = false
"#,
        )
        .expect("knowledge capture policy should parse");

        assert_eq!(policy.trigger_if_touched_files_gt, Some(3));
        assert!(policy.trigger_if_new_config);
        assert!(policy.trigger_if_behavior_change);
        assert!(policy.trigger_if_non_obvious_bugfix);
        assert!(policy.write_decision_note);
        assert!(!policy.update_project_memory);
    }

    #[test]
    fn workflow_spec_partial_nested_tables_inherit_subpolicy_defaults() {
        let workflow: WorkflowSpec = toml::from_str(
            r#"
enabled = true

[require_plan_when]
require_plan_if_parallel_agents = false

[knowledge_capture]
update_project_memory = true
"#,
        )
        .expect("workflow spec should parse");

        assert_eq!(
            workflow.require_plan_when.require_plan_if_touched_files_ge,
            Some(5)
        );
        assert!(workflow.require_plan_when.require_plan_if_cross_module);
        assert!(!workflow.require_plan_when.require_plan_if_parallel_agents);
        assert_eq!(
            workflow
                .require_plan_when
                .require_plan_if_estimated_runtime_minutes_ge,
            Some(15)
        );
        assert_eq!(
            workflow.knowledge_capture.trigger_if_touched_files_gt,
            Some(3)
        );
        assert!(workflow.knowledge_capture.trigger_if_new_config);
        assert!(workflow.knowledge_capture.trigger_if_behavior_change);
        assert!(workflow.knowledge_capture.trigger_if_non_obvious_bugfix);
        assert!(workflow.knowledge_capture.write_decision_note);
        assert!(workflow.knowledge_capture.update_project_memory);
    }
}
