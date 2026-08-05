#!/usr/bin/env python3
"""Validate sanitized live-runtime smoke evidence.

The evidence contract intentionally permits only a small, fixed set of fields.
It must never contain provider credentials, account identifiers, prompts,
responses, reasoning, tool arguments/results, command contents, cookies, or
control-plane tokens.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
ALLOWED_TOP_LEVEL_KEYS = {
    "schema_version",
    "generated_at",
    "source_commit",
    "runs",
}
ALLOWED_RUN_KEYS = {
    "provider",
    "adapter_version",
    "operating_system",
    "test_case",
    "result",
    "started_at",
    "completed_at",
    "owner",
    "follow_up_date",
}
ALLOWED_PROVIDERS = {"anthropic", "google", "openai", "control-plane"}
ALLOWED_OPERATING_SYSTEMS = {"linux", "macos", "windows"}
ALLOWED_RESULTS = {"pass", "fail", "unavailable"}
REQUIRED_TEST_CASES = {
    "claude_native_hooks",
    "gemini_native_hooks",
    "codex_proxy_transparency",
    "linux_proc_pid_merge",
    "macos_host_resources",
    "windows_host_resources",
    "confidence_unreported",
    "observe_only_native_hooks",
    "dashboard_observed_sources",
}
TEST_CASE_PROVIDER = {
    "claude_native_hooks": "anthropic",
    "gemini_native_hooks": "google",
    "codex_proxy_transparency": "openai",
    "linux_proc_pid_merge": "control-plane",
    "macos_host_resources": "control-plane",
    "windows_host_resources": "control-plane",
    "confidence_unreported": "control-plane",
    "observe_only_native_hooks": "control-plane",
    "dashboard_observed_sources": "control-plane",
}
TEST_CASE_OS = {
    "linux_proc_pid_merge": "linux",
    "macos_host_resources": "macos",
    "windows_host_resources": "windows",
}
SAFE_VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:+-]{0,63}$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
OWNER_RE = re.compile(r"^@[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$")
UTC_TIMESTAMP_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
ISO_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
FORBIDDEN_SUBSTRINGS = {
    "alexander.d.mills@gmail.com",
    "authorization",
    "bearer ",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "control_plane_token",
    "prompt",
    "response",
    "reasoning",
    "scratchpad",
    "chain_of_thought",
    "tool_argument",
    "tool_result",
    "command_content",
    "cookie",
    "browser_profile",
}


class EvidenceError(ValueError):
    """Raised when smoke evidence violates the safe evidence contract."""


def _expect(condition: bool, message: str) -> None:
    if not condition:
        raise EvidenceError(message)


def _parse_utc_timestamp(value: Any, field: str) -> datetime:
    _expect(isinstance(value, str), f"{field} must be a string")
    _expect(
        bool(UTC_TIMESTAMP_RE.fullmatch(value)),
        f"{field} must be second-precision UTC",
    )
    return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(
        tzinfo=timezone.utc
    )


def _parse_date(value: Any, field: str) -> date:
    _expect(isinstance(value, str), f"{field} must be a string")
    _expect(bool(ISO_DATE_RE.fullmatch(value)), f"{field} must be YYYY-MM-DD")
    return date.fromisoformat(value)


def _scan_strings(value: Any, path: str = "$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            _scan_strings(key, f"{path}.<key>")
            _scan_strings(child, f"{path}.{key}")
        return
    if isinstance(value, list):
        for index, child in enumerate(value):
            _scan_strings(child, f"{path}[{index}]")
        return
    if not isinstance(value, str):
        return

    _expect(len(value) <= 128, f"{path} exceeds the 128-character evidence limit")
    lowered = value.lower()
    for forbidden in FORBIDDEN_SUBSTRINGS:
        _expect(
            forbidden not in lowered,
            f"{path} contains forbidden evidence content",
        )
    _expect(EMAIL_RE.search(value) is None, f"{path} contains an email address")


def validate_document(document: Any) -> None:
    _expect(isinstance(document, dict), "evidence root must be an object")
    _expect(
        set(document) == ALLOWED_TOP_LEVEL_KEYS,
        "unexpected or missing top-level fields",
    )
    _expect(
        document["schema_version"] == SCHEMA_VERSION,
        "unsupported schema_version",
    )
    _parse_utc_timestamp(document["generated_at"], "generated_at")
    _expect(
        isinstance(document["source_commit"], str)
        and bool(SHA_RE.fullmatch(document["source_commit"])),
        "source_commit must be a lowercase 40-character Git SHA",
    )
    runs = document["runs"]
    _expect(isinstance(runs, list) and runs, "runs must be a non-empty array")
    _expect(len(runs) <= 100, "runs exceeds the bounded evidence limit")

    seen: set[tuple[str, str, str]] = set()
    observed_test_cases: set[str] = set()

    for index, run in enumerate(runs):
        prefix = f"runs[{index}]"
        _expect(isinstance(run, dict), f"{prefix} must be an object")
        _expect(
            set(run).issubset(ALLOWED_RUN_KEYS),
            f"{prefix} contains an unexpected field",
        )
        required = ALLOWED_RUN_KEYS - {"owner", "follow_up_date"}
        _expect(required.issubset(run), f"{prefix} is missing a required field")

        provider = run["provider"]
        operating_system = run["operating_system"]
        test_case = run["test_case"]
        result = run["result"]

        _expect(provider in ALLOWED_PROVIDERS, f"{prefix}.provider is unsupported")
        _expect(
            operating_system in ALLOWED_OPERATING_SYSTEMS,
            f"{prefix}.operating_system is unsupported",
        )
        _expect(test_case in REQUIRED_TEST_CASES, f"{prefix}.test_case is unsupported")
        _expect(result in ALLOWED_RESULTS, f"{prefix}.result is unsupported")
        _expect(
            isinstance(run["adapter_version"], str)
            and bool(SAFE_VERSION_RE.fullmatch(run["adapter_version"])),
            f"{prefix}.adapter_version has an unsafe format",
        )
        _expect(
            TEST_CASE_PROVIDER[test_case] == provider,
            f"{prefix}.provider does not match {test_case}",
        )
        expected_os = TEST_CASE_OS.get(test_case)
        if expected_os is not None:
            _expect(
                operating_system == expected_os,
                f"{prefix}.operating_system does not match {test_case}",
            )

        started = _parse_utc_timestamp(run["started_at"], f"{prefix}.started_at")
        completed = _parse_utc_timestamp(
            run["completed_at"], f"{prefix}.completed_at"
        )
        _expect(completed >= started, f"{prefix}.completed_at precedes started_at")
        _expect(
            completed - started <= timedelta(hours=6),
            f"{prefix} duration exceeds 6 hours",
        )

        identity = (test_case, provider, operating_system)
        _expect(
            identity not in seen,
            f"{prefix} duplicates a provider/OS/test-case record",
        )
        seen.add(identity)
        observed_test_cases.add(test_case)

        if result == "unavailable":
            _expect(
                "owner" in run and "follow_up_date" in run,
                f"{prefix} unavailable result needs owner and follow_up_date",
            )
            _expect(
                isinstance(run["owner"], str)
                and bool(OWNER_RE.fullmatch(run["owner"])),
                f"{prefix}.owner must be a GitHub handle",
            )
            _parse_date(run["follow_up_date"], f"{prefix}.follow_up_date")
        else:
            _expect(
                "owner" not in run and "follow_up_date" not in run,
                f"{prefix} owner/follow_up_date are only for unavailable results",
            )

    missing = REQUIRED_TEST_CASES - observed_test_cases
    _expect(
        not missing,
        f"missing required test cases: {', '.join(sorted(missing))}",
    )
    _scan_strings(document)


def load_and_validate(path: Path) -> None:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise EvidenceError(f"cannot read valid JSON from {path}: {exc}") from exc
    validate_document(document)


def _self_test() -> None:
    valid = {
        "schema_version": 1,
        "generated_at": "2026-08-05T17:20:00Z",
        "source_commit": "0" * 40,
        "runs": [],
    }
    provider_os = {
        "claude_native_hooks": ("anthropic", "linux"),
        "gemini_native_hooks": ("google", "linux"),
        "codex_proxy_transparency": ("openai", "linux"),
        "linux_proc_pid_merge": ("control-plane", "linux"),
        "macos_host_resources": ("control-plane", "macos"),
        "windows_host_resources": ("control-plane", "windows"),
        "confidence_unreported": ("control-plane", "linux"),
        "observe_only_native_hooks": ("control-plane", "linux"),
        "dashboard_observed_sources": ("control-plane", "linux"),
    }
    for test_case, (provider, operating_system) in provider_os.items():
        valid["runs"].append(
            {
                "provider": provider,
                "adapter_version": "git:0000000",
                "operating_system": operating_system,
                "test_case": test_case,
                "result": "pass",
                "started_at": "2026-08-05T17:00:00Z",
                "completed_at": "2026-08-05T17:01:00Z",
            }
        )
    validate_document(valid)

    unsafe = json.loads(json.dumps(valid))
    unsafe["runs"][0]["prompt"] = "secret"
    try:
        validate_document(unsafe)
    except EvidenceError:
        pass
    else:
        raise AssertionError("unexpected fields must be rejected")

    unsafe = json.loads(json.dumps(valid))
    unsafe["runs"][0]["adapter_version"] = "alexander.d.mills@gmail.com"
    try:
        validate_document(unsafe)
    except EvidenceError:
        pass
    else:
        raise AssertionError("email addresses must be rejected")

    unavailable = json.loads(json.dumps(valid))
    unavailable["runs"][5]["result"] = "unavailable"
    unavailable["runs"][5]["owner"] = "@oresoftware"
    unavailable["runs"][5]["follow_up_date"] = "2026-08-12"
    validate_document(unavailable)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    try:
        if args.self_test:
            _self_test()
        if args.evidence is not None:
            load_and_validate(args.evidence)
        if not args.self_test and args.evidence is None:
            parser.error("provide an evidence JSON path or --self-test")
    except EvidenceError as exc:
        print(f"runtime smoke evidence rejected: {exc}", file=sys.stderr)
        return 1

    print("runtime smoke evidence contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
