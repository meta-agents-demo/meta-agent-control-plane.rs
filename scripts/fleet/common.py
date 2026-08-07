"""Bounded job schema and filesystem helpers for real provider-agent work."""

from __future__ import annotations

import contextlib
import dataclasses
import datetime as dt
import json
import os
import re
import stat
import tempfile
from pathlib import Path
from typing import Any

PROVIDERS = {"openai", "anthropic"}
TERMINAL_STATES = {"succeeded", "failed", "canceled", "partial"}
MAX_CONCURRENCY_HARD_LIMIT = 15
MAX_JOB_BYTES = 128 * 1024
MAX_TASK_BYTES = 64 * 1024
MAX_LOG_TAIL_BYTES = 64 * 1024
ID_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$")
UNSAFE_REF_CHARS = set(" ~^:?*[\\")
QUOTA_PATTERNS = (
    "insufficient_quota", "insufficient quota", "out of credits", "credit balance",
    "usage limit", "quota exceeded", "billing hard limit",
)
RATE_LIMIT_PATTERNS = ("rate limit", "too many requests", "http 429", "status 429")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def env_bool(name: str, default: bool) -> bool:
    value = os.getenv(name)
    if value is None:
        return default
    normalized = value.strip().lower()
    if normalized in {"1", "true", "yes", "on"}:
        return True
    if normalized in {"0", "false", "no", "off"}:
        return False
    return default


def env_int(name: str, default: int, minimum: int, maximum: int) -> int:
    try:
        value = int(os.getenv(name, str(default)))
    except ValueError:
        value = default
    return min(maximum, max(minimum, value))


def read_secret(path_env: str, value_env: str) -> str | None:
    path = os.getenv(path_env)
    if path:
        value = Path(path).read_text(encoding="utf-8").strip()
        if not value:
            raise ValueError(f"{path_env} points to an empty file")
        return value
    value = os.getenv(value_env)
    return value.strip() if value and value.strip() else None


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True) + "\n"
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(fd, stat.S_IRUSR | stat.S_IWUSR)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        with contextlib.suppress(FileNotFoundError):
            os.unlink(temporary)


def load_json(path: Path, max_bytes: int = MAX_JOB_BYTES) -> dict[str, Any]:
    if path.stat().st_size > max_bytes:
        raise ValueError(f"{path} exceeds the {max_bytes}-byte limit")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def validate_git_ref(value: str, field: str) -> str:
    candidate = value.strip()
    if (
        not candidate
        or candidate.startswith(("-", "/", "."))
        or candidate.endswith(("/", ".", ".lock"))
        or ".." in candidate
        or "@{" in candidate
        or "//" in candidate
        or any(ord(char) < 32 or char in UNSAFE_REF_CHARS for char in candidate)
    ):
        raise ValueError(f"{field} contains unsafe git-ref characters")
    return candidate


def _bounded_strings(value: Any, field: str, maximum_values: int, maximum_bytes: int) -> tuple[str, ...]:
    if value is None:
        return ()
    if not isinstance(value, list) or len(value) > maximum_values:
        raise ValueError(f"{field} must be a list of at most {maximum_values} strings")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip() or len(item.encode("utf-8")) > maximum_bytes:
            raise ValueError(f"{field} contains an invalid string")
        result.append(item.strip())
    return tuple(result)


