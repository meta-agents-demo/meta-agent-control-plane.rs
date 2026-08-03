#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / ".github/workflows/ci.yml"
WORKFLOW_CONTRACT = ROOT / ".github/workflows/workflow-contract.yml"
LOCKFILE = ROOT / "Cargo.lock"
PINNED_ACTION = re.compile(r"uses:\s+[^@\s]+@[0-9a-f]{40}(?:\s+#.*)?$")


class WorkflowContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.ci = CI.read_text(encoding="utf-8")
        cls.contract = WORKFLOW_CONTRACT.read_text(encoding="utf-8")

    def test_cargo_lock_is_committed_and_generation_is_not_hidden_in_ci(self) -> None:
        self.assertTrue(LOCKFILE.is_file())
        self.assertGreater(LOCKFILE.stat().st_size, 100)
        self.assertNotIn("cargo generate-lockfile", self.ci)
        self.assertIn("cargo clippy --workspace --all-targets --locked", self.ci)
        self.assertIn("cargo test --workspace --all-targets --locked", self.ci)

    def test_all_repository_actions_are_immutable(self) -> None:
        for workflow in (CI, WORKFLOW_CONTRACT):
            for line in workflow.read_text(encoding="utf-8").splitlines():
                if "uses:" in line:
                    with self.subTest(workflow=workflow.name, line=line):
                        self.assertRegex(line.strip(), PINNED_ACTION)

    def test_checkout_never_persists_credentials(self) -> None:
        for workflow in (self.ci, self.contract):
            self.assertIn("persist-credentials: false", workflow)
            self.assertNotIn("persist-credentials: true", workflow)

    def test_workflows_are_read_only_and_bounded(self) -> None:
        for workflow in (self.ci, self.contract):
            self.assertIn("permissions:\n  contents: read", workflow)
            self.assertIn("timeout-minutes:", workflow)
            self.assertIn("cancel-in-progress: true", workflow)

    def test_ci_runs_product_and_protocol_contracts(self) -> None:
        for command in (
            "cargo fmt --all --check",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "cargo test --workspace --all-targets --locked",
            "bash tests/contract-fixtures.sh",
            "node --check dashboard/app.js",
            "docker build --tag meta-agent-control-plane:ci .",
        ):
            with self.subTest(command=command):
                self.assertIn(command, self.ci)

    def test_workflow_contract_uses_actionlint_and_tests_itself(self) -> None:
        self.assertIn("docker://rhysd/actionlint@sha256:", self.contract)
        self.assertIn(".github/workflows/*.yml", self.contract)
        self.assertIn(
            "python3 -m unittest -v scripts/ci/test_workflow_contract.py",
            self.contract,
        )


if __name__ == "__main__":
    unittest.main()
