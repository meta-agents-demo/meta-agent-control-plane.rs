"""Non-mutating live canary for the paired GitHub and Linear test portfolios.

The production dispatcher intentionally writes validated jobs into provider queue
volumes. This module reuses its discovery, repository resolution, provider
sharding, and job validation logic but never writes a queue, ledger, branch,
pull request, deployment, or source issue. It is therefore safe to run against
``*-test`` organizations before enabling production dispatch.
"""

from __future__ import annotations

import dataclasses
import json
import os
from pathlib import Path
from typing import Any

from . import dispatcher
from .common import atomic_write_json, read_secret, validate_admitted_job

CANARY_PREFIX = "[meta-agent-canary]"
TEST_ORGS = tuple(f"{org}-test" for org in dispatcher.DEFAULT_ORGS)
TEST_PROJECTS = tuple(f"github.com/{org}" for org in TEST_ORGS)


def is_canary(item: dict[str, Any]) -> bool:
    return str(item.get("title") or "").strip().lower().startswith(CANARY_PREFIX)


def _job_report(item: dict[str, Any], repository: str) -> dict[str, Any]:
    job = dispatcher.job_for(item, repository)
    validate_admitted_job(job)
    # Deliberately exclude the private execution task and source body. The report
    # proves routing and schema validity without becoming another prompt/log sink.
    return {
        "source_key": item["source_key"],
        "source": item["source"],
        "source_url": item["url"],
        "source_version": item.get("version", ""),
        "repository": repository,
        "job_id": job.job_id,
        "provider": job.provider,
        "base_ref": job.base_ref,
        "branch": job.effective_branch,
        "public_title": job.display_title,
        "require_pull_request": job.require_pull_request,
        "require_observation": job.require_observation,
        "require_test_evidence": job.require_test_evidence,
        "validated": True,
    }


def discover(
    *,
    github_token: str,
    linear_key: str | None,
    orgs: tuple[str, ...] = TEST_ORGS,
    projects: tuple[str, ...] = TEST_PROJECTS,
    per_source_limit: int = 25,
) -> dict[str, Any]:
    discovered: list[dict[str, Any]] = []
    repository_inventory: dict[str, set[str]] = {}
    errors: list[str] = []
    org_counts: dict[str, dict[str, int]] = {}

    for org in orgs:
        repositories, repo_error = dispatcher.github_repositories(org, github_token)
        repository_inventory[org] = repositories
        if repo_error:
            errors.append(repo_error)
        issues, issue_error = dispatcher.github_open_issues(org, github_token, per_source_limit)
        if issue_error:
            errors.append(issue_error)
        canaries = [item for item in issues if is_canary(item)]
        discovered.extend(canaries)
        org_counts[org] = {
            "repositories": len(repositories),
            "canary_issues": len(canaries),
        }

    linear_count = 0
    unresolved_linear: list[str] = []
    if linear_key:
        linear, linear_error = dispatcher.linear_active_issues(
            linear_key,
            projects,
            per_source_limit * max(1, len(projects)),
        )
        if linear_error:
            errors.append(linear_error)
        for item in linear:
            if not is_canary(item):
                continue
            org = dispatcher.project_org(item["project"], orgs)
            if not org:
                continue
            repository = dispatcher.resolve_linear_repository(
                item,
                org,
                repository_inventory.get(org, set()),
            )
            if not repository:
                unresolved_linear.append(item["source_key"])
                continue
            item["repository"] = repository
            discovered.append(item)
            linear_count += 1

    jobs: list[dict[str, Any]] = []
    skipped: list[str] = []
    for item in discovered:
        repository = str(item.get("repository") or "")
        if not repository:
            skipped.append(item["source_key"])
            continue
        jobs.append(_job_report(item, repository))

    jobs.sort(key=lambda value: (value["source_key"], value["job_id"]))
    return {
        "mode": "paired-test-org-canary",
        "dry_run": True,
        "mutation_enabled": False,
        "queue_writes": 0,
        "ledger_writes": 0,
        "github_orgs": list(orgs),
        "linear_projects": list(projects),
        "org_counts": org_counts,
        "github_canaries": sum(value["canary_issues"] for value in org_counts.values()),
        "linear_canaries": linear_count,
        "validated_jobs": len(jobs),
        "jobs": jobs,
        "skipped_sources": sorted(skipped),
        "unresolved_linear_sources": sorted(unresolved_linear),
        "errors": sorted(set(errors)),
    }


def _csv_env(name: str, defaults: tuple[str, ...]) -> tuple[str, ...]:
    return dispatcher.csv_env(name, defaults)


def main() -> int:
    github_token = read_secret("GH_TOKEN_FILE", "GH_TOKEN")
    if not github_token:
        dispatcher.log("paired test-org canary requires GH_TOKEN_FILE or GH_TOKEN")
        return 2
    linear_key = read_secret("LINEAR_API_KEY_FILE", "LINEAR_API_KEY")
    orgs = _csv_env("META_AGENT_CANARY_GITHUB_ORGS", TEST_ORGS)
    projects = _csv_env("META_AGENT_CANARY_LINEAR_PROJECTS", TEST_PROJECTS)
    limit = min(100, max(1, int(os.getenv("META_AGENT_CANARY_LIMIT", "25") or 25)))
    minimum = min(1000, max(0, int(os.getenv("META_AGENT_CANARY_MIN_JOBS", "1") or 1)))
    report = discover(
        github_token=github_token,
        linear_key=linear_key,
        orgs=orgs,
        projects=projects,
        per_source_limit=limit,
    )

    output = os.getenv("META_AGENT_CANARY_REPORT_PATH", "/tmp/meta-agent-canary-report.json").strip()
    if output:
        atomic_write_json(Path(output), report)
    print(json.dumps(report, indent=2, sort_keys=True))

    if report["errors"]:
        return 1
    if report["validated_jobs"] < minimum:
        dispatcher.log(
            f"paired test-org canary found {report['validated_jobs']} validated jobs; minimum is {minimum}"
        )
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