@dataclasses.dataclass(frozen=True)
class Job:
    job_id: str
    provider: str
    repository: str
    task: str
    base_ref: str = "main"
    branch: str | None = None
    timeout_seconds: int = 3600
    max_attempts: int = 3
    priority: int = 100
    model: str | None = None
    public_title: str | None = None
    success_criteria: tuple[str, ...] = ()
    constraints: tuple[str, ...] = ()
    require_pull_request: bool = True
    require_observation: bool = True
    require_test_evidence: bool = False
    allow_no_change: bool = False

    @classmethod
    def from_mapping(cls, value: dict[str, Any]) -> "Job":
        allowed = {
            "job_id", "provider", "repository", "task", "base_ref", "branch",
            "timeout_seconds", "max_attempts", "priority", "model", "public_title",
            "success_criteria", "constraints", "require_pull_request", "require_observation",
            "require_test_evidence", "allow_no_change",
        }
        extras = sorted(set(value) - allowed)
        if extras:
            raise ValueError(f"unknown job fields: {', '.join(extras)}")
        required = ("job_id", "provider", "repository", "task")
        missing = [name for name in required if not isinstance(value.get(name), str) or not value[name].strip()]
        if missing:
            raise ValueError(f"missing non-empty string fields: {', '.join(missing)}")
        job_id = value["job_id"].strip()
        if not ID_RE.fullmatch(job_id):
            raise ValueError("job_id must match [a-zA-Z0-9][a-zA-Z0-9._-]{0,127}")
        provider = value["provider"].strip().lower()
        if provider not in PROVIDERS:
            raise ValueError(f"provider must be one of: {', '.join(sorted(PROVIDERS))}")
        task = value["task"]
        if len(task.encode("utf-8")) > MAX_TASK_BYTES:
            raise ValueError(f"task exceeds the {MAX_TASK_BYTES}-byte limit")
        repository = value["repository"].strip()
        if "\n" in repository or "\r" in repository or repository.startswith("-"):
            raise ValueError("repository contains unsafe characters")
        base_ref = validate_git_ref(str(value.get("base_ref", "main")), "base_ref")
        branch = value.get("branch")
        if branch is not None:
            if not isinstance(branch, str):
                raise ValueError("branch must be a string")
            branch = validate_git_ref(branch, "branch")
        timeout_seconds = int(value.get("timeout_seconds", 3600))
        max_attempts = int(value.get("max_attempts", 3))
        priority = int(value.get("priority", 100))
        model = value.get("model")
        public_title = value.get("public_title")
        if not 60 <= timeout_seconds <= 24 * 3600:
            raise ValueError("timeout_seconds must be between 60 and 86400")
        if not 1 <= max_attempts <= 10:
            raise ValueError("max_attempts must be between 1 and 10")
        if not 0 <= priority <= 1000:
            raise ValueError("priority must be between 0 and 1000")
        if model is not None and (not isinstance(model, str) or len(model) > 128):
            raise ValueError("model must be a string of at most 128 characters")
        if public_title is not None and (
            not isinstance(public_title, str) or not public_title.strip() or len(public_title.encode("utf-8")) > 2_048
        ):
            raise ValueError("public_title must be a non-empty string of at most 2048 bytes")
        booleans = {}
        for name, default in (
            ("require_pull_request", True), ("require_observation", True),
            ("require_test_evidence", False), ("allow_no_change", False),
        ):
            current = value.get(name, default)
            if not isinstance(current, bool):
                raise ValueError(f"{name} must be a boolean")
            booleans[name] = current
        return cls(
            job_id=job_id,
            provider=provider,
            repository=repository,
            task=task,
            base_ref=base_ref,
            branch=branch,
            timeout_seconds=timeout_seconds,
            max_attempts=max_attempts,
            priority=priority,
            model=model,
            public_title=public_title.strip() if isinstance(public_title, str) else None,
            success_criteria=_bounded_strings(value.get("success_criteria"), "success_criteria", 32, 2_048),
            constraints=_bounded_strings(value.get("constraints"), "constraints", 32, 2_048),
            **booleans,
        )

    @property
    def effective_branch(self) -> str:
        return self.branch or f"agent/{self.job_id}"

    @property
    def display_title(self) -> str:
        return self.public_title or f"Real repository task {self.job_id}"

    def safe_prompt(self, resumed: bool) -> str:
        continuation = (
            "\nThis run is resuming after controlled shutdown. Inspect the existing branch and "
            "worktree first and preserve compatible prior work without rewriting history.\n"
            if resumed else ""
        )
        criteria = "\n".join(f"- {item}" for item in self.success_criteria) or "- Deliver the assigned repository result with verifiable evidence."
        constraints = "\n".join(f"- {item}" for item in self.constraints) or "- Follow repository-local instructions and preserve unrelated work."
        return (
            "You are executing a real repository task, not a simulation. Follow every repository-local "
            "agents.md/AGENTS.md instruction. Work only on the assigned branch. Never print, commit, "
            "copy, or expose credentials. Never invent actions, tests, commits, pull requests, evidence, "
            "or outcomes. Do not force-push, rebase, reset, clean, stash, or delete unrelated work.\n\n"
            f"Public task title: {self.display_title}\nSuccess criteria:\n{criteria}\nConstraints:\n{constraints}\n\n"
            "After inspecting the repository, after implementation, and after validation, publish concise "
            "observable progress with `meta-agent-observe progress`. Publish at least one evidence-backed "
            "reflection with `meta-agent-observe reflection`. Report conclusions and evidence references, "
            "never chain-of-thought, private scratchpads, raw prompts, provider transcripts, secrets, or "
            "sensitive tool arguments/results. Run relevant tests, commit focused changes, push the assigned "
            "branch, and open or update its pull request.\n\n"
            f"Private execution task:\n{self.task}\n{continuation}"
        )


def admitted_providers() -> set[str]:
    raw = os.getenv("META_AGENT_PROVIDER_ALLOWLIST", "openai,anthropic")
    values = {item.strip().lower() for item in raw.split(",") if item.strip()}
    invalid = values - PROVIDERS
    if invalid or not values:
        raise ValueError("META_AGENT_PROVIDER_ALLOWLIST contains an unsupported provider")
    return values


def validate_admitted_job(job: Job) -> None:
    if job.provider not in admitted_providers():
        raise ValueError(f"provider {job.provider} is not admitted by this runner")


def read_tail(path: Path) -> str:
    if not path.exists():
        return ""
    with path.open("rb") as handle:
        handle.seek(0, os.SEEK_END)
        size = handle.tell()
        handle.seek(max(0, size - MAX_LOG_TAIL_BYTES))
        return handle.read(MAX_LOG_TAIL_BYTES).decode("utf-8", errors="replace")


def classify_provider_error(text: str, return_code: int) -> str | None:
    if return_code == 0:
        return None
    normalized = text.lower()
    if any(pattern in normalized for pattern in QUOTA_PATTERNS):
        return "quota_exhausted"
    if any(pattern in normalized for pattern in RATE_LIMIT_PATTERNS):
        return "rate_limited"
    if any(pattern in normalized for pattern in ("authentication", "invalid api key", "unauthorized", "forbidden")):
        return "authentication_failed"
    return "provider_process_failed"
