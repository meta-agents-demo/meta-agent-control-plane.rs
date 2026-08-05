# Real agent runtime observability

The `/runtime` surface contains no generated demo rows. Every displayed value comes from an observed process counter or an accepted runtime hook.

The server merges two explicit sources:

1. **Host process counters**: PID, process state, delta CPU percentage, RSS bytes, and host-memory percentage read from Linux `/proc`.
2. **Runtime hooks**: agent identity, provider/model, session, visible activity summary, active tool, reported confidence, token deltas, and optional host-side CPU/RSS/memory samples posted by an agent wrapper or native provider hook.

Linux `/proc` samples take precedence when both sources report resources for the same PID. On macOS and Windows, the native host adapter samples the real agent process with operating-system tools and posts those resource fields into the containerized server.

Process inspection cannot recover semantic activity or model confidence. Those fields remain `null`/`unreported` until a hook supplies them. Confidence is never inferred from CPU, duration, token counts, or process behavior. The server rejects metadata keys associated with credentials, cookies, prompts, raw responses, scratchpads, hidden reasoning, chain-of-thought, or token secrets.

## Container startup

Create a local environment file with a synthetic control-plane token:

```bash
cp .env.example .env
# edit META_AGENT_AUTH_TOKEN; do not add personal browser/account credentials
```

Cross-platform hook mode:

```bash
docker compose up --build
```

Linux host-process mode:

```bash
docker compose -f compose.yaml -f compose.linux.yaml up --build
```

Open `http://127.0.0.1:8787/runtime`, enter the same synthetic token, and apply it. The Linux overlay mounts `/proc` read-only and uses the host PID namespace. It does not mount the Docker socket, add capabilities, or run the container as root.

Docker Desktop on macOS and Windows does not expose native host processes through Linux `/proc`. Keep the server containerized and run the native hook adapter on the host so CPU, RSS, lifecycle, and tool-state data cross the boundary explicitly.

## Runtime hook contract

POST to `/api/v1/runtime/hooks` with the control-plane bearer token:

```json
{
  "protocol_version": "v1",
  "event_id": "9d9ca303-27fe-43f4-8bc2-7a3ff47a6798",
  "occurred_at": "2026-08-05T14:00:00Z",
  "agent": {
    "agent_id": "claude-test-01",
    "provider": "anthropic",
    "model": "test-model",
    "instance_id": "ephemeral-session"
  },
  "session_id": "session-01",
  "pid": 4242,
  "kind": "model_response",
  "control_capable": false,
  "summary": "Completed a visible repository inspection step",
  "confidence": 0.78,
  "cpu_percent": 17.4,
  "rss_bytes": 134217728,
  "memory_percent": 2.1,
  "input_tokens_delta": 640,
  "output_tokens_delta": 181,
  "metadata": {
    "account_class": "ephemeral-test"
  }
}
```

Supported hook kinds are `session_started`, `heartbeat`, `activity`, `tool_started`, `tool_finished`, `model_response`, `confidence_reported`, `error_observed`, and `session_finished`.

The generic helper at `examples/hooks/runtime-hook.py` emits hooks and can poll or acknowledge cooperative control commands. Set `--control-capable` only in a long-running wrapper that actually polls the command endpoint, applies the command in its own lifecycle, and acknowledges it. One-shot native lifecycle hooks are observe-only.

## Native Claude Code hooks

Copy the relevant entries from `examples/hooks/claude-settings.json` into a disposable test account's Claude settings and replace the placeholder path with an absolute path to this repository. Export only the control-plane settings in the shell that launches Claude:

```bash
export META_AGENT_RUNTIME_URL=http://127.0.0.1:8787
export META_AGENT_AUTH_TOKEN='synthetic-control-plane-token'
export META_AGENT_ID='claude-ephemeral-01'
export META_AGENT_MODEL='test-claude-model'
claude
```

