# Meta-Agent Control Plane

A single-process Rust daemon and Leptos SSR dashboard for observing AI-agent work, coordinating meta-tasks, and retaining bounded, reusable lessons.

The server is provider-neutral. OpenAI API-backed agents, Anthropic/Claude-backed agents, Google/Gemini-backed agents, local models, and custom runtimes all emit the same structured event protocol over HTTP, WebSocket, TCP, or UDP. The human operator opens one web UI to see agents, goals, tasks, progress, blockers, explicit reflections, lessons, transport activity, memory pressure, explainable metacognition diagnostics, and deterministic coordination plans.

> This project records observable summaries, claims, evidence references, confidence, assumptions, risks, decisions, and next actions. It does not request, store, or expose hidden chain-of-thought or private model reasoning.

## One binary, four ingress paths

- **HTTP JSON** — reliable event ingestion at `POST /api/v1/events`.
- **WebSocket** — reliable interactive ingestion at `/ws/agent`.
- **TCP NDJSON** — reliable local-daemon or sidecar ingestion.
- **UDP JSON** — best-effort, low-authority telemetry only.

The same Tokio process serves Axum APIs, the Leptos SSR dashboards, WebSockets, TCP, UDP, health/readiness, Prometheus metrics, and OpenAPI.

## Quick start

Rust 1.97.1 is pinned in `rust-toolchain.toml`.

```bash
cargo run -- \
  --auth-token 'replace-with-at-least-16-bytes' \
  --protect-read-api
```

Open `http://127.0.0.1:8787`. The default listeners are:

| Transport | Address |
| --- | --- |
| HTTP / WebSocket / UI | `127.0.0.1:8787` |
| TCP NDJSON | `127.0.0.1:8788` |
| UDP JSON | `127.0.0.1:8789` |

The server refuses non-loopback bindings unless authentication and read protection are enabled, unless the operator explicitly selects the unsafe isolated-network override.

## Emit an event over HTTP

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  -H 'content-type: application/json' \
  --data @fixtures/progress-updated.json \
  http://127.0.0.1:8787/api/v1/events
```

Provider-neutral Rust examples are included:

```bash
META_AGENT_TOKEN='replace-with-at-least-16-bytes' cargo run --example tcp_progress
META_AGENT_TOKEN='replace-with-at-least-16-bytes' cargo run --example udp_heartbeat
META_AGENT_TOKEN='replace-with-at-least-16-bytes' cargo run --example mock_provider_parallel
```

## Provider sidecar and embedded Rust client

`meta-agent-sidecar` converts observable OpenAI, Anthropic, or Gemini lifecycle updates into the canonical protocol and sends them through the same Rust client used by tests and examples.

```bash
cargo run --bin meta-agent-sidecar -- \
  --provider openai \
  --dry-run \
  < fixtures/providers/openai-progress.json

META_AGENT_TOKEN='replace-with-at-least-16-bytes' \
  cargo run --bin meta-agent-sidecar -- \
    --provider anthropic \
    --transport websocket \
    --endpoint ws://127.0.0.1:8787/ws/agent \
    < fixtures/providers/anthropic-reflection.json
