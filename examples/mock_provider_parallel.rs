use std::{env, time::Duration};

use meta_agent_control_plane::{
    client::{ClientConfig, ClientTransport, EventClient},
    model::EventEnvelope,
    provider::{AdapterError, normalize_anthropic, normalize_gemini, normalize_openai},
};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("META_AGENT_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:8787/api/v1/events".to_owned());
    let token = env::var("META_AGENT_TOKEN").ok();
    let client = EventClient::new(
        ClientConfig::new(ClientTransport::Http { endpoint }, token)
            .with_timeout(Duration::from_secs(10)),
    )?;

    type Normalizer = fn(Value) -> Result<EventEnvelope, AdapterError>;
    let observations: [(&str, &str, Normalizer); 3] = [
        (
            "openai",
            include_str!("../fixtures/providers/openai-progress.json"),
            normalize_openai,
        ),
        (
            "anthropic",
            include_str!("../fixtures/providers/anthropic-reflection.json"),
            normalize_anthropic,
        ),
        (
            "gemini",
            include_str!("../fixtures/providers/gemini-completion.json"),
            normalize_gemini,
        ),
    ];

    for (provider, fixture, normalize) in observations {
        let event = normalize(serde_json::from_str(fixture)?)?;
        let acknowledgement = client.send(&event).await?;
        println!("{provider}: {}", serde_json::to_string(&acknowledgement)?);
    }
    Ok(())
}
