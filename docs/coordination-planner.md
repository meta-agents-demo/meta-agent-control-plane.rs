# Deterministic coordination planner

The coordination planner converts one bounded `Snapshot` into an explainable, dependency-safe next-action plan. It is deliberately a recommendation engine: it does not mutate the store, execute tools, reassign task ownership, or invent hidden reasoning.

## Inputs

The planner consumes only retained public state:

- agent status;
- goals and scoped task definitions;
- task status, progress, attempts, blockers, dependencies, and next actions;
- explicit reflections, confidence, evidence references, and artifacts;
- deterministic metacognition diagnostics and their retained source event IDs.

`build_plan` runs the existing metacognition engine against the same snapshot and revision. This prevents the planner from combining a fresh task graph with stale diagnostics.

## Outputs

A `CoordinationPlan` contains three separate classes of result:

- **assignments** — executable recommendations for the task's existing agent;
- **interventions** — graph or lifecycle inconsistencies requiring repair before execution;
- **held tasks** — dependency-blocked, offline, terminal, or capacity-suppressed work.

Every assignment and intervention includes a deterministic ID, priority, public rationale, recommended action, diagnostic IDs, and retained source event IDs when available.

Assignments are recommendations, not queue claims or distributed leases. A runtime that executes them must still apply its own authorization, concurrency, freshness, and idempotency controls. Rebuild the plan after accepted state changes rather than treating an old plan as an imperative command stream.

## Plan freshness

A plan is bound to the snapshot `revision` and `generated_at` values included in its JSON. Consumers must compare that revision with the current store before presenting an assignment as actionable. Any accepted event, operator correction, lease decision, or provider-status change invalidates the old plan for execution purposes and requires a rebuild. Stable assignment IDs support comparison and UI diffing; they are not authorization tokens or durable queue claims.

## Assignment precedence

For an otherwise eligible task, the highest-priority visible condition wins:

1. repeated attempts → `change_strategy`;
2. stale task → `request_checkpoint`;
3. blocked task → `resolve_blocker`;
4. low confidence or missing evidence → `gather_evidence`;
5. missing next action → `define_next_action`;
6. pending task → `start_task`;
7. running task → `continue_task`.

The planner never dispatches a task with unresolved or missing dependencies. Dependency cycles, unknown dependencies, unknown goals, and contradictory completion state become explicit interventions rather than assignments.

## Bounded fairness

`PlanningPolicy` bounds:

- total assignments;
- assignments per agent;
- retained interventions;
- retained held-task explanations.

Candidates are ranked deterministically within each agent, then selected in fair-share rounds across agents. A single agent cannot consume every slot merely because it owns many similarly ranked tasks. Candidates excluded by the configured limits are retained as `assignment_limit` holds up to the hold bound.

The summary distinguishes candidates suppressed by assignment capacity from intervention or hold records omitted by their independent retention bounds. This makes a truncated plan observable without converting omitted work into implied authorization. Increase the appropriate bound and rebuild from the same snapshot when an operator needs the additional explanations.

## Offline CLI

Export a protected snapshot and build a plan without connecting the CLI to the daemon:

```bash
curl --fail-with-body \
  -H 'authorization: Bearer replace-with-at-least-16-bytes' \
  http://127.0.0.1:8787/api/v1/snapshot \
  > snapshot.json

cargo run --bin meta-agent-plan -- \
  snapshot.json \
  --max-assignments 8 \
  --max-assignments-per-agent 2 \
  --pretty
```

The CLI also reads stdin when the path is omitted or `-`:

```bash
cat snapshot.json | cargo run --bin meta-agent-plan -- --pretty
```

The planner CLI does not accept an authentication token, provider credential, prompt, or hidden scratchpad. It reports parse and policy errors without echoing snapshot contents.

## Safety properties

- deterministic for the same snapshot and policies;
- no task is dispatched before its retained dependencies succeed;
- no cross-agent task ownership changes;
- offline agents receive no assignments;
- graph and terminal-state contradictions fail into interventions;
- global and per-agent assignment bounds are mandatory and non-zero;
- source event IDs are bounded and deduplicated;
- no hidden chain-of-thought is requested, reconstructed, or emitted.

## Test coverage

Unit and integration tests exercise:

- dependency holds;
- cycle intervention;
- fair-share assignment across agents;
- retry-loop priority over stall recovery;
- offline-agent holds;
- explicit blocker recovery;
- deterministic repeated planning;
- CLI stdin operation and bounded output;
- fail-closed zero-capacity policy errors without snapshot echo.
