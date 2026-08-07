"""Durable supervisor for verified real provider-agent repository work."""

from __future__ import annotations

import asyncio
import contextlib
import dataclasses
import json
import os
import signal
import subprocess
from pathlib import Path
from typing import Any

from .common import (
    MAX_CONCURRENCY_HARD_LIMIT, TERMINAL_STATES, Job, atomic_write_json,
    classify_provider_error, env_bool, env_int, load_json, read_tail, utc_now,
    validate_admitted_job,
)
from .introspection import ProviderDiscoveryError, discover_mcp_servers, discover_provider
from .observability import EventClient, EventIdentity, ObservationError, ObservationSummary, read_observation_summary
from .process import terminate_process_group
from .provider import (
    HookClient, ProviderCircuit, enforce_credential_expiry, provider_api_key,
    provider_command, provider_environment,
)
from .workspace import collect_delivery_evidence, missing_delivery_requirements, prepare_workspace


class FleetRunner:
    def __init__(self, state_dir: Path) -> None:
        self.state_dir = state_dir
        self.queue_dir = state_dir / "queue"
        self.runs_dir = state_dir / "runs"
        self.workspaces_dir = state_dir / "workspaces"
        self.archive_dir = state_dir / "archive"
        self.home_dir = state_dir / "home"
        for path in (self.queue_dir, self.runs_dir, self.workspaces_dir, self.archive_dir, self.home_dir):
            path.mkdir(parents=True, exist_ok=True)
        os.chmod(self.home_dir, 0o700)
        os.environ.setdefault("META_AGENT_RUNNER_HOME", str(self.home_dir))
        self.max_concurrency = env_int("META_AGENT_MAX_CONCURRENCY", 4, 1, MAX_CONCURRENCY_HARD_LIMIT)
        self.poll_seconds = env_int("META_AGENT_QUEUE_POLL_SECONDS", 5, 1, 300)
        self.shutdown_grace = env_int("META_AGENT_SHUTDOWN_GRACE_SECONDS", 20, 1, 300)
        self.resume_paused = env_bool("META_AGENT_RESUME_PAUSED", True)
        self.keep_raw_logs = env_bool("META_AGENT_KEEP_RAW_LOGS", False)
        self.stop_event = asyncio.Event()
        self.hooks = HookClient()
        self.circuit = ProviderCircuit(state_dir)
        self.running: dict[str, asyncio.Task[None]] = {}
        self.reconcile_interrupted_states()

    def install_signal_handlers(self) -> None:
        loop = asyncio.get_running_loop()
        for current_signal in (signal.SIGINT, signal.SIGTERM):
            with contextlib.suppress(NotImplementedError):
                loop.add_signal_handler(current_signal, self.stop_event.set)

    def state_path(self, job_id: str) -> Path:
        return self.runs_dir / job_id / "state.json"

    def load_state(self, job_id: str) -> dict[str, Any] | None:
        path = self.state_path(job_id)
        return load_json(path) if path.exists() else None

    def save_state(self, job: Job, **changes: Any) -> dict[str, Any]:
        previous = self.load_state(job.job_id) or {
            "schema_version": 2,
            "job_id": job.job_id,
            "provider": job.provider,
            "repository": job.repository,
            "base_ref": job.base_ref,
            "branch": job.effective_branch,
            "public_title": job.display_title,
            "attempt": 0,
            "status": "queued",
            "created_at": utc_now(),
        }
        previous.update(changes)
        previous["updated_at"] = utc_now()
        for forbidden in ("task", "prompt", "api_key", "token", "stdout", "stderr", "provider_output"):
            previous.pop(forbidden, None)
        atomic_write_json(self.state_path(job.job_id), previous)
        return previous

    def cleanup_logs(self, *paths: Path) -> None:
        if self.keep_raw_logs:
            return
        for path in paths:
            with contextlib.suppress(OSError):
                path.unlink(missing_ok=True)

    def reconcile_interrupted_states(self) -> None:
        for path in self.runs_dir.glob("*/state.json"):
            try:
                value = load_json(path)
            except (OSError, ValueError, json.JSONDecodeError):
                continue
            if value.get("status") in {"running", "starting", "stopping"}:
                value.update(status="paused", pause_reason="runner_restart_reconciliation", pid=None, updated_at=utc_now())
                atomic_write_json(path, value)

    def load_jobs(self) -> list[tuple[Path, Job]]:
        jobs: list[tuple[Path, Job]] = []
        for path in self.queue_dir.glob("*.json"):
            try:
                job = Job.from_mapping(load_json(path))
                validate_admitted_job(job)
            except (OSError, ValueError, json.JSONDecodeError) as error:
                target = self.archive_dir / "invalid" / path.name
                target.parent.mkdir(parents=True, exist_ok=True)
                path.replace(target)
                atomic_write_json(target.with_suffix(".error.json"), {"error": str(error), "at": utc_now()})
                continue
            state = self.load_state(job.job_id)
            if state and state.get("status") in TERMINAL_STATES:
                self.archive_job(path, state["status"])
                continue
            if state and state.get("status") == "paused" and not self.resume_paused:
                continue
            jobs.append((path, job))
        jobs.sort(key=lambda item: (item[1].priority, item[1].job_id))
        return jobs

    async def run(self) -> None:
        self.install_signal_handlers()
        while not self.stop_event.is_set():
            for queue_path, job in self.load_jobs():
                if self.stop_event.is_set() or len(self.running) >= self.max_concurrency:
                    break
                if job.job_id in self.running or self.circuit.is_blocked(job.provider):
                    continue
                task = asyncio.create_task(self.run_one(queue_path, job), name=f"agent-{job.job_id}")
                self.running[job.job_id] = task
                task.add_done_callback(lambda _task, job_id=job.job_id: self.running.pop(job_id, None))
            try:
                await asyncio.wait_for(self.stop_event.wait(), timeout=self.poll_seconds)
            except TimeoutError:
                pass
        if self.running:
            await asyncio.gather(*self.running.values(), return_exceptions=True)

    def observation_ledger(self, job: Job) -> Path:
        return self.runs_dir / job.job_id / "public-events.jsonl"

    def event_client(self, job: Job) -> EventClient:
        identity = EventIdentity(
            agent_id=f"fleet-{job.provider}-{job.job_id}", provider=job.provider,
            model=job.model or "provider-default", instance_id=job.job_id,
            session_id=job.job_id, correlation_id=job.job_id, task_id=job.job_id,
            ledger_path=self.observation_ledger(job),
        )
        return EventClient(identity)

    async def run_one(self, queue_path: Path, job: Job) -> None:
        state = self.load_state(job.job_id) or {}
        attempt = int(state.get("attempt", 0)) + 1
        if attempt > job.max_attempts:
            self.save_state(job, status="partial", error_class="attempts_exhausted", pid=None)
            self.archive_job(queue_path, "partial")
            return
        resumed = state.get("status") == "paused" or attempt > 1
        events = self.event_client(job)
        provider_snapshot = None
        mcp_snapshots = ()
        try:
            enforce_credential_expiry()
            provider_snapshot = await asyncio.to_thread(discover_provider, job.provider, provider_api_key(job.provider))
            mcp_snapshots = await asyncio.to_thread(discover_mcp_servers)
            workspace = await asyncio.to_thread(prepare_workspace, self.workspaces_dir, job, resumed)
            command = provider_command(job.provider)
            child_env = provider_environment(job.provider, job, self.observation_ledger(job))
            capabilities = list(provider_snapshot.capability_labels)
            metadata = provider_snapshot.public_metadata()
            for snapshot in mcp_snapshots:
                capabilities.extend(snapshot.capability_labels)
                metadata.update(snapshot.public_metadata())
            events.bootstrap(job, capabilities, metadata)
        except ProviderDiscoveryError as error:
            status = "paused" if error.recoverable else "failed"
            self.save_state(job, status=status, attempt=attempt, error_class=error.kind, error_summary=str(error), pid=None)
            with contextlib.suppress(ObservationError):
                events.error(error.kind, str(error), recoverable=error.recoverable, proposed_recovery="Retry after provider access is restored.")
                events.status("blocked" if error.recoverable else "failed", str(error))
            if not error.recoverable:
                self.archive_job(queue_path, "failed")
            return
        except (OSError, ValueError, subprocess.SubprocessError, ObservationError) as error:
            self.save_state(job, status="paused", attempt=attempt, pause_reason="configuration_error", error_summary=type(error).__name__, pid=None)
            self.hooks.emit(job, "error_observed", "Provider worker configuration or event ingestion is unavailable")
            return

        log_dir = self.runs_dir / job.job_id
        log_dir.mkdir(parents=True, exist_ok=True)
        stdout_path = log_dir / f"attempt-{attempt}.stdout.log"
        stderr_path = log_dir / f"attempt-{attempt}.stderr.log"
        self.save_state(
            job, status="starting", attempt=attempt, pid=None,
            provider_api_status=provider_snapshot.api_status if provider_snapshot else None,
            provider_model_count=provider_snapshot.model_count if provider_snapshot else None,
        )
        self.hooks.emit(job, "session_started", "Provider worker started an isolated real repository run")

        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            os.chmod(stdout_path, 0o600)
            os.chmod(stderr_path, 0o600)
            process = await asyncio.create_subprocess_exec(
                *command, cwd=workspace, env=child_env, stdin=asyncio.subprocess.PIPE,
                stdout=stdout, stderr=stderr, start_new_session=True,
            )
            self.save_state(job, status="running", attempt=attempt, pid=process.pid)
            self.hooks.emit(job, "activity", "Provider worker is executing the assigned real repository task", process.pid)
            with contextlib.suppress(ObservationError):
                events.status("running", "Provider process is active in the assigned repository worktree.")
            assert process.stdin is not None
            process.stdin.write(job.safe_prompt(resumed).encode("utf-8"))
            await process.stdin.drain()
            process.stdin.close()
            wait_task = asyncio.create_task(process.wait())
            stop_task = asyncio.create_task(self.stop_event.wait())
            done, pending = await asyncio.wait(
                {wait_task, stop_task}, timeout=job.timeout_seconds, return_when=asyncio.FIRST_COMPLETED,
            )
            for pending_task in pending:
                pending_task.cancel()
            if stop_task in done and self.stop_event.is_set():
                await terminate_process_group(process, self.shutdown_grace)
                self.cleanup_logs(stdout_path, stderr_path)
                self.save_state(job, status="paused", pause_reason="runner_shutdown", pid=None)
                self.hooks.emit(job, "session_finished", "Provider worker paused for durable restart")
                with contextlib.suppress(ObservationError):
                    events.status("waiting", "Runner shutdown preserved the real task worktree for resumption.")
                return
            if not done:
                await terminate_process_group(process, self.shutdown_grace)
                self.cleanup_logs(stdout_path, stderr_path)
                self.save_state(job, status="queued", error_class="timeout", pid=None)
                self.hooks.emit(job, "error_observed", "Provider worker exceeded its bounded run timeout")
                with contextlib.suppress(ObservationError):
                    events.error("timeout", "Provider process exceeded the bounded task timeout.", recoverable=True, proposed_recovery="Resume from the preserved branch and worktree.")
                return
            return_code = wait_task.result()

        output_tail = read_tail(stdout_path) + "\n" + read_tail(stderr_path)
        error_class = classify_provider_error(output_tail, return_code)
        self.cleanup_logs(stdout_path, stderr_path)
        if return_code == 0:
            evidence = await asyncio.to_thread(collect_delivery_evidence, workspace, job)
            try:
                observations = read_observation_summary(self.observation_ledger(job))
            except (OSError, ValueError, json.JSONDecodeError, ObservationError):
                observations = ObservationSummary({}, 0, None)
            missing = missing_delivery_requirements(job, evidence, observations)
            evidence_state = dataclasses.asdict(evidence)
            if not missing:
                self.circuit.clear(job.provider)
                self.save_state(
                    job, status="succeeded", exit_code=0, pid=None, error_class=None,
                    delivery=evidence_state, observation_counts=observations.counts,
                    test_evidence_count=observations.test_evidence_count,
                )
                with contextlib.suppress(ObservationError):
                    events.completed(
                        "succeeded", "The real repository task satisfied every independently verified delivery requirement.",
                        artifacts=evidence.artifacts, actual_result=evidence.actual_result(observations),
                    )
                    events.status("completed", "Independent repository and observation checks passed.")
                self.hooks.emit(job, "session_finished", "Provider worker completed a verified real repository run")
                self.archive_job(queue_path, "succeeded")
                return
            next_status = "queued" if attempt < job.max_attempts else "partial"
            self.save_state(
                job, status=next_status, exit_code=0, pid=None, error_class="delivery_contract_incomplete",
                missing_requirements=list(missing), delivery=evidence_state,
                observation_counts=observations.counts, test_evidence_count=observations.test_evidence_count,
            )
            with contextlib.suppress(ObservationError):
                events.error(
                    "delivery_contract_incomplete",
                    f"Provider exited successfully but independent delivery checks are incomplete: {', '.join(missing)}.",
                    recoverable=next_status == "queued",
                    proposed_recovery="Resume the preserved branch and complete the missing evidence or repository artifacts.",
                )
                if next_status == "partial":
                    events.completed(
                        "partial", "The provider process ended, but the independently verified delivery contract was incomplete.",
                        artifacts=evidence.artifacts, actual_result=evidence.actual_result(observations),
                    )
                    events.status("failed", "Delivery remained incomplete after all bounded attempts.")
            if next_status == "partial":
                self.archive_job(queue_path, "partial")
            return

        if error_class in {"quota_exhausted", "rate_limited"}:
            seconds = env_int(
                f"META_AGENT_{job.provider.upper()}_RETRY_SECONDS",
                6 * 3600 if job.provider == "anthropic" else 1800, 60, 7 * 24 * 3600,
            )
            self.circuit.block(job.provider, error_class, seconds)
            self.save_state(job, status="paused", pause_reason=error_class, pid=None, exit_code=return_code)
            with contextlib.suppress(ObservationError):
                events.error(error_class, "Provider is temporarily unavailable; repository work is checkpointed.", recoverable=True, proposed_recovery="Retry after the provider circuit reopens.")
            return
        next_status = "queued" if attempt < job.max_attempts and error_class != "authentication_failed" else "failed"
        self.save_state(job, status=next_status, exit_code=return_code, pid=None, error_class=error_class)
        with contextlib.suppress(ObservationError):
            events.error(error_class or "provider_process_failed", "Provider process exited before verified delivery.", recoverable=next_status == "queued", proposed_recovery="Resume the preserved repository worktree." if next_status == "queued" else None)
            if next_status == "failed":
                events.completed("failed", "The provider process failed before satisfying the verified delivery contract.")
                events.status("failed", "Provider execution failed.")
        if next_status == "failed":
            self.archive_job(queue_path, "failed")

    def archive_job(self, queue_path: Path, status: str) -> None:
        target = self.archive_dir / status / queue_path.name
        target.parent.mkdir(parents=True, exist_ok=True)
        if queue_path.exists():
            queue_path.replace(target)
