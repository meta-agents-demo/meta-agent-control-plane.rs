"""Authenticated, non-destructive worktree preparation and delivery evidence."""

from __future__ import annotations

import dataclasses
import json
import subprocess
import urllib.parse
from pathlib import Path

from .common import Job, read_secret
from .observability import ObservationSummary
from .provider import isolated_child_environment


def prepare_workspace(workspaces_dir: Path, job: Job, resumed: bool) -> Path:
    workspace = workspaces_dir / job.job_id
    git_env = isolated_child_environment()
    github_token = read_secret("GH_TOKEN_FILE", "GH_TOKEN")
    if github_token:
        git_env.update(GH_TOKEN=github_token, GITHUB_TOKEN=github_token)
        subprocess.run(
            ["gh", "auth", "setup-git"], env=git_env, check=True, timeout=30,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    if not workspace.exists():
        subprocess.run(
            ["git", "clone", "--filter=blob:none", "--no-tags", job.repository, str(workspace)],
            env=git_env, check=True, timeout=300,
        )
    if resumed:
        return workspace
    subprocess.run(["git", "fetch", "origin", "--prune"], cwd=workspace, env=git_env, check=True, timeout=180)
    branches = subprocess.run(
        ["git", "branch", "--format=%(refname:short)"], cwd=workspace, env=git_env,
        check=True, text=True, capture_output=True, timeout=30,
    ).stdout.splitlines()
    if job.effective_branch in branches:
        subprocess.run(["git", "checkout", job.effective_branch], cwd=workspace, env=git_env, check=True, timeout=60)
        return workspace
    remote = subprocess.run(
        ["git", "ls-remote", "--heads", "origin", job.effective_branch], cwd=workspace, env=git_env,
        check=True, text=True, capture_output=True, timeout=60,
    ).stdout.strip()
    if remote:
        subprocess.run(
            ["git", "checkout", "--track", f"origin/{job.effective_branch}"],
            cwd=workspace, env=git_env, check=True, timeout=60,
        )
    else:
        subprocess.run(["git", "checkout", job.base_ref], cwd=workspace, env=git_env, check=True, timeout=60)
        subprocess.run(["git", "checkout", "-b", job.effective_branch], cwd=workspace, env=git_env, check=True, timeout=60)
    return workspace


def safe_git_output(workspace: Path, *arguments: str, maximum: int = 2_048) -> str | None:
    try:
        result = subprocess.run(
            ["git", *arguments], cwd=workspace, check=True, text=True,
            capture_output=True, timeout=30,
        )
        value = result.stdout.strip()
        return value[:maximum] if value else None
    except (OSError, subprocess.SubprocessError):
        return None


def count_dirty_entries(workspace: Path) -> int | None:
    try:
        result = subprocess.run(
            ["git", "status", "--porcelain=v1"], cwd=workspace, check=True,
            text=True, capture_output=True, timeout=30,
        )
        return len(result.stdout.splitlines())
    except (OSError, subprocess.SubprocessError):
        return None


def github_repository_url(repository: str) -> str | None:
    candidate = repository.strip()
    if candidate.startswith("git@github.com:"):
        candidate = "https://github.com/" + candidate.removeprefix("git@github.com:")
    elif candidate.startswith("ssh://git@github.com/"):
        candidate = "https://github.com/" + candidate.removeprefix("ssh://git@github.com/")
    parsed = urllib.parse.urlsplit(candidate)
    if parsed.scheme not in {"http", "https"} or (parsed.hostname or "").lower() != "github.com":
        return None
    path = parsed.path.strip("/")
    if path.endswith(".git"):
        path = path[:-4]
    if path.count("/") != 1:
        return None
    return f"https://github.com/{path}"


def _safe_int(value: str | None) -> int | None:
    try:
        return int(value) if value is not None else None
    except ValueError:
        return None


def _pull_request(workspace: Path, job: Job, repository_url: str | None) -> dict[str, str | bool] | None:
    if not repository_url:
        return None
    environment = isolated_child_environment()
    token = read_secret("GH_TOKEN_FILE", "GH_TOKEN")
    if token:
        environment.update(GH_TOKEN=token, GITHUB_TOKEN=token)
    try:
        result = subprocess.run(
            [
                "gh", "pr", "view", job.effective_branch,
                "--repo", repository_url.removeprefix("https://github.com/"),
                "--json", "url,state,isDraft,headRefOid",
            ],
            cwd=workspace, env=environment, check=True, text=True,
            capture_output=True, timeout=30,
        )
        value = json.loads(result.stdout)
        return value if isinstance(value, dict) else None
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
        return None


@dataclasses.dataclass(frozen=True)
class DeliveryEvidence:
    repository_url: str | None
    branch_url: str | None
    commit_url: str | None
    pull_request_url: str | None
    pull_request_state: str | None
    pull_request_is_draft: bool | None
    pull_request_head: str | None
    head_commit: str | None
    base_commit: str | None
    remote_branch_commit: str | None
    commits_ahead: int | None
    dirty_entries: int | None

    @property
    def artifacts(self) -> tuple[str, ...]:
        return tuple(value for value in (self.repository_url, self.branch_url, self.commit_url, self.pull_request_url) if value)

    def actual_result(self, observations: ObservationSummary) -> str:
        parts = []
        if self.head_commit:
            parts.append(f"head={self.head_commit}")
        if self.commits_ahead is not None:
            parts.append(f"commits_ahead={self.commits_ahead}")
        if self.dirty_entries is not None:
            parts.append(f"dirty_entries={self.dirty_entries}")
        if self.pull_request_url:
            parts.append(f"pull_request={self.pull_request_url}")
        parts.extend((
            f"public_progress_events={observations.progress_count}",
            f"public_reflections={observations.reflection_count}",
            f"test_evidence={observations.test_evidence_count}",
        ))
        return "; ".join(parts)


def collect_delivery_evidence(workspace: Path, job: Job) -> DeliveryEvidence:
    head = safe_git_output(workspace, "rev-parse", "HEAD", maximum=128)
    base = safe_git_output(workspace, "rev-parse", f"origin/{job.base_ref}", maximum=128)
    ahead = _safe_int(safe_git_output(workspace, "rev-list", "--count", f"origin/{job.base_ref}..HEAD", maximum=32))
    remote_raw = safe_git_output(workspace, "ls-remote", "--heads", "origin", job.effective_branch, maximum=512)
    remote_head = remote_raw.split()[0] if remote_raw and remote_raw.split() else None
    repository_url = github_repository_url(job.repository)
    branch_url = f"{repository_url}/tree/{urllib.parse.quote(job.effective_branch, safe='/')}" if repository_url else None
    commit_url = f"{repository_url}/commit/{head}" if repository_url and head else None
    pull_request = _pull_request(workspace, job, repository_url)
    return DeliveryEvidence(
        repository_url=repository_url,
        branch_url=branch_url,
        commit_url=commit_url,
        pull_request_url=str(pull_request.get("url"))[:2_048] if pull_request and pull_request.get("url") else None,
        pull_request_state=str(pull_request.get("state"))[:64] if pull_request and pull_request.get("state") else None,
        pull_request_is_draft=bool(pull_request.get("isDraft")) if pull_request and "isDraft" in pull_request else None,
        pull_request_head=str(pull_request.get("headRefOid"))[:128] if pull_request and pull_request.get("headRefOid") else None,
        head_commit=head,
        base_commit=base,
        remote_branch_commit=remote_head,
        commits_ahead=ahead,
        dirty_entries=count_dirty_entries(workspace),
    )


def missing_delivery_requirements(job: Job, evidence: DeliveryEvidence, observations: ObservationSummary) -> tuple[str, ...]:
    missing: list[str] = []
    if evidence.dirty_entries is None:
        missing.append("worktree_state_unverified")
    elif evidence.dirty_entries:
        missing.append("worktree_not_clean")
    if not job.allow_no_change and (evidence.commits_ahead is None or evidence.commits_ahead < 1):
        missing.append("no_new_commit")
    if not evidence.remote_branch_commit:
        missing.append("remote_branch_missing")
    elif evidence.head_commit != evidence.remote_branch_commit:
        missing.append("remote_branch_head_mismatch")
    if job.require_pull_request:
        if not evidence.pull_request_url:
            missing.append("pull_request_missing")
        else:
            if (evidence.pull_request_state or "").upper() != "OPEN":
                missing.append("pull_request_not_open")
            if not evidence.pull_request_head:
                missing.append("pull_request_head_unverified")
            elif not evidence.remote_branch_commit or evidence.pull_request_head != evidence.remote_branch_commit:
                missing.append("pull_request_head_mismatch")
    if job.require_observation:
        if observations.progress_count < 2:
            missing.append("public_progress_incomplete")
        if observations.reflection_count < 1:
            missing.append("public_reflection_missing")
    if job.require_test_evidence and observations.test_evidence_count < 1:
        missing.append("test_evidence_missing")
    return tuple(missing)
