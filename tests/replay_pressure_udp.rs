use std::time::Duration;

use chrono::{TimeZone, Utc};
use meta_agent_control_plane::{
    auth::AuthPolicy,
    config::{CacheConfig, Config},
    model::{
        AgentEvent, AgentRef, AgentStatus, EventEnvelope, EvidenceReference, Goal, Heartbeat,
        Lesson, ProgressUpdate, Reflection, TaskCompleted, TaskOutcome, TaskSpec, TaskStarted,
        Transport, TransportFrame, TransportPayload,
    },
    store::{Snapshot, Store},
    udp,
};
use serde_json::{Value, json};
use tokio::{net::UdpSocket, sync::broadcast::error::RecvError, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn agent(agent_id: &str) -> AgentRef {
    AgentRef {
        agent_id: agent_id.to_owned(),
        provider: "test-provider".to_owned(),
        model: "test-model".to_owned(),
        instance_id: Some("ci-instance".to_owned()),
    }
}

fn fixed_event(
    event_id: u128,
    second_offset: i64,
    agent_id: &str,
    event: AgentEvent,
) -> EventEnvelope {
    let mut envelope = EventEnvelope::new(agent(agent_id), event);
    envelope.event_id = Uuid::from_u128(event_id);
    envelope.occurred_at = Utc
        .timestamp_opt(1_800_000_000 + second_offset, 0)
        .single()
        .expect("fixed timestamp");
    envelope.session_id = Some("session-replay-1".to_owned());
    envelope.correlation_id = Some("goal-replay-1".to_owned());
    envelope.sequence = Some(u64::try_from(second_offset + 1).expect("positive sequence"));
    envelope
}

fn canonical_sequence() -> Vec<(EventEnvelope, Transport)> {
    vec![
        (
            fixed_event(
                101,
                0,
                "agent-replay",
                AgentEvent::GoalDeclared(Goal {
                    goal_id: "goal-replay-1".to_owned(),
                    title: "Ship deterministic replay".to_owned(),
                    success_criteria: vec!["Projection matches exactly".to_owned()],
                    constraints: vec!["No hidden reasoning".to_owned()],
                    parent_goal_id: None,
                }),
            ),
            Transport::Http,
        ),
        (
            fixed_event(
                102,
                1,
                "agent-replay",
                AgentEvent::TaskCreated(TaskSpec {
                    task_id: "task-replay-1".to_owned(),
                    title: "Validate reducer replay".to_owned(),
                    goal_id: Some("goal-replay-1".to_owned()),
                    depends_on: Vec::new(),
                    tags: vec!["determinism".to_owned()],
                    expected_outcome: Some("Equivalent snapshots".to_owned()),
                }),
            ),
            Transport::Tcp,
        ),
        (
            fixed_event(
                103,
                2,
                "agent-replay",
                AgentEvent::TaskStarted(TaskStarted {
                    task_id: "task-replay-1".to_owned(),
                    attempt: 1,
                    plan_summary: Some("Replay the same ordered event log twice.".to_owned()),
                }),
            ),
            Transport::WebSocket,
        ),
        (
            fixed_event(
                104,
                3,
                "agent-replay",
                AgentEvent::ProgressUpdated(ProgressUpdate {
                    task_id: "task-replay-1".to_owned(),
                    progress: 0.65,
                    summary: "State projection is stable so far.".to_owned(),
                    blocker: None,
                    next_action: Some("Record evidence and complete.".to_owned()),
                }),
            ),
            Transport::Udp,
        ),
        (
            fixed_event(
                105,
                4,
                "agent-replay",
                AgentEvent::ReflectionRecorded(Reflection {
                    task_id: Some("task-replay-1".to_owned()),
                    summary: "The reducer is deterministic for the reviewed sequence.".to_owned(),
                    confidence: 0.95,
                    assumptions: vec!["Events are replayed in canonical order.".to_owned()],
                    evidence: vec![EvidenceReference {
                        kind: "test".to_owned(),
                        reference: "tests/replay_pressure_udp.rs".to_owned(),
                        summary: Some("Exact projection comparison".to_owned()),
                    }],
                    alternatives_considered: vec!["Compare only individual fields.".to_owned()],
                    risks: vec!["Transport counters are intentionally separate.".to_owned()],
                    next_action: Some("Persist the validated lesson.".to_owned()),
                }),
            ),
            Transport::Udp,
        ),
        (
            fixed_event(
                106,
                5,
                "agent-replay",
                AgentEvent::LessonLearned(Lesson {
                    lesson_id: "lesson-replay-1".to_owned(),
                    statement: "Canonical replay produces the same observable projection."
                        .to_owned(),
                    confidence: 0.9,
                    source_task_id: Some("task-replay-1".to_owned()),
                    evidence: Vec::new(),
                    tags: vec!["replay".to_owned(), "determinism".to_owned()],
                    applicability: Some("Ordered v1 event logs".to_owned()),
                }),
            ),
            Transport::Tcp,
        ),
        (
            fixed_event(
                107,
                6,
                "agent-replay",
                AgentEvent::TaskCompleted(TaskCompleted {
                    task_id: "task-replay-1".to_owned(),
                    outcome: TaskOutcome::Succeeded,
                    summary: "Replay and duplicate behavior are certified.".to_owned(),
                    artifacts: vec!["tests/replay_pressure_udp.rs".to_owned()],
                    actual_result: Some("Equivalent bounded projections".to_owned()),
                }),
            ),
            Transport::Http,
        ),
    ]
}

fn derived_projection(snapshot: &Snapshot) -> Value {
    json!({
        "revision": snapshot.revision,
        "agents": &snapshot.agents,
        "goals": &snapshot.goals,
        "tasks": &snapshot.tasks,
        "lessons": &snapshot.lessons,
    })
}

fn full_projection(snapshot: &Snapshot) -> Value {
    json!({
        "derived": derived_projection(snapshot),
        "caches": &snapshot.caches,
        "counters": &snapshot.counters,
    })
}

fn replay_cache() -> CacheConfig {
    CacheConfig {
        agents: 8,
        goals: 8,
        tasks: 16,
        lessons: 8,
        events: 32,
        idempotency: 64,
    }
}

#[tokio::test]
async fn deterministic_replay_reproduces_projection_and_duplicate_replay_is_inert() {
    let first_store = Store::new(replay_cache(), 16);
    let second_store = Store::new(replay_cache(), 16);
    let sequence = canonical_sequence();

    for (event, transport) in &sequence {
        first_store
            .ingest(event.clone(), *transport)
            .await
            .expect("first replay accepted");
        second_store
            .ingest(event.clone(), *transport)
            .await
            .expect("second replay accepted");
    }

    let first_snapshot = first_store.snapshot().await;
    let second_snapshot = second_store.snapshot().await;
    assert_eq!(
        full_projection(&first_snapshot),
        full_projection(&second_snapshot)
    );

    let stable_projection = derived_projection(&first_snapshot);
    let stable_revision = first_snapshot.revision;
    for (event, transport) in &sequence {
        let ack = first_store
            .ingest(event.clone(), *transport)
            .await
            .expect("duplicate replay accepted idempotently");
        assert!(ack.duplicate);
        assert_eq!(ack.revision, stable_revision);
    }

    let duplicate_snapshot = first_store.snapshot().await;
    assert_eq!(derived_projection(&duplicate_snapshot), stable_projection);
    assert_eq!(duplicate_snapshot.revision, stable_revision);
    assert_eq!(
        duplicate_snapshot.counters.duplicate,
        u64::try_from(sequence.len()).expect("sequence length fits u64")
    );
}

#[tokio::test]
async fn normalized_projection_is_transport_independent_for_allowed_telemetry() {
    let event = fixed_event(
        201,
        0,
        "agent-transport",
        AgentEvent::ProgressUpdated(ProgressUpdate {
            task_id: "task-transport".to_owned(),
            progress: 0.4,
            summary: "Equivalent normalized telemetry.".to_owned(),
            blocker: None,
            next_action: Some("Compare the projections.".to_owned()),
        }),
    );
    let mut projections = Vec::new();

    for transport in [
        Transport::Http,
        Transport::WebSocket,
        Transport::Tcp,
        Transport::Udp,
    ] {
        let store = Store::new(replay_cache(), 8);
        store
            .ingest(event.clone(), transport)
            .await
            .expect("telemetry accepted");
        projections.push(derived_projection(&store.snapshot().await));
    }

    for projection in &projections[1..] {
        assert_eq!(projection, &projections[0]);
    }
}

#[tokio::test]
async fn bounded_caches_and_update_channel_hold_under_sustained_slow_consumer_pressure() {
    let store = Store::new(
        CacheConfig {
            agents: 4,
            goals: 2,
            tasks: 5,
            lessons: 2,
            events: 6,
            idempotency: 7,
        },
        2,
    );
    let mut updates = store.subscribe();

    timeout(Duration::from_secs(5), async {
        for index in 0_u64..64 {
            let transport = match index % 4 {
                0 => Transport::Http,
                1 => Transport::WebSocket,
                2 => Transport::Tcp,
                _ => Transport::Udp,
            };
            let event = fixed_event(
                10_000 + u128::from(index),
                i64::try_from(index).expect("index fits i64"),
                &format!("agent-pressure-{index:03}"),
                AgentEvent::ProgressUpdated(ProgressUpdate {
                    task_id: format!("task-pressure-{index:03}"),
                    progress: f32::from(u8::try_from(index % 10).expect("digit")) / 10.0,
                    summary: "Exercise bounded projection pressure.".to_owned(),
                    blocker: None,
                    next_action: Some("Continue bounded ingestion.".to_owned()),
                }),
            );
            store
                .ingest(event, transport)
                .await
                .expect("pressure event accepted");
        }
    })
    .await
    .expect("bounded producer must not block behind slow subscriber");

    let snapshot = store.snapshot().await;
    assert_eq!(snapshot.revision, 64);
    assert_eq!(snapshot.counters.accepted, 64);
    assert_eq!(snapshot.caches.agents.length, 4);
    assert_eq!(snapshot.caches.tasks.length, 5);
    assert_eq!(snapshot.caches.events.length, 6);
    assert_eq!(snapshot.caches.idempotency.length, 7);
    assert_eq!(snapshot.caches.agents.evictions, 60);
    assert_eq!(snapshot.caches.tasks.evictions, 59);
    assert_eq!(snapshot.caches.events.evictions, 58);
    assert_eq!(snapshot.caches.idempotency.evictions, 57);
    assert_eq!(snapshot.recent_events.len(), 6);

    for pressure in [
        snapshot.caches.agents.pressure,
        snapshot.caches.tasks.pressure,
        snapshot.caches.events.pressure,
        snapshot.caches.idempotency.pressure,
    ] {
        assert!((pressure - 1.0).abs() < f64::EPSILON);
    }

    match timeout(Duration::from_secs(1), updates.recv())
        .await
        .expect("subscriber result available")
    {
        Err(RecvError::Lagged(skipped)) => assert!(skipped > 0),
        other => panic!("slow subscriber should observe bounded lag, got {other:?}"),
    }
    let retained = timeout(Duration::from_secs(1), updates.recv())
        .await
        .expect("retained update available")
        .expect("update channel remains open");
    assert!(retained.revision > 0 && retained.revision <= 64);
}

async fn udp_round_trip(
    client: &UdpSocket,
    server_address: std::net::SocketAddr,
    payload: &TransportPayload,
) -> Value {
    let bytes = serde_json::to_vec(payload).expect("serialize UDP payload");
    client
        .send_to(&bytes, server_address)
        .await
        .expect("send UDP payload");

    let mut response = vec![0_u8; 4_096];
    let (length, peer) = timeout(Duration::from_secs(2), client.recv_from(&mut response))
        .await
        .expect("UDP response timeout")
        .expect("receive UDP response");
    assert_eq!(peer, server_address);
    serde_json::from_slice(&response[..length]).expect("valid JSON UDP response")
}

#[tokio::test]
async fn udp_server_rejects_privileged_events_without_mutation_and_accepts_telemetry() {
    let config = Config::local_test();
    let store = Store::new(config.cache_config(), 8);
    let server = UdpSocket::bind(config.udp_addr)
        .await
        .expect("bind UDP server");
    let server_address = server.local_addr().expect("UDP server address");
    let cancellation = CancellationToken::new();
    let server_task = tokio::spawn(udp::serve(
        server,
        store.clone(),
        AuthPolicy::from_config(&config),
        config.max_udp_payload_bytes,
        cancellation.clone(),
    ));
    let client = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind UDP client");

    let privileged = TransportPayload::Frame(TransportFrame {
        token: config.auth_token.clone(),
        event: fixed_event(
            301,
            0,
            "agent-udp",
            AgentEvent::TaskCreated(TaskSpec {
                task_id: "task-privileged".to_owned(),
                title: "Must not be created over UDP".to_owned(),
                goal_id: None,
                depends_on: Vec::new(),
                tags: vec!["privileged".to_owned()],
                expected_outcome: None,
            }),
        ),
    });
    let policy_response = udp_round_trip(&client, server_address, &privileged).await;
    assert_eq!(policy_response["error"], "transport_policy");

    let after_rejection = store.snapshot().await;
    assert_eq!(after_rejection.counters.rejected, 1);
    assert_eq!(after_rejection.counters.accepted, 0);
    assert!(after_rejection.tasks.is_empty());

    let telemetry = TransportPayload::Frame(TransportFrame {
        token: config.auth_token.clone(),
        event: fixed_event(
            302,
            1,
            "agent-udp",
            AgentEvent::Heartbeat(Heartbeat {
                status: Some(AgentStatus::Running),
                active_task_id: None,
                load: Some(0.25),
            }),
        ),
    });
    let ack = udp_round_trip(&client, server_address, &telemetry).await;
    assert_eq!(ack["accepted"], true);
    assert_eq!(ack["duplicate"], false);
    assert_eq!(ack["transport"], "udp");

    let after_telemetry = store.snapshot().await;
    assert_eq!(after_telemetry.counters.accepted, 1);
    assert_eq!(after_telemetry.counters.rejected, 1);
    assert_eq!(after_telemetry.agents.len(), 1);
    assert!(after_telemetry.tasks.is_empty());

    cancellation.cancel();
    timeout(Duration::from_secs(2), server_task)
        .await
        .expect("UDP server shutdown timeout")
        .expect("UDP server task joined")
        .expect("UDP server stopped cleanly");
}
