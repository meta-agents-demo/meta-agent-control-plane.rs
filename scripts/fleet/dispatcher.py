"""Discover real portfolio issues and enqueue verified repository work.

The dispatcher is intentionally allowlist-first. It admits only GitHub issues and
Linear issues from the four configured product organizations. GitHub issues have
an intrinsic repository. Linear issues are admitted only when one repository in
the matching organization can be resolved unambiguously from the issue payload.
No guessed repository means no job.

This process never receives provider API keys. It writes validated Job objects to
the provider-specific durable queue volumes; the isolated provider runners own
repository execution, observations, commits, pushes, pull requests, and delivery
verification.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import re
import signal
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from .common import Job, atomic_write_json, read_secret

GITHUB_API = "https://api.github.com"
LINEAR_GRAPHQL_URL = "https://api.linear.app/graphql"
USER_AGENT = "meta-agent-scoped-dispatcher/1"

DEFAULT_ORGS = (
    "apostille-me",
    "embedded-alerts",
    "evento-globolo",
    "hacker-house-medellin",
)
DEFAULT_PROJECTS = tuple(f"github.com/{org}" for org in DEFAULT_ORGS)
PROVIDERS = ("openai", "anthropic")
SKIP_TITLES = (
    "github project and linear workspace links",
    "superseded by canonical short-name repository",
)
GITHUB_REPO_URL_RE = re.compile(
    r"https://github\.com/(?P<org>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)"
)


def log(message: str) -> None:
    print(f"[dispatcher] {message}", file=sys.stderr, flush=True)


def csv_env(name: str, defaults: tuple[str, ...]) -> tuple[str, ...]:
    raw = os.getenv(name, "").strip()
    values = tuple(item.strip() for item in raw.split(",") if item.strip()) if raw else defaults
    return tuple(dict.fromkeys(values))


def http_json(
    url: str,
    *,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    payload: dict[str, Any] | None = None,
    timeout: float = 30.0,
) -> tuple[int, dict[str, Any] | list[Any] | None, str]:
    request_headers = {"Accept": "application/json", "User-Agent": USER_AGENT}
    if headers:
        request_headers.update(headers)
    body = None
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=body, headers=request_headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8", "replace")
            status = response.status
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8", "replace") if error.fp else ""
        return error.code, _json(raw), f"http {error.code}"
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        return 0, None, type(error).__name__
    return status, _json(raw), ""


def _json(raw: str) -> dict[str, Any] | list[Any] | None:
    try:
        value = json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        return None
    return value if isinstance(value, (dict, list)) else None


def github_headers(token: str) -> dict[str, str]:
    return {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }


def actionable_title(title: str) -> bool:
    normalized = title.strip().lower()
    return bool(normalized) and not any(normalized.startswith(prefix) for prefix in SKIP_TITLES)


def github_open_issues(org: str, token: str, limit: int) -> tuple[list[dict[str, Any]], str]:
    query = urllib.parse.quote(f"org:{org} is:issue is:open")
    url = f"{GITHUB_API}/search/issues?q={query}&sort=updated&order=desc&per_page={min(100, limit)}"
    status, payload, error = http_json(url, headers=github_headers(token))
    if status != 200 or not isinstance(payload, dict):
        return [], f"github {org} unavailable ({status}): {error}"
    out: list[dict[str, Any]] = []
    for item in payload.get("items") or []:
        if not isinstance(item, dict):
            continue
        title = str(item.get("title") or "")
        if not actionable_title(title):
            continue
        repo_api = str(item.get("repository_url") or "")
        marker = "/repos/"
        full_name = repo_api.split(marker, 1)[1] if marker in repo_api else ""
        if not full_name.startswith(f"{org}/"):
            continue
        number = item.get("number")
        if not isinstance(number, int):
            continue
        out.append(
            {
                "source_key": f"github:{full_name}#{number}",
                "version": str(item.get("updated_at") or ""),
                "org": org,
                "repository": full_name,
                "title": title,
                "url": str(item.get("html_url") or f"https://github.com/{full_name}/issues/{number}"),
                "body": str(item.get("body") or ""),
                "number": number,
                "source": "github",
            }
        )
    return out, ""


def github_repositories(org: str, token: str) -> tuple[set[str], str]:
    status, payload, error = http_json(
        f"{GITHUB_API}/orgs/{org}/repos?per_page=100&type=all&sort=full_name",
        headers=github_headers(token),
    )
    if status != 200 or not isinstance(payload, list):
        return set(), f"github repo inventory {org} unavailable ({status}): {error}"
    names = {
        str(item.get("name"))
        for item in payload
        if isinstance(item, dict) and item.get("name") and not item.get("archived")
    }
    return names, ""


def linear_active_issues(
    key: str, projects: tuple[str, ...], limit: int
) -> tuple[list[dict[str, Any]], str]:
    query = """