Claude sends native hook JSON on stdin. `agent_hook_adapter.py claude` maps session, tool, stop, and failure events into fixed safe summaries. It uses command-line data only to find the real Claude process in its ancestry; it does not forward the command line.

## Native Gemini CLI hooks

Copy the relevant entries from `examples/hooks/gemini-settings.json` into the test workspace's `.gemini/settings.json`, replace the placeholder path, and launch Gemini from a shell containing only disposable credentials and the control-plane variables:

```bash
export META_AGENT_RUNTIME_URL=http://127.0.0.1:8787
export META_AGENT_AUTH_TOKEN='synthetic-control-plane-token'
export META_AGENT_ID='gemini-ephemeral-01'
export META_AGENT_MODEL='test-gemini-model'
gemini
```

The adapter maps session, agent-loop, model, and tool lifecycle events. It deliberately ignores request/response bodies, prompt fields, tool input, and tool output.

## OpenAI/Codex agents

For Codex app-server clients, place the transparent proxy where the client would normally launch `codex app-server`:

```bash
export META_AGENT_RUNTIME_URL=http://127.0.0.1:8787
export META_AGENT_AUTH_TOKEN='synthetic-control-plane-token'
export META_AGENT_ID='codex-ephemeral-01'
export META_AGENT_MODEL='test-codex-model'
python3 examples/hooks/codex_app_server_proxy.py -- codex app-server
```

The proxy forwards stdin, stdout, and stderr unchanged. In a bounded background queue it observes thread, turn, work-item, and token-usage notifications. Agent messages, user messages, reasoning items, command strings, arguments, and content fields are not emitted. Cumulative token notifications are converted into deltas before ingestion.

Other OpenAI/ChatGPT SDK agents can use `runtime-hook.py` or the same JSON contract from their wrapper. The server does not scrape a personal ChatGPT browser session or browser profile.

## Host resource sampling

`agent_hook_adapter.py` locates the real provider process by walking its process ancestry. Override discovery with `META_AGENT_TARGET_PID` only when the wrapper knows the correct PID.

- Linux and macOS use `ps` to sample CPU percentage, RSS, and memory percentage.
- Windows uses PowerShell/CIM and `Get-Process`.
- Linux container collection uses `/proc` and overrides matching hook resource fields in the merged snapshot.
- A failed resource sample leaves values unreported; it never generates fallback numbers.

## Controls

The dashboard queues `pause`, `resume`, and `stop` only for agents that have explicitly reported `control_capable: true`. The agent wrapper polls `/api/v1/runtime/commands/poll`, applies the command within its own lifecycle, and acknowledges it at `/api/v1/runtime/commands/ack` with both the command ID and its agent ID. A mismatched agent cannot acknowledge another agent's command.

Native Claude, Gemini, and Codex lifecycle adapters are observe-only because a one-shot hook cannot guarantee that it can pause or stop the parent agent. Process-only discoveries are also observe-only. This design deliberately avoids granting the container permission to send arbitrary signals to unrelated host processes.

The current integration token is a shared trust boundary for registered test agents. Distribute it only to the disposable runtimes that belong in this control plane.

## Privacy boundary

The native adapters retain only:

- provider/model/session identifiers;
- lifecycle event names and fixed summaries;
- safe tool names or Codex item types;
- PID and operating-system resource counters;
- provider-reported token totals/deltas;
- explicitly reported confidence.

They do not forward prompts, model responses, tool arguments, tool results, command contents, browser data, cookies, provider credentials, scratchpads, or hidden reasoning. Regression tests place sentinel secrets in those fields and assert that none appear in emitted envelopes.

## Test-account boundary

Run Gemini, OpenAI/ChatGPT/Codex, and Claude agents with disposable organization/project credentials or dedicated test accounts. Provide provider keys to the agent process through a secret manager or ephemeral shell environment; never bake them into the image, Compose files, repository, hook payloads, or control-plane metadata. Do not mount a personal browser profile or use `alexander.d.mills@gmail.com` for runtime tests.
