# Testing and merge gates

The ordinary pull-request workflow runs formatting, Clippy with warnings denied,
all Rust targets under the committed lockfile, protocol/OpenAPI/dashboard drift
checks, JavaScript syntax, conflict-marker rejection, and the production OCI
build and smoke test.

`Deep replay and pressure conformance` adds stateful and transport-policy gates
that are intentionally repeated on pull requests, on changes merged to `main`,
on manual dispatch, and twice weekly.

## Local commands

```bash
make check
make deep-test
make image
```

`make deep-test` runs `tests/replay_pressure_udp.rs` serially so ordering and
bounded-channel assertions are deterministic and easy to inspect.

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
3. **Bounded memory under sustained producers.** Sixty-four unique agent/task
   events are driven through much smaller LRU capacities. Length, pressure, and
   exact eviction counts are asserted for agent, task, recent-event, and
   idempotency caches.
4. **Bounded slow-consumer behavior.** A deliberately stalled broadcast receiver
   observes `Lagged` rather than blocking ingestion or creating an unbounded
   per-subscriber queue; retained updates remain consumable afterward.
5. **UDP authority boundary.** A real loopback UDP server rejects a valid but
   privileged `task_created` event before state mutation, returns the canonical
   `transport_policy` error, then accepts authenticated heartbeat telemetry and
   shuts down cleanly.

The scheduled workflow repeats the full deep target three times with one test
thread, then reruns formatting and Clippy. This catches order sensitivity,
accidental unbounded growth, flaky socket shutdown, and policy drift without
requiring provider credentials.

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

- real HTTP/WebSocket/TCP network-level cross-transport fixtures;
- sustained multi-process load and parser/framing fuzzing;
- browser automation for Leptos live reconnect and human controls;
- mock OpenAI, Anthropic, and Gemini adapter demonstrations;
- alternate-platform networking coverage;
- dependency advisory and license-policy enforcement.
