use meta_agent_control_plane::{
    model::{AgentEvent, TaskStatus, Transport},
    provider::{AdapterError, normalize_anthropic, normalize_gemini, normalize_openai},
    store::Store,
    Config,
};
use serde_json::{Value, json};

fn fixture(source: &str) -> Value {
    serde_json::from_str(source).expect("provider fixture must be valid JSON")
}

fn common_progress(provider_id_field: &str, provider_id: &str) -> Value {
    let mut value = json!({
        "context": {
            "agent_id": "provider-agent",
            "model": "provider-model",
            "instance_id": "fixture-worker",
            "session_id": "provider-session",
            "correlation_id": "provider-goal",
            "sequence": 21
        },
        "observation": "progress",
        "task_id": "provider-task",
        "progress": 0.55,
        "summary": "The same observable progress was emitted by each provider.",
        "blocker": null,
        "next_action": "Compare normalized event payloads."
    });
    value
        .as_object_mut()
        .expect("object")
        .insert(provider_id_field.to_owned(), Value::String(provider_id.to_owned()));
    value
}

#[test]
fn checked_in_provider_fixtures_normalize_to_expected_shared_events() {
    let openai = normalize_openai(fixture(include_str!("../fixtures/openai-observation.json")))
        .expect("OpenAI fixture normalizes");
    let anthropic = normalize_anthropic(fixture(include_str!(
        "../fixtures/anthropic-observation.json"
    )))
    .expect("Anthropic fixture normalizes");
    let gemini = normalize_gemini(fixture(include_str!("../fixtures/gemini-observation.json")))
        .expect("Gemini fixture normalizes");

    assert_eq!(openai.agent.provider, "openai");
    assert_eq!(anthropic.agent.provider, "anthropic");
    assert_eq!(gemini.agent.provider, "google");
    assert!(matches!(openai.event, AgentEvent::ProgressUpdated(_)));
    assert!(matches!(anthropic.event, AgentEvent::ReflectionRecorded(_)));
    assert!(matches!(gemini.event, AgentEvent::TaskStarted(_)));
    assert_eq!(openai.session_id.as_deref(), Some("run-42"));
    assert_eq!(anthropic.correlation_id.as_deref(), Some("goal-17"));
    assert_eq!(gemini.sequence, Some(14));
}

#[test]
fn equivalent_observations_produce_identical_domain_payloads() {
    let openai = normalize_openai(common_progress("response_id", "resp-openai"))
        .expect("OpenAI progress normalizes");
    let anthropic = normalize_anthropic(common_progress("message_id", "msg-anthropic"))
        .expect("Anthropic progress normalizes");
    let gemini = normalize_gemini(common_progress("response_id", "resp-gemini"))
        .expect("Gemini progress normalizes");

    assert_eq!(openai.event, anthropic.event);
    assert_eq!(anthropic.event, gemini.event);
    assert_eq!(openai.agent.agent_id, anthropic.agent.agent_id);
    assert_eq!(anthropic.agent.agent_id, gemini.agent.agent_id);
    assert_eq!(openai.session_id, gemini.session_id);
    assert_eq!(openai.sequence, gemini.sequence);
}

#[test]
fn hidden_reasoning_fields_are_rejected_at_any_depth() {
    for (name, payload) in [
        (
            "top-level chain of thought",
            json!({
                "response_id": "resp-private",
                "chain_of_thought": "private",
                "context": { "agent_id": "a", "model": "gpt" },
                "observation": "progress",
                "task_id": "t",
                "progress": 0.1,
                "summary": "public"
            }),
        ),
        (
            "nested scratchpad",
            json!({
                "response_id": "resp-private",
                "context": {
                    "agent_id": "a",
                    "model": "gpt",
                    "metadata": { "scratchpad": "private" }
                },
                "observation": "progress",
                "task_id": "t",
                "progress": 0.1,
                "summary": "public"
            }),
        ),
    ] {
        let error = normalize_openai(payload).expect_err(name);
        assert!(matches!(error, AdapterError::ForbiddenReasoningField { .. }));
    }
}

#[tokio::test]
async fn normalized_provider_progress_flows_through_the_existing_reducer() {
    let envelope = normalize_openai(fixture(include_str!("../fixtures/openai-observation.json")))
        .expect("OpenAI fixture normalizes");
    let config = Config::local_test();
    let store = Store::new(config.cache_config(), config.update_channel_capacity);

    store
        .ingest(envelope, Transport::Http)
        .await
        .expect("normalized event is accepted by the domain store");
    let snapshot = store.snapshot().await;

    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.agents.len(), 1);
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Running);
    assert_eq!(snapshot.tasks[0].progress, 0.7);
    assert_eq!(snapshot.counters.accepted_by_transport["http"], 1);
}
