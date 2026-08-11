# Verified real-agent fleet

The fleet runs Codex and Claude Code against real Git repositories and publishes privacy-bounded task telemetry to the Rust control plane. Production admission has no generated agents, seeded timelines, placeholder tasks, or synthetic completion records. An empty dashboard means no real work has been observed yet.

The system separates two operator views:

- `/` is the canonical work view: goals, tasks, progress, evidence-backed reflections, lessons, errors, and verified artifacts.
- `/runtime` is the process view: provider process lifecycle, PID/RSS when observable, provider identity, and sanitized activity summaries.

A provider process exiting with code zero does not complete a task. The runner independently verifies the configured delivery contract: a clean worktree, a real commit when changes are required, a pushed branch whose remote head matches local `HEAD`, an open pull request whose independently read head matches the branch, enough agent-authored public progress events, a reflection, and optional test or CI evidence.

## Container topology

The production topology is composed from two files in this order:

```sh
-f compose.agents.yaml -f compose.production.yaml
```

`compose.agents.yaml` declares the OpenAI runner, Anthropic runner, real-work dispatcher, isolated state volumes, and mounted secret names. `compose.production.yaml` is the final hardening overlay: it adds the authenticated control plane, read-only roots, dropped capabilities, `no-new-privileges`, resource and log limits, health-gated dependencies, supplementary secret-file group access, and the explicit mutation profile.

The provider runners are separate services:

- `agent-runner-openai` mounts only the OpenAI, GitHub, and control-plane credentials and admits only `provider: openai` jobs.
- `agent-runner-anthropic` mounts only the Anthropic, GitHub, and control-plane credentials and admits only `provider: anthropic` jobs.
- `task-dispatcher` mounts only GitHub and Linear credentials. It has no provider API key and is disabled unless the `production-mutation` profile is explicitly selected.

The control plane and provider runners run as non-root users. The root filesystem is read-only; writable state is limited to bounded tmpfs mounts and named state volumes. The authenticated HTTP/UI listener is published on loopback by default. TCP and UDP ingestion remain on the private Compose network.

## Credential boundary

Provider, GitHub, Linear, and control-plane credentials are runtime Docker secrets. No credential value is copied into a Dockerfile, image layer, Compose model, repository, task ledger, hook, event, or provider transcript.

Production values are maintained through the pinned `ores-sops` tool in the Nix development shell:

```sh
nix develop --no-write-lock-file
just env-init             # one-time local policy bootstrap
just env-edit prod        # edit ciphertext through SOPS
just env-verify           # keyless policy verification
just production-preflight prod
```

Use `config/runtime-env.example` only as the key inventory. The only committable ciphertext paths are:

```text
env/enc/dev.env.enc
env/enc/prod.env.enc
```

Decrypted dotenv files, private age identities, generated runtime secrets, `.env` files, and Compose env output are never committed and are excluded from the Docker build context.

The materializer validates the selected decrypted profile and writes:

- five mode-0640 credential files that share the deployment account's effective GID;
- one mode-0600 `compose.env` containing only absolute secret-file paths, that numeric GID, and validated non-secret tuning.

The production overlay adds the generated GID only to services that need those mounted files. Run deployment commands as a dedicated non-root deployment account or under a private deployment group. The control-plane entrypoint reads the mounted token after rejecting symlinks, non-regular files, oversized values, control characters, and weak tokens, then exports it only into the live daemon process immediately before `exec`.

Credentials pasted into chat, tickets, pull requests, CI output, or command history should be treated as exposed. Revoke them, rotate the SOPS data key when recipient trust changes, and replace affected service credentials before production use.

## Live provider and capability confirmation

The `doctor` command reads the mounted provider secret and performs a real API request. It reports only sanitized status and counts; it never prints a key or full model inventory.

```sh
just production-doctor prod
```

OpenAI preflight uses the provider model inventory. Anthropic preflight uses its model inventory, records declared model-capability labels, and can probe the managed-agent capability when available. The system does not build new work on deprecated provider APIs.

MCP discovery is capability-negotiated and version-aware. The runner prefers the current stateless protocol and falls back only to the reviewed legacy handshake when necessary. Discovery does not invoke tools and does not claim to inspect the private tool set of a ChatGPT or Claude chat session. Additional HTTPS MCP endpoints require explicit host allowlisting.

A missing, expired, invalid, unauthorized, quota-exhausted, or rate-limited provider is not admitted to repository execution. Provider-specific circuit breakers allow one provider to remain available while the other is paused.

## Safe startup and explicit mutation admission

Routine production startup reruns preflight and both live provider doctors, then starts only the control plane and provider runners:

