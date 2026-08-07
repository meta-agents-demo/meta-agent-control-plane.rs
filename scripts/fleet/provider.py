"""Provider process environment, health circuit, and safe runtime hooks."""

from __future__ import annotations

import datetime as dt
import json
import os
import shlex
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any

from .common import Job, atomic_write_json, env_bool, load_json, read_secret, utc_now


def enforce_credential_expiry() -> None:
    raw = os.getenv("META_AGENT_CREDENTIAL_EXPIRES_AT", "").strip()
    if not raw:
        return
    try:
        expires = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("META_AGENT_CREDENTIAL_EXPIRES_AT must be ISO-8601") from error
    if expires.tzinfo is None:
        raise ValueError("META_AGENT_CREDENTIAL_EXPIRES_AT must include a timezone")
    if dt.datetime.now(dt.timezone.utc) >= expires.astimezone(dt.timezone.utc):
        raise ValueError("temporary provider credentials have expired")


def isolated_child_environment() -> dict[str, str]:
    environment = dict(os.environ)
    runner_home = Path(os.getenv("META_AGENT_RUNNER_HOME", "/var/lib/meta-agent-runner/home"))
    runner_home.mkdir(parents=True, exist_ok=True)
    os.chmod(runner_home, 0o700)
    config_home = runner_home / ".config"
    cache_home = runner_home / ".cache"
    for path in (config_home, cache_home):
        path.mkdir(parents=True, exist_ok=True)
        os.chmod(path, 0o700)
    environment.update(HOME=str(runner_home), XDG_CONFIG_HOME=str(config_home), XDG_CACHE_HOME=str(cache_home))
    return environment


def provider_environment(provider: str) -> dict[str, str]:
    environment = isolated_child_environment()
    if provider == "openai":
        selected_key = read_secret("OPENAI_API_KEY_FILE", "OPENAI_API_KEY")
        if not selected_key:
            raise ValueError("OpenAI worker requires OPENAI_API_KEY_FILE or OPENAI_API_KEY")
        environment["OPENAI_API_KEY"] = selected_key
        environment.pop("ANTHROPIC_API_KEY", None)
    else:
        selected_key = read_secret("ANTHROPIC_API_KEY_FILE", "ANTHROPIC_API_KEY")
        if not selected_key:
            raise ValueError("Anthropic worker requires ANTHROPIC_API_KEY_FILE or ANTHROPIC_API_KEY")
        environment["ANTHROPIC_API_KEY"] = selected_key
        environment.pop("OPENAI_API_KEY", None)
    github_token = read_secret("GH_TOKEN_FILE", "GH_TOKEN")
    if github_token:
        environment.update(GH_TOKEN=github_token, GITHUB_TOKEN=github_token)
    environment["META_AGENT_EPHEMERAL_ACCOUNT"] = "true"
    environment.setdefault("GIT_AUTHOR_NAME", "meta-agent-fleet")
    environment.setdefault("GIT_AUTHOR_EMAIL", "meta-agent-fleet@users.noreply.github.com")
    environment.setdefault("GIT_COMMITTER_NAME", environment["GIT_AUTHOR_NAME"])
    environment.setdefault("GIT_COMMITTER_EMAIL", environment["GIT_AUTHOR_EMAIL"])
    return environment


def provider_command(provider: str) -> list[str]:
    if provider == "openai":
        raw = os.getenv(
            "META_AGENT_OPENAI_COMMAND",
            "codex exec --json --full-auto --config sandbox_workspace_write.network_access=true --skip-git-repo-check -",
        )
    else:
        raw = os.getenv(
            "META_AGENT_ANTHROPIC_COMMAND",
            "claude -p --permission-mode acceptEdits --output-format stream-json --verbose",
        )
    command = shlex.split(raw)
    if not command or any("\x00" in part for part in command):
        raise ValueError(f"invalid {provider} command")
    return command


class ProviderCircuit:
    def __init__(self, state_dir: Path) -> None:
        self.path = state_dir / "providers.json"
        self.value = load_json(self.path) if self.path.exists() else {"providers": {}}

    def blocked_until(self, provider: str) -> float:
        raw = self.value.get("providers", {}).get(provider, {}).get("blocked_until_epoch", 0)
        try:
            return float(raw)
        except (TypeError, ValueError):
            return 0.0

    def is_blocked(self, provider: str) -> bool:
        return self.blocked_until(provider) > time.time()

    def block(self, provider: str, reason: str, seconds: int) -> None:
        providers = self.value.setdefault("providers", {})
        providers[provider] = {
            "status": "unavailable",
            "reason": reason,
            "blocked_until_epoch": int(time.time()) + seconds,
            "updated_at": utc_now(),
        }
        atomic_write_json(self.path, self.value)

    def clear(self, provider: str) -> None:
        self.value.setdefault("providers", {})[provider] = {"status": "available", "updated_at": utc_now()}
        atomic_write_json(self.path, self.value)


class HookClient:
    def __init__(self) -> None:
        self.base_url = os.getenv("META_AGENT_RUNTIME_URL", "http://control-plane:8787").rstrip("/")
        self.token = read_secret("META_AGENT_AUTH_TOKEN_FILE", "META_AGENT_AUTH_TOKEN")
        self.enabled = env_bool("META_AGENT_HOOKS_ENABLED", True) and bool(self.token)

    def emit(self, job: Job, kind: str, summary: str, pid: int | None = None) -> None:
        if not self.enabled:
            return
        payload: dict[str, Any] = {
            "protocol_version": "v1",
            "event_id": str(uuid.uuid4()),
            "occurred_at": utc_now(),
            "agent": {
                "agent_id": f"fleet-{job.job_id}",
                "provider": job.provider,
                "model": job.model or "provider-default",
                "instance_id": job.job_id,
            },
            "session_id": job.job_id,
            "kind": kind,
            "control_capable": False,
            "summary": summary,
            "metadata": {"account_class": "ephemeral-project"},
        }
        if pid is not None:
            payload["pid"] = pid
            rss = read_rss_bytes(pid)
            if rss is not None:
                payload["rss_bytes"] = rss
        request = urllib.request.Request(
            f"{self.base_url}/api/v1/runtime/hooks",
            data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
            headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=3) as response:
                response.read(1024)
        except (OSError, urllib.error.URLError, urllib.error.HTTPError):
            return


def read_rss_bytes(pid: int) -> int | None:
    try:
        pages = int(Path(f"/proc/{pid}/statm").read_text(encoding="utf-8").split()[1])
        return pages * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError, IndexError):
        return None
