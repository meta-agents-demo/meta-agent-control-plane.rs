use std::{
    io::Write,
    process::{Command, Stdio},
};

use meta_agent_control_plane::{
    Config,
    coordination::{AssignmentAction, CoordinationPlan},
    model::{AgentEvent, AgentRef, EventEnvelope, TaskSpec, Transport},
    store::Store,
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_meta-agent-plan")
}

async fn snapshot_json() -> String {
    let config = Config::local_test();
    let store = Store::new(config.cache_config(), config.update_channel_capacity);
    store
        .ingest(
            EventEnvelope::new(
                AgentRef {
                    agent_id: "planner-cli-agent".to_owned(),
                    provider: "fixture".to_owned(),
                    model: "deterministic".to_owned(),
                    instance_id: None,
                },
                AgentEvent::TaskCreated(TaskSpec {
                    task_id: "planner-cli-task".to_owned(),
                    title: "Exercise the offline planner".to_owned(),
                    goal_id: None,
                    depends_on: Vec::new(),
                    tags: Vec::new(),
                    expected_outcome: Some("One bounded assignment".to_owned()),
                }),
            ),
            Transport::Http,
        )
        .await
        .unwrap();
    serde_json::to_string(&store.snapshot().await).unwrap()
}

#[tokio::test]
async fn cli_reads_snapshot_from_stdin_and_emits_a_bounded_plan() {
    let input = snapshot_json().await;
    let mut child = Command::new(binary())
        .args(["--max-assignments", "1", "--max-assignments-per-agent", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "planner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: CoordinationPlan = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan.summary.assignments, 1);
    assert_eq!(plan.assignments[0].agent_id, "planner-cli-agent");
    assert_eq!(plan.assignments[0].task_id, "planner-cli-task");
    assert!(matches!(
        plan.assignments[0].action,
        AssignmentAction::StartTask | AssignmentAction::DefineNextAction
    ));
}

#[tokio::test]
async fn cli_rejects_zero_capacity_without_echoing_snapshot_contents() {
    let input = snapshot_json().await;
    let mut child = Command::new(binary())
        .args(["--max-assignments", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("maximum assignments must be greater than zero"));
    assert!(!stderr.contains("planner-cli-task"));
}