```sh
just production-up prod
just production-status prod
```

This does not start the real-work dispatcher. After public exact-head CI, the paired `meta-agents-demo-test` gate, SOPS recipient review, and both live doctors are green, an operator may explicitly admit real GitHub/Linear mutation:

```sh
just production-admit prod ENABLE_REAL_PRODUCTION_MUTATION
```

The literal acknowledgment is intentional. The dispatcher scans the configured allowlisted organizations and Linear projects, resolves a real target repository, and enqueues repository-changing work. Do not use admission merely as a container smoke test.

Shutdown includes the mutation profile and removes generated plaintext:

```sh
just production-down prod
just env-lock
```

## Privacy-bounded introspection

The canonical protocol carries observable work products, not hidden model reasoning. Agents use `meta-agent-observe` to publish concise progress and reflection:

```sh
meta-agent-observe progress \
  --progress 0.45 \
  --summary "Verified the current branch and reproduced the failing delivery check." \
  --next-action "Implement the semantic fix and rerun the focused test suite."

meta-agent-observe reflection \
  --confidence 0.92 \
  --summary "The delivery gate now rejects a successful provider exit when remote branch or PR evidence is absent." \
  --evidence 'test=focused contract tests passed' \
  --evidence 'commit=verified candidate commit SHA' \
  --alternative "Trust the provider exit code without independent repository checks." \
  --risk "A temporary GitHub outage can defer final verification." \
  --next-action "Publish the pull request and verify its head SHA."
```

The observer rejects credential-shaped values, authorization headers, private-reasoning tags or fields, control characters, and oversized payloads. Accepted agent-authored observations are appended to a mode-0600 public ledger so the supervisor can verify that the reflection shown in the UI came from the admitted run rather than from a runner-generated completion claim.

Do not submit prompts, raw provider responses, chain-of-thought, private scratchpads, cookies, sensitive tool arguments or results, or credentials. Report conclusions, bounded evidence references, confidence, alternatives, risks, blockers, and next actions.

## Durable state and shutdown behavior

Each provider has a separate state volume containing queue files, sanitized run state, public observation ledgers, repository workspaces, and an isolated mode-0700 CLI home. `SIGTERM` stops admission, interrupts provider process groups, records in-flight jobs as paused, and preserves the branch and worktree. On restart, interrupted states reconcile to paused and resume by inspecting existing work rather than deleting or rewriting it.

This preserves repository work; it does not promise that a provider's conversational session survives container replacement. `META_AGENT_MAX_CONCURRENCY` is hard-clamped to 15 per runner.

## Manually validate and enqueue a bounded job

A job contains a private execution instruction plus a separate public title, success criteria, and constraints for the UI:

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
    "Pass focused repository validation."
  ],
  "constraints": [
    "Do not expose provider prompts, transcripts, private reasoning, or credentials."
  ],
  "require_pull_request": true,
  "require_observation": true,
  "require_test_evidence": true,
  "allow_no_change": false,
  "task": "Audit the current provider fleet, implement the requested change, run focused tests, push the assigned branch, and open or update the pull request."
}
```

Validate and enqueue it through the service matching its provider. Supply the job directory only to the one-shot command:

```sh
docker compose \
  --env-file env/dec/runtime-secrets/prod/compose.env \
  -f compose.agents.yaml -f compose.production.yaml \
  run --rm -v "$PWD/jobs:/jobs:ro" \
  agent-runner-openai validate-job /jobs/meta-real-introspection.json

docker compose \
  --env-file env/dec/runtime-secrets/prod/compose.env \
  -f compose.agents.yaml -f compose.production.yaml \
  run --rm -v "$PWD/jobs:/jobs:ro" \
  agent-runner-openai enqueue /jobs/meta-real-introspection.json
```

The queue retains the private task only until archival. Sanitized run state never copies the task, prompt, provider output, or secret values.

## Inspect status

```sh
docker compose \
  --env-file env/dec/runtime-secrets/prod/compose.env \
  -f compose.agents.yaml -f compose.production.yaml \
  run --rm agent-runner-openai status

docker compose \
  --env-file env/dec/runtime-secrets/prod/compose.env \
  -f compose.agents.yaml -f compose.production.yaml \
  run --rm agent-runner-anthropic status
```

Status output contains delivery evidence and counts, not provider transcripts. A run can be `succeeded` only after the independently verified contract passes. A provider-authentication failure is terminal; a recoverable provider or network failure pauses or requeues preserved work; an incomplete delivery contract is retried and eventually recorded as partial rather than falsely reported as successful.
