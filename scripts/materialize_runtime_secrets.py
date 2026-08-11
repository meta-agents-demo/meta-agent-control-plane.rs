#!/usr/bin/env python3
"""Materialize an ores-sops decrypted dotenv into runtime-only Docker secret files.

The command never prints secret values and never places them in the generated
Compose environment file. It accepts only the documented production keys,
rejects ambiguous dotenv syntax, and writes every generated file atomically
with owner/group-restricted permissions.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path
from typing import Final

MAX_DOTENV_BYTES: Final = 64 * 1024
MAX_SECRET_BYTES: Final = 16 * 1024
PROFILE_RE: Final = re.compile(r"^(dev|prod)$")
KEY_RE: Final = re.compile(r"^[A-Z][A-Z0-9_]*$")

SECRET_FILES: Final = {
    "OPENAI_API_KEY": "openai_api_key",
    "ANTHROPIC_API_KEY": "anthropic_api_key",
    "GH_TOKEN": "github_token",
    "LINEAR_API_KEY": "linear_api_key",
    "META_AGENT_AUTH_TOKEN": "control_plane_token",
}

# These values are configuration, not credentials. They may be copied into the
# generated Compose env file after validation. Unknown keys are rejected so a
# misspelled secret cannot silently land in plaintext output.
PASSTHROUGH_KEYS: Final = {
    "META_AGENT_ANTHROPIC_MAX_CONCURRENCY",
    "META_AGENT_ANTHROPIC_RETRY_SECONDS",
    "META_AGENT_CONTROL_PLANE_IMAGE",
    "META_AGENT_CONTROL_PLANE_CPUS",
    "META_AGENT_CONTROL_PLANE_MEMORY",
    "META_AGENT_CREDENTIAL_EXPIRES_AT",
    "META_AGENT_DISPATCH_INTERVAL_SECONDS",
    "META_AGENT_DISPATCH_LIMIT",
    "META_AGENT_GITHUB_ORGS",
    "META_AGENT_HTTP_PORT",
    "META_AGENT_LINEAR_PROJECTS",
    "META_AGENT_OPENAI_MAX_CONCURRENCY",
    "META_AGENT_OPENAI_RETRY_SECONDS",
    "META_AGENT_RELEASE_SHA",
    "META_AGENT_RUNNER_IMAGE",
    "META_AGENT_RUNNER_CPUS",
    "META_AGENT_RUNNER_MEMORY",
    "META_AGENT_RUNNER_PIDS",
    "META_AGENT_DISPATCHER_CPUS",
    "META_AGENT_DISPATCHER_MEMORY",
    "META_AGENT_DISPATCHER_PIDS",
    "META_AGENT_RUNTIME_DISCOVERY_ENABLED",
    "META_AGENT_RUNTIME_SAMPLE_INTERVAL_MS",
    "META_AGENT_RESTART_POLICY",
}

REQUIRED_KEYS: Final = set(SECRET_FILES) | {"META_AGENT_CREDENTIAL_EXPIRES_AT"}
GENERATED_FILES: Final = set(SECRET_FILES.values()) | {"compose.env"}
RELEASE_SHA_RE: Final = re.compile(r"^[0-9a-f]{40}$")
CONTROL_PLANE_IMAGE_RE: Final = re.compile(
    r"^ghcr\.io/meta-agents-demo/meta-agent-control-plane@sha256:[0-9a-f]{64}$"
)
RUNNER_IMAGE_RE: Final = re.compile(r"^ghcr\.io/meta-agents-demo/meta-agent-runner@sha256:[0-9a-f]{64}$")
CPU_KEYS: Final = {
    "META_AGENT_CONTROL_PLANE_CPUS",
    "META_AGENT_RUNNER_CPUS",
    "META_AGENT_DISPATCHER_CPUS",
}
MEMORY_KEYS: Final = {
    "META_AGENT_CONTROL_PLANE_MEMORY",
    "META_AGENT_RUNNER_MEMORY",
    "META_AGENT_DISPATCHER_MEMORY",
}
PIDS_KEYS: Final = {"META_AGENT_RUNNER_PIDS", "META_AGENT_DISPATCHER_PIDS"}
MEMORY_RE: Final = re.compile(r"^([1-9][0-9]*)([kKmMgG])$")


class MaterializationError(ValueError):
    """A safe, non-secret-bearing configuration error."""


def _is_trusted_platform_alias(path: Path) -> bool:
    """Allow only macOS's immutable compatibility aliases into /private.

    macOS commonly returns temporary paths below ``/var`` even though ``/var``
    is a root-owned compatibility symlink to ``/private/var``. Rejecting that
    OS alias makes otherwise safe materialization impossible. The exact target
    check keeps arbitrary parent symlinks fail-closed.
    """

    if sys.platform != "darwin":
        return False
    expected = {
        Path("/tmp"): Path("/private/tmp"),
        Path("/var"): Path("/private/var"),
    }.get(path)
    if expected is None:
        return False
    try:
        return path.resolve(strict=True) == expected
    except OSError:
        return False


def _assert_no_symlink(path: Path) -> None:
    """Reject a symlink at the path or any existing parent component."""

    candidates = [path, *path.parents]
    for candidate in candidates:
        try:
            mode = candidate.lstat().st_mode
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(mode) and not _is_trusted_platform_alias(candidate):
            raise MaterializationError(f"managed path must not be a symlink: {candidate}")


def _ensure_private_directory(path: Path) -> None:
    _assert_no_symlink(path)
    path.mkdir(parents=True, exist_ok=True)
    if not path.is_dir():
        raise MaterializationError(f"managed path is not a directory: {path}")
    if os.name == "posix":
        os.chmod(path, 0o700)


def _read_private_dotenv(path: Path) -> str:
    _assert_no_symlink(path)
    try:
        metadata = path.stat()
    except OSError as error:
        raise MaterializationError(f"unable to read decrypted dotenv: {path}") from error
    if not stat.S_ISREG(metadata.st_mode):
        raise MaterializationError("decrypted dotenv must be a regular file")
    if metadata.st_size > MAX_DOTENV_BYTES:
        raise MaterializationError(f"decrypted dotenv exceeds {MAX_DOTENV_BYTES} bytes")
    if os.name == "posix" and metadata.st_mode & 0o077:
        raise MaterializationError("decrypted dotenv must have mode 0600 or stricter")
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise MaterializationError("decrypted dotenv must be readable UTF-8") from error


def _parse_quoted_value(raw: str, line_number: int) -> str:
    if not raw:
        return ""
    if raw.startswith("'"):
        if len(raw) < 2 or not raw.endswith("'"):
            raise MaterializationError(f"unterminated single-quoted value on line {line_number}")
        return raw[1:-1]
    if raw.startswith('"'):
        try:
            decoded = json.loads(raw)
        except json.JSONDecodeError as error:
            raise MaterializationError(f"invalid double-quoted value on line {line_number}") from error
        if not isinstance(decoded, str):
            raise MaterializationError(f"dotenv value on line {line_number} must be a string")
        return decoded
    return raw


def parse_dotenv(text: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_number, original in enumerate(text.splitlines(), start=1):
        line = original.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise MaterializationError(f"expected KEY=value on line {line_number}")
        key, raw_value = line.split("=", 1)
        key = key.strip()
        if not KEY_RE.fullmatch(key):
            raise MaterializationError(f"invalid dotenv key on line {line_number}")
        if key in values:
            raise MaterializationError(f"duplicate dotenv key: {key}")
        value = _parse_quoted_value(raw_value.strip(), line_number)
        if "\x00" in value or "\n" in value or "\r" in value:
            raise MaterializationError(f"dotenv value on line {line_number} must be single-line")
        values[key] = value
    return values


def _validate_expiry(value: str, now: dt.datetime | None = None) -> None:
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise MaterializationError("META_AGENT_CREDENTIAL_EXPIRES_AT must be ISO-8601") from error
    if parsed.tzinfo is None:
        raise MaterializationError("META_AGENT_CREDENTIAL_EXPIRES_AT must include a timezone")
    current = now or dt.datetime.now(dt.timezone.utc)
    if parsed.astimezone(dt.timezone.utc) <= current.astimezone(dt.timezone.utc):
        raise MaterializationError("META_AGENT_CREDENTIAL_EXPIRES_AT must be in the future")


def validate_values(values: dict[str, str], *, require_release_sha: bool = False) -> None:
    production_keys = {
        "META_AGENT_RELEASE_SHA",
        "META_AGENT_CONTROL_PLANE_IMAGE",
        "META_AGENT_RUNNER_IMAGE",
    }
    required = REQUIRED_KEYS | (production_keys if require_release_sha else set())
    missing = sorted(required - values.keys())
    if missing:
        raise MaterializationError(f"missing required keys: {', '.join(missing)}")
    unknown = sorted(values.keys() - set(SECRET_FILES) - PASSTHROUGH_KEYS)
    if unknown:
        raise MaterializationError(f"unknown keys are not allowed: {', '.join(unknown)}")

    for key in SECRET_FILES:
        value = values[key]
        encoded = value.encode("utf-8")
        if len(encoded) < 16:
            raise MaterializationError(f"{key} must contain at least 16 bytes")
        if len(encoded) > MAX_SECRET_BYTES:
            raise MaterializationError(f"{key} exceeds {MAX_SECRET_BYTES} bytes")
        if any(ord(character) < 32 or ord(character) == 127 for character in value):
            raise MaterializationError(f"{key} contains a control character")

    for key in PASSTHROUGH_KEYS & values.keys():
        value = values[key]
        if len(value.encode("utf-8")) > 4_096:
            raise MaterializationError(f"{key} exceeds 4096 bytes")
        if any(ord(character) < 32 or ord(character) == 127 for character in value):
            raise MaterializationError(f"{key} contains a control character")

    restart_policy = values.get("META_AGENT_RESTART_POLICY")
    if restart_policy and restart_policy not in {"no", "always", "on-failure", "unless-stopped"}:
        raise MaterializationError("META_AGENT_RESTART_POLICY is invalid")

    release_sha = values.get("META_AGENT_RELEASE_SHA")
    if release_sha and not RELEASE_SHA_RE.fullmatch(release_sha):
        raise MaterializationError("META_AGENT_RELEASE_SHA must be a lowercase 40-character Git SHA")

    control_plane_image = values.get("META_AGENT_CONTROL_PLANE_IMAGE")
    if require_release_sha and control_plane_image and not CONTROL_PLANE_IMAGE_RE.fullmatch(control_plane_image):
        raise MaterializationError("META_AGENT_CONTROL_PLANE_IMAGE must use the canonical GHCR sha256 digest")
    runner_image = values.get("META_AGENT_RUNNER_IMAGE")
    if require_release_sha and runner_image and not RUNNER_IMAGE_RE.fullmatch(runner_image):
        raise MaterializationError("META_AGENT_RUNNER_IMAGE must use the canonical GHCR sha256 digest")

    for key in CPU_KEYS & values.keys():
        try:
            cpus = float(values[key])
        except ValueError as error:
            raise MaterializationError(f"{key} must be a number from 0.1 through 32") from error
        if not 0.1 <= cpus <= 32:
            raise MaterializationError(f"{key} must be a number from 0.1 through 32")

    for key in PIDS_KEYS & values.keys():
        try:
            pids = int(values[key])
        except ValueError as error:
            raise MaterializationError(f"{key} must be an integer from 32 through 4096") from error
        if not 32 <= pids <= 4_096:
            raise MaterializationError(f"{key} must be an integer from 32 through 4096")

    memory_multipliers = {"k": 1, "m": 1_024, "g": 1_024 * 1_024}
    for key in MEMORY_KEYS & values.keys():
        match = MEMORY_RE.fullmatch(values[key])
        if not match:
            raise MaterializationError(f"{key} must use a positive k, m, or g suffix")
        kibibytes = int(match.group(1)) * memory_multipliers[match.group(2).lower()]
        if not 64 * 1_024 <= kibibytes <= 64 * 1_024 * 1_024:
            raise MaterializationError(f"{key} must be between 64m and 64g")

    _validate_expiry(values["META_AGENT_CREDENTIAL_EXPIRES_AT"])


def _atomic_write(path: Path, content: str, mode: int = 0o600) -> None:
    _assert_no_symlink(path)
    _ensure_private_directory(path.parent)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        if os.name == "posix":
            os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        if os.name == "posix":
            os.chmod(path, mode)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def _compose_quote(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def _assert_managed_directory(output_directory: Path) -> None:
    """Fail before writes if the managed directory contains an unknown path."""

    if not output_directory.exists():
        return
    if not output_directory.is_dir():
        raise MaterializationError("runtime secret path is not a directory")
    for child in output_directory.iterdir():
        if child.name not in GENERATED_FILES or child.is_symlink() or not child.is_file():
            raise MaterializationError(f"unexpected managed runtime path: {child.name}")


def materialize(profile: str, input_path: Path, output_root: Path) -> Path:
    if not PROFILE_RE.fullmatch(profile):
        raise MaterializationError("profile must be dev or prod")

    values = parse_dotenv(_read_private_dotenv(input_path))
    validate_values(values, require_release_sha=profile == "prod")

    output_directory = output_root / profile
    _ensure_private_directory(output_root)
    _ensure_private_directory(output_directory)
    _assert_managed_directory(output_directory)

    for environment_key, filename in SECRET_FILES.items():
        path = output_directory / filename
        _atomic_write(path, values[environment_key], mode=0o640)

    secret_gids = {(output_directory / filename).stat().st_gid for filename in SECRET_FILES.values()}
    if len(secret_gids) != 1:
        raise MaterializationError("runtime secret files must share one deployment group")
    secret_gid = next(iter(secret_gids))

    compose_lines = [
        "# Generated runtime paths only. This file is local, mode 0600, and must never be committed.",
        f"META_AGENT_SECRET_GID={_compose_quote(str(secret_gid))}",
    ]
    for environment_key, filename in SECRET_FILES.items():
        file_key = {
            "OPENAI_API_KEY": "OPENAI_API_KEY_FILE",
            "ANTHROPIC_API_KEY": "ANTHROPIC_API_KEY_FILE",
            "GH_TOKEN": "GH_TOKEN_FILE",
            "LINEAR_API_KEY": "LINEAR_API_KEY_FILE",
            "META_AGENT_AUTH_TOKEN": "META_AGENT_AUTH_TOKEN_FILE",
        }[environment_key]
        compose_lines.append(f"{file_key}={_compose_quote(str((output_directory / filename).resolve()))}")
    for key in sorted(PASSTHROUGH_KEYS & values.keys()):
        compose_lines.append(f"{key}={_compose_quote(values[key])}")
    compose_path = output_directory / "compose.env"
    _atomic_write(compose_path, "\n".join(compose_lines) + "\n")

    _assert_managed_directory(output_directory)

    return compose_path


def clean(profile: str, output_root: Path) -> None:
    if not PROFILE_RE.fullmatch(profile):
        raise MaterializationError("profile must be dev or prod")
    directory = output_root / profile
    _assert_no_symlink(directory)
    if not directory.exists():
        return
    if not directory.is_dir():
        raise MaterializationError("runtime secret path is not a directory")
    children = list(directory.iterdir())
    for child in children:
        if child.name not in GENERATED_FILES or child.is_symlink() or not child.is_file():
            raise MaterializationError(f"refusing to remove unexpected runtime path: {child.name}")
    for child in children:
        child.unlink()
    directory.rmdir()
    try:
        output_root.rmdir()
    except OSError:
        pass


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("dev", "prod"), default="prod")
    parser.add_argument("--input", type=Path, help="decrypted dotenv path")
    parser.add_argument(
        "--output-root",
        type=Path,
        default=Path("env/dec/runtime-secrets"),
        help="runtime-only output root",
    )
    parser.add_argument("--clean", action="store_true", help="remove generated runtime files")
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    input_path = arguments.input or Path("env/dec") / f"{arguments.profile}.env"
    try:
        if arguments.clean:
            clean(arguments.profile, arguments.output_root)
            print(f"removed generated runtime secrets for {arguments.profile}")
        else:
            compose_path = materialize(arguments.profile, input_path, arguments.output_root)
            print(f"materialized {len(SECRET_FILES)} runtime secret files for {arguments.profile}")
            print(f"compose environment: {compose_path}")
    except MaterializationError as error:
        print(f"runtime secret materialization failed: {error}", file=os.sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
