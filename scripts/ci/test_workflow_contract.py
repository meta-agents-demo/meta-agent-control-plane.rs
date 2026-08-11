#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
WORKFLOW_CONTRACT = ROOT / ".github/workflows/workflow-contract.yml"
DEEP_CONFORMANCE = ROOT / ".github/workflows/deep-conformance.yml"
PRODUCTION_COMPOSE = ROOT / ".github/workflows/production-compose.yml"
PRODUCTION_OVERLAY = ROOT / "compose.production.yaml"
JUSTFILE = ROOT / "justfile"
DOCKERFILE = ROOT / "Dockerfile"
RUNNER_DOCKERFILE = ROOT / "Dockerfile.agent-runner"
RUNNER_PACKAGE = ROOT / "config/agent-runner/package.json"
RUNNER_LOCK = ROOT / "config/agent-runner/package-lock.json"
FLAKE = ROOT / "flake.nix"
FLAKE_LOCK = ROOT / "flake.lock"
MAKEFILE = ROOT / "Makefile"
NETWORK_TEST = ROOT / "tests/network_transport_conformance.rs"
LOCKFILE = ROOT / "Cargo.lock"
PINNED_ACTION = re.compile(r"uses:\s+[^@\s]+@[0-9a-f]{40}(?:\s+#.*)?$")
PINNED_ACTIONLINT_IMAGE = re.compile(r"rhysd/actionlint@sha256:[0-9a-f]{64}")
PINNED_BASE_IMAGE = re.compile(
    r"^FROM [A-Za-z0-9._/-]+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}"
    r"(?: AS [A-Za-z0-9_.-]+)?$"
)
EXACT_SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")


class WorkflowContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.ci = CI.read_text(encoding="utf-8")
        cls.contract = WORKFLOW_CONTRACT.read_text(encoding="utf-8")
        cls.deep = DEEP_CONFORMANCE.read_text(encoding="utf-8")
        cls.production_compose = PRODUCTION_COMPOSE.read_text(encoding="utf-8")
        cls.production_overlay = PRODUCTION_OVERLAY.read_text(encoding="utf-8")
        cls.justfile = JUSTFILE.read_text(encoding="utf-8")
        cls.dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        cls.runner_dockerfile = RUNNER_DOCKERFILE.read_text(encoding="utf-8")
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
        for workflow in (CI, WORKFLOW_CONTRACT, DEEP_CONFORMANCE, PRODUCTION_COMPOSE):
            for line in workflow.read_text(encoding="utf-8").splitlines():
                if "uses:" in line:
                    with self.subTest(workflow=workflow.name, line=line):
                        self.assertRegex(line.strip(), PINNED_ACTION)

    def test_checkout_never_persists_credentials(self) -> None:
        for workflow in (self.ci, self.contract, self.deep, self.production_compose):
            self.assertIn("persist-credentials: false", workflow)
            self.assertNotIn("persist-credentials: true", workflow)

    def test_workflows_are_read_only_and_bounded(self) -> None:
        for workflow in (self.ci, self.contract, self.deep, self.production_compose):
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

    def test_base_images_and_runner_cli_dependencies_are_immutable(self) -> None:
        from_lines = [
            line
            for dockerfile in (self.dockerfile, self.runner_dockerfile)
            for line in dockerfile.splitlines()
            if line.startswith("FROM ")
        ]
        self.assertEqual(len(from_lines), 3)
        for line in from_lines:
            with self.subTest(line=line):
                self.assertRegex(line, PINNED_BASE_IMAGE)

        package = json.loads(RUNNER_PACKAGE.read_text(encoding="utf-8"))
        lock = json.loads(RUNNER_LOCK.read_text(encoding="utf-8"))
        dependencies = package["dependencies"]
        self.assertEqual(
            set(dependencies),
            {"@anthropic-ai/claude-code", "@openai/codex"},
        )
        for name, version in dependencies.items():
            with self.subTest(direct_dependency=name):
                self.assertRegex(version, EXACT_SEMVER)
        self.assertEqual(lock["lockfileVersion"], 3)
        self.assertEqual(lock["packages"][""]["dependencies"], dependencies)
        for name, version in dependencies.items():
            self.assertEqual(lock["packages"][f"node_modules/{name}"]["version"], version)
        for name, metadata in lock["packages"].items():
            if not name:
                continue
            with self.subTest(package=name):
                self.assertRegex(metadata.get("resolved", ""), r"^https://registry\.npmjs\.org/")
                self.assertTrue(metadata.get("integrity", "").startswith("sha512-"))

        for package_name in (
            "@anthropic-ai/claude-code-linux-x64",
            "@anthropic-ai/claude-code-linux-arm64",
            "@openai/codex-linux-x64",
            "@openai/codex-linux-arm64",
        ):
            with self.subTest(native_package=package_name):
                metadata = lock["packages"][f"node_modules/{package_name}"]
                self.assertTrue(metadata["optional"])
                self.assertTrue(metadata["integrity"].startswith("sha512-"))

        for contract in (
            "npm ci --prefix /opt/meta-agent-cli --omit=dev --include=optional --ignore-scripts",
            "node /opt/meta-agent-cli/node_modules/@anthropic-ai/claude-code/install.cjs",
            "codex --version",
            "claude --version",
        ):
            self.assertIn(contract, self.runner_dockerfile)
        self.assertNotIn("npm install --global", self.runner_dockerfile)
        self.assertNotIn("ARG CODEX_VERSION", self.runner_dockerfile)
        self.assertNotIn("ARG CLAUDE_CODE_VERSION", self.runner_dockerfile)
        self.assertIn("docker buildx imagetools inspect --raw", self.ci)

    def test_nix_inputs_have_a_committed_content_lock(self) -> None:
        flake = FLAKE.read_text(encoding="utf-8")
        lock = json.loads(FLAKE_LOCK.read_text(encoding="utf-8"))
        self.assertEqual(lock["version"], 7)
        for input_name in ("nixpkgs", "ores-sops"):
            revision = re.search(
                rf'inputs\.{re.escape(input_name)}\.url\s*=\s*"github:[^/]+/[^/]+/([0-9a-f]{{40}})"',
                flake,
            )
            self.assertIsNotNone(revision, input_name)
            node = lock["nodes"][input_name]
            self.assertEqual(node["locked"]["rev"], revision.group(1))
            self.assertEqual(node["original"]["rev"], revision.group(1))

        for name, node in lock["nodes"].items():
            if "locked" not in node:
                continue
            with self.subTest(node=name):
                self.assertRegex(node["locked"]["rev"], r"^[0-9a-f]{40}$")
                self.assertTrue(node["locked"]["narHash"].startswith("sha256-"))

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

    def test_production_contract_is_reusable_and_pins_the_candidate(self) -> None:
        for contract in (
            "workflow_call:",
            "candidate_sha:",
            "required: true",
            "repository: meta-agents-demo/meta-agent-control-plane.rs",
            "EXPECTED_CANDIDATE_SHA:",
            "^[0-9a-f]{40}$",
            'test "$(git rev-parse HEAD)" = "$EXPECTED_CANDIDATE_SHA"',
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, self.production_compose)

    def test_production_workers_and_dispatcher_are_explicitly_profile_gated(self) -> None:
        self.assertEqual(
            self.production_overlay.count(
                "profiles: [production-workers, production-mutation]"
            ),
            2,
        )
        self.assertEqual(
            self.production_overlay.count("profiles: [production-mutation]"),
            1,
        )
        for contract in (
            'up --detach --no-build --pull never control-plane',
            'test "{{ acknowledgment }}" = "ENABLE_PROVIDER_WORKERS"',
            'test "{{ acknowledgment }}" = "ENABLE_REAL_PRODUCTION_MUTATION"',
            "--profile production-workers",
            "--profile production-mutation",
        ):
            with self.subTest(contract=contract):
                self.assertIn(contract, self.justfile)
        for assertion in (
            'set(normal["services"]) != {"control-plane"}',
            'set(workers["services"]) != expected_workers',
            'set(admitted["services"]) != expected_admitted',
        ):
            with self.subTest(assertion=assertion):
                self.assertIn(assertion, self.production_compose)


if __name__ == "__main__":
    unittest.main()