```

The sidecar rejects hidden-reasoning fields recursively, never inserts provider credentials or the control-plane token into an event, and never echoes a rejected payload. The embedded client supports authenticated HTTP, WebSocket, TCP, and UDP acknowledgements with event-ID and transport validation. UDP command/control events fail locally before transmission.

This is an integration surface for programmable API-backed agents. It does not claim that the ChatGPT consumer product offers a direct daemon connection. See [`docs/provider-sidecar.md`](docs/provider-sidecar.md) for the exact payloads, privacy boundary, TLS guidance, and runnable demos.

## Event model

Every message uses a versioned envelope:

```json
{
  "protocol_version": "v1",
  "event_id": "018f5c8a-d5a7-7f7c-8d61-84f55a35fe91",
  "occurred_at": "2026-07-30T20:00:00Z",
  "agent": {
    "agent_id": "review-agent",
    "provider": "anthropic",
    "model": "claude",
    "instance_id": "local-daemon-1"
  },
  "session_id": "run-42",
  "correlation_id": "goal-17",
  "sequence": 8,
  "kind": "progress_updated",
  "data": {
    "task_id": "task-semantic-merge",
    "progress": 0.65,
    "summary": "Reconciled protocol and state semantics.",
    "next_action": "Run the transport conformance suite."
  }
}
```

See [`docs/protocol.md`](docs/protocol.md) for event shapes, reliability semantics, UDP policy, idempotency, ordering, and privacy boundaries.

## Explainable metacognition

The deterministic metacognition engine derives visible diagnostics only from retained public state. It detects stalls, retry loops, unresolved or missing dependencies, dependency cycles, orphan goals, missing evidence, low confidence, missing next actions, and contradictory completion state. It keeps self-reported progress separate from evidence-backed progress and exposes event IDs supporting each explanation when those events remain retained.

Open `/metacognition` for the Leptos operator view or read `/api/v1/metacognition` with the configured read token. No hidden reasoning is requested or reconstructed.

## Deterministic coordination plan

The coordination planner turns the same bounded snapshot and explainable diagnostics into a dependency-safe, fair-share plan. It emits assignments, operator interventions, and held tasks with stable IDs, priorities, rationales, recommended actions, diagnostic links, and retained source-event provenance.

Open `/coordination` for the Leptos operator view or read `GET /api/v1/coordination` with the configured read token. Both surfaces are advisory and read-only: they do not dispatch work, mutate task ownership, or call any provider. The page authenticates to same-origin `/ws/ui` with a first-message token frame, coalesces revision updates and lag-resync notices into bounded refetches, reconnects with capped exponential backoff, and retains a 30-second safety poll. The same planner is available offline through `meta-agent-plan`; see [`docs/coordination-planner.md`](docs/coordination-planner.md) for policy limits and deterministic semantics.

## Bounded operator explorer

Open `/explorer` for the Leptos operator view or read `GET /api/v1/explorer` with the configured read token. The explorer derives one operator-oriented read model from a coherent bounded snapshot: agents in stable ID order, retained session summaries, a deterministic recent-event timeline, retained lessons, cache pressure and evictions, ingestion counters, uptime, and explicit counts for events, sessions, and lessons omitted by the requested response limits.

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  'http://127.0.0.1:8787/api/v1/explorer?timeline_limit=100&session_limit=100&lesson_limit=250'
```

The endpoint authenticates before parsing limits. Overrides apply only to that response, cannot mutate daemon configuration or store recency, and are independently capped by the server. Session summaries are derived only from currently retained recent events; absence from the response is never represented as historical absence.

The page keeps the token in session storage, sends it only as a Bearer header or first same-origin `/ws/ui` message, coalesces newer revisions and lag-resync notices into bounded refetches, reconnects with capped exponential backoff, and retains a 30-second safety poll. Client filtering operates only on the already returned retained projection, and every dynamic value is escaped before HTML insertion. See [`docs/operator-explorer.md`](docs/operator-explorer.md) for retention semantics, ordering guarantees, limits, live-update behavior, and test coverage.
Open `/coordination` for the Leptos operator view or read `GET /api/v1/coordination` with the configured read token. Both surfaces are advisory and read-only: they do not dispatch work, mutate task ownership, or call any provider. The same planner is available offline through `meta-agent-plan`; see [`docs/coordination-planner.md`](docs/coordination-planner.md) for policy limits and deterministic semantics.

## Bounded in-memory state

The MVP deliberately has no database. `lru::LruCache` instances bound agents, goals, tasks, lessons, recent events, and the independent event-ID idempotency window. Capacities are configurable, pressure and evictions are exposed in the snapshot/UI/metrics, and domain storage is isolated behind the `Store` API so a durable backend can follow without changing transports.

The reducer resolves out-of-order and conflicting observations semantically:

- an inferred task can later receive an authoritative definition without losing progress, attempts, blockers, reflections, or outcome state;
- an older task definition cannot overwrite a newer authoritative definition;
- task/goal/lesson identifiers are scoped by agent;
- repeated completion claims do not inflate counters;
- corrected outcomes move counters between success/failure buckets;
- a newer progress or start event can reopen a completed task cleanly;
- event deduplication has its own bounded LRU window and is not coupled to the shorter UI timeline.

## UDP safety boundary

UDP accepts only these low-authority telemetry events:

- `heartbeat`
- `progress_updated`
- `reflection_recorded`
- `error_observed`
- `agent_status_changed`

Agent registration, goal/task definitions, task start/completion, and learned lessons require HTTP, WebSocket, or TCP. UDP delivery, ordering, and acknowledgements are never treated as reliable. Use an authenticated private network or tunnel for remote UDP; the token does not provide encryption.

## Operator endpoints

