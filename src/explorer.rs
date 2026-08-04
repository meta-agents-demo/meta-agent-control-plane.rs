use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::store::{AgentState, CacheSnapshot, Counters, EventRecord, LessonState, Snapshot};

pub const MAX_TIMELINE_LIMIT: usize = 250;
pub const MAX_SESSION_LIMIT: usize = 250;
pub const MAX_LESSON_LIMIT: usize = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExplorerPolicy {
    pub timeline_limit: usize,
    pub session_limit: usize,
    pub lesson_limit: usize,
}

impl Default for ExplorerPolicy {
    fn default() -> Self {
        Self {
            timeline_limit: 100,
            session_limit: 100,
            lesson_limit: 250,
        }
    }
}

impl ExplorerPolicy {
    pub fn validate(self) -> Result<Self, ExplorerError> {
        validate_limit("timeline_limit", self.timeline_limit, MAX_TIMELINE_LIMIT)?;
        validate_limit("session_limit", self.session_limit, MAX_SESSION_LIMIT)?;
        validate_limit("lesson_limit", self.lesson_limit, MAX_LESSON_LIMIT)?;
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExplorerSnapshot {
    pub generated_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub revision: u64,
    pub agents: Vec<AgentState>,
    pub sessions: Vec<SessionSummary>,
    pub lessons: Vec<LessonState>,
    pub timeline: Vec<EventRecord>,
    pub system: SystemSummary,
    pub retention: RetentionSummary,
    pub policy: ExplorerPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub agent_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub event_kinds: BTreeMap<String, u64>,
    pub transports: BTreeMap<String, u64>,
    pub event_count: u64,
    pub first_occurred_at: DateTime<Utc>,
    pub last_occurred_at: DateTime<Utc>,
    pub latest_event_id: String,
    pub latest_event_kind: String,
    pub latest_task_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemSummary {
    pub uptime_seconds: u64,
    pub agents: usize,
    pub goals: usize,
    pub tasks: usize,
    pub lessons: usize,
    pub retained_events: usize,
    pub retained_sessions: usize,
    pub caches: CacheSnapshot,
    pub counters: Counters,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetentionSummary {
    pub total_timeline_events: usize,
    pub returned_timeline_events: usize,
    pub omitted_timeline_events: usize,
    pub total_sessions: usize,
    pub returned_sessions: usize,
    pub omitted_sessions: usize,
    pub total_lessons: usize,
    pub returned_lessons: usize,
    pub omitted_lessons: usize,
}

#[derive(Debug, Error)]
pub enum ExplorerError {
    #[error("{name} must be between 1 and {maximum}")]
    InvalidLimit { name: &'static str, maximum: usize },
}

#[derive(Debug)]
struct SessionAccumulator {
    agent_ids: BTreeSet<String>,
    task_ids: BTreeSet<String>,
    event_kinds: BTreeMap<String, u64>,
    transports: BTreeMap<String, u64>,
    event_count: u64,
    first_occurred_at: DateTime<Utc>,
    last_occurred_at: DateTime<Utc>,
    latest_event_id: String,
    latest_event_kind: String,
    latest_task_id: Option<String>,
}

impl SessionAccumulator {
    fn new(record: &EventRecord) -> Self {
        let event = &record.event;
        let mut agent_ids = BTreeSet::new();
        agent_ids.insert(event.agent.agent_id.clone());
        let mut task_ids = BTreeSet::new();
        if let Some(task_id) = event.task_id() {
            task_ids.insert(task_id.to_owned());
        }
        let mut event_kinds = BTreeMap::new();
        event_kinds.insert(event.kind().to_owned(), 1);
        let mut transports = BTreeMap::new();
        transports.insert(record.transport.to_string(), 1);
        Self {
            agent_ids,
            task_ids,
            event_kinds,
            transports,
            event_count: 1,
            first_occurred_at: event.occurred_at,
            last_occurred_at: event.occurred_at,
            latest_event_id: event.event_id.to_string(),
            latest_event_kind: event.kind().to_owned(),
            latest_task_id: event.task_id().map(str::to_owned),
        }
    }

    fn observe(&mut self, record: &EventRecord) {
        let event = &record.event;
        self.agent_ids.insert(event.agent.agent_id.clone());
        if let Some(task_id) = event.task_id() {
            self.task_ids.insert(task_id.to_owned());
        }
        *self.event_kinds.entry(event.kind().to_owned()).or_default() += 1;
        *self
            .transports
            .entry(record.transport.to_string())
            .or_default() += 1;
        self.event_count = self.event_count.saturating_add(1);
        self.first_occurred_at = self.first_occurred_at.min(event.occurred_at);

        let event_id = event.event_id.to_string();
        if event.occurred_at > self.last_occurred_at
            || (event.occurred_at == self.last_occurred_at && event_id > self.latest_event_id)
        {
            self.last_occurred_at = event.occurred_at;
            self.latest_event_id = event_id;
            self.latest_event_kind = event.kind().to_owned();
            self.latest_task_id = event.task_id().map(str::to_owned);
        }
    }

    fn finish(self, session_id: String) -> SessionSummary {
        SessionSummary {
            session_id,
            agent_ids: self.agent_ids.into_iter().collect(),
            task_ids: self.task_ids.into_iter().collect(),
            event_kinds: self.event_kinds,
            transports: self.transports,
            event_count: self.event_count,
            first_occurred_at: self.first_occurred_at,
            last_occurred_at: self.last_occurred_at,
            latest_event_id: self.latest_event_id,
            latest_event_kind: self.latest_event_kind,
            latest_task_id: self.latest_task_id,
        }
    }
}

pub fn build_explorer(
    snapshot: &Snapshot,
    policy: ExplorerPolicy,
) -> Result<ExplorerSnapshot, ExplorerError> {
    let policy = policy.validate()?;

    let mut agents = snapshot.agents.clone();
    agents.sort_by(|left, right| left.agent.agent_id.cmp(&right.agent.agent_id));

    let mut lessons = snapshot.lessons.clone();
    lessons.sort_by(|left, right| {
        right
            .learned_at
            .cmp(&left.learned_at)
            .then_with(|| left.agent_id.cmp(&right.agent_id))
            .then_with(|| left.lesson.lesson_id.cmp(&right.lesson.lesson_id))
    });
    let total_lessons = lessons.len();
    lessons.truncate(policy.lesson_limit);

    let mut timeline = snapshot.recent_events.clone();
    timeline.sort_by(|left, right| {
        right
            .event
            .occurred_at
            .cmp(&left.event.occurred_at)
            .then_with(|| right.event.event_id.cmp(&left.event.event_id))
    });
    let total_timeline_events = timeline.len();

    let mut sessions = BTreeMap::<String, SessionAccumulator>::new();
    for record in &timeline {
        let Some(session_id) = record.event.session_id.as_deref() else {
            continue;
        };
        match sessions.get_mut(session_id) {
            Some(accumulator) => accumulator.observe(record),
            None => {
                sessions.insert(session_id.to_owned(), SessionAccumulator::new(record));
            }
        }
    }
    let mut sessions = sessions
        .into_iter()
        .map(|(session_id, accumulator)| accumulator.finish(session_id))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .last_occurred_at
            .cmp(&left.last_occurred_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let total_sessions = sessions.len();

    timeline.truncate(policy.timeline_limit);
    sessions.truncate(policy.session_limit);

    let generated_at = snapshot.generated_at;
    let uptime_seconds = generated_at
        .signed_duration_since(snapshot.started_at)
        .num_seconds()
        .max(0) as u64;
    let system = SystemSummary {
        uptime_seconds,
        agents: snapshot.agents.len(),
        goals: snapshot.goals.len(),
        tasks: snapshot.tasks.len(),
        lessons: snapshot.lessons.len(),
        retained_events: total_timeline_events,
        retained_sessions: total_sessions,
        caches: snapshot.caches.clone(),
        counters: snapshot.counters.clone(),
    };
    let retention = RetentionSummary {
        total_timeline_events,
        returned_timeline_events: timeline.len(),
        omitted_timeline_events: total_timeline_events.saturating_sub(timeline.len()),
        total_sessions,
        returned_sessions: sessions.len(),
        omitted_sessions: total_sessions.saturating_sub(sessions.len()),
        total_lessons,
        returned_lessons: lessons.len(),
        omitted_lessons: total_lessons.saturating_sub(lessons.len()),
    };

    Ok(ExplorerSnapshot {
        generated_at,
        started_at: snapshot.started_at,
        revision: snapshot.revision,
        agents,
        sessions,
        lessons,
        timeline,
        system,
        retention,
        policy,
    })
}

fn validate_limit(name: &'static str, value: usize, maximum: usize) -> Result<(), ExplorerError> {
    if value == 0 || value > maximum {
        return Err(ExplorerError::InvalidLimit { name, maximum });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use serde_json::Value;

    use crate::{
        model::{EventEnvelope, Transport},
        store::{CacheSnapshot, CacheStats, Counters, EventRecord, Snapshot},
    };

    use super::*;

    fn cache_stats() -> CacheStats {
        CacheStats {
            length: 0,
            capacity: 32,
            evictions: 0,
            pressure: 0.0,
        }
    }

    fn record(
        event_id: &str,
        occurred_at: &str,
        session_id: Option<&str>,
        agent_id: &str,
        task_id: &str,
        transport: Transport,
    ) -> EventRecord {
        let mut value: Value =
            serde_json::from_str(include_str!("../fixtures/progress-updated.json")).unwrap();
        value["event_id"] = Value::String(event_id.to_owned());
        value["occurred_at"] = Value::String(occurred_at.to_owned());
        value["agent"]["agent_id"] = Value::String(agent_id.to_owned());
        value["session_id"] = session_id
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null);
        value["data"]["task_id"] = Value::String(task_id.to_owned());
        let event: EventEnvelope = serde_json::from_value(value).unwrap();
        EventRecord {
            received_at: event.occurred_at + Duration::seconds(1),
            event,
            transport,
        }
    }

    fn snapshot(records: Vec<EventRecord>) -> Snapshot {
        let started_at = Utc.with_ymd_and_hms(2026, 7, 30, 19, 0, 0).unwrap();
        let stats = cache_stats();
        Snapshot {
            generated_at: started_at + Duration::hours(1),
            started_at,
            revision: records.len() as u64,
            agents: Vec::new(),
            goals: Vec::new(),
            tasks: Vec::new(),
            lessons: Vec::new(),
            recent_events: records,
            caches: CacheSnapshot {
                agents: stats,
                goals: stats,
                tasks: stats,
                lessons: stats,
                events: stats,
                idempotency: stats,
            },
            counters: Counters::default(),
        }
    }

    #[test]
    fn groups_sessions_and_sorts_timeline_deterministically() {
        let snapshot = snapshot(vec![
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91",
                "2026-07-30T20:00:00Z",
                Some("session-a"),
                "agent-b",
                "task-2",
                Transport::Tcp,
            ),
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92",
                "2026-07-30T20:05:00Z",
                Some("session-a"),
                "agent-a",
                "task-1",
                Transport::Http,
            ),
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe93",
                "2026-07-30T20:03:00Z",
                Some("session-b"),
                "agent-c",
                "task-3",
                Transport::WebSocket,
            ),
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe94",
                "2026-07-30T20:04:00Z",
                None,
                "agent-d",
                "task-4",
                Transport::Udp,
            ),
        ]);

        let explorer = build_explorer(&snapshot, ExplorerPolicy::default()).unwrap();

        assert_eq!(explorer.sessions.len(), 2);
        assert_eq!(explorer.sessions[0].session_id, "session-a");
        assert_eq!(
            explorer.sessions[0].agent_ids,
            vec!["agent-a".to_owned(), "agent-b".to_owned()]
        );
        assert_eq!(
            explorer.sessions[0].task_ids,
            vec!["task-1".to_owned(), "task-2".to_owned()]
        );
        assert_eq!(explorer.sessions[0].event_count, 2);
        assert_eq!(
            explorer.sessions[0].latest_task_id.as_deref(),
            Some("task-1")
        );
        assert_eq!(explorer.sessions[0].transports["http"], 1);
        assert_eq!(explorer.sessions[0].transports["tcp"], 1);
        assert_eq!(
            explorer.timeline[0].event.event_id.to_string(),
            "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92"
        );
        assert_eq!(explorer.retention.total_sessions, 2);
        assert_eq!(explorer.system.retained_events, 4);
        assert_eq!(explorer.generated_at, snapshot.generated_at);

        let replay = build_explorer(&snapshot, ExplorerPolicy::default()).unwrap();
        assert_eq!(
            serde_json::to_value(explorer).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
    }

    #[test]
    fn reports_omitted_retained_items_without_implying_deletion() {
        let snapshot = snapshot(vec![
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91",
                "2026-07-30T20:00:00Z",
                Some("session-a"),
                "agent-a",
                "task-1",
                Transport::Http,
            ),
            record(
                "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92",
                "2026-07-30T20:01:00Z",
                Some("session-b"),
                "agent-b",
                "task-2",
                Transport::Tcp,
            ),
        ]);

        let explorer = build_explorer(
            &snapshot,
            ExplorerPolicy {
                timeline_limit: 1,
                session_limit: 1,
                lesson_limit: 1,
            },
        )
        .unwrap();

        assert_eq!(explorer.timeline.len(), 1);
        assert_eq!(explorer.sessions.len(), 1);
        assert_eq!(explorer.retention.omitted_timeline_events, 1);
        assert_eq!(explorer.retention.omitted_sessions, 1);
    }

    #[test]
    fn rejects_zero_and_above_cap_policies() {
        assert!(matches!(
            ExplorerPolicy {
                timeline_limit: 0,
                ..ExplorerPolicy::default()
            }
            .validate(),
            Err(ExplorerError::InvalidLimit {
                name: "timeline_limit",
                ..
            })
        ));
        assert!(matches!(
            ExplorerPolicy {
                session_limit: MAX_SESSION_LIMIT + 1,
                ..ExplorerPolicy::default()
            }
            .validate(),
            Err(ExplorerError::InvalidLimit {
                name: "session_limit",
                ..
            })
        ));
    }
}
