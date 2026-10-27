use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    sync::Arc,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use crate::model::Transport;

pub const BRIDGE_PROTOCOL_VERSION: &str = "v1";
const MAX_ROOMS: usize = 64;
const MAX_MEMBERS_PER_ROOM: usize = 64;
const MAX_MESSAGES_PER_ROOM: usize = 512;
const MAX_CONTACTS_PER_ROOM: usize = 512;
const MAX_SEEN_MESSAGE_IDS: usize = 8_192;
const MAX_SUMMARY_BYTES: usize = 4_096;

#[derive(Clone)]
pub struct BridgeHub {
    state: Arc<BridgeState>,
}

impl fmt::Debug for BridgeHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("BridgeHub").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct BridgeState {
    inner: RwLock<BridgeInner>,
    updates: broadcast::Sender<BridgeUpdate>,
}

#[derive(Debug)]
struct BridgeInner {
    rooms: BTreeMap<String, RoomState>,
    seen_message_ids: HashMap<Uuid, String>,
    seen_message_order: VecDeque<Uuid>,
    revision: u64,
    started_at: DateTime<Utc>,
    counters: BridgeCounters,
}

#[derive(Debug)]
struct RoomState {
    room: BridgeRoom,
    members: BTreeMap<String, BridgeParticipant>,
    messages: VecDeque<BridgeMessage>,
    contacts: VecDeque<BridgeContactPoint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeParticipantKind {
    Human,
    Agent,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeParticipantInput {
    pub participant_id: String,
    pub display_name: String,
    pub kind: BridgeParticipantKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_agent_id: Option<String>,
}

impl BridgeParticipantInput {
    pub fn validate(&self) -> Result<(), BridgeError> {
        validate_text("participant_id", &self.participant_id, 256)?;
        validate_visible_text("display_name", &self.display_name, 256)?;
        validate_optional_text("provider", self.provider.as_deref(), 128)?;
        validate_optional_visible_text("model", self.model.as_deref(), 256)?;
        validate_optional_text("runtime_agent_id", self.runtime_agent_id.as_deref(), 256)?;
        if self.kind == BridgeParticipantKind::Agent && self.provider.is_none() {
            return Err(BridgeError::MissingField("provider"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeParticipant {
    pub participant_id: String,
    pub display_name: String,
    pub kind: BridgeParticipantKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_agent_id: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub websocket_connected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeRoomInput {
    pub slug: String,
    pub title: String,
    pub objective: String,
}

impl BridgeRoomInput {
    pub fn validate(&self) -> Result<(), BridgeError> {
        validate_slug(&self.slug)?;
        validate_visible_text("title", &self.title, 256)?;
        validate_visible_text("objective", &self.objective, 2_048)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeRoom {
    pub slug: String,
    pub title: String,
    pub objective: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeMessageInput {
    pub protocol_version: String,
    pub message_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub author: BridgeParticipantInput,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
}

impl BridgeMessageInput {
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.protocol_version != BRIDGE_PROTOCOL_VERSION {
            return Err(BridgeError::UnsupportedProtocol);
        }
        if self.message_id.is_nil() {
            return Err(BridgeError::InvalidField("message_id"));
        }
        self.author.validate()?;
        validate_text("summary", &self.summary, MAX_SUMMARY_BYTES)?;
        if contains_secret_material(&self.summary) {
            return Err(BridgeError::SecretMaterial);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeMessage {
    pub protocol_version: String,
    pub message_id: Uuid,
    pub room_slug: String,
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub author: BridgeParticipantInput,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
    pub transport: Transport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeContactPoint {
    pub contact_id: Uuid,
    pub room_slug: String,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub participants: Vec<String>,
    pub message_id: Uuid,
    pub previous_message_id: Uuid,
    pub summary: String,
    pub transport: Transport,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeCounters {
    pub accepted_messages: u64,
    pub duplicate_messages: u64,
    pub rejected_messages: u64,
    pub contacts: u64,
    pub accepted_by_transport: BTreeMap<String, u64>,
    pub rejected_by_transport: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeRoomSummary {
    pub room: BridgeRoom,
    pub members: usize,
    pub connected_members: usize,
    pub messages: usize,
    pub contacts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeSnapshot {
    pub generated_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub revision: u64,
    pub room: BridgeRoom,
    pub members: Vec<BridgeParticipant>,
    pub messages: Vec<BridgeMessage>,
    pub contacts: Vec<BridgeContactPoint>,
    pub counters: BridgeCounters,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeUpdate {
    pub revision: u64,
    pub room_slug: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BridgeAck {
    pub accepted: bool,
    pub duplicate: bool,
    pub revision: u64,
    pub message_id: Uuid,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<BridgeContactPoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeTcpFrame {
    BridgeCreateRoom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        room: BridgeRoomInput,
    },
    BridgeJoin {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        room_slug: String,
        participant: BridgeParticipantInput,
    },
    BridgeMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        room_slug: String,
        message: BridgeMessageInput,
    },
    BridgeSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        room_slug: String,
    },
}

impl BridgeTcpFrame {
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::BridgeCreateRoom { token, .. }
            | Self::BridgeJoin { token, .. }
            | Self::BridgeMessage { token, .. }
            | Self::BridgeSnapshot { token, .. } => token.as_deref(),
        }
    }

    pub const fn is_read(&self) -> bool {
        matches!(self, Self::BridgeSnapshot { .. })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BridgeError {
    #[error("unsupported bridge protocol")]
    UnsupportedProtocol,
    #[error("invalid bridge field: {0}")]
    InvalidField(&'static str),
    #[error("missing bridge field: {0}")]
    MissingField(&'static str),
    #[error("bridge room was not found")]
    RoomNotFound,
    #[error("bridge room capacity is full")]
    RoomCapacity,
    #[error("bridge room member capacity is full")]
    MemberCapacity,
    #[error("bridge message author has not joined the room")]
    ParticipantNotJoined,
    #[error("bridge message author does not match the joined participant")]
    ParticipantIdentityMismatch,
    #[error("bridge message id was already used in another room")]
    MessageIdConflict,
    #[error("bridge summary appears to contain credential material")]
    SecretMaterial,
}

impl Default for BridgeHub {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeHub {
    pub fn new() -> Self {
        let (updates, _) = broadcast::channel(1_024);
        Self {
            state: Arc::new(BridgeState {
                inner: RwLock::new(BridgeInner {
                    rooms: BTreeMap::new(),
                    seen_message_ids: HashMap::new(),
                    seen_message_order: VecDeque::new(),
                    revision: 0,
                    started_at: Utc::now(),
                    counters: BridgeCounters::default(),
                }),
                updates,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BridgeUpdate> {
        self.state.updates.subscribe()
    }

    pub async fn create_room(&self, input: BridgeRoomInput) -> Result<BridgeRoom, BridgeError> {
        input.validate()?;
        let now = Utc::now();
        let mut inner = self.state.inner.write().await;
        if let Some(existing) = inner.rooms.get(&input.slug) {
            return Ok(existing.room.clone());
        }
        if inner.rooms.len() >= MAX_ROOMS {
            return Err(BridgeError::RoomCapacity);
        }
        inner.revision = inner.revision.saturating_add(1);
        let revision = inner.revision;
        let room = BridgeRoom {
            slug: input.slug,
            title: input.title,
            objective: input.objective,
            created_at: now,
            updated_at: now,
            revision,
        };
        inner.rooms.insert(
            room.slug.clone(),
            RoomState {
                room: room.clone(),
                members: BTreeMap::new(),
                messages: VecDeque::new(),
                contacts: VecDeque::new(),
            },
        );
        drop(inner);
        self.publish_update(BridgeUpdate {
            revision,
            room_slug: room.slug.clone(),
            kind: "room_created".to_owned(),
            participant_id: None,
            message_id: None,
            occurred_at: now,
        });
        Ok(room)
    }

    pub async fn join(
        &self,
        room_slug: &str,
        participant: BridgeParticipantInput,
        websocket_connected: bool,
    ) -> Result<BridgeParticipant, BridgeError> {
        validate_slug(room_slug)?;
        participant.validate()?;
        let now = Utc::now();
        let mut inner = self.state.inner.write().await;
        let room = inner
            .rooms
            .get_mut(room_slug)
            .ok_or(BridgeError::RoomNotFound)?;
        if room.members.len() >= MAX_MEMBERS_PER_ROOM
            && !room.members.contains_key(&participant.participant_id)
        {
            return Err(BridgeError::MemberCapacity);
        }
        let member = room
            .members
            .entry(participant.participant_id.clone())
            .and_modify(|member| {
                member.display_name.clone_from(&participant.display_name);
                member.kind = participant.kind;
                member.provider.clone_from(&participant.provider);
                member.model.clone_from(&participant.model);
                member
                    .runtime_agent_id
                    .clone_from(&participant.runtime_agent_id);
                member.last_seen_at = now;
                member.websocket_connected |= websocket_connected;
            })
            .or_insert_with(|| BridgeParticipant {
                participant_id: participant.participant_id.clone(),
                display_name: participant.display_name,
                kind: participant.kind,
                provider: participant.provider,
                model: participant.model,
                runtime_agent_id: participant.runtime_agent_id,
                joined_at: now,
                last_seen_at: now,
                websocket_connected,
            })
            .clone();
        inner.revision = inner.revision.saturating_add(1);
        let revision = inner.revision;
        if let Some(room) = inner.rooms.get_mut(room_slug) {
            room.room.updated_at = now;
            room.room.revision = revision;
        }
        drop(inner);
        self.publish_update(BridgeUpdate {
            revision,
            room_slug: room_slug.to_owned(),
            kind: "participant_joined".to_owned(),
            participant_id: Some(member.participant_id.clone()),
            message_id: None,
            occurred_at: now,
        });
        Ok(member)
    }

    pub async fn set_websocket_connected(
        &self,
        room_slug: &str,
        participant_id: &str,
        connected: bool,
    ) {
        let now = Utc::now();
        let mut inner = self.state.inner.write().await;
        let Some(room) = inner.rooms.get_mut(room_slug) else {
            return;
        };
        let Some(member) = room.members.get_mut(participant_id) else {
            return;
        };
        if member.websocket_connected == connected {
            return;
        }
        member.websocket_connected = connected;
        member.last_seen_at = now;
        inner.revision = inner.revision.saturating_add(1);
        let revision = inner.revision;
        if let Some(room) = inner.rooms.get_mut(room_slug) {
            room.room.updated_at = now;
            room.room.revision = revision;
        }
        drop(inner);
        self.publish_update(BridgeUpdate {
            revision,
            room_slug: room_slug.to_owned(),
            kind: if connected {
                "participant_connected"
            } else {
                "participant_disconnected"
            }
            .to_owned(),
            participant_id: Some(participant_id.to_owned()),
            message_id: None,
            occurred_at: now,
        });
    }

    pub async fn post_message(
        &self,
        room_slug: &str,
        input: BridgeMessageInput,
        transport: Transport,
    ) -> Result<BridgeAck, BridgeError> {
        validate_slug(room_slug)?;
        input.validate()?;
        let received_at = Utc::now();
        let mut inner = self.state.inner.write().await;
        let room = inner
            .rooms
            .get(room_slug)
            .ok_or(BridgeError::RoomNotFound)?;
        let author = input.author.clone();
        let member = room
            .members
            .get(&author.participant_id)
            .ok_or(BridgeError::ParticipantNotJoined)?;
        if member.display_name != author.display_name
            || member.kind != author.kind
            || member.provider != author.provider
            || member.model != author.model
            || member.runtime_agent_id != author.runtime_agent_id
        {
            return Err(BridgeError::ParticipantIdentityMismatch);
        }
        if let Some(seen_room) = inner.seen_message_ids.get(&input.message_id) {
            if seen_room != room_slug {
                return Err(BridgeError::MessageIdConflict);
            }
            inner.counters.duplicate_messages = inner.counters.duplicate_messages.saturating_add(1);
            return Ok(BridgeAck {
                accepted: true,
                duplicate: true,
                revision: inner.revision,
                message_id: input.message_id,
                received_at,
                contact: None,
            });
        }

        let room = inner
            .rooms
            .get_mut(room_slug)
            .expect("room remains present while bridge lock is held");
        room.members
            .get_mut(&author.participant_id)
            .expect("participant membership was validated while lock is held")
            .last_seen_at = received_at;

        let previous = input
            .reply_to
            .and_then(|message_id| {
                room.messages
                    .iter()
                    .find(|message| message.message_id == message_id)
            })
            .or_else(|| {
                room.messages
                    .iter()
                    .rev()
                    .find(|message| message.author.participant_id != author.participant_id)
            })
            .cloned();

        inner.revision = inner.revision.saturating_add(1);
        let revision = inner.revision;
        let message = BridgeMessage {
            protocol_version: input.protocol_version,
            message_id: input.message_id,
            room_slug: room_slug.to_owned(),
            sequence: revision,
            occurred_at: input.occurred_at,
            received_at,
            author,
            summary: input.summary,
            reply_to: input.reply_to,
            transport,
        };
        let contact = previous.map(|previous| BridgeContactPoint {
            contact_id: Uuid::new_v4(),
            room_slug: room_slug.to_owned(),
            occurred_at: message.occurred_at,
            received_at,
            participants: vec![
                previous.author.participant_id,
                message.author.participant_id.clone(),
            ],
            message_id: message.message_id,
            previous_message_id: previous.message_id,
            summary: message.summary.clone(),
            transport,
        });

        let room = inner
            .rooms
            .get_mut(room_slug)
            .expect("room remains present while bridge lock is held");
        room.messages.push_back(message.clone());
        while room.messages.len() > MAX_MESSAGES_PER_ROOM {
            room.messages.pop_front();
        }
        let recorded_contact = contact.is_some();
        if let Some(contact) = &contact {
            room.contacts.push_back(contact.clone());
            while room.contacts.len() > MAX_CONTACTS_PER_ROOM {
                room.contacts.pop_front();
            }
        }
        room.room.updated_at = received_at;
        room.room.revision = revision;
        if recorded_contact {
            inner.counters.contacts = inner.counters.contacts.saturating_add(1);
        }
        inner
            .seen_message_ids
            .insert(message.message_id, room_slug.to_owned());
        inner.seen_message_order.push_back(message.message_id);
        while inner.seen_message_order.len() > MAX_SEEN_MESSAGE_IDS {
            if let Some(evicted) = inner.seen_message_order.pop_front() {
                inner.seen_message_ids.remove(&evicted);
            }
        }
        inner.counters.accepted_messages = inner.counters.accepted_messages.saturating_add(1);
        *inner
            .counters
            .accepted_by_transport
            .entry(transport.to_string())
            .or_default() += 1;
        drop(inner);

        self.publish_update(BridgeUpdate {
            revision,
            room_slug: room_slug.to_owned(),
            kind: "message_accepted".to_owned(),
            participant_id: Some(message.author.participant_id),
            message_id: Some(message.message_id),
            occurred_at: received_at,
        });
        Ok(BridgeAck {
            accepted: true,
            duplicate: false,
            revision,
            message_id: message.message_id,
            received_at,
            contact,
        })
    }

    pub async fn record_rejection(&self, transport: Transport) {
        let mut inner = self.state.inner.write().await;
        inner.counters.rejected_messages = inner.counters.rejected_messages.saturating_add(1);
        *inner
            .counters
            .rejected_by_transport
            .entry(transport.to_string())
            .or_default() += 1;
    }

    pub async fn list_rooms(&self) -> Vec<BridgeRoomSummary> {
        let inner = self.state.inner.read().await;
        inner
            .rooms
            .values()
            .map(|room| BridgeRoomSummary {
                room: room.room.clone(),
                members: room.members.len(),
                connected_members: room
                    .members
                    .values()
                    .filter(|member| member.websocket_connected)
                    .count(),
                messages: room.messages.len(),
                contacts: room.contacts.len(),
            })
            .collect()
    }

    pub async fn snapshot(&self, room_slug: &str) -> Result<BridgeSnapshot, BridgeError> {
        validate_slug(room_slug)?;
        let inner = self.state.inner.read().await;
        let room = inner
            .rooms
            .get(room_slug)
            .ok_or(BridgeError::RoomNotFound)?;
        Ok(BridgeSnapshot {
            generated_at: Utc::now(),
            started_at: inner.started_at,
            revision: inner.revision,
            room: room.room.clone(),
            members: room.members.values().cloned().collect(),
            messages: room.messages.iter().cloned().collect(),
            contacts: room.contacts.iter().rev().cloned().collect(),
            counters: inner.counters.clone(),
        })
    }

    fn publish_update(&self, update: BridgeUpdate) {
        let _ = self.state.updates.send(update);
    }
}

fn validate_slug(value: &str) -> Result<(), BridgeError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || value.starts_with('-')
        || value.ends_with('-')
    {
        return Err(BridgeError::InvalidField("room_slug"));
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str, maximum: usize) -> Result<(), BridgeError> {
    if value.trim().is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(BridgeError::InvalidField(field));
    }
    Ok(())
}

fn validate_optional_text(
    field: &'static str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), BridgeError> {
    if let Some(value) = value {
        validate_text(field, value, maximum)?;
    }
    Ok(())
}

fn validate_visible_text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), BridgeError> {
    validate_text(field, value, maximum)?;
    if contains_secret_material(value) {
        return Err(BridgeError::SecretMaterial);
    }
    Ok(())
}

fn validate_optional_visible_text(
    field: &'static str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), BridgeError> {
    if let Some(value) = value {
        validate_visible_text(field, value, maximum)?;
    }
    Ok(())
}

fn contains_secret_material(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    if [
        "authorization: bearer ",
        "openai_api_key=",
        "anthropic_api_key=",
        "openai_api_key:",
        "anthropic_api_key:",
        "api-key:",
        "x-api-key:",
        "-----begin private key-----",
        "-----begin rsa private key-----",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return true;
    }
    value.split_whitespace().any(|word| {
        (word.starts_with("sk-") && word.len() > 20)
            || (word.starts_with("ghp_") && word.len() > 20)
            || (word.starts_with("github_pat_") && word.len() > 30)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> BridgeRoomInput {
        BridgeRoomInput {
            slug: "agent-lab".to_owned(),
            title: "Agent lab".to_owned(),
            objective: "Cross-check a bounded engineering decision".to_owned(),
        }
    }

    fn participant(id: &str, provider: Option<&str>) -> BridgeParticipantInput {
        BridgeParticipantInput {
            participant_id: id.to_owned(),
            display_name: id.to_owned(),
            kind: if provider.is_some() {
                BridgeParticipantKind::Agent
            } else {
                BridgeParticipantKind::Human
            },
            provider: provider.map(str::to_owned),
            model: None,
            runtime_agent_id: None,
        }
    }

    fn message(author: BridgeParticipantInput, summary: &str) -> BridgeMessageInput {
        BridgeMessageInput {
            protocol_version: BRIDGE_PROTOCOL_VERSION.to_owned(),
            message_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            author,
            summary: summary.to_owned(),
            reply_to: None,
        }
    }

    #[tokio::test]
    async fn records_cross_participant_contact_points_without_directed_routing() {
        let hub = BridgeHub::new();
        hub.create_room(room()).await.unwrap();
        hub.join("agent-lab", participant("human", None), false)
            .await
            .unwrap();
        hub.join("agent-lab", participant("codex", Some("openai")), true)
            .await
            .unwrap();
        let human = message(
            participant("human", None),
            "Please cross-check the bridge contract.",
        );
        let human_id = human.message_id;
        hub.post_message("agent-lab", human, Transport::Http)
            .await
            .unwrap();
        let mut agent = message(
            participant("codex", Some("openai")),
            "The HTTP and WebSocket contracts agree; TCP remains to verify.",
        );
        agent.reply_to = Some(human_id);
        let ack = hub
            .post_message("agent-lab", agent, Transport::WebSocket)
            .await
            .unwrap();

        let contact = ack.contact.unwrap();
        assert_eq!(contact.participants, vec!["human", "codex"]);
        let snapshot = hub.snapshot("agent-lab").await.unwrap();
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.contacts.len(), 1);
        assert_eq!(snapshot.counters.accepted_messages, 2);
        assert_eq!(snapshot.counters.accepted_by_transport["http"], 1);
        assert_eq!(snapshot.counters.accepted_by_transport["websocket"], 1);
    }

    #[tokio::test]
    async fn duplicate_messages_are_idempotent() {
        let hub = BridgeHub::new();
        hub.create_room(room()).await.unwrap();
        hub.join("agent-lab", participant("human", None), false)
            .await
            .unwrap();
        let message = message(participant("human", None), "One bounded message");
        hub.post_message("agent-lab", message.clone(), Transport::Http)
            .await
            .unwrap();
        let duplicate = hub
            .post_message("agent-lab", message, Transport::Tcp)
            .await
            .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(hub.snapshot("agent-lab").await.unwrap().messages.len(), 1);
    }

    #[test]
    fn rejects_secret_like_summaries_and_agent_without_provider() {
        let secret = message(
            participant("human", None),
            "Authorization: Bearer should-never-enter-a-room",
        );
        assert_eq!(secret.validate(), Err(BridgeError::SecretMaterial));

        let invalid_agent = participant("anonymous-agent", None);
        let mut invalid_agent = invalid_agent;
        invalid_agent.kind = BridgeParticipantKind::Agent;
        assert_eq!(
            invalid_agent.validate(),
            Err(BridgeError::MissingField("provider"))
        );
    }

    #[tokio::test]
    async fn requires_joined_identity_and_rejects_cross_room_id_reuse() {
        let hub = BridgeHub::new();
        hub.create_room(room()).await.unwrap();
        let author = participant("human", None);
        let input = message(author.clone(), "A joined identity is required.");
        assert_eq!(
            hub.post_message("agent-lab", input.clone(), Transport::Http)
                .await,
            Err(BridgeError::ParticipantNotJoined)
        );

        hub.join("agent-lab", author.clone(), false).await.unwrap();
        hub.post_message("agent-lab", input.clone(), Transport::Http)
            .await
            .unwrap();
        hub.create_room(BridgeRoomInput {
            slug: "second-room".to_owned(),
            title: "Second room".to_owned(),
            objective: "Check idempotency boundaries".to_owned(),
        })
        .await
        .unwrap();
        hub.join("second-room", author, false).await.unwrap();
        assert_eq!(
            hub.post_message("second-room", input, Transport::Http)
                .await,
            Err(BridgeError::MessageIdConflict)
        );
    }
}
