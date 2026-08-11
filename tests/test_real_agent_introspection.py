from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import fleet as agent_fleet


class FakeResponse:
    def __init__(self, payload=None, *, status=200, headers=None):
        self.status = status
        self.headers = headers or {"Content-Type": "application/json"}
        self.raw = b"" if payload is None else json.dumps(payload).encode("utf-8")

    def read(self, _maximum=-1):
        return self.raw

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False


class SequenceOpen:
    def __init__(self, responses):
        self.responses = list(responses)
        self.requests = []

    def __call__(self, request, timeout=0):
        self.requests.append((request, timeout))
        response = self.responses.pop(0)
        if isinstance(response, BaseException):
            raise response
        return response


class RealTaskContractTests(unittest.TestCase):
    def job(self):
        return agent_fleet.Job.from_mapping({
            "job_id": "meta-runtime-proof",
            "provider": "openai",
            "repository": "https://github.com/meta-agents-demo/meta-agent-control-plane.rs.git",
            "task": "Improve real-agent delivery verification and publish a pull request.",
            "public_title": "Verify real agent delivery",
            "success_criteria": ["Publish a clean branch and evidence-backed reflection."],
            "require_test_evidence": True,
        })

    def identity(self, ledger=None):
        return agent_fleet.EventIdentity(
            agent_id="fleet-openai-real-task", provider="openai", model="provider-default",
            instance_id="real-task", session_id="real-task", correlation_id="real-task",
            task_id="real-task", ledger_path=ledger,
        )

    def test_prompt_declares_real_work_and_observation_contract(self):
        prompt = self.job().safe_prompt(False)
        self.assertIn("real repository task, not a simulation", prompt)
        self.assertIn("Never invent actions, tests, commits", prompt)
        self.assertIn("meta-agent-observe", prompt)
        self.assertIn("never chain-of-thought", prompt)

    def test_provider_allowlist_is_enforced_at_admission(self):
        with mock.patch.dict(os.environ, {"META_AGENT_PROVIDER_ALLOWLIST": "anthropic"}, clear=False):
            with self.assertRaisesRegex(ValueError, "not admitted"):
                agent_fleet.validate_admitted_job(self.job())

    def test_public_text_rejects_credentials_and_private_reasoning(self):
        with self.assertRaisesRegex(agent_fleet.ObservationError, "credential"):
            agent_fleet.public_text("Authorization: Bearer synthetic-secret-value", "summary")
        with self.assertRaisesRegex(agent_fleet.ObservationError, "private reasoning"):
            agent_fleet.public_text("<thinking>private scratchpad</thinking>", "summary")

    def test_observer_requires_admitted_real_task_environment(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(agent_fleet.ObservationError, "admitted real task"):
                agent_fleet.EventIdentity.from_environment()

    def test_event_client_posts_canonical_progress_and_records_mode_600_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "events.jsonl"
            opener = SequenceOpen([FakeResponse({"accepted": True}, status=202)])
            client = agent_fleet.EventClient(
                self.identity(ledger), base_url="http://control-plane:8787",
                token="synthetic-control-plane-token-32-bytes", urlopen=opener,
            )
            with mock.patch.dict(os.environ, {"META_AGENT_EVENTS_ENABLED": "true"}, clear=False):
                client.progress(0.5, "Verified the live repository state.", record_local=True)
            payload = json.loads(opener.requests[0][0].data)
            self.assertEqual(payload["kind"], "progress_updated")
            self.assertNotIn("prompt", json.dumps(payload).lower())
            self.assertEqual(ledger.stat().st_mode & 0o777, 0o600)
            self.assertEqual(agent_fleet.read_observation_summary(ledger).progress_count, 1)

    def test_reflection_ledger_counts_test_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "events.jsonl"
            client = agent_fleet.EventClient(self.identity(ledger), token=None)
            with mock.patch.dict(os.environ, {"META_AGENT_EVENTS_ENABLED": "false"}, clear=False):
                client.reflection(
                    "Delivery now fails closed when remote evidence is absent.", 0.91,
                    evidence=[{"kind": "test", "reference": "python3 -m unittest"}],
                    record_local=True,
                )
            summary = agent_fleet.read_observation_summary(ledger)
            self.assertEqual(summary.reflection_count, 1)
            self.assertEqual(summary.test_evidence_count, 1)

    def test_openai_model_inventory_is_live_but_public_metadata_hides_ids(self):
        opener = SequenceOpen([FakeResponse({"data": [{"id": "gpt-real-a"}, {"id": "gpt-real-b"}]})])
        snapshot = agent_fleet.discover_provider("openai", "synthetic-openai-key", urlopen=opener)
        self.assertEqual(snapshot.model_count, 2)
        self.assertNotIn("gpt-real-a", json.dumps(snapshot.public_metadata()))
        self.assertEqual(opener.requests[0][0].full_url, "https://api.openai.com/v1/models")

    def test_anthropic_discovers_capabilities_and_managed_agents(self):
        opener = SequenceOpen([
            FakeResponse({"data": [{"id": "claude-real", "capabilities": {"thinking": {"supported": True}}}]}),
            FakeResponse({"data": [{"id": "agent-a"}, {"id": "agent-b"}], "next_page": None}),
        ])
        with mock.patch.dict(os.environ, {"META_AGENT_ANTHROPIC_DISCOVER_MANAGED_AGENTS": "true"}, clear=False):
            snapshot = agent_fleet.discover_provider("anthropic", "synthetic-anthropic-key", urlopen=opener)
        self.assertIn("anthropic.model_capability.thinking", snapshot.capability_labels)
        self.assertEqual(snapshot.managed_agents_count, 2)
        headers = {key.lower(): value for key, value in opener.requests[1][0].header_items()}
        self.assertEqual(headers["anthropic-beta"], "managed-agents-2026-04-01")

    def test_provider_authentication_failure_is_sanitized(self):
        error = urllib.error.HTTPError(
            "https://api.openai.com/v1/models", 401, "Unauthorized", {}, io.BytesIO(b"secret body")
        )
        with self.assertRaises(agent_fleet.ProviderDiscoveryError) as caught:
            agent_fleet.discover_provider("openai", "synthetic-key", urlopen=SequenceOpen([error]))
        self.assertEqual(caught.exception.kind, "authentication_failed")
        self.assertNotIn("secret body", str(caught.exception))

    def test_mcp_modern_discovery_lists_only_declared_capabilities(self):
        opener = SequenceOpen([
            FakeResponse({
                "jsonrpc": "2.0", "id": "server-discover",
                "result": {
                    "supportedVersions": ["2026-07-28"],
                    "capabilities": {"tools": {}, "resources": {}},
                    "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "docs", "version": "2"}},
                },
            }),
            FakeResponse({"jsonrpc": "2.0", "id": "tools/list-0", "result": {"tools": [{"name": "search"}]}}),
            FakeResponse({"jsonrpc": "2.0", "id": "resources/list-0", "result": {"resources": []}}),
            FakeResponse({"jsonrpc": "2.0", "id": "resources/templates/list-0", "result": {"resourceTemplates": []}}),
        ])
        snapshot = agent_fleet.probe_mcp_server(
            agent_fleet.MCPServerConfig("openai_docs", "https://developers.openai.com/mcp"), urlopen=opener,
        )
        self.assertEqual(snapshot.protocol_mode, "modern_server_discover")
        self.assertEqual(snapshot.tool_count, 1)
        methods = [json.loads(request.data)["method"] for request, _ in opener.requests]
        self.assertNotIn("initialize", methods)
        self.assertNotIn("prompts/list", methods)
        for request, _ in opener.requests:
            headers = {key.lower(): value for key, value in request.header_items()}
            self.assertEqual(headers["mcp-method"], json.loads(request.data)["method"])

    def test_mcp_falls_back_to_legacy_initialize(self):
        opener = SequenceOpen([
            FakeResponse({"jsonrpc": "2.0", "id": "server-discover", "error": {"code": -32601}}),
            FakeResponse({
                "jsonrpc": "2.0", "id": "initialize",
                "result": {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}},
            }, headers={"Content-Type": "application/json", "Mcp-Session-Id": "session-1"}),
            FakeResponse(None, status=202),
            FakeResponse({"jsonrpc": "2.0", "id": "tools/list-0", "result": {"tools": []}}),
        ])
        snapshot = agent_fleet.probe_mcp_server(
            agent_fleet.MCPServerConfig("openai_docs", "https://developers.openai.com/mcp"), urlopen=opener,
        )
        self.assertEqual(snapshot.protocol_mode, "legacy_initialize")
        methods = [json.loads(request.data).get("method") for request, _ in opener.requests]
        self.assertEqual(methods, ["server/discover", "initialize", "notifications/initialized", "tools/list"])
        headers = {key.lower(): value for key, value in opener.requests[-1][0].header_items()}
        self.assertEqual(headers["mcp-session-id"], "session-1")

    def test_mcp_configuration_requires_https_allowlist(self):
        with mock.patch.dict(os.environ, {
            "META_AGENT_MCP_SERVERS_JSON": '[{"name":"bad","url":"http://localhost:8000/mcp"}]',
            "META_AGENT_MCP_ALLOWED_HOSTS": "developers.openai.com",
        }, clear=False):
            with self.assertRaisesRegex(ValueError, "explicitly allowed host"):
                agent_fleet.load_mcp_server_configs()

    def evidence(self, **updates):
        values = dict(
            repository_url="https://github.com/meta-agents-demo/meta-agent-control-plane.rs",
            branch_url="https://github.com/meta-agents-demo/meta-agent-control-plane.rs/tree/agent/meta-runtime-proof",
            commit_url="https://github.com/meta-agents-demo/meta-agent-control-plane.rs/commit/abc",
            pull_request_url="https://github.com/meta-agents-demo/meta-agent-control-plane.rs/pull/99",
            pull_request_state="OPEN", pull_request_is_draft=True, pull_request_head="abc",
            head_commit="abc", base_commit="base", remote_branch_commit="abc",
            commits_ahead=1, dirty_entries=0,
        )
        values.update(updates)
        return agent_fleet.DeliveryEvidence(**values)

    def observations(self):
        return agent_fleet.ObservationSummary(
            {"progress_updated": 2, "reflection_recorded": 1}, 1, "Validated the result."
        )

    def test_verified_delivery_contract_passes(self):
        self.assertEqual(
            agent_fleet.missing_delivery_requirements(self.job(), self.evidence(), self.observations()), ()
        )

    def test_zero_exit_cannot_mask_missing_or_mismatched_artifacts(self):
        missing = agent_fleet.missing_delivery_requirements(
            self.job(), self.evidence(remote_branch_commit=None, pull_request_head=None, commits_ahead=0, dirty_entries=2),
            agent_fleet.ObservationSummary({}, 0, None),
        )
        for item in (
            "worktree_not_clean", "no_new_commit", "remote_branch_missing", "pull_request_head_unverified",
            "public_progress_incomplete", "public_reflection_missing", "test_evidence_missing",
        ):
            self.assertIn(item, missing)


if __name__ == "__main__":
    unittest.main()
