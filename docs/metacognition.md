# Explainable metacognition projection

The control plane derives a deterministic, read-only metacognition projection from the bounded in-memory snapshot. It never asks for, reconstructs, or stores hidden chain-of-thought. Every diagnostic is based on operator-visible protocol fields and, when retained, links to the exact source event IDs that caused the projection.

## What is derived

The engine keeps self-reported progress separate from evidence-backed progress and computes:

- active, blocked, stalled, and terminal task counts;
- retry-loop detection from repeated attempts;
- unresolved, missing, and cyclic dependencies;
- critical-path remaining depth for acyclic goal task graphs;
- evidence coverage from visible reflection references and completion artifacts;
- low-confidence, missing-evidence, missing-next-action, orphan-goal, and contradictory-completion diagnostics;
- goal-level progress, evidence coverage, critical-path tasks, and data-quality warnings.

Each diagnostic contains a stable rule ID, severity, concise explanation, recommended next action, scoped agent/goal/task identifiers, and a bounded list of retained source event IDs. When the relevant timeline events have already been evicted, the response says that source events are not retained rather than pretending the explanation is complete.

## Determinism and replay

`metacognition::analyze_with_policy` is a pure projection over `store::Snapshot`. Given the same snapshot and policy it returns byte-equivalent serialized values with deterministic ordering. It does not mutate store state, counters, LRU recency, or transport behavior.

The default policy flags:

- active tasks with no update for 15 minutes;
- tasks on their third or later attempt that have not succeeded or been canceled;
- declared confidence below 45 percent.

Callers can supply a different policy for tests or deployment-specific alerting without changing event semantics.

## Progress semantics

`self_reported_progress` is the agent's declared task percentage. `evidence_backed_progress` contributes the same percentage only when a visible evidence reference or completion artifact is retained. Goal and global summaries report both values plus evidence coverage, making uncertainty visible instead of collapsing it into one unqualified number.

## Dependency and critical-path rules

Task identifiers are scoped by agent. A dependency is complete only when its scoped task has succeeded. Missing dependencies and cycles are critical diagnostics. Critical-path depth is available only for acyclic goal graphs; when a cycle exists, the goal projection returns no critical-path value and includes a data-quality warning.

## Privacy boundary

The engine consumes only normalized, allowlisted event fields already present in the store. It does not inspect raw provider payloads, authentication material, prompts, private scratchpads, or hidden reasoning. Provider adapters continue to reject hidden-reasoning field names before normalization.

## Validation

Focused tests certify deterministic replay, source-event linkage, retry and stall rules, dependency-cycle detection, orphan-goal detection, and the separation between self-reported and evidence-backed progress.
