"""Authenticated, non-destructive Git worktree preparation."""

from __future__ import annotations

import subprocess
from pathlib import Path

from .common import Job, read_secret
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
        subprocess.run(
            ["git", "checkout", "-b", job.effective_branch], cwd=workspace, env=git_env, check=True, timeout=60,
        )
    return workspace


def safe_git_output(workspace: Path, *arguments: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", *arguments], cwd=workspace, check=True, text=True,
            capture_output=True, timeout=30,
        )
        value = result.stdout.strip()
        return value[:256] if value else None
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
