# Verified real-agent fleet

The fleet runs Codex and Claude Code against real Git repositories and publishes privacy-bounded task introspection to the Rust control plane. Production admission has no generated agents, seeded timelines, placeholder tasks, or synthetic completion records. An empty dashboard means no real work has been observed yet.

The system deliberately separates two views:

- `/` is the canonical work view: goals, tasks, progress, evidence-backed reflections, lessons, errors, and verified artifacts.
- `/runtime` is the process view: real provider process lifecycle, PID/RSS when observable, provider identity, and sanitized activity summaries.

A provider process exiting with code zero does **not** complete a task. The runner independently verifies the configured delivery contract: a clean worktree, a real commit when changes are required, a pushed branch whose remote head matches local `HEAD`, an open pull request whose independently read head matches the branch, enough agent-authored public progress events, a reflection, and optional test/CI evidence.

## Credential boundary

Provider and GitHub credentials are runtime Docker secrets. No credential value is copied into the Dockerfile, image layer, Compose model, repository, task ledger, hook, event, or provider transcript.

The provider runners are separate services:

- `agent-runner-openai` mounts the OpenAI secret, GitHub secret, and control-plane token. It does not mount the Anthropic secret and admits only `provider: openai` jobs.
- `agent-runner-anthropic` mounts the Anthropic secret, GitHub secret, and control-plane token. It does not mount the OpenAI secret and admits only `provider: anthropic` jobs.

Inside each admitted provider process, the runner exports only that provider's API-key variable. Provider stdout/stderr are transient mode-0600 files used for bounded error classification and deleted after each attempt unless an operator explicitly enables local retention.

Credentials pasted into chat, tickets, or command history should be treated as exposed. Revoke them after the bounded service-account window and replace them before production use.

## Live provider and capability confirmation

The `doctor` command reads the mounted secret and performs a real API request. It reports only sanitized status and counts; it never prints the key or full model inventory.

```bash
docker compose -f compose.yaml -f compose.agents.yaml run --rm \
  agent-runner-openai doctor --provider openai

docker compose -f compose.yaml -f compose.agents.yaml run --rm \
  agent-runner-anthropic doctor --provider anthropic
```

OpenAI preflight uses `GET /v1/models`. Anthropic preflight uses `GET /v1/models`, records declared model-capability labels, and optionally probes the Managed Agents beta `GET /v1/agents` endpoint with `anthropic-beta: managed-agents-2026-04-01`. The system does not build new work on OpenAI's deprecated Assistants API.

MCP discovery is capability-negotiated and version-aware. The runner first uses the stateless `2026-07-28` protocol: `server/discover` carries the standard per-request `_meta`, every HTTP request includes `Mcp-Method`, and list methods are called only for capabilities the server declared. When `server/discover` is unavailable or reports only a 2025-era version, the client falls back to the `2025-11-25` `initialize`/`notifications/initialized` handshake and honors any returned session ID. The discovery path does not call deprecated `logging/setLevel` or `completion/complete`, does not invoke tools, and does not claim to inspect the private tool set of a ChatGPT or Claude chat session.

The default Compose model probes the official read-only OpenAI documentation MCP endpoint. Additional HTTPS MCP endpoints require explicit host allowlisting through `META_AGENT_MCP_ALLOWED_HOSTS`.

## Privacy-bounded introspection

The canonical protocol carries observable work products, not hidden model reasoning. Agents use `meta-agent-observe` to publish concise progress and reflection:

```bash
meta-agent-observe progress \
  --progress 0.45 \
  --summary "Verified the current branch and reproduced the failing delivery check." \
  --next-action "Implement the semantic fix and rerun the focused test suite."

meta-agent-observe reflection \
  --confidence 0.92 \
  --summary "The delivery gate now rejects a successful provider exit when remote branch or PR evidence is absent." \
  --evidence 'test=python3 -m unittest::All focused contract tests passed' \
  --evidence 'commit=https://github.com/meta-agents-demo/meta-agent-control-plane.rs/commit/COMMIT_SHA' \
  --alternative "Trust the provider exit code without independent repository checks." \
  --risk "A temporary GitHub outage can defer final verification." \
  --next-action "Publish the pull request and verify its head SHA."
```

The observer rejects credential-shaped values, authorization headers, private-reasoning tags/fields, control characters, and oversized payloads. Accepted agent-authored observations are also appended to a mode-0600 public ledger so the supervisor can verify that the reflection shown in the UI came from the admitted run rather than from a runner-generated completion claim.

Do not submit prompts, raw provider responses, chain-of-thought, private scratchpads, cookies, sensitive tool arguments/results, or credentials. Report the conclusion, supporting evidence reference, confidence, alternatives, risks, blocker, and next action.

