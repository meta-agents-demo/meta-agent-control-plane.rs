# Real agent runtime observability

The `/runtime` surface contains no generated demo rows. It merges two explicit sources:

1. **Host process counters**: PID, process state, delta CPU percentage, RSS bytes, and host-memory percentage read from Linux `/proc`.
2. **Runtime hooks**: agent identity, provider/model, session, visible activity summary, active tool, model-reported confidence, and token deltas posted by an agent wrapper or provider integration.

Process inspection cannot recover semantic activity or model confidence. Those fields remain `null`/`unreported` until a hook supplies them. The server rejects metadata keys associated with prompts, raw responses, scratchpads, hidden reasoning, or chain-of-thought.

## Container startup

Create a local environment file with a synthetic control-plane token:

```bash
cp .env.example .env
# edit META_AGENT_AUTH_TOKEN; do not add personal browser/account credentials
```

Cross-platform hook-only mode:

```bash
docker compose up --build
```

Linux host-process mode:

```bash
docker compose -f compose.yaml -f compose.linux.yaml up --build
```

Open `http://127.0.0.1:8787/runtime`, enter the same synthetic token, and apply it. The Linux overlay mounts `/proc` read-only and uses the host PID namespace. It does not mount the Docker socket, add capabilities, or run the container as root.

Docker Desktop on macOS and Windows does not expose native host processes through Linux `/proc`. Keep the server containerized and use runtime hooks from host-side agent wrappers on those platforms.

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
  "summary": "Completed a visible repository inspection step",
  "confidence": 0.78,
  "input_tokens_delta": 640,
  "output_tokens_delta": 181,
  "metadata": {
    "account_class": "ephemeral-test"
  }
}
```

Supported hook kinds are:

- `session_started`
- `heartbeat`
- `activity`
- `tool_started`
- `tool_finished`
- `model_response`
- `confidence_reported`
- `error_observed`
- `session_finished`

The example helper at `examples/hooks/runtime-hook.py` emits hooks and polls/acknowledges cooperative control commands. Use `pid=os.getppid()` from child hooks when the parent is the agent process so the server can merge semantic data with the matching process sample.

## Controls

The dashboard queues `pause`, `resume`, and `stop` commands only for hook-aware agents. An agent wrapper polls `/api/v1/runtime/commands/poll`, applies the command within its own lifecycle, and acknowledges it at `/api/v1/runtime/commands/ack` with both the command ID and its agent ID. A mismatched agent cannot acknowledge another agent's command.

This design deliberately avoids granting the container permission to send arbitrary signals to unrelated host processes. A process discovered only through `/proc` is observe-only. The current integration token is a shared trust boundary for hook-aware test agents, so distribute it only to the disposable test runtimes that belong in this control plane.

## Test-account boundary

Run Gemini, OpenAI/ChatGPT/Codex, and Claude agents with disposable organization/project credentials or dedicated test accounts. Provide provider keys to the agent process through a secret manager or ephemeral shell environment; never bake them into the image, compose files, repository, hook payloads, or control-plane metadata. Do not mount a personal browser profile or use `alexander.d.mills@gmail.com` for runtime tests.
