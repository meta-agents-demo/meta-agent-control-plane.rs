use std::{
    io::Write,
    process::{Command, Stdio},
};

use serde_json::Value;

fn run_sidecar(provider: &str, input: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_meta-agent-sidecar");
    let mut child = Command::new(binary)
        .args(["--provider", provider, "--transport", "http", "--dry-run"])
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
    child.wait_with_output().unwrap()
}

#[test]
fn deterministic_provider_fixtures_normalize_without_network_access() {
    let fixtures = [
        (
            "openai",
            include_str!("../fixtures/providers/openai-progress.json"),
            "openai",
            "progress_updated",
        ),
        (
            "anthropic",
            include_str!("../fixtures/providers/anthropic-reflection.json"),
            "anthropic",
            "reflection_recorded",
        ),
        (
            "gemini",
            include_str!("../fixtures/providers/gemini-completion.json"),
            "google",
            "task_completed",
        ),
    ];

    for (provider_arg, fixture, expected_provider, expected_kind) in fixtures {
        let output = run_sidecar(provider_arg, fixture);
        assert!(
            output.status.success(),
            "sidecar failed for {provider_arg}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let event: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(event["protocol_version"], "v1");
        assert_eq!(event["agent"]["provider"], expected_provider);
        assert_eq!(event["kind"], expected_kind);
        assert!(event.get("chain_of_thought").is_none());
        assert!(event.get("scratchpad").is_none());
    }
}

#[test]
fn sidecar_rejects_hidden_reasoning_without_echoing_its_value() {
    let secret = "private-hidden-reasoning-must-not-be-logged";
    let payload = format!(
        r#"{{
          "response_id": "resp-secret",
          "context": {{
            "agent_id": "agent",
            "model": "model",
            "scratchpad": "{secret}"
          }},
          "observation": "progress",
          "task_id": "task",
          "progress": 0.5,
          "summary": "observable"
        }}"#
    );
    let output = run_sidecar("openai", &payload);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("forbidden hidden-reasoning field"));
    assert!(!stderr.contains(secret));
    assert!(output.stdout.is_empty());
}
