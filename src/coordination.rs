use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    metacognition::{
        AnalysisPolicy, Diagnostic, DiagnosticRule, DiagnosticSeverity, MetacognitionSnapshot,
        TaskAnalysis, analyze_with_policy,
    },
    model::AgentStatus,
    store::{Snapshot, TaskState, TaskStatus},
};

const MAX_SOURCE_EVENT_IDS: usize = 16;

type TaskKey = (String, String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningPolicy {
    pub max_assignments: usize,
    pub max_assignments_per_agent: usize,
    pub max_interventions: usize,
    pub max_holds: usize,
}

impl Default for PlanningPolicy {
    fn default() -> Self {
        Self {
            max_assignments: 16,
            max_assignments_per_agent: 2,
            max_interventions: 32,
            max_holds: 64,
        }
    }
}

impl PlanningPolicy {
    pub fn validate(self) -> Result<Self, PlanningError> {
        for (name, value) in [
            ("maximum assignments", self.max_assignments),
            (
                "maximum assignments per agent",
                self.max_assignments_per_agent,
            ),
            ("maximum interventions", self.max_interventions),
            ("maximum holds", self.max_holds),
        ] {
            if value == 0 {
                return Err(PlanningError::InvalidPolicy { name });
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentAction {
    StartTask,
    ContinueTask,
    ResolveBlocker,
    RequestCheckpoint,
    ChangeStrategy,
    GatherEvidence,
    DefineNextAction,
}

impl AssignmentAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StartTask => "start_task",
            Self::ContinueTask => "continue_task",
            Self::ResolveBlocker => "resolve_blocker",
            Self::RequestCheckpoint => "request_checkpoint",
            Self::ChangeStrategy => "change_strategy",
            Self::GatherEvidence => "gather_evidence",
            Self::DefineNextAction => "define_next_action",
        }
    }

    const fn base_priority(self) -> u32 {
        match self {
            Self::ChangeStrategy => 900,
            Self::RequestCheckpoint => 850,
            Self::ResolveBlocker => 800,
            Self::GatherEvidence => 700,
            Self::DefineNextAction => 650,
            Self::StartTask => 600,
            Self::ContinueTask => 500,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    RepairDependencyGraph,
    DeclareMissingGoal,
    ReconcileCompletion,
}

impl InterventionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RepairDependencyGraph => "repair_dependency_graph",
            Self::DeclareMissingGoal => "declare_missing_goal",
            Self::ReconcileCompletion => "reconcile_completion",
        }
    }

    const fn base_priority(self) -> u32 {
        match self {
            Self::ReconcileCompletion => 1_200,
            Self::RepairDependencyGraph => 1_100,
            Self::DeclareMissingGoal => 1_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    AgentOffline,
    WaitingOnDependencies,
    Terminal,
    AssignmentLimit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub assignment_id: String,
    pub agent_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub action: AssignmentAction,
    pub priority: u32,
    pub rationale: String,
    pub recommended_action: String,
    pub on_critical_path: bool,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    pub source_events_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intervention {
    pub intervention_id: String,
    pub agent_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub kind: InterventionKind,
    pub priority: u32,
    pub rationale: String,
    pub recommended_action: String,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    pub source_events_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeldTask {
    pub agent_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub reason: HoldReason,
    pub explanation: String,
    #[serde(default)]
    pub unresolved_dependencies: Vec<String>,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub source_event_ids: Vec<Uuid>,
    pub source_events_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinationSummary {
    pub total_tasks: usize,
    pub assignment_candidates: usize,
    pub assignments: usize,
    pub agents_with_assignments: usize,
    pub interventions: usize,
    pub held_tasks: usize,
    pub suppressed_by_assignment_limits: usize,
    pub omitted_interventions: usize,
    pub omitted_holds: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinationPlan {
    pub generated_at: DateTime<Utc>,
    pub revision: u64,
    pub planning_policy: PlanningPolicy,
    pub analysis_policy: AnalysisPolicy,
    pub summary: CoordinationSummary,
    pub assignments: Vec<Assignment>,
    pub interventions: Vec<Intervention>,
    pub held_tasks: Vec<HeldTask>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum PlanningError {
    #[error("{name} must be greater than zero")]
    InvalidPolicy { name: &'static str },
}

pub fn build_plan(snapshot: &Snapshot) -> Result<CoordinationPlan, PlanningError> {
    build_plan_with_policy(
        snapshot,
        AnalysisPolicy::default(),
        PlanningPolicy::default(),
    )
}

pub fn build_plan_with_policy(
    snapshot: &Snapshot,
    analysis_policy: AnalysisPolicy,
    planning_policy: PlanningPolicy,
) -> Result<CoordinationPlan, PlanningError> {
    let planning_policy = planning_policy.validate()?;
    let analysis = analyze_with_policy(snapshot, analysis_policy);
    Ok(plan_from_analysis(snapshot, analysis, planning_policy))
}

fn plan_from_analysis(
    snapshot: &Snapshot,
    analysis: MetacognitionSnapshot,
    planning_policy: PlanningPolicy,
) -> CoordinationPlan {
    let task_lookup = snapshot
        .tasks
        .iter()
        .map(|task| ((task.agent_id.clone(), task.task.task_id.clone()), task))
        .collect::<BTreeMap<_, _>>();
    let agent_statuses = snapshot
        .agents
        .iter()
        .map(|agent| (agent.agent.agent_id.clone(), agent.status.clone()))
        .collect::<BTreeMap<_, _>>();
    let diagnostics_by_task = diagnostics_by_task(&analysis.diagnostics);

    let mut assignment_candidates = Vec::new();
    let mut interventions = Vec::new();
    let mut held_tasks = Vec::new();

    let mut tasks = analysis.tasks.iter().collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        (&left.agent_id, &left.task_id).cmp(&(&right.agent_id, &right.task_id))
    });

    for task in tasks {
        let key = (task.agent_id.clone(), task.task_id.clone());
        let task_state = task_lookup.get(&key).copied();
        let diagnostics = diagnostics_by_task.get(&key).map_or(&[][..], Vec::as_slice);

        if let Some(intervention) = intervention_for(task, diagnostics) {
            interventions.push(intervention);
            continue;
        }

        if agent_statuses.get(&task.agent_id) == Some(&AgentStatus::Offline) {
            held_tasks.push(hold(
                task,
                HoldReason::AgentOffline,
                "The owning agent is currently offline, so no executable assignment is emitted."
                    .to_owned(),
                diagnostics,
            ));
            continue;
        }

        if !task.missing_dependencies.is_empty() || !task.unresolved_dependencies.is_empty() {
            held_tasks.push(hold(
                task,
                HoldReason::WaitingOnDependencies,
                format!(
                    "The task is waiting on unresolved dependencies: {}.",
                    task.unresolved_dependencies
                        .iter()
                        .chain(task.missing_dependencies.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                diagnostics,
            ));
            continue;
        }

        if let Some(action) = assignment_action(task, diagnostics) {
            assignment_candidates.push(assignment(task, task_state, diagnostics, action));
        } else {
            held_tasks.push(hold(
                task,
                HoldReason::Terminal,
                "The task is terminal and has no retained diagnostic requiring follow-up."
                    .to_owned(),
                diagnostics,
            ));
        }
    }

    assignment_candidates.sort_by(assignment_order);
    interventions.sort_by(intervention_order);
    held_tasks.sort_by(hold_order);

    let candidate_count = assignment_candidates.len();
    let (assignments, suppressed) = select_assignments(
        assignment_candidates,
        planning_policy.max_assignments,
        planning_policy.max_assignments_per_agent,
    );
    let suppressed_count = suppressed.len();
    held_tasks.extend(suppressed.into_iter().map(|assignment| {
        HeldTask {
            agent_id: assignment.agent_id,
            task_id: assignment.task_id,
            goal_id: assignment.goal_id,
            reason: HoldReason::AssignmentLimit,
            explanation:
                "A higher-priority fair-share assignment consumed the configured planning capacity."
                    .to_owned(),
            unresolved_dependencies: Vec::new(),
            diagnostic_ids: assignment.diagnostic_ids,
            source_event_ids: assignment.source_event_ids,
            source_events_retained: assignment.source_events_retained,
        }
    }));
    held_tasks.sort_by(hold_order);

    let omitted_interventions = interventions
        .len()
        .saturating_sub(planning_policy.max_interventions);
    interventions.truncate(planning_policy.max_interventions);
    let omitted_holds = held_tasks.len().saturating_sub(planning_policy.max_holds);
    held_tasks.truncate(planning_policy.max_holds);

    let agents_with_assignments = assignments
        .iter()
        .map(|assignment| assignment.agent_id.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let summary = CoordinationSummary {
        total_tasks: analysis.tasks.len(),
        assignment_candidates: candidate_count,
        assignments: assignments.len(),
        agents_with_assignments,
        interventions: interventions.len(),
        held_tasks: held_tasks.len(),
        suppressed_by_assignment_limits: suppressed_count,
        omitted_interventions,
        omitted_holds,
    };

    CoordinationPlan {
        generated_at: analysis.generated_at,
        revision: analysis.revision,
        planning_policy,
        analysis_policy: analysis.policy,
        summary,
        assignments,
        interventions,
        held_tasks,
    }
}

fn diagnostics_by_task<'a>(
    diagnostics: &'a [Diagnostic],
) -> BTreeMap<TaskKey, Vec<&'a Diagnostic>> {
    let mut result = BTreeMap::new();
    for diagnostic in diagnostics {
        if let Some(task_id) = diagnostic.task_id.as_deref() {
            result
                .entry((diagnostic.agent_id.clone(), task_id.to_owned()))
                .or_insert_with(Vec::new)
                .push(diagnostic);
        }
    }
    for values in result.values_mut() {
        values.sort_by(|left, right| left.diagnostic_id.cmp(&right.diagnostic_id));
    }
    result
}

fn intervention_for(task: &TaskAnalysis, diagnostics: &[&Diagnostic]) -> Option<Intervention> {
    let kind = if has_rule(diagnostics, DiagnosticRule::CompletionMismatch) {
        Some(InterventionKind::ReconcileCompletion)
    } else if has_rule(diagnostics, DiagnosticRule::DependencyCycle)
        || has_rule(diagnostics, DiagnosticRule::OrphanDependency)
    {
        Some(InterventionKind::RepairDependencyGraph)
    } else if has_rule(diagnostics, DiagnosticRule::OrphanGoal) {
        Some(InterventionKind::DeclareMissingGoal)
    } else {
        None
    }?;

    let priority = kind
        .base_priority()
        .saturating_add(if task.on_critical_path { 100 } else { 0 })
        .saturating_add(max_severity_bonus(diagnostics));
    Some(Intervention {
        intervention_id: format!("{}:{}:{}", kind.as_str(), task.agent_id, task.task_id),
        agent_id: task.agent_id.clone(),
        task_id: task.task_id.clone(),
        goal_id: task.goal_id.clone(),
        kind,
        priority,
        rationale: diagnostic_rationale(diagnostics),
        recommended_action: recommended_action(diagnostics).unwrap_or_else(|| match kind {
            InterventionKind::RepairDependencyGraph => {
                "Repair the dependency graph before dispatching this task.".to_owned()
            }
            InterventionKind::DeclareMissingGoal => {
                "Declare the referenced goal or explicitly reassign the task.".to_owned()
            }
            InterventionKind::ReconcileCompletion => {
                "Replay the visible lifecycle events and publish one consistent terminal state."
                    .to_owned()
            }
        }),
        diagnostic_ids: diagnostic_ids(diagnostics),
        source_event_ids: source_event_ids(task, diagnostics),
        source_events_retained: task.source_events_retained,
    })
}

fn assignment_action(task: &TaskAnalysis, diagnostics: &[&Diagnostic]) -> Option<AssignmentAction> {
    if has_rule(diagnostics, DiagnosticRule::RetryLoop) {
        Some(AssignmentAction::ChangeStrategy)
    } else if has_rule(diagnostics, DiagnosticRule::StalledTask) {
        Some(AssignmentAction::RequestCheckpoint)
    } else if task.status == TaskStatus::Blocked {
        Some(AssignmentAction::ResolveBlocker)
    } else if has_rule(diagnostics, DiagnosticRule::LowConfidence)
        || has_rule(diagnostics, DiagnosticRule::MissingEvidence)
    {
        Some(AssignmentAction::GatherEvidence)
    } else if has_rule(diagnostics, DiagnosticRule::MissingNextAction) {
        Some(AssignmentAction::DefineNextAction)
    } else {
        match task.status {
            TaskStatus::Pending => Some(AssignmentAction::StartTask),
            TaskStatus::Running => Some(AssignmentAction::ContinueTask),
            TaskStatus::Blocked => Some(AssignmentAction::ResolveBlocker),
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Partial => None,
        }
    }
}

fn assignment(
    task: &TaskAnalysis,
    task_state: Option<&TaskState>,
    diagnostics: &[&Diagnostic],
    action: AssignmentAction,
) -> Assignment {
    let priority = action
        .base_priority()
        .saturating_add(if task.on_critical_path { 100 } else { 0 })
        .saturating_add(max_severity_bonus(diagnostics))
        .saturating_add(task.attempt.min(10).saturating_mul(5))
        .saturating_add((task.stale_for_seconds.max(0) as u32 / 60).min(100));
    Assignment {
        assignment_id: format!("{}:{}:{}", action.as_str(), task.agent_id, task.task_id),
        agent_id: task.agent_id.clone(),
        task_id: task.task_id.clone(),
        goal_id: task.goal_id.clone(),
        action,
        priority,
        rationale: if diagnostics.is_empty() {
            match action {
                AssignmentAction::StartTask => {
                    "The task is pending and all retained dependencies are complete.".to_owned()
                }
                AssignmentAction::ContinueTask => {
                    "The task is active, dependency-safe, and has no higher-severity retained diagnostic."
                        .to_owned()
                }
                _ => "The retained task state requires an explicit next step.".to_owned(),
            }
        } else {
            diagnostic_rationale(diagnostics)
        },
        recommended_action: task_state
            .and_then(|state| state.next_action.clone())
            .or_else(|| recommended_action(diagnostics))
            .unwrap_or_else(|| default_action(action)),
        on_critical_path: task.on_critical_path,
        diagnostic_ids: diagnostic_ids(diagnostics),
        source_event_ids: source_event_ids(task, diagnostics),
        source_events_retained: task.source_events_retained,
    }
}

fn hold(
    task: &TaskAnalysis,
    reason: HoldReason,
    explanation: String,
    diagnostics: &[&Diagnostic],
) -> HeldTask {
    HeldTask {
        agent_id: task.agent_id.clone(),
        task_id: task.task_id.clone(),
        goal_id: task.goal_id.clone(),
        reason,
        explanation,
        unresolved_dependencies: task
            .unresolved_dependencies
            .iter()
            .chain(task.missing_dependencies.iter())
            .cloned()
            .collect(),
        diagnostic_ids: diagnostic_ids(diagnostics),
        source_event_ids: source_event_ids(task, diagnostics),
        source_events_retained: task.source_events_retained,
    }
}

fn select_assignments(
    candidates: Vec<Assignment>,
    maximum: usize,
    maximum_per_agent: usize,
) -> (Vec<Assignment>, Vec<Assignment>) {
    let mut grouped = BTreeMap::<String, VecDeque<Assignment>>::new();
    for candidate in candidates {
        grouped
            .entry(candidate.agent_id.clone())
            .or_default()
            .push_back(candidate);
    }
    let mut agent_order = grouped.keys().cloned().collect::<Vec<_>>();
    agent_order.sort_by(|left, right| {
        let left_priority = grouped
            .get(left)
            .and_then(|values| values.front())
            .map_or(0, |value| value.priority);
        let right_priority = grouped
            .get(right)
            .and_then(|values| values.front())
            .map_or(0, |value| value.priority);
        right_priority
            .cmp(&left_priority)
            .then_with(|| left.cmp(right))
    });

    let mut selected = Vec::new();
    let mut selected_per_agent = BTreeMap::<String, usize>::new();
    loop {
        let mut made_progress = false;
        for agent_id in &agent_order {
            if selected.len() >= maximum {
                break;
            }
            if selected_per_agent.get(agent_id).copied().unwrap_or(0) >= maximum_per_agent {
                continue;
            }
            let Some(candidate) = grouped.get_mut(agent_id).and_then(VecDeque::pop_front) else {
                continue;
            };
            selected.push(candidate);
            *selected_per_agent.entry(agent_id.clone()).or_default() += 1;
            made_progress = true;
        }
        if selected.len() >= maximum || !made_progress {
            break;
        }
    }

    let mut suppressed = grouped
        .into_values()
        .flat_map(VecDeque::into_iter)
        .collect::<Vec<_>>();
    suppressed.sort_by(assignment_order);
    (selected, suppressed)
}

fn has_rule(diagnostics: &[&Diagnostic], rule: DiagnosticRule) -> bool {
    diagnostics.iter().any(|diagnostic| diagnostic.rule == rule)
}

fn max_severity_bonus(diagnostics: &[&Diagnostic]) -> u32 {
    diagnostics
        .iter()
        .map(|diagnostic| match diagnostic.severity {
            DiagnosticSeverity::Critical => 80,
            DiagnosticSeverity::Warning => 40,
            DiagnosticSeverity::Info => 10,
        })
        .max()
        .unwrap_or(0)
}

fn diagnostic_ids(diagnostics: &[&Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.diagnostic_id.clone())
        .collect()
}

fn source_event_ids(task: &TaskAnalysis, diagnostics: &[&Diagnostic]) -> Vec<Uuid> {
    let mut values = task.source_event_ids.clone();
    values.extend(
        diagnostics
            .iter()
            .flat_map(|diagnostic| diagnostic.source_event_ids.iter().copied()),
    );
    values.sort();
    values.dedup();
    values.truncate(MAX_SOURCE_EVENT_IDS);
    values
}

fn diagnostic_rationale(diagnostics: &[&Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return "No retained diagnostic was attached.".to_owned();
    }
    diagnostics
        .iter()
        .take(3)
        .map(|diagnostic| diagnostic.summary.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

fn recommended_action(diagnostics: &[&Diagnostic]) -> Option<String> {
    diagnostics
        .iter()
        .find_map(|diagnostic| diagnostic.recommended_action.clone())
}

fn default_action(action: AssignmentAction) -> String {
    match action {
        AssignmentAction::StartTask => "Start the task and publish an observable plan summary.",
        AssignmentAction::ContinueTask => {
            "Execute the next observable step and publish progress with evidence."
        }
        AssignmentAction::ResolveBlocker => {
            "Test the blocker, record the result, and either clear it or revise the plan."
        }
        AssignmentAction::RequestCheckpoint => {
            "Publish a checkpoint with current state, blocker, evidence, and one next action."
        }
        AssignmentAction::ChangeStrategy => {
            "Change a causal variable before another attempt and record the comparison."
        }
        AssignmentAction::GatherEvidence => {
            "Attach a test, artifact, measurement, or independent evidence reference."
        }
        AssignmentAction::DefineNextAction => {
            "Record one concrete, observable, and testable next action."
        }
    }
    .to_owned()
}

fn assignment_order(left: &Assignment, right: &Assignment) -> std::cmp::Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| left.agent_id.cmp(&right.agent_id))
        .then_with(|| left.task_id.cmp(&right.task_id))
        .then_with(|| left.assignment_id.cmp(&right.assignment_id))
}

fn intervention_order(left: &Intervention, right: &Intervention) -> std::cmp::Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| left.agent_id.cmp(&right.agent_id))
        .then_with(|| left.task_id.cmp(&right.task_id))
        .then_with(|| left.intervention_id.cmp(&right.intervention_id))
}

fn hold_order(left: &HeldTask, right: &HeldTask) -> std::cmp::Ordering {
    left.agent_id
        .cmp(&right.agent_id)
        .then_with(|| left.task_id.cmp(&right.task_id))
        .then_with(|| hold_rank(left.reason).cmp(&hold_rank(right.reason)))
}

const fn hold_rank(reason: HoldReason) -> u8 {
    match reason {
        HoldReason::AgentOffline => 1,
        HoldReason::WaitingOnDependencies => 2,
        HoldReason::AssignmentLimit => 3,
        HoldReason::Terminal => 4,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use crate::{
        Config,
        model::{
            AgentEvent, AgentRef, AgentStatus, EventEnvelope, ProgressUpdate, StatusChange,
            TaskSpec, TaskStarted, Transport,
        },
        store::Store,
    };

    use super::*;

    fn agent(agent_id: &str) -> AgentRef {
        AgentRef {
            agent_id: agent_id.to_owned(),
            provider: "test".to_owned(),
            model: "fixture".to_owned(),
            instance_id: None,
        }
    }

    fn store() -> Store {
        let config = Config::local_test();
        Store::new(config.cache_config(), config.update_channel_capacity)
    }

    async fn create_task(store: &Store, agent_id: &str, task_id: &str, dependencies: Vec<&str>) {
        store
            .ingest(
                EventEnvelope::new(
                    agent(agent_id),
                    AgentEvent::TaskCreated(TaskSpec {
                        task_id: task_id.to_owned(),
                        title: task_id.to_owned(),
                        goal_id: None,
                        depends_on: dependencies.into_iter().map(str::to_owned).collect(),
                        tags: Vec::new(),
                        expected_outcome: None,
                    }),
                ),
                Transport::Http,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dependency_waiting_is_never_dispatched() {
        let store = store();
        create_task(&store, "agent-a", "prerequisite", Vec::new()).await;
        create_task(&store, "agent-a", "dependent", vec!["prerequisite"]).await;

        let plan = build_plan(&store.snapshot().await).unwrap();
        assert!(
            plan.assignments
                .iter()
                .any(|assignment| assignment.task_id == "prerequisite")
        );
        assert!(
            !plan
                .assignments
                .iter()
                .any(|assignment| assignment.task_id == "dependent")
        );
        assert!(plan.held_tasks.iter().any(|held| {
            held.task_id == "dependent" && held.reason == HoldReason::WaitingOnDependencies
        }));
    }

    #[tokio::test]
    async fn dependency_cycles_require_intervention_instead_of_dispatch() {
        let store = store();
        create_task(&store, "agent-a", "a", vec!["b"]).await;
        create_task(&store, "agent-a", "b", vec!["a"]).await;

        let plan = build_plan(&store.snapshot().await).unwrap();
        assert!(plan.assignments.is_empty());
        assert_eq!(plan.interventions.len(), 2);
        assert!(
            plan.interventions.iter().all(|intervention| {
                intervention.kind == InterventionKind::RepairDependencyGraph
            })
        );
    }

    #[tokio::test]
    async fn fair_share_selection_includes_multiple_agents() {
        let store = store();
        for task_id in ["a-1", "a-2", "a-3"] {
            create_task(&store, "agent-a", task_id, Vec::new()).await;
        }
        create_task(&store, "agent-b", "b-1", Vec::new()).await;

        let plan = build_plan_with_policy(
            &store.snapshot().await,
            AnalysisPolicy::default(),
            PlanningPolicy {
                max_assignments: 2,
                max_assignments_per_agent: 2,
                max_interventions: 8,
                max_holds: 8,
            },
        )
        .unwrap();
        let agents = plan
            .assignments
            .iter()
            .map(|assignment| assignment.agent_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(agents, BTreeSet::from(["agent-a", "agent-b"]));
        assert_eq!(plan.summary.suppressed_by_assignment_limits, 2);
    }

    #[tokio::test]
    async fn retry_loop_precedes_stall_and_is_deterministic() {
        let store = store();
        create_task(&store, "agent-a", "retry-task", Vec::new()).await;
        let started = EventEnvelope::new(
            agent("agent-a"),
            AgentEvent::TaskStarted(TaskStarted {
                task_id: "retry-task".to_owned(),
                attempt: 4,
                plan_summary: Some("Repeat the same plan".to_owned()),
            }),
        );
        store.ingest(started, Transport::Http).await.unwrap();
        let mut snapshot = store.snapshot().await;
        snapshot.generated_at += Duration::minutes(30);

        let policy = AnalysisPolicy {
            stale_after_seconds: 60,
            retry_loop_attempts: 3,
            low_confidence_threshold: 0.45,
        };
        let first = build_plan_with_policy(&snapshot, policy, PlanningPolicy::default()).unwrap();
        let second = build_plan_with_policy(&snapshot, policy, PlanningPolicy::default()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.assignments[0].action,
            AssignmentAction::ChangeStrategy
        );
    }

    #[tokio::test]
    async fn offline_agents_are_held_without_assignments() {
        let store = store();
        create_task(&store, "agent-a", "task-a", Vec::new()).await;
        store
            .ingest(
                EventEnvelope::new(
                    agent("agent-a"),
                    AgentEvent::AgentStatusChanged(StatusChange {
                        status: AgentStatus::Offline,
                        reason: Some("maintenance".to_owned()),
                    }),
                ),
                Transport::Http,
            )
            .await
            .unwrap();

        let plan = build_plan(&store.snapshot().await).unwrap();
        assert!(plan.assignments.is_empty());
        assert_eq!(plan.held_tasks[0].reason, HoldReason::AgentOffline);
    }

    #[tokio::test]
    async fn blocked_task_gets_explicit_recovery_assignment() {
        let store = store();
        create_task(&store, "agent-a", "blocked-task", Vec::new()).await;
        store
            .ingest(
                EventEnvelope::new(
                    agent("agent-a"),
                    AgentEvent::ProgressUpdated(ProgressUpdate {
                        task_id: "blocked-task".to_owned(),
                        progress: 0.4,
                        summary: "Blocked on a deterministic fixture.".to_owned(),
                        blocker: Some("fixture missing".to_owned()),
                        next_action: Some("Create the fixture and rerun the test.".to_owned()),
                    }),
                ),
                Transport::Http,
            )
            .await
            .unwrap();

        let plan = build_plan(&store.snapshot().await).unwrap();
        assert_eq!(plan.assignments[0].action, AssignmentAction::ResolveBlocker);
        assert_eq!(
            plan.assignments[0].recommended_action,
            "Create the fixture and rerun the test."
        );
    }
}
