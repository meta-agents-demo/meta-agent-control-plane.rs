use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    model::TaskOutcome,
    store::{EventRecord, Snapshot, TaskState, TaskStatus},
};

const MAX_SOURCE_EVENT_IDS: usize = 12;

type TaskKey = (String, String);
type GoalKey = (String, String);
type TaskLookup<'a> = BTreeMap<TaskKey, &'a TaskState>;
type TaskEventIndex<'a> = BTreeMap<TaskKey, Vec<&'a EventRecord>>;
type CriticalPathResult = (BTreeSet<TaskKey>, BTreeMap<GoalKey, Option<usize>>);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPolicy {
    pub stale_after_seconds: i64,
    pub retry_loop_attempts: u32,
    pub low_confidence_threshold: f32,
}

impl Default for AnalysisPolicy {
    fn default() -> Self {
        Self {
            stale_after_seconds: 15 * 60,
            retry_loop_attempts: 3,
            low_confidence_threshold: 0.45,
        }
    }
}

impl AnalysisPolicy {
    fn normalized(self) -> Self {
        Self {
            stale_after_seconds: self.stale_after_seconds.max(1),
            retry_loop_attempts: self.retry_loop_attempts.max(2),
            low_confidence_threshold: if self.low_confidence_threshold.is_finite() {
                self.low_confidence_threshold.clamp(0.0, 1.0)
            } else {
                Self::default().low_confidence_threshold
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Critical,
}

impl DiagnosticSeverity {
    const fn rank(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Critical => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticRule {
    StalledTask,
    RetryLoop,
    BlockedDependency,
    OrphanDependency,
    DependencyCycle,
    OrphanGoal,
    MissingEvidence,
    LowConfidence,
    MissingNextAction,
    CompletionMismatch,
}

impl DiagnosticRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StalledTask => "stalled_task",
            Self::RetryLoop => "retry_loop",
            Self::BlockedDependency => "blocked_dependency",
            Self::OrphanDependency => "orphan_dependency",
            Self::DependencyCycle => "dependency_cycle",
            Self::OrphanGoal => "orphan_goal",
            Self::MissingEvidence => "missing_evidence",
            Self::LowConfidence => "low_confidence",
            Self::MissingNextAction => "missing_next_action",
            Self::CompletionMismatch => "completion_mismatch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub diagnostic_id: String,
    pub rule: DiagnosticRule,
    pub severity: DiagnosticSeverity,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub summary: String,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    pub source_events_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskAnalysis {
    pub agent_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub status: TaskStatus,
    pub self_reported_progress: f32,
    pub evidence_backed_progress: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub evidence_count: usize,
    pub attempt: u32,
    pub stale_for_seconds: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker_age_seconds: Option<i64>,
    #[serde(default)]
    pub unresolved_dependencies: Vec<String>,
    #[serde(default)]
    pub missing_dependencies: Vec<String>,
    pub on_critical_path: bool,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    pub source_events_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalAnalysis {
    pub agent_id: String,
    pub goal_id: String,
    pub title: String,
    pub success_criteria_count: usize,
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub blocked_tasks: usize,
    pub completed_tasks: usize,
    pub stalled_tasks: usize,
    pub self_reported_progress: f32,
    pub evidence_backed_progress: f32,
    pub evidence_coverage: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_path_remaining: Option<usize>,
    #[serde(default)]
    pub critical_path_task_ids: Vec<String>,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub data_quality_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetacognitionSummary {
    pub total_goals: usize,
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub blocked_tasks: usize,
    pub stalled_tasks: usize,
    pub retry_loops: usize,
    pub critical_diagnostics: usize,
    pub warning_diagnostics: usize,
    pub info_diagnostics: usize,
    pub self_reported_progress: f32,
    pub evidence_backed_progress: f32,
    pub evidence_coverage: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetacognitionSnapshot {
    pub generated_at: DateTime<Utc>,
    pub revision: u64,
    pub policy: AnalysisPolicy,
    pub summary: MetacognitionSummary,
    pub goals: Vec<GoalAnalysis>,
    pub tasks: Vec<TaskAnalysis>,
    pub diagnostics: Vec<Diagnostic>,
}
