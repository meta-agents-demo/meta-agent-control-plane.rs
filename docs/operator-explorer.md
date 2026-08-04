# Bounded operator explorer

The operator explorer is a deterministic, read-only projection over one coherent `store::Snapshot`. It provides the data needed for agent, session, retained-timeline, lesson, and system-pressure views without introducing a second mutable state model.

## Endpoint

```text
GET /api/v1/explorer
```

The endpoint uses the same read-authentication policy as snapshots, metrics, metacognition, coordination, and the operator WebSocket.

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  'http://127.0.0.1:8787/api/v1/explorer?timeline_limit=100&session_limit=100&lesson_limit=250'
```

Authentication is evaluated before query parsing. An unauthenticated malformed query returns `401`, not query-validation details.

## Leptos operator page

Open `/explorer` for the server-rendered operator shell. Protected data is not embedded in the HTML response. The browser reads `/api/v1/explorer` only after applying the session-scoped read token.

The page displays:

- current agent identity, status, provider/model, session, goal, active task, capabilities, latest reflection, and latest error;
- retained session summaries and transport counts;
- a retained event timeline with visible normalized payloads;
- retained learned lessons and confidence;
- cache length, capacity, pressure, and eviction counts;
- accepted, duplicate, and rejected ingestion counters;
- explicit response-retention totals and omissions.

The limit controls use the same server defaults and maxima as the API. A client-side search filters only the already returned retained projection and does not alter the server query or imply historical completeness.

The page authenticates to same-origin `/ws/ui` with the read token in the first JSON frame, never in the URL. Newer revisions and `resync_required` notices coalesce into bounded refetches, reconnect delay is capped at 15 seconds, and a 30-second poll remains as a safety net.

## Projection contents

The response contains:

- agents sorted by stable `agent_id`;
- session summaries derived from retained recent events;
- retained events sorted by occurrence time and event ID;
- retained lessons sorted by learned time, agent ID, and lesson ID;
- current cache lengths, capacities, pressure, and eviction counts;
- accepted, duplicate, and rejected event counters;
- projection counts and non-negative uptime;
- explicit totals, returned counts, and omitted counts for timeline events, sessions, and lessons;
- the exact per-read policy applied to the response.

A session summary contains the session ID, sorted agent and task IDs, counts by event kind and transport, first and last retained occurrence times, the latest retained event ID and kind, and the latest retained task ID.

## Retention semantics

The explorer does not claim complete historical storage.

The base store snapshot exposes at most 250 recent events. Session summaries are therefore summaries of sessions visible in that retained window, not authoritative lifetime session records. Events without a `session_id` remain visible in the timeline but do not create synthetic sessions.

The response distinguishes:

- total items visible in the coherent snapshot;
- items returned under the requested response limits;
- items omitted from the response by those limits.

An omitted item is still retained in the source snapshot. An item absent from the snapshot may have never existed, may be outside the recent-event window, or may have been evicted by its configured LRU capacity. The API never converts absence into a historical claim.

## Deterministic ordering

For the same serialized snapshot and policy, the projection is byte-equivalent apart from ordinary JSON object formatting choices:

- agents: `agent_id` ascending;
- lessons: `learned_at` descending, then `agent_id`, then `lesson_id`;
- timeline: `occurred_at` descending, then event ID descending;
- sessions: latest retained occurrence descending, then session ID ascending;
- session agent and task IDs: lexical ascending;
- event-kind and transport counts: `BTreeMap` key order.

The explorer reuses `snapshot.generated_at` rather than reading the clock again, so repeated analysis of the same snapshot does not drift.

## Bounded per-read policy

| Parameter | Default | Server maximum |
| --- | ---: | ---: |
| `timeline_limit` | 100 | 250 |
| `session_limit` | 100 | 250 |
| `lesson_limit` | 250 | 1000 |

Parameters are strict positive base-10 integers. Each may appear at most once. Unknown names, missing `=`, zero, values above the server cap, integer overflow, and non-integer values fail with:

```json
{
  "error": "invalid_explorer_policy",
  "message": "bounded validation explanation"
}
```

The error does not echo the raw query or the retained snapshot. Overrides affect only that response and do not change daemon configuration, LRU recency, future projections, provider behavior, or store state.

## Safety boundary

The explorer:

- performs no ingestion;
- dispatches no work;
- changes no task ownership;
- calls no provider;
- writes no database or file;
- does not touch LRU recency because it operates on an already materialized snapshot;
- exposes only normalized visible state;
- HTML-escapes every dynamic value before inserting it into the operator page;
- never requests, reconstructs, or emits hidden reasoning.

The browser token remains in session storage, is sent only as a Bearer header or first WebSocket message, and is never included in a URL.

## Test coverage

Rust and repository-contract tests cover:

- authentication before parsing;
- default and custom bounded policies;
- strict rejection of malformed, zero, excessive, duplicate, unknown, and non-integer values;
- deterministic session grouping and timeline ordering;
- events without session IDs;
- event-kind and transport aggregation;
- explicit omission counts;
- repeated projection equality for one snapshot and policy;
- runtime and checked-in OpenAPI path and parameter synchronization;
- static-shell privacy and composed-router page coverage;
- same-origin WebSocket use and first-message authentication;
- absence of query-string credentials and mutation methods;
- revision/resync coalescing and bounded reconnect;
- client limit validation, retention display, and dynamic HTML escaping.
