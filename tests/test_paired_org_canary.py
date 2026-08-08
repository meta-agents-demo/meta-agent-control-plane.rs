import unittest
from unittest import mock

from scripts.fleet import canary, dispatcher


class PairedOrgCanaryTests(unittest.TestCase):
    def test_scope_is_exactly_the_four_paired_test_portfolios(self):
        self.assertEqual(
            canary.TEST_ORGS,
            (
                "apostille-me-test",
                "embedded-alerts-test",
                "evento-globolo-test",
                "hacker-house-medellin-test",
            ),
        )
        self.assertEqual(
            canary.TEST_PROJECTS,
            tuple(f"github.com/{org}" for org in canary.TEST_ORGS),
        )

    def test_only_explicit_canaries_are_admitted(self):
        self.assertTrue(
            canary.is_canary(
                {"title": "[meta-agent-canary] validate test-org dispatch"}
            )
        )
        self.assertFalse(canary.is_canary({"title": "ordinary test-fleet work"}))
        self.assertFalse(canary.is_canary({"title": ""}))

    def test_live_shaped_discovery_validates_jobs_without_queue_or_ledger_writes(self):
        inventories = {
            "apostille-me-test": set(),
            "embedded-alerts-test": {".github", "web-ui-e2e"},
            "evento-globolo-test": {".github", "api-contract-e2e", "mash-web-e2e"},
            "hacker-house-medellin-test": set(),
        }
        github_items = {
            "apostille-me-test": [],
            "embedded-alerts-test": [
                {
                    "source_key": "github:embedded-alerts-test/web-ui-e2e#7",
                    "version": "2026-08-08T17:41:36Z",
                    "org": "embedded-alerts-test",
                    "repository": "embedded-alerts-test/web-ui-e2e",
                    "title": "[meta-agent-canary] validate scoped issue discovery and public introspection",
                    "url": "https://github.com/embedded-alerts-test/web-ui-e2e/issues/7",
                    "body": "Test-only acceptance criteria.",
                    "number": 7,
                    "source": "github",
                },
                {
                    "source_key": "github:embedded-alerts-test/.github#5",
                    "version": "2026-08-08T16:00:00Z",
                    "org": "embedded-alerts-test",
                    "repository": "embedded-alerts-test/.github",
                    "title": "GitHub Project and Linear workspace links",
                    "url": "https://github.com/embedded-alerts-test/.github/issues/5",
                    "body": "Permanent routing card.",
                    "number": 5,
                    "source": "github",
                },
            ],
            "evento-globolo-test": [
                {
                    "source_key": "github:evento-globolo-test/api-contract-e2e#5",
                    "version": "2026-08-08T17:41:46Z",
                    "org": "evento-globolo-test",
                    "repository": "evento-globolo-test/api-contract-e2e",
                    "title": "[meta-agent-canary] validate test-org dispatch and verified event bridge",
                    "url": "https://github.com/evento-globolo-test/api-contract-e2e/issues/5",
                    "body": "Test-only acceptance criteria.",
                    "number": 5,
                    "source": "github",
                },
                {
                    "source_key": "github:evento-globolo-test/mash-web-e2e#5",
                    "version": "2026-08-08T16:30:00Z",
                    "org": "evento-globolo-test",
                    "repository": "evento-globolo-test/mash-web-e2e",
                    "title": "Port provider/WebSocket hardening coverage onto the canonical harness",
                    "url": "https://github.com/evento-globolo-test/mash-web-e2e/issues/5",
                    "body": "Real ordinary maintenance, not this canary.",
                    "number": 5,
                    "source": "github",
                },
            ],
            "hacker-house-medellin-test": [],
        }
        linear_items = [
            {
                "source_key": "linear:DEN-CANARY",
                "version": "2026-08-08T17:45:00Z",
                "project": "github.com/embedded-alerts-test",
                "title": "[meta-agent-canary] validate web-ui-e2e repository routing",
                "url": "https://linear.app/example/DEN-CANARY",
                "body": "Target https://github.com/embedded-alerts-test/web-ui-e2e.",
                "identifier": "DEN-CANARY",
                "priority": 2,
                "source": "linear",
            },
            {
                "source_key": "linear:DEN-ORDINARY",
                "version": "2026-08-08T17:44:00Z",
                "project": "github.com/evento-globolo-test",
                "title": "Upgrade ordinary browser fixtures",
                "url": "https://linear.app/example/DEN-ORDINARY",
                "body": "Target evento-globolo-test/mash-web-e2e.",
                "identifier": "DEN-ORDINARY",
                "priority": 3,
                "source": "linear",
            },
        ]

        def repositories(org, _token):
            return inventories[org], ""

        def issues(org, _token, _limit):
            return github_items[org], ""

        with (
            mock.patch.object(dispatcher, "github_repositories", side_effect=repositories),
            mock.patch.object(dispatcher, "github_open_issues", side_effect=issues),
            mock.patch.object(
                dispatcher,
                "linear_active_issues",
                return_value=(linear_items, ""),
            ),
            mock.patch.object(
                dispatcher,
                "queue_job",
                side_effect=AssertionError("canary must never queue mutation-capable work"),
            ),
            mock.patch.object(
                dispatcher,
                "atomic_write_json",
                side_effect=AssertionError("canary must never write the production ledger"),
            ),
        ):
            report = canary.discover(
                github_token="test-token",
                linear_key="linear-token",
            )

        self.assertTrue(report["dry_run"])
        self.assertFalse(report["mutation_enabled"])
        self.assertEqual(report["queue_writes"], 0)
        self.assertEqual(report["ledger_writes"], 0)
        self.assertEqual(report["github_canaries"], 2)
        self.assertEqual(report["linear_canaries"], 1)
        self.assertEqual(report["validated_jobs"], 3)
        self.assertEqual(report["errors"], [])
        self.assertEqual(report["org_counts"]["apostille-me-test"]["repositories"], 0)
        self.assertEqual(
            report["org_counts"]["hacker-house-medellin-test"]["repositories"],
            0,
        )

        repositories = {job["repository"] for job in report["jobs"]}
        self.assertEqual(
            repositories,
            {
                "embedded-alerts-test/web-ui-e2e",
                "evento-globolo-test/api-contract-e2e",
            },
        )
        for job in report["jobs"]:
            self.assertTrue(job["validated"])
            self.assertIn(job["provider"], dispatcher.PROVIDERS)
            self.assertTrue(job["branch"].startswith("agent/"))
            self.assertNotIn("task", job)
            self.assertNotIn("body", job)
            self.assertNotIn("constraints", job)

    def test_linear_canary_without_one_repository_is_reported_not_guessed(self):
        linear_item = {
            "source_key": "linear:DEN-AMBIGUOUS",
            "version": "v1",
            "project": "github.com/evento-globolo-test",
            "title": "[meta-agent-canary] compare api-contract-e2e and mash-web-e2e",
            "url": "https://linear.app/example/DEN-AMBIGUOUS",
            "body": "Both repositories are relevant.",
            "identifier": "DEN-AMBIGUOUS",
            "priority": 2,
            "source": "linear",
        }
        with (
            mock.patch.object(
                dispatcher,
                "github_repositories",
                side_effect=lambda org, _token: (
                    {"api-contract-e2e", "mash-web-e2e"}
                    if org == "evento-globolo-test"
                    else set(),
                    "",
                ),
            ),
            mock.patch.object(dispatcher, "github_open_issues", return_value=([], "")),
            mock.patch.object(
                dispatcher,
                "linear_active_issues",
                return_value=([linear_item], ""),
            ),
        ):
            report = canary.discover(
                github_token="test-token",
                linear_key="linear-token",
            )
        self.assertEqual(report["validated_jobs"], 0)
        self.assertEqual(
            report["unresolved_linear_sources"],
            ["linear:DEN-AMBIGUOUS"],
        )


if __name__ == "__main__":
    unittest.main()
