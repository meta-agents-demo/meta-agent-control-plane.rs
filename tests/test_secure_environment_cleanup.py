from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class SecureEnvironmentCleanupTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / "bin").mkdir()
        (self.root / "scripts" / "secure_environment_cleanup.sh").write_bytes(
            (ROOT / "scripts" / "secure_environment_cleanup.sh").read_bytes()
        )
        (self.root / "scripts" / "materialize_runtime_secrets.py").write_text(
            """import os, sys
profile = sys.argv[sys.argv.index('--profile') + 1]
with open(os.environ['CLEANUP_LOG'], 'a', encoding='utf-8') as handle:
    handle.write(f'materialize:{profile}\\n')
raise SystemExit(int(os.environ.get(f'MATERIALIZER_RC_{profile.upper()}', '0')))
""",
            encoding="utf-8",
        )
        self._stub(
            "docker",
            "printf 'docker\\n' >> \"$CLEANUP_LOG\"\nexit \"${DOCKER_RC:-0}\"",
        )
        self._stub(
            "ores-sops",
            "printf 'ores-sops:%s\\n' \"$1\" >> \"$CLEANUP_LOG\"\nexit \"${ORES_SOPS_RC:-0}\"",
        )
        self.log = self.root / "cleanup.log"
        self.environment = dict(os.environ)
        self.environment.update(
            {
                "PATH": f"{self.root / 'bin'}:{os.environ['PATH']}",
                "CLEANUP_LOG": str(self.log),
            }
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _stub(self, name: str, body: str) -> None:
        path = self.root / "bin" / name
        path.write_text(f"#!/bin/sh\nset -u\n{body}\n", encoding="utf-8")
        path.chmod(0o755)

    def _run(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", "scripts/secure_environment_cleanup.sh", *arguments],
            cwd=self.root,
            env=self.environment,
            check=False,
            text=True,
            capture_output=True,
        )

    def test_lock_is_attempted_after_container_shutdown_failure(self) -> None:
        compose_env = self.root / "env" / "dec" / "runtime-secrets" / "prod" / "compose.env"
        compose_env.parent.mkdir(parents=True)
        compose_env.write_text("paths-only\n", encoding="utf-8")
        self.environment["DOCKER_RC"] = "19"

        result = self._run("--with-containers", "prod")

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            self.log.read_text(encoding="utf-8").splitlines(),
            ["docker", "materialize:prod", "ores-sops:lock"],
        )
        self.assertEqual(stat.S_IMODE((self.root / "env" / "dec").stat().st_mode), 0o700)

    def test_all_profiles_and_lock_run_after_one_materializer_failure(self) -> None:
        self.environment["MATERIALIZER_RC_DEV"] = "7"

        result = self._run("dev", "prod")

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            self.log.read_text(encoding="utf-8").splitlines(),
            ["materialize:dev", "materialize:prod", "ores-sops:lock"],
        )

    def test_missing_container_metadata_fails_but_still_locks_plaintext(self) -> None:
        result = self._run("--with-containers", "prod")

        self.assertEqual(result.returncode, 1)
        self.assertIn("container shutdown metadata is missing", result.stderr)
        self.assertEqual(
            self.log.read_text(encoding="utf-8").splitlines(),
            ["materialize:prod", "ores-sops:lock"],
        )


if __name__ == "__main__":
    unittest.main()
