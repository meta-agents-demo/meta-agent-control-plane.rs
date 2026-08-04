# Revision-bound retained timeline pagination

The retained timeline API pages the daemon's existing bounded recent-event ring. It is designed for windowed operator views and compact payload inspection; it is not a durable history API.

## Endpoint

```text
GET /api/v1/timeline
```

The first page uses a default limit of 50:

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  'http://127.0.0.1:8787/api/v1/timeline?limit=50'
```

If `next_cursor` is non-null, pass it unchanged:

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  "http://127.0.0.1:8787/api/v1/timeline?limit=50&cursor=${NEXT_CURSOR}"
```

Authentication is evaluated before the query or cursor is parsed.

## Ordering and cursor

Events use a total descending order:

1. `occurred_at` descending;
2. event UUID descending when timestamps are equal.

The opaque URL-safe cursor contains:

- cursor format version;
- snapshot revision;
- exact signed Unix seconds;
- nanoseconds;
- event UUID.

The cursor is not an authorization token and contains no event payload, agent identifier, session identifier, or secret. Consumers must treat its encoding as opaque.

A cursor is valid only for the exact snapshot revision that produced it. If accepted state changes between pages, the endpoint returns `409 timeline_revision_changed`; the client must restart from the first page. This fail-closed behavior prevents a page sequence from silently combining two different state revisions.

## Response

A page contains:

- the coherent snapshot's `generated_at`, `started_at`, and `revision`;
- ordered retained `events`;
- `next_cursor` when older retained events remain;
- `retained_total` visible in the snapshot;
- `returned` event count;
- `remaining_older` count after this page;
- `newer_retained_events_skipped` when continuing from a cursor;
- event-cache eviction count;
- the applied page policy.

`remaining_older` describes only events retained in this snapshot. A zero value does not prove that no older events ever existed.

## Bounded policy

| Parameter | Default | Maximum |
| --- | ---: | ---: |
| `limit` | 50 | 100 |

The query permits only `limit` and `cursor`, each at most once. Zero, values above the cap, integer overflow, malformed cursors, duplicate parameters, unknown parameters, non-integer limits, and missing `=` fail with a bounded `400 invalid_timeline_query` response. Errors do not echo the cursor, raw query, or event payload.

## Retention boundary

The source ring retains at most the configured recent-event capacity, currently capped at 250. Capacity eviction, process restart, or a future durable-backend policy may remove events that were previously visible.

The endpoint exposes the event-cache eviction count so operators can distinguish an unpressured current process from a ring that has discarded retained events. Even when that count is zero, the API makes no claim about events before the current process start.

## Safety properties

- read authorization precedes parsing;
- no offset scans or unbounded page sizes;
- no state, LRU recency, ownership, or provider mutation;
- stable keyset ordering for equal timestamps;
- nanosecond timestamp fidelity;
- explicit conflict on revision drift;
- no cursor or query echo in errors;
- no hidden reasoning requested or emitted.

## Test coverage

Rust tests cover:

- cursor encode/parse round trips with nanoseconds and UUIDs;
- stable non-overlapping first and second pages;
- equal-timestamp UUID ordering;
- explicit remaining and skipped retained counts;
- revision-conflict behavior;
- authentication before parsing;
- default and custom limits;
- malformed, duplicate, unknown, zero, excessive, and non-integer query rejection;
- composed-router read protection.