## Durable state and shutdown

Each provider has a separate state volume containing queue files, sanitized run state, public observation ledgers, repository workspaces, and an isolated mode-0700 CLI home. `SIGTERM` stops admission, interrupts provider process groups, records in-flight jobs as paused, and preserves the branch/worktree. On restart, interrupted states reconcile to paused and resume by inspecting existing work rather than deleting or rewriting it.

This preserves repository work; it does not promise that a provider's conversational session survives container replacement.

`META_AGENT_MAX_CONCURRENCY` is hard-clamped to 15 per runner. Quota and rate-limit failures open a bounded provider-specific circuit, so one provider can continue while the other is temporarily unavailable.

## Secret setup

Create local files outside every repository and make them owner-readable only:

```bash
install -d -m 700 "$HOME/.config/meta-agent/secrets"
printf '%s' "$OPENAI_API_KEY" > "$HOME/.config/meta-agent/secrets/openai-api-key"
printf '%s' "$ANTHROPIC_API_KEY" > "$HOME/.config/meta-agent/secrets/anthropic-api-key"
printf '%s' "$GH_TOKEN" > "$HOME/.config/meta-agent/secrets/github-token"
openssl rand -hex 32 > "$HOME/.config/meta-agent/secrets/control-plane-token"
chmod 600 "$HOME/.config/meta-agent/secrets/"*
```

Copy `.env.agent-runner.example` to an untracked operator environment or export its path variables directly. Do not put secret values into `.env` files. `META_AGENT_CREDENTIAL_EXPIRES_AT` is enforced before API discovery and every provider launch.

## Start

The control plane receives the shared ingestion token as an environment variable; provider runners receive the same value as a mounted file:

```bash
export META_AGENT_AUTH_TOKEN="$(cat "$META_AGENT_AUTH_TOKEN_FILE")"
docker compose -f compose.yaml -f compose.agents.yaml up --build
```

The image pins Codex CLI and Claude Code versions through reviewed build arguments rather than using `latest`. Codex runs in its workspace-write sandbox with network access for dependencies and GitHub delivery. Claude uses `acceptEdits` rather than bypassing all permission checks.

Run both live doctors before admitting jobs, then open the control-plane dashboard. A provider with a missing, expired, invalid, unauthorized, or rate-limited credential is not admitted to repository execution.

## Queue a real task

A job contains a private execution instruction plus a separate bounded public title and success criteria for the UI:

```json
{
  "job_id": "meta-real-introspection",
  "provider": "openai",
  "repository": "https://github.com/meta-agents-demo/meta-agent-control-plane.rs.git",
  "base_ref": "main",
  "branch": "agent/meta-real-introspection",
  "priority": 10,
  "timeout_seconds": 7200,
  "max_attempts": 3,
  "public_title": "Publish verified real-agent introspection",
  "success_criteria": [
    "Emit canonical progress, reflection, and evidence from the admitted repository run.",
    "Publish a clean branch and open a pull request whose head matches the pushed commit.",
    "Pass the focused Python and Compose contract checks."
  ],
  "constraints": [
    "Do not expose provider prompts, transcripts, private reasoning, or credentials."
  ],
  "require_pull_request": true,
  "require_observation": true,
  "require_test_evidence": true,
  "allow_no_change": false,
  "task": "Audit the current provider fleet, implement privacy-bounded canonical events and independent delivery verification, run the focused tests, push the assigned branch, and open or update the pull request."
}
```

Validate and enqueue it through the service matching its provider. The read-only job mount is supplied only for the one-shot command:

```bash
docker compose -f compose.yaml -f compose.agents.yaml run --rm \
  -v "$PWD/jobs:/jobs:ro" \
  agent-runner-openai validate-job /jobs/meta-real-introspection.json

docker compose -f compose.yaml -f compose.agents.yaml run --rm \
  -v "$PWD/jobs:/jobs:ro" \
  agent-runner-openai enqueue /jobs/meta-real-introspection.json
```

The queue retains the private task only until archival. The sanitized run state never copies the task, prompt, provider output, or secret values.

## Inspect status

```bash
docker compose -f compose.yaml -f compose.agents.yaml run --rm agent-runner-openai status
docker compose -f compose.yaml -f compose.agents.yaml run --rm agent-runner-anthropic status
```

The status output contains delivery evidence and counts, not provider transcripts. A run can be `succeeded` only after the independently verified contract passes. A provider-authentication failure is terminal; a recoverable provider/network failure pauses or requeues the preserved work; an incomplete delivery contract is retried and eventually recorded as partial rather than falsely reported as successful.
