import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts.fleet import dispatcher


class ScopedDispatcherTests(unittest.TestCase):
    def test_default_scope_is_exactly_four_portfolios(self):
        self.assertEqual(
            dispatcher.DEFAULT_ORGS,
            (
                "apostille-me",
                "embedded-alerts",
                "evento-globolo",
                "hacker-house-medellin",
            ),
        )
        self.assertEqual(
            dispatcher.DEFAULT_PROJECTS,
            tuple(f"github.com/{org}" for org in dispatcher.DEFAULT_ORGS),
        )

    def test_stable_routing_and_superseded_markers_are_not_execution_jobs(self):
        self.assertFalse(dispatcher.actionable_title("GitHub Project and Linear workspace links"))
        self.assertFalse(dispatcher.actionable_title("Superseded by canonical short-name repository"))
        self.assertTrue(dispatcher.actionable_title("Create apme-e2e with canonical dependencies"))

    def test_linear_repository_requires_one_unambiguous_match(self):
        repos = {"apme-e2e", "apme-clients", "apme-monorepo"}
        explicit = {
            "title": "Create end to end package",
            "body": "Target https://github.com/apostille-me/apme-e2e and validate it.",
        }
        self.assertEqual(
            dispatcher.resolve_linear_repository(explicit, "apostille-me", repos),
            "apostille-me/apme-e2e",
        )
        named = {"title": "Harden apme-clients", "body": "Run the package tests."}
        self.assertEqual(
            dispatcher.resolve_linear_repository(named, "apostille-me", repos),
            "apostille-me/apme-clients",
        )
        ambiguous = {
            "title": "Align apme-clients and apme-e2e",
            "body": "Cross-package planning task",
        }
        self.assertIsNone(
            dispatcher.resolve_linear_repository(ambiguous, "apostille-me", repos)
        )

    def test_provider_assignment_is_deterministic(self):
        key = "github:apostille-me/apme-monorepo#10"
        provider = dispatcher.provider_for(key)
        self.assertIn(provider, dispatcher.PROVIDERS)
        self.assertEqual(provider, dispatcher.provider_for(key))

    def test_job_is_real_delivery_work_with_source_evidence(self):
        item = {
            "source_key": "github:apostille-me/apme-monorepo#10",
            "version": "2026-08-07T20:00:00Z",
            "source": "github",
            "number": 10,
            "title": "Create apme-e2e",
            "url": "https://github.com/apostille-me/apme-monorepo/issues/10",
            "body": "Acceptance criteria from the real issue.",
        }
        job = dispatcher.job_for(item, "apostille-me/apme-monorepo")
        self.assertEqual(job.repository, "https://github.com/apostille-me/apme-monorepo.git")
        self.assertTrue(job.require_pull_request)
        self.assertTrue(job.require_observation)
        self.assertIn(item["url"], job.task)
        self.assertIn("do not invent success", job.task.lower())

    def test_cycle_queues_github_and_only_resolvable_linear_items(self):
        gh_item = {
            "source_key": "github:apostille-me/apme-monorepo#10",
            "version": "v1",
            "org": "apostille-me",
            "repository": "apostille-me/apme-monorepo",
            "title": "Create apme-e2e",
            "url": "https://github.com/apostille-me/apme-monorepo/issues/10",
            "body": "real issue body",
            "number": 10,
            "source": "github",
        }
        linear_item = {
            "source_key": "linear:DEN-2285",
            "version": "v2",
            "project": "github.com/apostille-me",
            "title": "Create apme-mcp-server.rs canonical Zed package",
            "url": "https://linear.app/denman/issue/DEN-2285/example",
            "body": "Implement apostille-me/apme-mcp-server.rs.",
            "identifier": "DEN-2285",
            "priority": 1,
            "source": "linear",
        }
        unresolved = {
            "source_key": "linear:DEN-X",
            "version": "v3",
            "project": "github.com/apostille-me",
            "title": "Cross-repository planning",
            "url": "https://linear.app/denman/issue/DEN-X/example",
            "body": "No target repository is named.",
            "identifier": "DEN-X",
            "priority": 2,
            "source": "linear",
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            states = {
                "openai": root / "openai",
                "anthropic": root / "anthropic",
            }
            ledger = root / "dispatcher" / "ledger.json"
            with (
                mock.patch.object(
                    dispatcher,
                    "github_repositories",
                    return_value=({"apme-monorepo", "apme-mcp-server.rs"}, ""),
                ),
                mock.patch.object(dispatcher, "github_open_issues", return_value=([gh_item], "")),
                mock.patch.object(
                    dispatcher,
                    "linear_active_issues",
                    return_value=([linear_item, unresolved], ""),
                ),
            ):
                summary = dispatcher.run_cycle(
                    orgs=("apostille-me",),
                    projects=("github.com/apostille-me",),
                    github_token="token",
                    linear_key="linear",
                    state_roots=states,
                    ledger_path=ledger,
                    per_source_limit=10,
                )
            self.assertEqual(summary["queued"], 2)
            queue_files = list((root / "openai" / "queue").glob("*.json")) + list(
                (root / "anthropic" / "queue").glob("*.json")
            )
            self.assertEqual(len(queue_files), 2)
            payloads = [json.loads(path.read_text()) for path in queue_files]
            repos = {payload["repository"] for payload in payloads}
            self.assertEqual(
                repos,
                {
                    "https://github.com/apostille-me/apme-monorepo.git",
                    "https://github.com/apostille-me/apme-mcp-server.rs.git",
                },
            )


if __name__ == "__main__":
    unittest.main()
