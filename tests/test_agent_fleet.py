from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import fleet as agent_fleet


class JobTests(unittest.TestCase):
    def valid(self):
        return {
            "job_id": "hhm-e2e-contracts",
            "provider": "openai",
            "repository": "https://github.com/hacker-house-medellin/hhm-e2e.git",
            "task": "Add bounded API and browser contract tests.",
        }

    def test_valid_job_gets_safe_branch_and_prompt(self):
        job = agent_fleet.Job.from_mapping(self.valid())
        self.assertEqual(job.effective_branch, "agent/hhm-e2e-contracts")
        self.assertIn("Never print, commit, or copy credentials", job.safe_prompt(False))

    def test_unknown_fields_fail_closed(self):
        value = self.valid()
        value["api_key"] = "never-allowed"
        with self.assertRaisesRegex(ValueError, "unknown job fields"):
            agent_fleet.Job.from_mapping(value)

    def test_unsupported_provider_is_rejected(self):
        value = self.valid()
        value["provider"] = "browser-session"
        with self.assertRaisesRegex(ValueError, "provider must be"):
            agent_fleet.Job.from_mapping(value)

    def test_task_is_bounded(self):
        value = self.valid()
        value["task"] = "x" * (agent_fleet.MAX_TASK_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "task exceeds"):
            agent_fleet.Job.from_mapping(value)

    def test_unsafe_git_refs_are_rejected(self):
        value = self.valid()
        value["branch"] = "agent/../../main"
        with self.assertRaisesRegex(ValueError, "unsafe git-ref"):
            agent_fleet.Job.from_mapping(value)


class PersistenceTests(unittest.TestCase):
    def test_atomic_ledger_is_mode_600_and_strips_sensitive_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.dict(os.environ, {"META_AGENT_HOOKS_ENABLED": "false"}, clear=False):
                runner = agent_fleet.FleetRunner(Path(directory))
            job = agent_fleet.Job.from_mapping({
                "job_id": "audit-1", "provider": "openai",
                "repository": "https://github.com/example/repo.git", "task": "Audit it",
            })
            runner.save_state(job, status="running", api_key="secret", prompt="private")
            path = runner.state_path(job.job_id)
            value = json.loads(path.read_text())
            self.assertNotIn("api_key", value)
            self.assertNotIn("prompt", value)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_reconcile_marks_interrupted_run_paused(self):
        with tempfile.TemporaryDirectory() as directory:
            state_dir = Path(directory)
            state_path = state_dir / "runs" / "audit-1" / "state.json"
            agent_fleet.atomic_write_json(state_path, {
                "job_id": "audit-1", "status": "running", "pid": 999, "updated_at": "old",
            })
            with mock.patch.dict(os.environ, {"META_AGENT_HOOKS_ENABLED": "false"}, clear=False):
                agent_fleet.FleetRunner(state_dir)
            value = json.loads(state_path.read_text())
            self.assertEqual(value["status"], "paused")
            self.assertEqual(value["pause_reason"], "runner_restart_reconciliation")
            self.assertIsNone(value["pid"])

    def test_concurrency_is_hard_clamped_to_fifteen(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.dict(os.environ, {
                "META_AGENT_MAX_CONCURRENCY": "99", "META_AGENT_HOOKS_ENABLED": "false",
            }, clear=False):
                runner = agent_fleet.FleetRunner(Path(directory))
            self.assertEqual(runner.max_concurrency, 15)

    def test_enqueue_never_copies_task_into_state_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            job_path = root / "job.json"
            job_path.write_text(json.dumps({
                "job_id": "audit-1", "provider": "openai",
                "repository": "https://github.com/example/repo.git", "task": "Audit it",
            }))
            queued = agent_fleet.enqueue(root / "state", job_path)
            self.assertTrue(queued.exists())
            self.assertFalse((root / "state" / "runs" / "audit-1" / "state.json").exists())


class ProviderTests(unittest.TestCase):
    def test_quota_and_rate_limit_classification(self):
        self.assertEqual(agent_fleet.classify_provider_error("insufficient_quota", 1), "quota_exhausted")
        self.assertEqual(agent_fleet.classify_provider_error("HTTP 429", 1), "rate_limited")
        self.assertEqual(agent_fleet.classify_provider_error("invalid api key", 1), "authentication_failed")
        self.assertIsNone(agent_fleet.classify_provider_error("ok", 0))

    def test_expired_temporary_credentials_are_rejected(self):
        with mock.patch.dict(os.environ, {"META_AGENT_CREDENTIAL_EXPIRES_AT": "2000-01-01T00:00:00Z"}, clear=False):
            with self.assertRaisesRegex(ValueError, "expired"):
                agent_fleet.enforce_credential_expiry()

    def test_child_home_is_isolated_and_owner_only(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.dict(os.environ, {"META_AGENT_RUNNER_HOME": str(Path(directory) / "home")}, clear=False):
                environment = agent_fleet.isolated_child_environment()
            home = Path(environment["HOME"])
            self.assertEqual(home.stat().st_mode & 0o777, 0o700)
            self.assertEqual(environment["XDG_CONFIG_HOME"], str(home / ".config"))

    def test_provider_environment_only_exposes_selected_provider(self):
        with tempfile.TemporaryDirectory() as directory:
            openai = Path(directory) / "openai"
            anthropic = Path(directory) / "anthropic"
            github = Path(directory) / "github"
            openai.write_text("openai-test")
            anthropic.write_text("anthropic-test")
            github.write_text("github-test")
            with mock.patch.dict(os.environ, {
                "OPENAI_API_KEY_FILE": str(openai), "ANTHROPIC_API_KEY_FILE": str(anthropic),
                "GH_TOKEN_FILE": str(github), "META_AGENT_RUNNER_HOME": str(Path(directory) / "home"),
            }, clear=False):
                environment = agent_fleet.provider_environment("openai")
            self.assertEqual(environment["OPENAI_API_KEY"], "openai-test")
            self.assertNotIn("ANTHROPIC_API_KEY", environment)
            self.assertEqual(environment["GH_TOKEN"], "github-test")


if __name__ == "__main__":
    unittest.main()
