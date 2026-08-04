# Testing and merge gates

The ordinary pull-request workflow runs formatting, Clippy with warnings denied,
all Rust targets under the committed lockfile, protocol/OpenAPI/dashboard drift
checks, JavaScript syntax, conflict-marker rejection, and the production OCI
build and smoke test.

`Deep network and state conformance` adds actual loopback transport, stateful,
bounded-memory, and authority-policy gates that are intentionally repeated on
pull requests, on changes merged to `main`, on manual dispatch, and twice
weekly.

## Local commands

```bash
make check
make deep-test
make image
```

`make deep-test` runs `tests/replay_pressure_udp.rs` and
`tests/network_transport_conformance.rs` serially so ordering, socket shutdown,
and bounded-channel assertions are deterministic and easy to inspect.

## Deep conformance guarantees

The deep suite currently proves:

1. **Deterministic reducer replay.** Two empty stores ingest the same fixed event
   log and produce byte-equivalent observable projections, cache statistics, and
   counters. Replaying the identical event IDs is idempotent: revision and
   derived state do not move, while duplicate accounting remains visible.
2. **Transport-independent normalization.** The same permitted telemetry event
   produces the same agent/task projection when admitted as HTTP, WebSocket,
   TCP, or UDP. Transport-specific counters and delivery metadata remain
   intentionally outside that comparison.
3. **Actual HTTP/WebSocket/TCP equivalence.** Separate real daemons receive the
   same fixed heartbeat and privileged task event through raw HTTP/1.1, a real
   WebSocket handshake/frame exchange, and newline-framed TCP. The normalized
   agent/goal/task/lesson projection must match exactly after each transport.
4. **Network authentication rejection without mutation.** Wrong bearer or frame
   credentials receive HTTP 401, TCP `unauthorized`, and WebSocket handshake 401
   responses. Revision remains zero, projections remain empty, and rejection
   counters identify all three transports.
5. **Bounded memory under sustained producers.** Sixty-four unique agent/task
   events are driven through much smaller LRU capacities. Length, pressure, and
   exact eviction counts are asserted for agent, task, recent-event, and
   idempotency caches.
6. **Bounded slow-consumer behavior.** A deliberately stalled broadcast receiver
   observes `Lagged` rather than blocking ingestion or creating an unbounded
   per-subscriber queue; retained updates remain consumable afterward.
7. **UDP authority boundary.** A real loopback UDP server rejects a valid but
   privileged `task_created` event before state mutation, returns the canonical
   `transport_policy` error, then accepts authenticated heartbeat telemetry and
   shuts down cleanly.

The scheduled workflow repeats both deep targets three times with one test
thread, then reruns formatting and Clippy. This catches order sensitivity,
accidental unbounded growth, flaky listener/handshake/shutdown behavior, and
policy drift without requiring provider credentials.

## Merge procedure

Before merging a test or product PR:

1. Inspect the complete patch and exact head SHA.
2. Confirm every review thread is resolved and no requested-change review
   remains.
3. Require ordinary CI, workflow-contract validation, secret scanning, OCI
   smoke, and the deep conformance workflow to pass on that exact head.
4. Resolve conflicts by preserving compatible protocol and state semantics,
   updating fixtures/contracts, and adding a regression test. Never select an
   entire side with blanket `ours` or `theirs`.
5. Merge with expected-head protection and record the PR, exact head, checks,
   merge method, and merge commit in DEN-1069 and its parent issue.

## Deferred gates

The following remain separate follow-up work and must not be inferred from the
current suite:

- sustained multi-process load and parser/framing fuzzing;
- browser automation for Leptos live reconnect and human controls;
- mock OpenAI, Anthropic, and Gemini adapter demonstrations;
- alternate-platform networking coverage;
- TLS/reverse-proxy and cross-origin deployment coverage;
- dependency advisory and license-policy enforcement.
