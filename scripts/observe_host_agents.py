#!/usr/bin/env python3
"""Publish privacy-minimized macOS agent process samples to the control plane.

Only fixed `ps` columns are collected. Command arguments, environment variables,
open files, prompt text, and credentials are intentionally never read.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

PS_COLUMNS = "pid=,ppid=,pgid=,%cpu=,rss=,%mem=,state=,comm="


def classify_process(command: str) -> tuple[str, str] | None:
    normalized = command.casefold()
    executable = Path(command).name.casefold()
    if "ai-agent-bridge" in executable:
        return "local", "bridge"
    if "claude" in executable:
        return "anthropic", "agent_cli"
    if "chatgpt" in executable:
        role = "app_helper" if "helper" in executable else "app"
        return "openai", role
    if "codex" in executable:
        role = "agent_service" if "service" in executable else "agent_cli"
        return "openai", role
    if "claude" in normalized:
        return "anthropic", "agent_helper"
    if "chatgpt" in normalized or "codex" in normalized or "openai" in normalized:
        return "openai", "agent_service"
    return None


def parse_ps_lines(lines: Iterable[str]) -> list[dict[str, object]]:
    processes: list[dict[str, object]] = []
    for line in lines:
        fields = line.strip().split(None, 7)
        if len(fields) != 8:
            continue
        pid, ppid, pgid, cpu, rss_kib, memory, state, command = fields
        classification = classify_process(command)
        if classification is None:
            continue
        provider, role = classification
        try:
            processes.append(
                {
                    "pid": int(pid),
                    "ppid": int(ppid),
                    "pgid": int(pgid),
                    "provider": provider,
                    "process_name": Path(command).name,
                    "process_role": role,
                    "process_state": state,
                    "cpu_percent": float(cpu),
                    "rss_bytes": int(rss_kib) * 1024,
                    "memory_percent": float(memory),
                }
            )
        except ValueError:
            continue
    return processes


def sample_processes() -> list[dict[str, object]]:
    result = subprocess.run(
        ["ps", "-axo", PS_COLUMNS],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    return parse_ps_lines(result.stdout.splitlines())


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def make_observation(
    processes: list[dict[str, object]], observer_id: str, host_id: str
) -> dict[str, object]:
    return {
        "protocol_version": "v1",
        "observation_id": str(uuid.uuid4()),
        "observed_at": utc_now(),
        "observer_id": observer_id,
        "host_id": host_id,
        "platform": f"{platform.system().lower()}-{platform.machine().lower()}",
        "processes": processes,
    }


def read_token(token_file: str | None) -> str:
    if token_file:
        token = Path(token_file).read_text(encoding="utf-8").strip()
    else:
        token = os.environ.get("META_AGENT_AUTH_TOKEN", "").strip()
    if not token:
        raise ValueError("set META_AGENT_AUTH_TOKEN or pass --token-file")
    return token


def post_observation(base_url: str, token: str, payload: dict[str, object]) -> int:
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/v1/runtime/host-observations",
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        response.read()
        return response.status


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8787")
    parser.add_argument("--token-file")
    parser.add_argument("--interval", type=float, default=3.0)
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--host-id", default=socket.gethostname())
    parser.add_argument("--observer-id")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.interval < 0.5:
        raise SystemExit("--interval must be at least 0.5 seconds")
    token = read_token(args.token_file)
    observer_id = args.observer_id or f"macos-ps:{args.host_id}"
    try:
        while True:
            try:
                processes = sample_processes()
                status = post_observation(
                    args.base_url,
                    token,
                    make_observation(processes, observer_id, args.host_id),
                )
                print(
                    f"host observation accepted status={status} processes={len(processes)}",
                    flush=True,
                )
            except (OSError, ValueError, subprocess.SubprocessError, urllib.error.URLError) as error:
                print(f"host observation failed: {error}", file=sys.stderr, flush=True)
                if args.once:
                    return 1
            if args.once:
                return 0
            time.sleep(args.interval)
    except KeyboardInterrupt:
        print("host observation stopped", flush=True)
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
