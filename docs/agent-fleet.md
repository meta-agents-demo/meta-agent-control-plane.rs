# Durable provider-agent fleet

The agent fleet runs Codex and Claude Code inside a separate hardened container while the Rust control plane remains credential-free. It is intended for temporary provider projects or service accounts, isolated repository branches, and bounded audit/test jobs.

## Trust boundary

Provider and GitHub credentials are Docker secrets mounted only into `agent-runner`. The runner reads each secret into only the selected child process environment and never writes it to the queue, ledger, hook payload, image, Compose file, or repository. Provider stdout/stderr are transient mode-0600 files used only for bounded error classification; they are deleted after each attempt by default, while abrupt host loss can leave them inside the protected state volume for operator cleanup. The control plane receives fixed lifecycle summaries, identity, PID/RSS when available, and status only. It does not receive prompts, model responses, command contents, tool arguments/results, cookies, or provider credentials.

Because credentials pasted into a chat or ticket are no longer pristine secrets, revoke them after the bounded test window and replace them before any production use.

## Durable state and shutdown

`agent-runner-state` retains queue files, one sanitized state document per run, repository workspaces, and a mode-0700 provider/GitHub CLI home. The isolated home is writable even though the image root is read-only, and it prevents reuse of personal CLI profiles. `SIGTERM` stops admission, interrupts child process groups, records each in-flight job as `paused`, and leaves its worktree and branch intact. On restart, interrupted `running` states are reconciled to `paused`; with `META_AGENT_RESUME_PAUSED=true`, work resumes by inspecting the existing branch rather than deleting or resetting it.

This is work resumption, not a promise that a provider's conversational session survives container replacement.

## Concurrency and provider circuit breaking

`META_AGENT_MAX_CONCURRENCY` is hard-clamped to 15. Quota and rate-limit failures pause affected jobs and open a provider circuit for a bounded retry interval. This lets OpenAI work continue when Claude is out of credits without repeatedly burning failed Claude attempts. When the Claude circuit interval expires, the next queued Claude job is the bounded availability probe; a quota failure reopens the circuit.

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

Copy `.env.agent-runner.example` to an untracked shell environment or pass those path variables directly. Do not put secret values into `.env` files. `META_AGENT_CREDENTIAL_EXPIRES_AT` is enforced before every provider launch, so the current temporary credentials cannot start new work after the declared test window.

## Start

```bash
export META_AGENT_AUTH_TOKEN="$(cat "$META_AGENT_AUTH_TOKEN_FILE")"
docker compose -f compose.yaml -f compose.agents.yaml up --build
```

The provider image pins Codex CLI `0.146.1` and Claude Code `2.1.220` by default; update those pins through review rather than using `latest`. Codex runs in its workspace-write sandbox with network enabled so it can fetch dependencies and use `gh`; Claude uses `acceptEdits` rather than bypassing all permission checks.

## Queue a job

```json
{
  "job_id": "hhm-e2e-contracts",
  "provider": "openai",
  "repository": "https://github.com/hacker-house-medellin/hhm-e2e.git",
  "base_ref": "main",
  "branch": "agent/hhm-e2e-contracts",
  "priority": 10,
  "timeout_seconds": 7200,
  "max_attempts": 3,
  "task": "Audit the E2E harness, add privacy-safe contract and browser tests, run them, push the branch, and open or update a draft PR linked to the organization project and Linear work."
}
```

Validate and enqueue without placing the task in the sanitized run ledger:

```bash
docker compose -f compose.yaml -f compose.agents.yaml run --rm agent-runner \
  validate-job /jobs/hhm-e2e-contracts.json

docker compose -f compose.yaml -f compose.agents.yaml run --rm agent-runner \
  enqueue /jobs/hhm-e2e-contracts.json
```

Mount a read-only jobs directory for these one-shot commands, or copy the validated JSON into the `queue` directory of the named state volume through an approved operator workflow.

## Monitoring

Open `/runtime` in the control plane. Fleet hooks are observe-only: the runner does not claim cooperative pause/resume control through the runtime command API. Container shutdown is the durable pause boundary.

Inspect the local sanitized ledger with:

```bash
docker compose -f compose.yaml -f compose.agents.yaml run --rm agent-runner status
```
