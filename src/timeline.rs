use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::store::{EventRecord, Snapshot};

pub const MAX_TIMELINE_PAGE_LIMIT: usize = 100;
const CURSOR_VERSION: &str = "v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelinePolicy {
    pub limit: usize,
}

impl Default for TimelinePolicy {
    fn default() -> Self {
        Self { limit: 50 }
    }
}

impl TimelinePolicy {
    pub fn validate(self) -> Result<Self, TimelineError> {
        if self.limit == 0 || self.limit > MAX_TIMELINE_PAGE_LIMIT {
            return Err(TimelineError::InvalidLimit);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelinePage {
    pub generated_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub revision: u64,
    pub events: Vec<EventRecord>,
    pub next_cursor: Option<String>,
    pub retained_total: usize,
    pub returned: usize,
    pub remaining_older: usize,
    pub newer_retained_events_skipped: usize,
    pub event_cache_evictions: u64,
    pub policy: TimelinePolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineCursor {
    pub revision: u64,
    pub occurred_at: DateTime<Utc>,
    pub event_id: Uuid,
}

impl TimelineCursor {
    pub fn encode(self) -> String {
        let seconds_bits = self.occurred_at.timestamp() as u64;
        format!(
            "{CURSOR_VERSION}.{:016x}.{seconds_bits:016x}.{:08x}.{}",
            self.revision,
            self.occurred_at.timestamp_subsec_nanos(),
            self.event_id.simple()
        )
    }

    pub fn parse(value: &str) -> Result<Self, TimelineError> {
        if value.len() > 96 {
            return Err(TimelineError::InvalidCursor);
        }
        let mut fields = value.split('.');
        let version = fields.next().ok_or(TimelineError::InvalidCursor)?;
        let revision = fields.next().ok_or(TimelineError::InvalidCursor)?;
        let seconds = fields.next().ok_or(TimelineError::InvalidCursor)?;
        let nanos = fields.next().ok_or(TimelineError::InvalidCursor)?;
        let event_id = fields.next().ok_or(TimelineError::InvalidCursor)?;
        if fields.next().is_some() || version != CURSOR_VERSION {
            return Err(TimelineError::InvalidCursor);
        }
        if revision.len() != 16 || seconds.len() != 16 || nanos.len() != 8 || event_id.len() != 32 {
            return Err(TimelineError::InvalidCursor);
        }
        let revision =
            u64::from_str_radix(revision, 16).map_err(|_| TimelineError::InvalidCursor)?;
        let seconds_bits =
            u64::from_str_radix(seconds, 16).map_err(|_| TimelineError::InvalidCursor)?;
        let seconds = seconds_bits as i64;
        let nanos = u32::from_str_radix(nanos, 16).map_err(|_| TimelineError::InvalidCursor)?;
        if nanos >= 1_000_000_000 {
            return Err(TimelineError::InvalidCursor);
        }
        let occurred_at = Utc
            .timestamp_opt(seconds, nanos)
            .single()
            .ok_or(TimelineError::InvalidCursor)?;
        let event_id = Uuid::parse_str(event_id).map_err(|_| TimelineError::InvalidCursor)?;
        Ok(Self {
            revision,
            occurred_at,
            event_id,
        })
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TimelineError {
    #[error("timeline limit must be between 1 and {MAX_TIMELINE_PAGE_LIMIT}")]
    InvalidLimit,
    #[error("timeline cursor is invalid")]
    InvalidCursor,
    #[error("timeline cursor revision {requested} does not match current revision {current}")]
    RevisionChanged { requested: u64, current: u64 },
}

pub fn build_timeline_page(
    snapshot: &Snapshot,
    policy: TimelinePolicy,
    cursor: Option<TimelineCursor>,
) -> Result<TimelinePage, TimelineError> {
    let policy = policy.validate()?;
    if let Some(cursor) = cursor {
        if cursor.revision != snapshot.revision {
            return Err(TimelineError::RevisionChanged {
                requested: cursor.revision,
                current: snapshot.revision,
            });
        }
    }

    let mut retained = snapshot.recent_events.clone();
    retained.sort_by(|left, right| {
        right
            .event
            .occurred_at
            .cmp(&left.event.occurred_at)
            .then_with(|| right.event.event_id.cmp(&left.event.event_id))
    });
    let retained_total = retained.len();

    let eligible = retained
        .into_iter()
        .filter(|record| {
            cursor.is_none_or(|cursor| {
                record.event.occurred_at < cursor.occurred_at
                    || (record.event.occurred_at == cursor.occurred_at
                        && record.event.event_id < cursor.event_id)
            })
        })
        .collect::<Vec<_>>();
    let newer_retained_events_skipped = retained_total.saturating_sub(eligible.len());
    let has_more = eligible.len() > policy.limit;
    let mut events = eligible;
    events.truncate(policy.limit);
    let remaining_older = if has_more {
        newer_retained_events_skipped
            .checked_add(events.len())
            .map(|consumed| retained_total.saturating_sub(consumed))
            .unwrap_or(0)
    } else {
        0
    };
    let next_cursor = has_more.then(|| {
        let last = events
            .last()
            .expect("has_more requires at least one returned event");
        TimelineCursor {
            revision: snapshot.revision,
            occurred_at: last.event.occurred_at,
            event_id: last.event.event_id,
        }
        .encode()
    });

    Ok(TimelinePage {
        generated_at: snapshot.generated_at,
        started_at: snapshot.started_at,
        revision: snapshot.revision,
        returned: events.len(),
        events,
        next_cursor,
        retained_total,
        remaining_older,
        newer_retained_events_skipped,
        event_cache_evictions: snapshot.caches.events.evictions,
        policy,
    })
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

    fn record(event_id: &str, occurred_at: &str) -> EventRecord {
        let mut value: Value =
            serde_json::from_str(include_str!("../fixtures/progress-updated.json")).unwrap();
        value["event_id"] = Value::String(event_id.to_owned());
        value["occurred_at"] = Value::String(occurred_at.to_owned());
        let event: EventEnvelope = serde_json::from_value(value).unwrap();
        EventRecord {
            received_at: event.occurred_at + Duration::seconds(1),
            event,
            transport: Transport::Http,
        }
    }

    fn snapshot(records: Vec<EventRecord>, revision: u64) -> Snapshot {
        let started_at = Utc.with_ymd_and_hms(2026, 7, 30, 19, 0, 0).unwrap();
        let stats = cache_stats();
        Snapshot {
            generated_at: started_at + Duration::hours(1),
            started_at,
            revision,
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
    fn cursor_round_trip_preserves_nanoseconds_and_uuid() {
        let cursor = TimelineCursor {
            revision: 42,
            occurred_at: Utc.with_ymd_and_hms(2026, 7, 30, 20, 0, 0).unwrap()
                + Duration::nanoseconds(987_654_321),
            event_id: Uuid::parse_str("018f5c8a-d5a7-7f7c-8d61-84f55a35fe91").unwrap(),
        };
        assert_eq!(TimelineCursor::parse(&cursor.encode()).unwrap(), cursor);
    }

    #[test]
    fn first_and_second_pages_are_stable_and_non_overlapping() {
        let snapshot = snapshot(
            vec![
                record(
                    "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91",
                    "2026-07-30T20:00:00Z",
                ),
                record(
                    "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92",
                    "2026-07-30T20:02:00Z",
                ),
                record(
                    "018f5c8a-d5a7-7f7c-8d61-84f55a35fe93",
                    "2026-07-30T20:01:00Z",
                ),
            ],
            9,
        );
        let first = build_timeline_page(&snapshot, TimelinePolicy { limit: 2 }, None).unwrap();
        assert_eq!(first.returned, 2);
        assert_eq!(first.remaining_older, 1);
        assert_eq!(first.newer_retained_events_skipped, 0);
        assert_eq!(
            first.events[0].event.event_id.to_string(),
            "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92"
        );
        assert_eq!(
            first.events[1].event.event_id.to_string(),
            "018f5c8a-d5a7-7f7c-8d61-84f55a35fe93"
        );

        let cursor = TimelineCursor::parse(first.next_cursor.as_deref().unwrap()).unwrap();
        let second =
            build_timeline_page(&snapshot, TimelinePolicy { limit: 2 }, Some(cursor)).unwrap();
        assert_eq!(second.returned, 1);
        assert_eq!(second.remaining_older, 0);
        assert_eq!(second.newer_retained_events_skipped, 2);
        assert!(second.next_cursor.is_none());
        assert_eq!(
            second.events[0].event.event_id.to_string(),
            "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91"
        );
    }

    #[test]
    fn equal_timestamps_use_event_id_as_the_keyset_tiebreaker() {
        let snapshot = snapshot(
            vec![
                record(
                    "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91",
                    "2026-07-30T20:00:00Z",
                ),
                record(
                    "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92",
                    "2026-07-30T20:00:00Z",
                ),
            ],
            1,
        );
        let page = build_timeline_page(&snapshot, TimelinePolicy { limit: 1 }, None).unwrap();
        assert_eq!(
            page.events[0].event.event_id.to_string(),
            "018f5c8a-d5a7-7f7c-8d61-84f55a35fe92"
        );
    }

    #[test]
    fn cursor_revision_must_match_the_current_snapshot() {
        let snapshot = snapshot(Vec::new(), 11);
        let cursor = TimelineCursor {
            revision: 10,
            occurred_at: snapshot.generated_at,
            event_id: Uuid::nil(),
        };
        assert!(matches!(
            build_timeline_page(&snapshot, TimelinePolicy::default(), Some(cursor)),
            Err(TimelineError::RevisionChanged {
                requested: 10,
                current: 11,
            })
        ));
    }

    #[test]
    fn invalid_limits_and_cursors_fail_closed() {
        assert_eq!(
            TimelinePolicy { limit: 0 }.validate(),
            Err(TimelineError::InvalidLimit)
        );
        assert_eq!(
            TimelineCursor::parse("v1.invalid"),
            Err(TimelineError::InvalidCursor)
        );
    }
}
