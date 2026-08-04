#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
WORKFLOW_CONTRACT = ROOT / ".github/workflows/workflow-contract.yml"
DEEP_CONFORMANCE = ROOT / ".github/workflows/deep-conformance.yml"
DOCKERFILE = ROOT / "Dockerfile"
MAKEFILE = ROOT / "Makefile"
NETWORK_TEST = ROOT / "tests/network_transport_conformance.rs"
LOCKFILE = ROOT / "Cargo.lock"
PINNED_ACTION = re.compile(r"uses:\s+[^@\s]+@[0-9a-f]{40}(?:\s+#.*)?$")
PINNED_ACTIONLINT_IMAGE = re.compile(r"rhysd/actionlint@sha256:[0-9a-f]{64}")


class WorkflowContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.ci = CI.read_text(encoding="utf-8")
        cls.contract = WORKFLOW_CONTRACT.read_text(encoding="utf-8")
        cls.deep = DEEP_CONFORMANCE.read_text(encoding="utf-8")
        cls.dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        cls.makefile = MAKEFILE.read_text(encoding="utf-8")
        cls.network_test = NETWORK_TEST.read_text(encoding="utf-8")

    def test_cargo_lock_is_committed_and_generation_is_not_hidden_in_ci(self) -> None:
        self.assertTrue(LOCKFILE.is_file())
        self.assertGreater(LOCKFILE.stat().st_size, 100)
        self.assertNotIn("cargo generate-lockfile", self.ci)
        self.assertNotIn("cargo generate-lockfile", self.deep)
        self.assertIn("cargo clippy --workspace --all-targets --locked", self.ci)
        self.assertIn("cargo test --workspace --all-targets --locked", self.ci)
        self.assertIn(
            "cargo build --locked --release --bin meta-agent-control-plane",
            self.dockerfile,
        )

    def test_all_repository_actions_are_immutable(self) -> None:
        for workflow in (CI, WORKFLOW_CONTRACT, DEEP_CONFORMANCE):
            for line in workflow.read_text(encoding="utf-8").splitlines():
                if "uses:" in line:
                    with self.subTest(workflow=workflow.name, line=line):
                        self.assertRegex(line.strip(), PINNED_ACTION)

    def test_checkout_never_persists_credentials(self) -> None:
        for workflow in (self.ci, self.contract, self.deep):
            self.assertIn("persist-credentials: false", workflow)
            self.assertNotIn("persist-credentials: true", workflow)

    def test_workflows_are_read_only_and_bounded(self) -> None:
        for workflow in (self.ci, self.contract, self.deep):
            self.assertIn("permissions:\n  contents: read", workflow)
            self.assertIn("timeout-minutes:", workflow)
            self.assertIn("cancel-in-progress: true", workflow)

    def test_ci_runs_product_and_protocol_contracts(self) -> None:
        for command in (
            "cargo fmt --all --check",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "cargo test --workspace --all-targets --locked",
            "python3 scripts/verify_contract.py",
            "node --check scripts/dashboard.js",
            "docker build --tag meta-agent-control-plane:ci .",
        ):
            with self.subTest(command=command):
                self.assertIn(command, self.ci)

    def test_ci_boots_the_production_image_with_runtime_hardening(self) -> None:
        for contract in (
            "--read-only",
            "--tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m",
            "--cap-drop ALL",
            "--security-opt no-new-privileges",
            "META_AGENT_AUTH_TOKEN='ci-runtime-token-at-least-16-bytes'",
            "META_AGENT_PROTECT_READ_API=true",
            "{{.Config.User}}",
            "{{.HostConfig.ReadonlyRootfs}}",
            "/healthz",
            "/readyz",
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, self.ci)

    def test_deep_conformance_is_scheduled_repeatable_and_locked(self) -> None:
        for contract in (
            "schedule:",
            "cron: '17 7 * * 1,4'",
            "workflow_dispatch:",
            "for attempt in 1 2 3; do",
            "cargo test --locked --test replay_pressure_udp -- --nocapture --test-threads=1",
            "cargo test --locked --test network_transport_conformance -- --nocapture --test-threads=1",
            "cargo fmt --all --check",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "RUST_BACKTRACE: '1'",
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, self.deep)
        self.assertNotIn("secrets.", self.deep)
        self.assertNotIn("id-token: write", self.deep)

    def test_actual_network_fixture_is_real_bounded_and_locally_reproducible(self) -> None:
        self.assertTrue(NETWORK_TEST.is_file())
        for contract in (
            "Daemon::bind(config)",
            "TcpStream::connect(address)",
            "connect_async(request)",
            'WebSocketError::Http(response) => assert_eq!(response.status().as_u16(), 401)',
            "snapshot.counters.rejected, 3",
            "actual_http_websocket_and_tcp_telemetry_produce_identical_projection",
            "actual_http_websocket_and_tcp_privileged_events_produce_identical_projection",
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, self.network_test)
        self.assertIn(
            "cargo test --locked --test network_transport_conformance -- --nocapture --test-threads=1",
            self.makefile,
        )

    def test_workflow_contract_uses_actionlint_and_tests_itself(self) -> None:
        self.assertIn("docker run --rm", self.contract)
        self.assertRegex(self.contract, PINNED_ACTIONLINT_IMAGE)
        self.assertIn('--volume "$PWD:/repo:ro"', self.contract)
        self.assertIn(".github/workflows/*.yml", self.contract)
        self.assertNotIn("args: .github/workflows/*.yml", self.contract)
        self.assertIn(
            "python3 -m unittest -v scripts/ci/test_workflow_contract.py",
            self.contract,
        )


if __name__ == "__main__":
    unittest.main()
