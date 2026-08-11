from __future__ import annotations

import contextlib
import io
import os
import stat
import tempfile
import unittest
from pathlib import Path

from scripts.materialize_runtime_secrets import (
    GENERATED_FILES,
    MaterializationError,
    clean,
    materialize,
    parse_dotenv,
)


SYNTHETIC_VALUES = {
    "OPENAI_API_KEY": "synthetic-openai-contract-key-0001",
    "ANTHROPIC_API_KEY": "synthetic-anthropic-contract-key-0001",
    "GH_TOKEN": "synthetic-github-contract-token-0001",
    "LINEAR_API_KEY": "synthetic-linear-contract-token-0001",
    "META_AGENT_AUTH_TOKEN": "synthetic-control-plane-token-0001",
    "META_AGENT_CREDENTIAL_EXPIRES_AT": "2099-01-01T00:00:00Z",
    "META_AGENT_HTTP_PORT": "18787",
    "META_AGENT_RESTART_POLICY": "no",
}


def dotenv(values: dict[str, str]) -> str:
    return "\n".join(f"{key}={value}" for key, value in values.items()) + "\n"


class RuntimeSecretMaterializationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.input = self.root / "env" / "dec" / "prod.env"
        self.output_root = self.root / "env" / "dec" / "runtime-secrets"
        self.input.parent.mkdir(parents=True, mode=0o700)
        self.input.write_text(dotenv(SYNTHETIC_VALUES), encoding="utf-8")
        if os.name == "posix":
            os.chmod(self.input.parent, 0o700)
            os.chmod(self.input, 0o600)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_materializes_group_restricted_secret_files_and_path_only_compose_env(self) -> None:
        compose_path = materialize("prod", self.input, self.output_root)
        output = compose_path.parent

        self.assertEqual({path.name for path in output.iterdir()}, GENERATED_FILES)
        for path in output.iterdir():
            if os.name == "posix":
                expected_mode = 0o600 if path.name == "compose.env" else 0o640
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected_mode)

        compose = compose_path.read_text(encoding="utf-8")
        for secret in (
            SYNTHETIC_VALUES["OPENAI_API_KEY"],
            SYNTHETIC_VALUES["ANTHROPIC_API_KEY"],
            SYNTHETIC_VALUES["GH_TOKEN"],
            SYNTHETIC_VALUES["LINEAR_API_KEY"],
            SYNTHETIC_VALUES["META_AGENT_AUTH_TOKEN"],
        ):
            self.assertNotIn(secret, compose)
        self.assertIn("OPENAI_API_KEY_FILE=", compose)
        self.assertIn("META_AGENT_AUTH_TOKEN_FILE=", compose)
        secret_gid = (output / "control_plane_token").stat().st_gid
        self.assertIn(f'META_AGENT_SECRET_GID="{secret_gid}"', compose)
        self.assertIn('META_AGENT_HTTP_PORT="18787"', compose)
        self.assertIn('META_AGENT_RESTART_POLICY="no"', compose)

    def test_rejects_missing_unknown_duplicate_and_expired_values(self) -> None:
        cases = []

        missing = dict(SYNTHETIC_VALUES)
        del missing["GH_TOKEN"]
        cases.append(dotenv(missing))

        unknown = dict(SYNTHETIC_VALUES)
        unknown["ACCIDENTAL_SECRET"] = "must-not-be-copied"
        cases.append(dotenv(unknown))

        cases.append(dotenv(SYNTHETIC_VALUES) + "GH_TOKEN=duplicate-value-that-is-long-enough\n")

        expired = dict(SYNTHETIC_VALUES)
        expired["META_AGENT_CREDENTIAL_EXPIRES_AT"] = "2000-01-01T00:00:00Z"
        cases.append(dotenv(expired))

        invalid_restart = dict(SYNTHETIC_VALUES)
        invalid_restart["META_AGENT_RESTART_POLICY"] = "surprise"
        cases.append(dotenv(invalid_restart))

        for content in cases:
            with self.subTest(content=content.splitlines()[-1].split("=", 1)[0]):
                self.input.write_text(content, encoding="utf-8")
                if os.name == "posix":
                    os.chmod(self.input, 0o600)
                with self.assertRaises(MaterializationError):
                    materialize("prod", self.input, self.output_root)

    def test_rejects_permissive_plaintext_and_symlinked_output(self) -> None:
        if os.name != "posix":
            self.skipTest("POSIX permission and symlink contract")

        os.chmod(self.input, 0o644)
        with self.assertRaisesRegex(MaterializationError, "mode 0600"):
            materialize("prod", self.input, self.output_root)

        os.chmod(self.input, 0o600)
        target = self.root / "outside"
        target.mkdir()
        self.output_root.parent.mkdir(parents=True, exist_ok=True)
        self.output_root.symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(MaterializationError, "symlink"):
            materialize("prod", self.input, self.output_root)

    def test_clean_removes_only_known_generated_files(self) -> None:
        compose_path = materialize("prod", self.input, self.output_root)
        self.assertTrue(compose_path.exists())
        clean("prod", self.output_root)
        self.assertFalse((self.output_root / "prod").exists())

        compose_path = materialize("prod", self.input, self.output_root)
        unexpected = compose_path.parent / "unexpected"
        unexpected.write_text("sentinel", encoding="utf-8")
        with self.assertRaisesRegex(MaterializationError, "unexpected"):
            materialize("prod", self.input, self.output_root)
        self.assertTrue(unexpected.exists())
        with self.assertRaisesRegex(MaterializationError, "unexpected"):
            clean("prod", self.output_root)
        self.assertTrue(unexpected.exists())

    def test_parser_treats_values_as_data_and_never_expands_shell_syntax(self) -> None:
        parsed = parse_dotenv("VALUE='literal $HOME `command`'\nOTHER=plain#value\n")
        self.assertEqual(parsed["VALUE"], "literal $HOME `command`")
        self.assertEqual(parsed["OTHER"], "plain#value")

    def test_errors_do_not_echo_secret_values(self) -> None:
        bad = dict(SYNTHETIC_VALUES)
        bad["GH_TOKEN"] = "short"
        self.input.write_text(dotenv(bad), encoding="utf-8")
        if os.name == "posix":
            os.chmod(self.input, 0o600)
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            with self.assertRaises(MaterializationError) as raised:
                materialize("prod", self.input, self.output_root)
        self.assertNotIn("short", str(raised.exception))
        self.assertNotIn("short", captured.getvalue())


if __name__ == "__main__":
    unittest.main()
