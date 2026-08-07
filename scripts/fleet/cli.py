"""Operator CLI for the durable verified provider-agent fleet."""

from __future__ import annotations

import argparse
import asyncio
import dataclasses
import json
import os
import sys
from pathlib import Path
from typing import Iterable

from .common import Job, atomic_write_json, load_json, validate_admitted_job
from .introspection import ProviderDiscoveryError, discover_mcp_servers, discover_provider
from .provider import enforce_credential_expiry, provider_api_key
from .supervisor import FleetRunner


def state_dir() -> Path:
    return Path(os.getenv("META_AGENT_STATE_DIR", "/var/lib/meta-agent-runner"))


def enqueue(root: Path, job_path: Path) -> Path:
    job = Job.from_mapping(load_json(job_path))
    validate_admitted_job(job)
    queue_path = root / "queue" / f"{job.job_id}.json"
    if queue_path.exists():
        existing = Job.from_mapping(load_json(queue_path))
        if existing != job:
            raise ValueError(f"queued job {job.job_id} already exists with different content")
        return queue_path
    atomic_write_json(queue_path, dataclasses.asdict(job))
    return queue_path


def doctor(provider: str) -> int:
    enforce_credential_expiry()
    try:
        snapshot = discover_provider(provider, provider_api_key(provider))
    except ProviderDiscoveryError as error:
        print(json.dumps({
            "provider": provider,
            "credential_loaded": error.kind != "missing_credential",
            "api_status": error.kind,
            "ready": False,
            "recoverable": error.recoverable,
        }, sort_keys=True))
        return 1
    mcp = discover_mcp_servers()
    value = {
        "provider": provider,
        "credential_loaded": snapshot.credential_loaded,
        "api_status": snapshot.api_status,
        "model_count": snapshot.model_count,
        "capability_count": len(snapshot.capability_labels),
        "managed_agents_status": snapshot.managed_agents_status,
        "managed_agents_count": snapshot.managed_agents_count,
        "mcp_servers": [
            {
                "name": item.name,
                "status": item.status,
                "protocol_mode": item.protocol_mode,
                "tool_count": item.tool_count,
                "resource_count": item.resource_count,
                "resource_template_count": item.resource_template_count,
                "prompt_count": item.prompt_count,
            }
            for item in mcp
        ],
        "ready": snapshot.api_status == "ready" and all(item.status == "ready" for item in mcp),
    }
    print(json.dumps(value, indent=2, sort_keys=True))
    return 0 if value["ready"] else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run verified real provider-agent repository tasks")
    parser.add_argument("--state-dir", type=Path, default=state_dir())
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("run")
    validate = subparsers.add_parser("validate-job")
    validate.add_argument("job", type=Path)
    enqueue_parser = subparsers.add_parser("enqueue")
    enqueue_parser.add_argument("job", type=Path)
    subparsers.add_parser("status")
    doctor_parser = subparsers.add_parser("doctor")
    doctor_parser.add_argument("--provider", choices=("openai", "anthropic"), required=True)
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "run":
            asyncio.run(FleetRunner(args.state_dir).run())
            return 0
        if args.command == "validate-job":
            job = Job.from_mapping(load_json(args.job))
            validate_admitted_job(job)
            print(json.dumps({
                "job_id": job.job_id,
                "provider": job.provider,
                "public_title": job.display_title,
                "branch": job.effective_branch,
                "real_task": True,
            }, sort_keys=True))
            return 0
        if args.command == "enqueue":
            print(enqueue(args.state_dir, args.job))
            return 0
        if args.command == "doctor":
            return doctor(args.provider)
        values = []
        for path in sorted((args.state_dir / "runs").glob("*/state.json")):
            values.append(load_json(path))
        print(json.dumps(values, indent=2, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
