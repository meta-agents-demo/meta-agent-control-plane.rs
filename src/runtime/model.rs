use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::RUNTIME_PROTOCOL_VERSION;

const MAX_TEXT_BYTES: usize = 4_096;
const MAX_METADATA_ENTRIES: usize = 32;
const MAX_METADATA_KEY_BYTES: usize = 128;
const MAX_METADATA_VALUE_BYTES: usize = 1_024;
const MAX_CPU_PERCENT: f64 = 100_000.0;
const FORBIDDEN_METADATA_KEY_FRAGMENTS: &[&str] = &[
    "authorization",
    "chain_of_thought",
    "chain-of-thought",
    "cookie",
    "credential",
    "hidden_reasoning",
    "internal_reasoning",
    "api_key",
    "apikey",
    "prompt",
    "raw_response",
    "reasoning_content",
    "scratchpad",
    "secret",
    "token",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAgentRef {
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHookKind {
    SessionStarted,
    Heartbeat,
    Activity,
    ToolStarted,
    ToolFinished,
    ModelResponse,
    ConfidenceReported,
    ErrorObserved,
    SessionFinished,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHookEnvelope {
    pub protocol_version: String,
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub agent: RuntimeAgentRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub kind: RuntimeHookKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_percent: Option<f64>,
    #[serde(default)]
    pub input_tokens_delta: u64,
    #[serde(default)]
    pub output_tokens_delta: u64,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl RuntimeHookEnvelope {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.protocol_version != RUNTIME_PROTOCOL_VERSION {
            return Err(RuntimeError::UnsupportedProtocol);
        }
        if self.event_id.is_nil() {
            return Err(RuntimeError::InvalidField("event_id"));
        }
        validate_text("agent.agent_id", &self.agent.agent_id, 256)?;
        validate_text("agent.provider", &self.agent.provider, 128)?;
        validate_text("agent.model", &self.agent.model, 256)?;
        validate_optional_text("agent.instance_id", self.agent.instance_id.as_deref(), 256)?;
        validate_optional_text("session_id", self.session_id.as_deref(), 256)?;
        validate_optional_text("summary", self.summary.as_deref(), MAX_TEXT_BYTES)?;
        validate_optional_text("tool_name", self.tool_name.as_deref(), 256)?;
        if self.pid == Some(0) {
            return Err(RuntimeError::InvalidField("pid"));
        }
        if self
            .confidence
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(RuntimeError::InvalidField("confidence"));
        }
        if self.kind == RuntimeHookKind::ConfidenceReported && self.confidence.is_none() {
            return Err(RuntimeError::MissingField("confidence"));
        }
        if self
            .cpu_percent
            .is_some_and(|value| !value.is_finite() || !(0.0..=MAX_CPU_PERCENT).contains(&value))
        {
            return Err(RuntimeError::InvalidField("cpu_percent"));
        }
        if self
            .memory_percent
            .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        {
            return Err(RuntimeError::InvalidField("memory_percent"));
        }
        if self.metadata.len() > MAX_METADATA_ENTRIES {
            return Err(RuntimeError::TooManyMetadataEntries);
        }
        for (key, value) in &self.metadata {
            validate_text("metadata.key", key, MAX_METADATA_KEY_BYTES)?;
            validate_text("metadata.value", value, MAX_METADATA_VALUE_BYTES)?;
            let normalized_key = key.trim().to_ascii_lowercase();
            if FORBIDDEN_METADATA_KEY_FRAGMENTS
                .iter()
                .any(|fragment| normalized_key.contains(*fragment))
            {
                return Err(RuntimeError::ForbiddenMetadataKey);
            }
        }
        Ok(())
    }
}

pub(super) fn validate_text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), RuntimeError> {
    if value.trim().is_empty() || value.len() > maximum {
        return Err(RuntimeError::InvalidField(field));
    }
    Ok(())
}

fn validate_optional_text(
    field: &'static str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), RuntimeError> {
    if let Some(value) = value {
        validate_text(field, value, maximum)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Pause,
    Resume,
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStatus {
    Pending,
    Acknowledged,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCommandRequest {
    pub agent_id: String,
    pub action: ControlAction,
}

impl ControlCommandRequest {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        validate_text("agent_id", &self.agent_id, 256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCommandAck {
    pub command_id: Uuid,
    pub agent_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ControlCommandAck {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.command_id.is_nil() {
            return Err(RuntimeError::InvalidField("command_id"));
        }
        validate_text("agent_id", &self.agent_id, 256)?;
        validate_optional_text("message", self.message.as_deref(), 1_024)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlCommand {
    pub command_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub agent_id: String,
    pub action: ControlAction,
    pub status: ControlStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeProcessTelemetry {
    pub pid: u32,
    pub provider: String,
    pub process_name: String,
    pub matched_pattern: String,
    pub process_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    pub rss_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_percent: Option<f64>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAgentTelemetry {
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_confidence: Option<f32>,
    pub confidence_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_percent: Option<f64>,
    pub resource_source: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub process_backed: bool,
    pub hook_backed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hook_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_process_sample_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTotals {
    pub agents: usize,
    pub process_backed_agents: usize,
    pub hook_backed_agents: usize,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub confidence_reported_agents: usize,
    pub confidence_unreported_agents: usize,
    pub pending_commands: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCollectionStatus {
    pub configured: bool,
    pub enabled: bool,
    pub proc_root: String,
    pub sample_interval_ms: u64,
    pub process_patterns: Vec<String>,
    pub cpu_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sample_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub collection_errors: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub generated_at: DateTime<Utc>,
    pub collection: RuntimeCollectionStatus,
    pub totals: RuntimeTotals,
    pub agents: Vec<RuntimeAgentTelemetry>,
    pub processes: Vec<RuntimeProcessTelemetry>,
    pub recent_hooks: Vec<RuntimeHookEnvelope>,
    pub recent_commands: Vec<ControlCommand>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RuntimeError {
    #[error("unsupported runtime hook protocol")]
    UnsupportedProtocol,
    #[error("invalid runtime hook field: {0}")]
    InvalidField(&'static str),
    #[error("missing runtime hook field: {0}")]
    MissingField(&'static str),
    #[error("runtime hook metadata contains too many entries")]
    TooManyMetadataEntries,
    #[error("runtime hook metadata contains a forbidden content key")]
    ForbiddenMetadataKey,
    #[error("runtime hook event has already been accepted")]
    DuplicateHook,
    #[error("control command was not found")]
    CommandNotFound,
    #[error("agent has not established a cooperative runtime hook channel")]
    AgentNotHookBacked,
    #[error("control command belongs to a different agent")]
    CommandAgentMismatch,
}
