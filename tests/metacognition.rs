use chrono::{DateTime, Duration, Utc};

use meta_agent_control_plane::{
    Config,
    metacognition::{analyze, analyze_with_policy, AnalysisPolicy, DiagnosticRule},
    model::{
        AgentEvent, AgentRef, EvidenceReference, EventEnvelope, Goal, ProgressUpdate,
        Reflection, TaskSpec, TaskStarted, Transport,
    },
    store::Store,
};

fn agent() -> AgentRef {
    AgentRef {
        agent_id: "agent-1".to_owned(),
        provider: "test".to_owned(),
        model: "fixture".to_owned(),
        instance_id: None,
    }
}

fn envelope(event: AgentEvent, occurred_at: DateTime<Utc>) -> EventEnvelope {
    let mut envelope = EventEnvelope::new(agent(), event);
    envelope.occurred_at = occurred_at;
    envelope
}

fn store() -> Store {
    let config = Config::local_test();
    Store::new(config.cache_config(), config.update_channel_capacity)
}

#[tokio::test]
async fn repeated_blocked_attempt_is_explainable_and_deterministic() {
    let store = store();
    let now = Utc::now();
    let task_created = envelope(
        AgentEvent::TaskCreated(TaskSpec {
            task_id: "task-1".to_owned(),
            title: "Stabilize transport".to_owned(),
            goal_id: Some("goal-1".to_owned()),
            depends_on: Vec::new(),
            tags: Vec::new(),
            expected_outcome: Some("Green conformance suite".to_owned()),
        }),
        now - Duration::minutes(40),
    );
    let task_started = envelope(
        AgentEvent::TaskStarted(TaskStarted {
            task_id: "task-1".to_owned(),
            attempt: 4,
            plan_summary: Some("Retry the same network path".to_owned()),
        }),
        now - Duration::minutes(35),
    );
    let progress = envelope(
        AgentEvent::ProgressUpdated(ProgressUpdate {
            task_id: "task-1".to_owned(),
            progress: 0.6,
            summary: "Still blocked".to_owned(),
            blocker: Some("Connection reset".to_owned()),
            next_action: None,
        }),
        now - Duration::minutes(30),
    );
    let source_ids = [task_created.event_id, task_started.event_id, progress.event_id];

    for event in [task_created, task_started, progress] {
        store.ingest(event, Transport::Http).await.unwrap();
    }
    let mut snapshot = store.snapshot().await;
    snapshot.generated_at = now;
    let policy = AnalysisPolicy {
        stale_after_seconds: 60,
        retry_loop_attempts: 3,
        low_confidence_threshold: 0.45,
    };
    let first = analyze_with_policy(&snapshot, policy);
    let second = analyze_with_policy(&snapshot, policy);

    assert_eq!(first, second);
    assert!(first.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule == DiagnosticRule::StalledTask
            && diagnostic.source_event_ids.iter().any(|id| source_ids.contains(id))
    }));
    assert!(first
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule == DiagnosticRule::RetryLoop));
    assert!(first
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule == DiagnosticRule::MissingEvidence));
    assert!(first
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule == DiagnosticRule::MissingNextAction));
}

#[tokio::test]
async fn detects_dependency_cycle_and_orphan_goal() {
    let store = store();
    let now = Utc::now();
    for (task_id, dependency) in [("a", "b"), ("b", "a")] {
        store
            .ingest(
                envelope(
                    AgentEvent::TaskCreated(TaskSpec {
                        task_id: task_id.to_owned(),
                        title: task_id.to_owned(),
                        goal_id: Some("missing-goal".to_owned()),
                        depends_on: vec![dependency.to_owned()],
                        tags: Vec::new(),
                        expected_outcome: None,
                    }),
                    now,
                ),
                Transport::Http,
            )
            .await
            .unwrap();
    }

    let snapshot = store.snapshot().await;
    let analysis = analyze(&snapshot);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == DiagnosticRule::DependencyCycle)
            .count(),
        2
    );
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == DiagnosticRule::OrphanGoal)
            .count(),
        2
    );
}

#[tokio::test]
async fn separates_self_reported_and_evidence_backed_progress() {
    let store = store();
    let now = Utc::now();
    store
        .ingest(
            envelope(
                AgentEvent::GoalDeclared(Goal {
                    goal_id: "goal-1".to_owned(),
                    title: "Evidence-backed delivery".to_owned(),
                    success_criteria: vec!["Tests pass".to_owned()],
                    constraints: Vec::new(),
                    parent_goal_id: None,
                }),
                now,
            ),
            Transport::Http,
        )
        .await
        .unwrap();
    store
        .ingest(
            envelope(
                AgentEvent::TaskCreated(TaskSpec {
                    task_id: "task-1".to_owned(),
                    title: "Run tests".to_owned(),
                    goal_id: Some("goal-1".to_owned()),
                    depends_on: Vec::new(),
                    tags: Vec::new(),
                    expected_outcome: None,
                }),
                now,
            ),
            Transport::Http,
        )
        .await
        .unwrap();
    store
        .ingest(
            envelope(
                AgentEvent::ProgressUpdated(ProgressUpdate {
                    task_id: "task-1".to_owned(),
                    progress: 0.8,
                    summary: "Tests mostly pass".to_owned(),
                    blocker: None,
                    next_action: Some("Fix the final failure".to_owned()),
                }),
                now,
            ),
            Transport::Http,
        )
        .await
        .unwrap();

    let before = analyze(&store.snapshot().await);
    assert_eq!(before.summary.self_reported_progress, 0.8);
    assert_eq!(before.summary.evidence_backed_progress, 0.0);

    store
        .ingest(
            envelope(
                AgentEvent::ReflectionRecorded(Reflection {
                    task_id: Some("task-1".to_owned()),
                    summary: "The test run is observable".to_owned(),
                    confidence: 0.9,
                    assumptions: Vec::new(),
                    evidence: vec![EvidenceReference {
                        kind: "test".to_owned(),
                        reference: "ci/run/42".to_owned(),
                        summary: Some("Exact-head test run".to_owned()),
                    }],
                    alternatives_considered: Vec::new(),
                    risks: Vec::new(),
                    next_action: Some("Fix the final failure".to_owned()),
                }),
                now + Duration::seconds(1),
            ),
            Transport::Http,
        )
        .await
        .unwrap();

    let after = analyze(&store.snapshot().await);
    assert_eq!(after.summary.self_reported_progress, 0.8);
    assert_eq!(after.summary.evidence_backed_progress, 0.8);
    assert_eq!(after.summary.evidence_coverage, 1.0);
    assert_eq!(after.goals[0].critical_path_remaining, Some(1));
}