query ScopedIssues($first: Int!, $projects: [String!]!) {
  issues(first: $first, orderBy: updatedAt, filter: { project: { name: { in: $projects } } }) {
    nodes {
      identifier
      title
      description
      updatedAt
      url
      priority
      project { name }
      state { name type }
    }
  }
}
""".strip()
    status, payload, error = http_json(
        LINEAR_GRAPHQL_URL,
        method="POST",
        headers={"Authorization": key},
        payload={"query": query, "variables": {"first": min(100, limit), "projects": list(projects)}},
    )
    if status != 200 or not isinstance(payload, dict):
        return [], f"linear unavailable ({status}): {error}"
    if payload.get("errors"):
        return [], "linear returned GraphQL errors"
    allowed = set(projects)
    out: list[dict[str, Any]] = []
    nodes = (((payload.get("data") or {}).get("issues") or {}).get("nodes")) or []
    for item in nodes:
        if not isinstance(item, dict) or not item.get("identifier"):
            continue
        project = str((item.get("project") or {}).get("name") or "")
        if project not in allowed:
            continue
        state = item.get("state") or {}
        state_type = str(state.get("type") or "").lower()
        if state_type in {"completed", "canceled", "cancelled"}:
            continue
        title = str(item.get("title") or "")
        if not actionable_title(title):
            continue
        out.append(
            {
                "source_key": f"linear:{item['identifier']}",
                "version": str(item.get("updatedAt") or ""),
                "project": project,
                "title": title,
                "url": str(item.get("url") or item["identifier"]),
                "body": str(item.get("description") or ""),
                "identifier": str(item["identifier"]),
                "priority": int(item.get("priority") or 0),
                "source": "linear",
            }
        )
    return out, ""


def project_org(project: str, orgs: tuple[str, ...]) -> str | None:
    prefix = "github.com/"
    if not project.startswith(prefix):
        return None
    org = project[len(prefix):]
    return org if org in orgs else None


def resolve_linear_repository(
    issue: dict[str, Any], org: str, repositories: set[str]
) -> str | None:
    text = f"{issue.get('title', '')}\n{issue.get('body', '')}"
    explicit: set[str] = set()
    for match in GITHUB_REPO_URL_RE.finditer(text):
        if match.group("org") == org and match.group("repo") in repositories:
            explicit.add(match.group("repo"))
    if len(explicit) == 1:
        return f"{org}/{next(iter(explicit))}"
    if len(explicit) > 1:
        return None

    lowered = text.lower()
    mentioned = {
        repo for repo in repositories
        if re.search(rf"(?<![A-Za-z0-9_.-]){re.escape(repo.lower())}(?![A-Za-z0-9_.-])", lowered)
    }
    if len(mentioned) == 1:
        return f"{org}/{next(iter(mentioned))}"
    return None


def provider_for(source_key: str) -> str:
    digest = hashlib.sha256(source_key.encode("utf-8")).digest()
    return PROVIDERS[digest[0] % len(PROVIDERS)]


def version_suffix(version: str) -> str:
    return hashlib.sha256(version.encode("utf-8")).hexdigest()[:8]


def slug(value: str, maximum: int = 48) -> str:
    out = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return (out or "issue")[:maximum].rstrip("-")


def job_for(item: dict[str, Any], repository: str) -> Job:
    provider = provider_for(item["source_key"])
    suffix = version_suffix(item.get("version", ""))
    source_id = str(item.get("identifier") or f"issue-{item.get('number')}")
    job_id = slug(f"{item['source']}-{repository.replace('/', '-')}-{source_id}-{suffix}", 120)
    branch = f"agent/{slug(source_id, 36)}-{suffix}"
    public_title = f"{source_id}: {item['title']}"[:300]
    task = (
        f"Work the real {item['source']} item at {item['url']}.\n"
        f"Target repository: {repository}.\n"
        f"Title: {item['title']}\n\n"
        f"Issue context:\n{item.get('body') or '(no description)'}\n\n"
        "Inspect the current repository and issue state before changing anything. Implement the smallest "
        "coherent change that satisfies the issue, preserve unrelated work, run the relevant validation, "
        "publish bounded progress/reflection through meta-agent-observe, commit focused changes, push the "
        "assigned branch, and open or update a pull request. If the issue cannot be completed safely from "
        "this repository, do not invent success: publish a blocker with evidence and leave delivery partial."
    )
    priority = int(item.get("priority") or 0)
    queue_priority = 20 if priority == 1 else 40 if priority == 2 else 80
    return Job(
        job_id=job_id,
        provider=provider,
        repository=f"https://github.com/{repository}.git",
        task=task,
        base_ref="main",
        branch=branch,
        timeout_seconds=7200,
        max_attempts=3,
        priority=queue_priority,
        public_title=public_title,
        success_criteria=(
            f"Address the real source item {item['url']} without fabricated evidence.",
            "Publish at least one bounded progress event and one evidence-backed reflection.",
            "Run repository-relevant validation and record real test evidence when applicable.",
            "Push a focused branch and open or update a pull request whose head matches the pushed commit.",
        ),
        constraints=(
            "Never expose provider prompts, transcripts, private reasoning, credentials, or authorization values.",
            "Do not force-push, rewrite unrelated history, or claim an external action that was not verified.",
            "Do not broaden work outside the source issue unless required to satisfy its stated acceptance criteria.",
        ),
        require_pull_request=True,
        require_observation=True,
        require_test_evidence=False,
        allow_no_change=False,
    )


def load_ledger(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) and all(isinstance(k, str) and isinstance(v, str) for k, v in value.items()) else {}


def queue_job(job: Job, state_roots: dict[str, Path]) -> Path:
    root = state_roots[job.provider]
    path = root / "queue" / f"{job.job_id}.json"
    if path.exists():
        return path
    atomic_write_json(path, dataclasses.asdict(job))
    return path


def run_cycle(
    *,
    orgs: tuple[str, ...],
    projects: tuple[str, ...],
    github_token: str,
    linear_key: str | None,
    state_roots: dict[str, Path],
    ledger_path: Path,
    per_source_limit: int,
) -> dict[str, int]:
    ledger = load_ledger(ledger_path)
    discovered: list[dict[str, Any]] = []
    repository_inventory: dict[str, set[str]] = {}
    errors = 0

    for org in orgs:
        repos, repo_error = github_repositories(org, github_token)
        repository_inventory[org] = repos
        if repo_error:
            errors += 1
            log(repo_error)
        issues, issue_error = github_open_issues(org, github_token, per_source_limit)
        discovered.extend(issues)
        if issue_error:
            errors += 1
            log(issue_error)

    if linear_key:
        linear, linear_error = linear_active_issues(linear_key, projects, per_source_limit * max(1, len(projects)))
        if linear_error:
            errors += 1
            log(linear_error)
        for item in linear:
            org = project_org(item["project"], orgs)
            if not org:
                continue
            repository = resolve_linear_repository(item, org, repository_inventory.get(org, set()))
            if not repository:
                log(f"skip {item['source_key']}: no unambiguous repository in {org}")
                continue
            item["repository"] = repository
            discovered.append(item)
    else:
        log("Linear credential absent; GitHub issue execution remains active")

    queued = 0
    skipped = 0
    for item in discovered:
        version = str(item.get("version") or "")
        if ledger.get(item["source_key"]) == version:
            skipped += 1
            continue
        repository = str(item.get("repository") or "")
        if not repository:
            skipped += 1
            continue
        job = job_for(item, repository)
        path = queue_job(job, state_roots)
        ledger[item["source_key"]] = version
        queued += 1
        log(f"queued {item['source_key']} -> {job.provider}:{path.name}")

    atomic_write_json(ledger_path, ledger)
    return {"discovered": len(discovered), "queued": queued, "skipped": skipped, "errors": errors}


def main() -> int:
    github_token = read_secret("GH_TOKEN_FILE", "GH_TOKEN")
    if not github_token:
        log("GitHub credential is required for scoped execution discovery")
        return 2
    linear_key = read_secret("LINEAR_API_KEY_FILE", "LINEAR_API_KEY")
    orgs = csv_env("META_AGENT_GITHUB_ORGS", DEFAULT_ORGS)
    projects = csv_env("META_AGENT_LINEAR_PROJECTS", DEFAULT_PROJECTS)
    interval = max(300, int(os.getenv("META_AGENT_DISPATCH_INTERVAL_SECONDS", "900") or 900))
    per_source_limit = min(50, max(1, int(os.getenv("META_AGENT_DISPATCH_LIMIT", "12") or 12)))
    state_roots = {
        "openai": Path(os.getenv("META_AGENT_OPENAI_STATE_DIR", "/state/openai")),
        "anthropic": Path(os.getenv("META_AGENT_ANTHROPIC_STATE_DIR", "/state/anthropic")),
    }
    ledger_path = Path(os.getenv("META_AGENT_DISPATCH_LEDGER", "/state/dispatcher/ledger.json"))

    stopped = False

    def stop(*_args: object) -> None:
        nonlocal stopped
        stopped = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)

    while not stopped:
        summary = run_cycle(
            orgs=orgs,
            projects=projects,
            github_token=github_token,
            linear_key=linear_key,
            state_roots=state_roots,
            ledger_path=ledger_path,
            per_source_limit=per_source_limit,
        )
        log("cycle " + " ".join(f"{key}={value}" for key, value in summary.items()))
        deadline = time.monotonic() + interval
        while not stopped and time.monotonic() < deadline:
            time.sleep(min(1.0, deadline - time.monotonic()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
