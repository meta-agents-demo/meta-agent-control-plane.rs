# Disposable-account runtime smoke evidence

This runbook defines the evidence boundary for live Claude Code, Gemini CLI, and Codex app-server smoke tests. It complements the deterministic and container tests already in CI; it does not replace a real provider-backed run.

## Account boundary

Use only disposable provider accounts, short-lived test projects, and isolated browser or CLI profiles created for this matrix. Never use a personal browser profile, production credential, or `alexander.d.mills@gmail.com`.

Keep provider credentials outside the repository and outside the evidence file. Prefer an ephemeral shell environment or secret manager, revoke the credential after the run, and delete the disposable provider project or account when the matrix is complete.

## Evidence contract

The validator accepts only:

- schema version, sanitized generation timestamp, and tested source commit;
- provider, adapter version, operating system, fixed test-case identifier, result, and second-precision UTC start/end timestamps;
- a GitHub owner and follow-up date only when a combination is unavailable.

It rejects extra fields, email addresses, long strings, and strings associated with credentials, prompts, responses, reasoning, scratchpads, tool arguments/results, command contents, cookies, or browser profiles.

Allowed results are `pass`, `fail`, and `unavailable`. `unavailable` is not a pass and must include an owner and follow-up date.

## Required matrix

The evidence document must contain each fixed test case at least once:

- `claude_native_hooks`
- `gemini_native_hooks`
- `codex_proxy_transparency`
- `linux_proc_pid_merge`
- `macos_host_resources`
- `windows_host_resources`
- `confidence_unreported`
- `observe_only_native_hooks`
- `dashboard_observed_sources`

The validator enforces provider and operating-system assignments for provider-specific and platform-specific cases.

## Running the validator

```bash
python3 scripts/verify_runtime_smoke_evidence.py --self-test
python3 scripts/verify_runtime_smoke_evidence.py path/to/sanitized-evidence.json
```

`tests/fixtures/runtime-smoke-evidence.valid.json` is a schema fixture with every result marked unavailable. It is not proof that a live smoke test ran.

## Live execution procedure

1. Start the hardened Docker deployment from the tested source commit.
2. Configure the matching native hook or Codex app-server proxy in an isolated disposable profile.
3. Perform a benign provider action that exercises the relevant lifecycle events.
4. Confirm the dashboard row is backed by an accepted hook or observed process, and confirm resource provenance matches the platform expectation.
5. Record only the bounded evidence fields accepted by the validator.
6. Validate the evidence file before attaching it to issue #27 or committing it.
7. Revoke credentials and remove the disposable account, project, and profile.

A structurally valid JSON file does not prove the data came from a real provider run. The operator remains responsible for executing the live matrix and reporting failures honestly.
