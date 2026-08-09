"""Privacy-bounded canonical task observations for admitted real repository runs."""

from __future__ import annotations

import argparse
import dataclasses
import json
import math
import os
import re
import stat
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Iterable

from .common import Job, env_bool, env_int, read_secret, utc_now

MAX_PUBLIC_TEXT_BYTES = 16 * 1024
SECRET_PATTERNS = (
    re.compile(r"sk-(?:svcacct|ant-api|proj)-[A-Za-z0-9_-]{16,}", re.I),
    re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}", re.I),
    re.compile(r"lin_api_[A-Za-z0-9]{16,}", re.I),
    re.compile(r"authorization\s*:\s*bearer\s+\S+", re.I),
    re.compile(r"(?:api[_ -]?key|token|secret)\s*[=:]\s*\S{12,}", re.I),
)
PRIVATE_REASONING_PATTERNS = (
    re.compile(r"</?(?:thinking|analysis|scratchpad|chain[-_ ]?of[-_ ]?thought)>?", re.I),
    re.compile(r'"(?:thinking|analysis|scratchpad|chain_of_thought)"\s*:', re.I),
)


class ObservationError(ValueError):
    pass


def public_text(value: str, field: str, maximum: int = MAX_PUBLIC_TEXT_BYTES) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ObservationError(f"{field} must be non-empty")
    candidate = value.strip()
    if len(candidate.encode("utf-8")) > maximum:
        raise ObservationError(f"{field} exceeds its public size limit")
    if any(ord(char) < 32 and char not in "\n\r\t" for char in candidate):
        raise ObservationError(f"{field} contains control characters")
    if any(pattern.search(candidate) for pattern in SECRET_PATTERNS):
        raise ObservationError(f"{field} contains credential-shaped material")
    if any(pattern.search(candidate) for pattern in PRIVATE_REASONING_PATTERNS):
        raise ObservationError(f"{field} contains private reasoning material")
    return candidate


def _public_list(values: Iterable[str] | None, field: str, maximum_values: int = 64) -> list[str]:
    if values is None:
        return []
    result = [public_text(value, field, 4_096) for value in values]
    if len(result) > maximum_values:
        raise ObservationError(f"{field} contains too many values")
    return result


