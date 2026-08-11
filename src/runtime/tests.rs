use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use uuid::Uuid;

use super::*;

fn temp_proc_root() -> PathBuf {
    // Wall-clock nanoseconds are not guaranteed to be unique when Rust runs
    // tests concurrently, especially on filesystems with coarser timestamp
    // resolution. A UUID keeps each fixture isolated from sibling tests.
    let root = env::temp_dir().join(format!("meta-agent-runtime-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_proc_fixture(root: &Path, total_ticks: u64, process_ticks: u64) {
    fs::write(
        root.join("stat"),
        format!("cpu  {total_ticks} 0 0 0 0 0 0 0 0 0\ncpu0 1 0 0 0\ncpu1 1 0 0 0\n"),
    )
    .unwrap();
    fs::write(root.join("meminfo"), "MemTotal:       1000000 kB\n").unwrap();
    let process_root = root.join("4242");
    fs::create_dir_all(&process_root).unwrap();
    let stat = format!(
        "4242 (node) R 1 0 0 0 0 0 0 0 0 0 {} {} 0 0 0 0 0 0 0 0 0 0 0",
        process_ticks / 2,
        process_ticks - process_ticks / 2
    );
    fs::write(process_root.join("stat"), stat).unwrap();
    fs::write(process_root.join("comm"), "node\n").unwrap();
    fs::write(
        process_root.join("cmdline"),
        b"node\0/usr/local/bin/claude\0--safe-mode\0",
    )
    .unwrap();
    fs::write(process_root.join("status"), "VmRSS:\t2048 kB\n").unwrap();
}

fn hook(event_id: Uuid, kind: RuntimeHookKind) -> RuntimeHookEnvelope {
    RuntimeHookEnvelope {
        protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        event_id,
        occurred_at: Utc::now(),
        agent: RuntimeAgentRef {
            agent_id: "claude-test".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-test-model".to_owned(),
            instance_id: Some("test-instance".to_owned()),
        },
        session_id: Some("session-1".to_owned()),
        pid: Some(4242),
        kind,
        control_capable: true,
        summary: Some("Visible activity summary".to_owned()),
        tool_name: None,
        confidence: Some(0.75),
        cpu_percent: None,
        rss_bytes: None,
        memory_percent: None,
        input_tokens_delta: 10,
        output_tokens_delta: 4,
        metadata: BTreeMap::new(),
    }
}

#[tokio::test]
async fn collects_real_proc_fields_and_computes_delta_cpu() {
    let root = temp_proc_root();
    write_proc_fixture(&root, 1_000, 100);
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    monitor.collect_once().await;
    write_proc_fixture(&root, 1_200, 120);
    monitor.collect_once().await;

    let snapshot = monitor.snapshot().await;
    assert_eq!(snapshot.processes.len(), 1);
    assert_eq!(snapshot.processes[0].pid, 4242);
    assert_eq!(snapshot.processes[0].provider, "anthropic");
    assert_eq!(snapshot.processes[0].process_name, "claude-agent");
    assert!(!snapshot.processes[0].process_name.contains("safe-mode"));
    assert_eq!(snapshot.processes[0].rss_bytes, 2 * 1_024 * 1_024);
    assert_eq!(snapshot.processes[0].cpu_percent, Some(20.0));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn merges_hook_confidence_and_tokens_with_process_telemetry() {
    let root = temp_proc_root();
    write_proc_fixture(&root, 1_000, 100);
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    monitor.collect_once().await;
    monitor
        .ingest_hook(hook(Uuid::new_v4(), RuntimeHookKind::ModelResponse))
        .await
        .unwrap();

    let snapshot = monitor.snapshot().await;
    assert_eq!(snapshot.agents.len(), 1);
    let agent = &snapshot.agents[0];
    assert!(agent.process_backed);
    assert!(agent.hook_backed);
    assert!(agent.control_capable);
    assert_eq!(agent.resource_source, "host_process");
    assert_eq!(agent.reported_confidence, Some(0.75));
    assert_eq!(agent.confidence_source, "hook");
    assert_eq!(agent.input_tokens, 10);
    assert_eq!(agent.output_tokens, 4);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn uses_hook_resource_samples_when_host_proc_is_unavailable() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    let mut event = hook(Uuid::new_v4(), RuntimeHookKind::Heartbeat);
    event.pid = None;
    event.cpu_percent = Some(37.5);
    event.rss_bytes = Some(128 * 1_024 * 1_024);
    event.memory_percent = Some(3.25);
    monitor.ingest_hook(event).await.unwrap();

    let snapshot = monitor.snapshot().await;
    assert_eq!(snapshot.agents.len(), 1);
    let agent = &snapshot.agents[0];
    assert!(!agent.process_backed);
    assert!(agent.hook_backed);
    assert_eq!(agent.resource_source, "hook");
    assert_eq!(agent.cpu_percent, Some(37.5));
    assert_eq!(agent.rss_bytes, Some(128 * 1_024 * 1_024));
    assert_eq!(agent.memory_percent, Some(3.25));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn older_hooks_do_not_replace_current_activity_or_resources() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    let mut current = hook(Uuid::new_v4(), RuntimeHookKind::Activity);
    current.occurred_at = Utc::now();
    current.summary = Some("Current visible activity".to_owned());
    current.confidence = Some(0.9);
    current.cpu_percent = Some(12.0);
    monitor.ingest_hook(current.clone()).await.unwrap();

    let mut older = hook(Uuid::new_v4(), RuntimeHookKind::ErrorObserved);
    older.occurred_at = current.occurred_at - chrono::Duration::seconds(5);
    older.summary = Some("Older delayed activity".to_owned());
    older.confidence = Some(0.1);
    older.cpu_percent = Some(99.0);
    monitor.ingest_hook(older).await.unwrap();

    let snapshot = monitor.snapshot().await;
    assert_eq!(snapshot.agents[0].status, "running");
    assert_eq!(
        snapshot.agents[0].current_activity.as_deref(),
        Some("Current visible activity")
    );
    assert_eq!(snapshot.agents[0].reported_confidence, Some(0.9));
    assert_eq!(snapshot.agents[0].cpu_percent, Some(12.0));
    assert_eq!(snapshot.agents[0].input_tokens, 20);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn rejects_hidden_reasoning_metadata_and_duplicate_hooks() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    let event_id = Uuid::new_v4();
    monitor
        .ingest_hook(hook(event_id, RuntimeHookKind::Heartbeat))
        .await
        .unwrap();
    assert_eq!(
        monitor
            .ingest_hook(hook(event_id, RuntimeHookKind::Heartbeat))
            .await,
        Err(RuntimeError::DuplicateHook)
    );

    let mut invalid = hook(Uuid::new_v4(), RuntimeHookKind::Activity);
    invalid
        .metadata
        .insert("chain_of_thought".to_owned(), "private".to_owned());
    assert_eq!(invalid.validate(), Err(RuntimeError::ForbiddenMetadataKey));

    let mut secret = hook(Uuid::new_v4(), RuntimeHookKind::Activity);
    secret.metadata.insert(
        "provider_api_token".to_owned(),
        "must-not-cross-boundary".to_owned(),
    );
    assert_eq!(secret.validate(), Err(RuntimeError::ForbiddenMetadataKey));

    let mut invalid_cpu = hook(Uuid::new_v4(), RuntimeHookKind::Heartbeat);
    invalid_cpu.cpu_percent = Some(f64::NAN);
    assert_eq!(
        invalid_cpu.validate(),
        Err(RuntimeError::InvalidField("cpu_percent"))
    );

    let mut invalid_memory = hook(Uuid::new_v4(), RuntimeHookKind::Heartbeat);
    invalid_memory.memory_percent = Some(100.1);
    assert_eq!(
        invalid_memory.validate(),
        Err(RuntimeError::InvalidField("memory_percent"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn rejects_controls_for_observe_only_hooks() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    let mut event = hook(Uuid::new_v4(), RuntimeHookKind::Heartbeat);
    event.control_capable = false;
    monitor.ingest_hook(event).await.unwrap();

    assert_eq!(
        monitor
            .enqueue_command(ControlCommandRequest {
                agent_id: "claude-test".to_owned(),
                action: ControlAction::Pause,
            })
            .await,
        Err(RuntimeError::AgentNotHookBacked)
    );
    assert!(!monitor.snapshot().await.agents[0].control_capable);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn rejects_controls_without_a_hook_channel() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    assert_eq!(
        monitor
            .enqueue_command(ControlCommandRequest {
                agent_id: "process:anthropic:4242".to_owned(),
                action: ControlAction::Pause,
            })
            .await,
        Err(RuntimeError::AgentNotHookBacked)
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn queues_and_acknowledges_hook_driven_controls() {
    let root = temp_proc_root();
    let monitor = RuntimeMonitor::new(RuntimeConfig::test(root.clone()));
    monitor
        .ingest_hook(hook(Uuid::new_v4(), RuntimeHookKind::Heartbeat))
        .await
        .unwrap();
    let command = monitor
        .enqueue_command(ControlCommandRequest {
            agent_id: "claude-test".to_owned(),
            action: ControlAction::Pause,
        })
        .await
        .unwrap();
    assert_eq!(
        monitor.pending_commands("claude-test").await.unwrap().len(),
        1
    );

    assert_eq!(
        monitor
            .acknowledge_command(ControlCommandAck {
                command_id: command.command_id,
                agent_id: "different-agent".to_owned(),
                accepted: true,
                message: None,
            })
            .await,
        Err(RuntimeError::CommandAgentMismatch)
    );

    let acknowledged = monitor
        .acknowledge_command(ControlCommandAck {
            command_id: command.command_id,
            agent_id: "claude-test".to_owned(),
            accepted: true,
            message: Some("paused".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(acknowledged.status, ControlStatus::Acknowledged);
    assert!(
        monitor
            .pending_commands("claude-test")
            .await
            .unwrap()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}
