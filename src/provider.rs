use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::model::{
    AgentEvent, AgentRef, ErrorObservation, EventEnvelope, EvidenceReference, ProgressUpdate,
    Reflection, TaskCompleted, TaskOutcome, TaskStarted,
};

const FORBIDDEN_REASONING_KEYS: &[&str] = &[
    "chain_of_thought",
    "chain-of-thought",
    "hidden_reasoning",
    "internal_reasoning",
    "reasoning_content",
    "scratchpad",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
}

impl ProviderKind {
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "google",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterContext {
    pub agent_id: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "observation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservableAgentUpdate {
    Progress {
        task_id: String,
        progress: f32,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocker: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_action: Option<String>,
    },
    Reflection {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        summary: String,
        confidence: f32,
        #[serde(default)]
        assumptions: Vec<String>,
        #[serde(default)]
        evidence: Vec<EvidenceReference>,
        #[serde(default)]
        alternatives_considered: Vec<String>,
        #[serde(default)]
        risks: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_action: Option<String>,
    },
    ToolCall {
        task_id: String,
        tool_name: String,
        call_id: String,
        summary: String,
        #[serde(default = "default_attempt")]
        attempt: u32,
    },
    Completion {
        task_id: String,
        outcome: TaskOutcome,
        summary: String,
        #[serde(default)]
        artifacts: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actual_result: Option<String>,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        code: String,
        message: String,
        #[serde(default)]
        recoverable: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proposed_recovery: Option<String>,
    },
}

const fn default_attempt() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenAiObservation {
    pub response_id: String,
    pub context: AdapterContext,
    #[serde(flatten)]
    pub update: ObservableAgentUpdate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnthropicObservation {
    pub message_id: String,
    pub context: AdapterContext,
    #[serde(flatten)]
    pub update: ObservableAgentUpdate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeminiObservation {
    pub response_id: String,
    pub context: AdapterContext,
    #[serde(flatten)]
    pub update: ObservableAgentUpdate,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AdapterError {
    #[error("provider adapter payload contains forbidden hidden-reasoning field: {field}")]
    ForbiddenReasoningField { field: String },
    #[error("invalid provider adapter payload: {0}")]
    InvalidPayload(String),
}

pub fn normalize_openai(value: Value) -> Result<EventEnvelope, AdapterError> {
    reject_hidden_reasoning(&value)?;
    let observation: OpenAiObservation = serde_json::from_value(value)
        .map_err(|error| AdapterError::InvalidPayload(error.to_string()))?;
    Ok(normalize(
        ProviderKind::OpenAi,
        &observation.response_id,
        observation.context,
        observation.update,
    ))
}

pub fn normalize_anthropic(value: Value) -> Result<EventEnvelope, AdapterError> {
    reject_hidden_reasoning(&value)?;
    let observation: AnthropicObservation = serde_json::from_value(value)
        .map_err(|error| AdapterError::InvalidPayload(error.to_string()))?;
    Ok(normalize(
        ProviderKind::Anthropic,
        &observation.message_id,
        observation.context,
        observation.update,
    ))
}

pub fn normalize_gemini(value: Value) -> Result<EventEnvelope, AdapterError> {
    reject_hidden_reasoning(&value)?;
    let observation: GeminiObservation = serde_json::from_value(value)
        .map_err(|error| AdapterError::InvalidPayload(error.to_string()))?;
    Ok(normalize(
        ProviderKind::Gemini,
        &observation.response_id,
        observation.context,
        observation.update,
    ))
}

fn normalize(
    provider: ProviderKind,
    provider_response_id: &str,
    context: AdapterContext,
    update: ObservableAgentUpdate,
) -> EventEnvelope {
    let event = match update {
        ObservableAgentUpdate::Progress {
            task_id,
            progress,
            summary,
            blocker,
            next_action,
        } => AgentEvent::ProgressUpdated(ProgressUpdate {
            task_id,
            progress,
            summary,
            blocker,
            next_action,
        }),
        ObservableAgentUpdate::Reflection {
            task_id,
            summary,
            confidence,
            assumptions,
            mut evidence,
            alternatives_considered,
            risks,
            next_action,
        } => {
            evidence.push(EvidenceReference {
                kind: "provider_response".to_owned(),
                reference: provider_response_id.to_owned(),
                summary: Some(format!("{} response identifier", provider.protocol_name())),
            });
            AgentEvent::ReflectionRecorded(Reflection {
                task_id,
                summary,
                confidence,
                assumptions,
                evidence,
                alternatives_considered,
                risks,
                next_action,
            })
        }
        ObservableAgentUpdate::ToolCall {
            task_id,
            tool_name,
            call_id,
            summary,
            attempt,
        } => AgentEvent::TaskStarted(TaskStarted {
            task_id,
            attempt,
            plan_summary: Some(format!("{summary} Tool: {tool_name}; call: {call_id}.")),
        }),
        ObservableAgentUpdate::Completion {
            task_id,
            outcome,
            summary,
            mut artifacts,
            actual_result,
        } => {
            artifacts.push(format!("provider-response:{provider_response_id}"));
            AgentEvent::TaskCompleted(TaskCompleted {
                task_id,
                outcome,
                summary,
                artifacts,
                actual_result,
            })
        }
        ObservableAgentUpdate::Error {
            task_id,
            code,
            message,
            recoverable,
            proposed_recovery,
        } => AgentEvent::ErrorObserved(ErrorObservation {
            task_id,
            code,
            message,
            recoverable,
            proposed_recovery,
        }),
    };

    let mut envelope = EventEnvelope::new(
        AgentRef {
            agent_id: context.agent_id,
            provider: provider.protocol_name().to_owned(),
            model: context.model,
            instance_id: context.instance_id,
        },
        event,
    );
    envelope.session_id = context.session_id;
    envelope.correlation_id = context.correlation_id;
    envelope.sequence = context.sequence;
    envelope
}

fn reject_hidden_reasoning(value: &Value) -> Result<(), AdapterError> {
    match value {
        Value::Object(values) => {
            for (key, nested) in values {
                let normalized = key.to_ascii_lowercase();
                if FORBIDDEN_REASONING_KEYS.contains(&normalized.as_str()) {
                    return Err(AdapterError::ForbiddenReasoningField { field: key.clone() });
                }
                reject_hidden_reasoning(nested)?;
            }
        }
        Value::Array(values) => {
            for nested in values {
                reject_hidden_reasoning(nested)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_hidden_reasoning_recursively() {
        let error = normalize_openai(json!({
            "response_id": "resp-1",
            "context": {
                "agent_id": "agent-1",
                "model": "gpt",
                "scratchpad": "must never cross the adapter boundary"
            },
            "observation": "progress",
            "task_id": "task-1",
            "progress": 0.5,
            "summary": "Observable progress"
        }))
        .expect_err("hidden reasoning key must be rejected");

        assert!(matches!(
            error,
            AdapterError::ForbiddenReasoningField { ref field } if field == "scratchpad"
        ));
    }
}
