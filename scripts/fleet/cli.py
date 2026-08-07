"""Command-line entry points for the durable provider-agent fleet."""

from __future__ import annotations

import argparse
import contextlib
import dataclasses
import json
import os
from pathlib import Path
from typing import Any, Iterable

from .common import Job, atomic_write_json, load_json
from .supervisor import FleetRunner


def enqueue(state_dir: Path, job_path: Path) -> Path:
    job = Job.from_mapping(load_json(job_path))
    target = state_dir / "queue" / f"{job.job_id}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        raise FileExistsError(f"job already queued: {job.job_id}")
    atomic_write_json(target, dataclasses.asdict(job))
    return target


def print_status(state_dir: Path) -> None:
    values: list[dict[str, Any]] = []
    for path in sorted((state_dir / "runs").glob("*/state.json")):
        with contextlib.suppress(OSError, ValueError, json.JSONDecodeError):
            values.append(load_json(path))
    print(json.dumps(values, indent=2, sort_keys=True))


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Durable, bounded provider coding-agent supervisor")
    parser.add_argument(
        "--state-dir", type=Path,
        default=Path(os.getenv("META_AGENT_STATE_DIR", "/var/lib/meta-agent-runner")),
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("run", help="run the durable queue supervisor")
    validate = subparsers.add_parser("validate-job", help="validate one job JSON file")
    validate.add_argument("job", type=Path)
    add = subparsers.add_parser("enqueue", help="copy one validated job into the queue")
    add.add_argument("job", type=Path)
    subparsers.add_parser("status", help="print the sanitized durable ledger")
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    arguments = build_parser().parse_args(argv)
    if arguments.command == "validate-job":
        print(json.dumps(dataclasses.asdict(Job.from_mapping(load_json(arguments.job))), indent=2, sort_keys=True))
        return 0
    if arguments.command == "enqueue":
        print(enqueue(arguments.state_dir, arguments.job))
        return 0
    if arguments.command == "status":
        print_status(arguments.state_dir)
        return 0
    import asyncio
    runner = FleetRunner(arguments.state_dir)
    asyncio.run(runner.run())
    return 0