def _append_jsonl(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, stat.S_IRUSR | stat.S_IWUSR)
    try:
        with os.fdopen(fd, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
    except Exception:
        raise


@dataclasses.dataclass(frozen=True)
class EventIdentity:
    agent_id: str
    provider: str
    model: str
    instance_id: str
    session_id: str
    correlation_id: str
    task_id: str
    ledger_path: Path | None = None

    @classmethod
    def from_environment(cls) -> "EventIdentity":
        if os.getenv("META_AGENT_REAL_TASK", "").lower() != "true":
            raise ObservationError("observer is available only inside an admitted real task")
        required = {
            name: os.getenv(name, "").strip()
            for name in (
                "META_AGENT_AGENT_ID", "META_AGENT_PROVIDER", "META_AGENT_MODEL",
                "META_AGENT_INSTANCE_ID", "META_AGENT_SESSION_ID", "META_AGENT_CORRELATION_ID",
                "META_AGENT_TASK_ID",
            )
        }
        missing = [name for name, value in required.items() if not value]
        if missing:
            raise ObservationError(f"missing observer identity: {', '.join(missing)}")
        ledger = os.getenv("META_AGENT_OBSERVATION_LEDGER", "").strip()
        return cls(
            agent_id=required["META_AGENT_AGENT_ID"],
            provider=required["META_AGENT_PROVIDER"],
            model=required["META_AGENT_MODEL"],
            instance_id=required["META_AGENT_INSTANCE_ID"],
            session_id=required["META_AGENT_SESSION_ID"],
            correlation_id=required["META_AGENT_CORRELATION_ID"],
            task_id=required["META_AGENT_TASK_ID"],
            ledger_path=Path(ledger) if ledger else None,
        )


@dataclasses.dataclass(frozen=True)
class ObservationSummary:
    counts: dict[str, int]
    test_evidence_count: int
    last_reflection_summary: str | None

    @property
    def progress_count(self) -> int:
        return self.counts.get("progress_updated", 0)

    @property
    def reflection_count(self) -> int:
        return self.counts.get("reflection_recorded", 0)


class EventClient:
    def __init__(
        self,
        identity: EventIdentity,
        *,
        base_url: str | None = None,
        token: str | None = None,
        urlopen=urllib.request.urlopen,
    ) -> None:
        self.identity = identity
        self.base_url = (base_url or os.getenv("META_AGENT_RUNTIME_URL", "http://control-plane:8787")).rstrip("/")
        self.token = token if token is not None else read_secret("META_AGENT_AUTH_TOKEN_FILE", "META_AGENT_AUTH_TOKEN")
        self.urlopen = urlopen
        self.sequence = 0

    def _event(self, kind: str, data: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        return {
            "protocol_version": "v1",
            "event_id": str(uuid.uuid4()),
            "occurred_at": utc_now(),
            "agent": {
                "agent_id": self.identity.agent_id,
                "provider": self.identity.provider,
                "model": self.identity.model,
                "instance_id": self.identity.instance_id,
            },
            "session_id": self.identity.session_id,
            "correlation_id": self.identity.correlation_id,
            "sequence": self.sequence,
            "kind": kind,
            "data": data,
        }

    def emit(self, kind: str, data: dict[str, Any], *, record_local: bool = False) -> str:
        event = self._event(kind, data)
        enabled = env_bool("META_AGENT_EVENTS_ENABLED", True)
        required = env_bool("META_AGENT_REQUIRE_EVENT_INGEST", True)
        if enabled:
            if not self.token:
                if required:
                    raise ObservationError("control-plane event token is not configured")
            else:
                request = urllib.request.Request(
                    f"{self.base_url}/api/v1/events",
                    data=json.dumps(event, separators=(",", ":")).encode("utf-8"),
                    headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"},
                    method="POST",
                )
                try:
                    with self.urlopen(request, timeout=env_int("META_AGENT_EVENT_TIMEOUT_SECONDS", 10, 2, 60)) as response:
                        response.read(4_096)
                        if int(getattr(response, "status", 202)) not in {200, 202}:
                            raise ObservationError("control-plane rejected event")
                except (OSError, urllib.error.URLError, urllib.error.HTTPError) as error:
                    if required:
                        raise ObservationError("control-plane event ingestion failed") from error
        if record_local:
            if not self.identity.ledger_path:
                raise ObservationError("local observation ledger is not configured")
            _append_jsonl(self.identity.ledger_path, event)
        return event["event_id"]

    def bootstrap(self, job: Job, capabilities: Iterable[str], metadata: dict[str, str]) -> None:
        self.emit("agent_registered", {
            "display_name": f"{job.provider} real-task worker",
            "capabilities": sorted(set(capabilities))[:128],
            "metadata": {key: public_text(str(value), f"metadata.{key}", 2_048) for key, value in metadata.items()},
            "status": "planning",
        })
        self.emit("goal_declared", {
            "goal_id": f"goal-{job.job_id}",
            "title": public_text(job.display_title, "goal.title", 2_048),
            "success_criteria": _public_list(job.success_criteria or ("Deliver independently verified repository artifacts.",), "success_criteria"),
            "constraints": _public_list(job.constraints, "constraints"),
        })
        self.emit("task_created", {
            "task_id": self.identity.task_id,
            "title": public_text(job.display_title, "task.title", 2_048),
            "goal_id": f"goal-{job.job_id}",
            "depends_on": [],
            "tags": ["real-repository-task", job.provider],
            "expected_outcome": "A clean pushed branch, independently verified pull request, public progress, reflection, and evidence.",
        })
        self.emit("task_started", {
            "task_id": self.identity.task_id,
            "attempt": 1,
            "plan_summary": "Inspect the live repository, implement the requested change, validate it, and publish independently verifiable artifacts.",
        })

    def progress(self, progress: float, summary: str, *, blocker: str | None = None, next_action: str | None = None, record_local: bool = False) -> str:
        if not math.isfinite(progress) or not 0.0 <= progress <= 1.0:
            raise ObservationError("progress must be between zero and one")
        data: dict[str, Any] = {
            "task_id": self.identity.task_id,
            "progress": progress,
            "summary": public_text(summary, "summary"),
        }
        if blocker:
            data["blocker"] = public_text(blocker, "blocker", 8_192)
        if next_action:
            data["next_action"] = public_text(next_action, "next_action", 8_192)
        return self.emit("progress_updated", data, record_local=record_local)

    def reflection(
        self,
        summary: str,
        confidence: float,
        *,
        assumptions: Iterable[str] | None = None,
        evidence: Iterable[dict[str, str]] | None = None,
        alternatives: Iterable[str] | None = None,
        risks: Iterable[str] | None = None,
        next_action: str | None = None,
        record_local: bool = False,
    ) -> str:
        if not math.isfinite(confidence) or not 0.0 <= confidence <= 1.0:
            raise ObservationError("confidence must be between zero and one")
        public_evidence: list[dict[str, str]] = []
        for item in evidence or ():
            if not isinstance(item, dict) or not item.get("kind") or not item.get("reference"):
                raise ObservationError("evidence requires kind and reference")
            current = {
                "kind": public_text(item["kind"], "evidence.kind", 128),
                "reference": public_text(item["reference"], "evidence.reference", 4_096),
            }
            if item.get("summary"):
                current["summary"] = public_text(item["summary"], "evidence.summary", 4_096)
            public_evidence.append(current)
        data: dict[str, Any] = {
            "task_id": self.identity.task_id,
            "summary": public_text(summary, "reflection.summary"),
            "confidence": confidence,
            "assumptions": _public_list(assumptions, "assumptions"),
            "evidence": public_evidence,
            "alternatives_considered": _public_list(alternatives, "alternatives"),
            "risks": _public_list(risks, "risks"),
        }
        if next_action:
            data["next_action"] = public_text(next_action, "next_action", 8_192)
        return self.emit("reflection_recorded", data, record_local=record_local)

    def lesson(self, lesson_id: str, statement: str, confidence: float, *, evidence: Iterable[dict[str, str]] | None = None, tags: Iterable[str] | None = None, applicability: str | None = None, record_local: bool = False) -> str:
        if not 0.0 <= confidence <= 1.0:
            raise ObservationError("confidence must be between zero and one")
        public_evidence = []
        for item in evidence or ():
            public_evidence.append({
                "kind": public_text(item["kind"], "evidence.kind", 128),
                "reference": public_text(item["reference"], "evidence.reference", 4_096),
                **({"summary": public_text(item["summary"], "evidence.summary", 4_096)} if item.get("summary") else {}),
            })
        data: dict[str, Any] = {
            "lesson_id": public_text(lesson_id, "lesson_id", 256),
            "statement": public_text(statement, "lesson.statement"),
            "confidence": confidence,
            "source_task_id": self.identity.task_id,
            "evidence": public_evidence,
            "tags": _public_list(tags, "tags", 128),
        }
        if applicability:
            data["applicability"] = public_text(applicability, "applicability", 8_192)
        return self.emit("lesson_learned", data, record_local=record_local)

    def error(self, code: str, message: str, *, recoverable: bool, proposed_recovery: str | None = None, record_local: bool = False) -> str:
        data: dict[str, Any] = {
            "task_id": self.identity.task_id,
            "code": public_text(code, "error.code", 256),
            "message": public_text(message, "error.message"),
            "recoverable": bool(recoverable),
        }
        if proposed_recovery:
            data["proposed_recovery"] = public_text(proposed_recovery, "proposed_recovery", 8_192)
        return self.emit("error_observed", data, record_local=record_local)

    def status(self, status: str, reason: str | None = None) -> str:
        if status not in {"idle", "planning", "running", "waiting", "blocked", "completed", "failed", "offline"}:
            raise ObservationError("invalid agent status")
        data: dict[str, Any] = {"status": status}
        if reason:
            data["reason"] = public_text(reason, "status.reason", 4_096)
        return self.emit("agent_status_changed", data)

    def completed(self, outcome: str, summary: str, *, artifacts: Iterable[str] = (), actual_result: str | None = None) -> str:
        if outcome not in {"succeeded", "failed", "canceled", "partial"}:
            raise ObservationError("invalid task outcome")
        data: dict[str, Any] = {
            "task_id": self.identity.task_id,
            "outcome": outcome,
            "summary": public_text(summary, "completion.summary"),
            "artifacts": _public_list(artifacts, "artifacts", 256),
        }
        if actual_result:
            data["actual_result"] = public_text(actual_result, "actual_result")
        return self.emit("task_completed", data)


def read_observation_summary(path: Path) -> ObservationSummary:
    counts: dict[str, int] = {}
    tests = 0
    last_reflection: str | None = None
    if not path.exists():
        return ObservationSummary(counts, tests, last_reflection)
    if path.stat().st_size > 4 * 1024 * 1024:
        raise ObservationError("observation ledger exceeds its size limit")
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        value = json.loads(line)
        if not isinstance(value, dict):
            raise ObservationError("observation ledger contains invalid JSON")
        kind = value.get("kind")
        data = value.get("data")
        if not isinstance(kind, str) or not isinstance(data, dict):
            raise ObservationError("observation ledger contains an invalid event")
        counts[kind] = counts.get(kind, 0) + 1
        if kind == "reflection_recorded":
            last_reflection = str(data.get("summary"))[:MAX_PUBLIC_TEXT_BYTES] if data.get("summary") else None
            for item in data.get("evidence", []):
                if isinstance(item, dict) and str(item.get("kind", "")).lower() in {"test", "ci", "validation"}:
                    tests += 1
    return ObservationSummary(counts, tests, last_reflection)


def _parse_evidence(values: list[str]) -> list[dict[str, str]]:
    result = []
    for value in values:
        kind, separator, rest = value.partition("=")
        if not separator:
            raise ObservationError("evidence must use KIND=REFERENCE or KIND=REFERENCE::SUMMARY")
        reference, summary_separator, summary = rest.partition("::")
        item = {"kind": kind, "reference": reference}
        if summary_separator:
            item["summary"] = summary
        result.append(item)
    return result


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Publish privacy-bounded observations from an admitted real task")
    subparsers = parser.add_subparsers(dest="command", required=True)
    progress = subparsers.add_parser("progress")
    progress.add_argument("--progress", type=float, required=True)
    progress.add_argument("--summary", required=True)
    progress.add_argument("--blocker")
    progress.add_argument("--next-action")
    reflection = subparsers.add_parser("reflection")
    reflection.add_argument("--confidence", type=float, required=True)
    reflection.add_argument("--summary", required=True)
    reflection.add_argument("--assumption", action="append", default=[])
    reflection.add_argument("--evidence", action="append", default=[])
    reflection.add_argument("--alternative", action="append", default=[])
    reflection.add_argument("--risk", action="append", default=[])
    reflection.add_argument("--next-action")
    lesson = subparsers.add_parser("lesson")
    lesson.add_argument("--lesson-id", required=True)
    lesson.add_argument("--statement", required=True)
    lesson.add_argument("--confidence", type=float, required=True)
    lesson.add_argument("--evidence", action="append", default=[])
    lesson.add_argument("--tag", action="append", default=[])
    lesson.add_argument("--applicability")
    error = subparsers.add_parser("error")
    error.add_argument("--code", required=True)
    error.add_argument("--message", required=True)
    error.add_argument("--recoverable", action="store_true")
    error.add_argument("--proposed-recovery")
    status = subparsers.add_parser("status")
    status.add_argument("--status", required=True)
    status.add_argument("--reason")
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    client = EventClient(EventIdentity.from_environment())
    if args.command == "progress":
        client.progress(args.progress, args.summary, blocker=args.blocker, next_action=args.next_action, record_local=True)
    elif args.command == "reflection":
        client.reflection(
            args.summary, args.confidence, assumptions=args.assumption, evidence=_parse_evidence(args.evidence),
            alternatives=args.alternative, risks=args.risk, next_action=args.next_action, record_local=True,
        )
    elif args.command == "lesson":
        client.lesson(
            args.lesson_id, args.statement, args.confidence, evidence=_parse_evidence(args.evidence),
            tags=args.tag, applicability=args.applicability, record_local=True,
        )
    elif args.command == "error":
        client.error(args.code, args.message, recoverable=args.recoverable, proposed_recovery=args.proposed_recovery, record_local=True)
    else:
        client.status(args.status, args.reason)
    return 0
