#!/usr/bin/env python3
"""Transparent Codex app-server proxy that emits safe runtime lifecycle hooks."""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import agent_hook_adapter as adapter  # noqa: E402

QUEUE_CAPACITY = 1024
RESOURCE_SAMPLE_SECONDS = 1.0


class ResourceSampler:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.last_sample_at = 0.0
        self.last_sample: dict[str, float | int] = {}

    def sample(self) -> dict[str, float | int]:
        now = time.monotonic()
        if now - self.last_sample_at >= RESOURCE_SAMPLE_SECONDS:
            self.last_sample = adapter.sample_process(self.pid)
            self.last_sample_at = now
        return dict(self.last_sample)


def copy_stdin(process: subprocess.Popen[str]) -> None:
    assert process.stdin is not None
    try:
        for line in sys.stdin:
            process.stdin.write(line)
            process.stdin.flush()
    except (BrokenPipeError, OSError):
        pass
    finally:
        try:
            process.stdin.close()
        except OSError:
            pass


def copy_stderr(process: subprocess.Popen[str]) -> None:
    assert process.stderr is not None
    for line in process.stderr:
        sys.stderr.write(line)
        sys.stderr.flush()


def hook_worker(
    events: queue.Queue[dict[str, Any] | None],
    *,
    process_pid: int,
    agent_id: str | None,
    model: str | None,
) -> None:
    token_state: dict[str, tuple[int, int]] = {}
    resources = ResourceSampler(process_pid)
    while True:
        event = events.get()
        if event is None:
            return
        try:
            envelope = adapter.map_codex_notification(
                event,
                token_state=token_state,
                agent_id_override=agent_id,
                model_override=model,
                pid=process_pid,
                resources=resources.sample(),
            )
            if envelope is not None:
                adapter.post_hook(envelope)
        except Exception as error:  # Keep the app-server stream alive on telemetry failure.
            print(
                f"meta-agent codex proxy telemetry: {type(error).__name__}",
                file=sys.stderr,
            )


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    root.add_argument("--agent-id")
    root.add_argument("--model")
    root.add_argument("command", nargs=argparse.REMAINDER)
    return root


def main() -> int:
    args = parser().parse_args()
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        command = ["codex", "app-server"]

    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    events: queue.Queue[dict[str, Any] | None] = queue.Queue(QUEUE_CAPACITY)
    stdin_thread = threading.Thread(target=copy_stdin, args=(process,), daemon=True)
    stderr_thread = threading.Thread(target=copy_stderr, args=(process,), daemon=True)
    telemetry_thread = threading.Thread(
        target=hook_worker,
        kwargs={
            "events": events,
            "process_pid": process.pid,
            "agent_id": args.agent_id,
            "model": args.model,
        },
        daemon=True,
    )
    stdin_thread.start()
    stderr_thread.start()
    telemetry_thread.start()

    assert process.stdout is not None
    try:
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(payload, dict) and isinstance(payload.get("method"), str):
                try:
                    events.put_nowait(payload)
                except queue.Full:
                    print(
                        "meta-agent codex proxy telemetry queue full; event dropped",
                        file=sys.stderr,
                    )
    except KeyboardInterrupt:
        process.terminate()
    finally:
        try:
            events.put_nowait(None)
        except queue.Full:
            pass

    return_code = process.wait()
    telemetry_thread.join(timeout=5)
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
