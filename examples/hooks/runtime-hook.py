#!/usr/bin/env python3
"""Emit safe runtime metadata and consume cooperative control commands."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone


def endpoint(path: str) -> str:
    base = os.environ.get("META_AGENT_RUNTIME_URL", "http://127.0.0.1:8787").rstrip("/")
    return f"{base}{path}"


def token() -> str:
    value = os.environ.get("META_AGENT_AUTH_TOKEN", "")
    if len(value) < 16:
        raise SystemExit("META_AGENT_AUTH_TOKEN must contain at least 16 bytes")
    return value


def post(path: str, payload: dict[str, object]) -> object:
    request = urllib.request.Request(
        endpoint(path),
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": f"Bearer {token()}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        detail = error.read().decode(errors="replace")
        raise SystemExit(f"runtime API returned HTTP {error.code}: {detail}") from error


def emit(args: argparse.Namespace) -> None:
    payload: dict[str, object] = {
        "protocol_version": "v1",
        "event_id": str(uuid.uuid4()),
        "occurred_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "agent": {
            "agent_id": args.agent_id,
            "provider": args.provider,
            "model": args.model,
            "instance_id": args.instance_id,
        },
        "session_id": args.session_id,
        "pid": args.pid,
        "kind": args.kind,
        "summary": args.summary,
        "tool_name": args.tool_name,
        "confidence": args.confidence,
        "input_tokens_delta": args.input_tokens,
        "output_tokens_delta": args.output_tokens,
        "metadata": {"account_class": "ephemeral-test", "source": "runtime-hook.py"},
    }
    payload = {key: value for key, value in payload.items() if value is not None}
    agent = payload["agent"]
    assert isinstance(agent, dict)
    payload["agent"] = {key: value for key, value in agent.items() if value is not None}
    print(json.dumps(post("/api/v1/runtime/hooks", payload), indent=2))


def poll(args: argparse.Namespace) -> None:
    commands = post("/api/v1/runtime/commands/poll", {"agent_id": args.agent_id})
    print(json.dumps(commands, indent=2))


def acknowledge(args: argparse.Namespace) -> None:
    payload = {
        "command_id": args.command_id,
        "agent_id": args.agent_id,
        "accepted": args.accepted,
        "message": args.message,
    }
    print(json.dumps(post("/api/v1/runtime/commands/ack", payload), indent=2))


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    subcommands = root.add_subparsers(dest="command", required=True)

    emit_parser = subcommands.add_parser("emit")
    emit_parser.add_argument("--agent-id", required=True)
    emit_parser.add_argument("--provider", choices=["openai", "anthropic", "google"], required=True)
    emit_parser.add_argument("--model", required=True)
    emit_parser.add_argument(
        "--kind",
        choices=[
            "session_started",
            "heartbeat",
            "activity",
            "tool_started",
            "tool_finished",
            "model_response",
            "confidence_reported",
            "error_observed",
            "session_finished",
        ],
        required=True,
    )
    emit_parser.add_argument("--instance-id")
    emit_parser.add_argument("--session-id")
    emit_parser.add_argument("--pid", type=int, default=os.getppid())
    emit_parser.add_argument("--summary")
    emit_parser.add_argument("--tool-name")
    emit_parser.add_argument("--confidence", type=float)
    emit_parser.add_argument("--input-tokens", type=int, default=0)
    emit_parser.add_argument("--output-tokens", type=int, default=0)
    emit_parser.set_defaults(func=emit)

    poll_parser = subcommands.add_parser("poll")
    poll_parser.add_argument("--agent-id", required=True)
    poll_parser.set_defaults(func=poll)

    ack_parser = subcommands.add_parser("ack")
    ack_parser.add_argument("--command-id", required=True)
    ack_parser.add_argument("--agent-id", required=True)
    ack_parser.add_argument("--accepted", action=argparse.BooleanOptionalAction, default=True)
    ack_parser.add_argument("--message")
    ack_parser.set_defaults(func=acknowledge)
    return root


def main() -> int:
    args = parser().parse_args()
    args.func(args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