| Route | Purpose |
| --- | --- |
| `/` | Leptos SSR operator dashboard |
| `/metacognition` | Explainable metacognition dashboard |
| `/coordination` | Deterministic coordination-plan dashboard |
| `/explorer` | Agents, retained sessions, timeline, lessons, and system-pressure dashboard |
| `/healthz` | Liveness and current revision |
| `/readyz` | Readiness |
| `/metrics` | Prometheus text exposition |
| `/openapi.json` | Runtime OpenAPI document |
| `/api/v1/snapshot` | Current bounded projection |
| `/api/v1/explorer` | Bounded operator explorer projection |
| `/api/v1/metacognition` | Deterministic diagnostic projection |
| `/api/v1/coordination` | Deterministic dependency-safe coordination plan |
| `/api/v1/events` | HTTP event ingestion |
| `/ws/agent` | Agent ingestion socket |
| `/ws/ui` | Authenticated revision invalidation stream for operator pages |

## Configuration

All flags have `META_AGENT_*` environment-variable equivalents. Run `cargo run -- --help` for the complete list.

Important controls include listener addresses, auth/read protection, payload/datagram limits, maximum TCP connections, cache capacities, update-channel capacity, CORS policy, and structured logging.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
node --check scripts/dashboard.js
node --check scripts/metacognition-dashboard.js
node --check scripts/coordination-dashboard.js
node --check scripts/explorer-dashboard.js
python3 scripts/verify_contract.py
python3 scripts/test_coordination_dashboard.py
python3 scripts/test_explorer_dashboard.py
```

CI runs the real-daemon client transport tests and executes the provider sidecar binary against deterministic fixtures. It also builds the OCI image from `Cargo.lock`, verifies its non-root entrypoint, boots it with a read-only root and dropped capabilities, and probes liveness and readiness.

## Deployment

```bash
docker build -t meta-agent-control-plane:local .
docker run --rm \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  -p 8787:8787 -p 8788:8788 -p 8789:8789/udp \
  -e META_AGENT_AUTH_TOKEN='replace-with-at-least-16-bytes' \
  -e META_AGENT_PROTECT_READ_API=true \
  meta-agent-control-plane:local \
  --http-addr 0.0.0.0:8787 \
  --tcp-addr 0.0.0.0:8788 \
  --udp-addr 0.0.0.0:8789
```

Terminate TLS at a trusted reverse proxy for remote HTTP/WebSocket deployments. TCP and UDP need a private network, VPN, or transport-level security wrapper for confidentiality. The small embedded client deliberately supports only local/plain `http://` and `ws://`; use a local TLS terminator or private tunnel rather than assuming it validates remote TLS.

## Repository layout

```text
src/model.rs                  versioned provider-neutral protocol
src/store.rs                  bounded projections and semantic reducer
src/explorer.rs               deterministic agents/sessions/timeline/lesson projection
src/explorer_api.rs           protected bounded operator explorer API and page route
src/explorer_ui.rs            Leptos explorer dashboard
src/metacognition/            deterministic explainable analysis engine
src/metacognition_api.rs      protected analysis API
src/metacognition_ui.rs       Leptos analysis dashboard
src/coordination.rs           deterministic fair-share coordination planner
src/coordination_api.rs       protected read-only coordination API and page route
src/coordination_ui.rs        Leptos coordination-plan dashboard
src/client.rs                 provider-neutral HTTP/WS/TCP/UDP Rust client
src/bin/meta-agent-sidecar.rs provider observation sidecar
src/provider.rs               OpenAI, Anthropic, and Gemini normalizers
src/http.rs                   Axum API, WebSockets, metrics, security headers
src/tcp.rs                    bounded NDJSON TCP listener
src/udp.rs                    telemetry-only UDP listener
src/ui.rs                     Leptos SSR dashboard
src/daemon.rs                 one-process lifecycle and listener binding
src/auth.rs                   constant-time shared-token policy
src/config.rs                 validated CLI/environment configuration
docs/                         architecture, protocol, OpenAPI, and sidecar guides
fixtures/                     canonical and deterministic provider fixtures
```

## Project tracking

The canonical implementation issue is `DEN-1057`. Repository publication and the recovered implementation were completed through reviewed GitHub history; bootstrap lifecycle and publication-infrastructure evidence is tracked in `DEN-1058` and `DEN-319`. Protocol, transport, bounded state, metacognition, UI, provider adapters, security, and merge gates are tracked in `DEN-1061` through `DEN-1069`.
