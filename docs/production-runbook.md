# Production release runbook

This runbook is the promotion boundary for the Meta Agents control plane and the real-provider fleet. It separates credential-free container certification from protected live-provider canaries and keeps decrypted values outside Git, Docker build contexts, Compose models, logs, artifacts, GitHub, and Linear.

## Release invariants

A production candidate must satisfy all of these conditions:

1. `cargo fmt`, Clippy, Rust tests, protocol/dashboard contracts, and the existing OCI runtime contract pass at the exact candidate head.
2. `just env-ci` proves the revision-pinned nixpkgs and `ores-sops` inputs, the Nix shell, the runtime-secret materializer, and the control-plane secret adapter. A reviewed `flake.lock` remains mandatory before final promotion.
3. The hardened production Compose model renders using synthetic secrets and starts every service as UID 10001 with a read-only root, all capabilities dropped, `no-new-privileges`, bounded PID/memory/CPU settings, rotated logs, and only loopback HTTP published. The dispatcher image must also prove it can atomically write its fresh named volume.
4. Secret payload files are mode 0640, share one deployment GID, and are readable only by the deployment account and the explicitly added container group. The generated path-only `compose.env` remains mode 0600.
5. `.dockerignore` excludes `.env`, nested dotenv files, `.sops.yaml`, the complete `env` tree, and Python bytecode so plaintext cannot enter a persistent build layer.
6. The exact candidate is exercised from `meta-agents-demo-test`; production mutation remains disabled until that organization and its GitHub App installation are verifiably available.
7. Live OpenAI and Anthropic doctors pass from protected runtime secret files. Provider keys are not needed for the earlier gates.
8. Production uses the two immutable `@sha256:` references recorded by the exact commit's `Publish OCI images` artifact; the pulled control-plane and runner OCI revision labels must also match the 40-character release SHA. Mutable tags and local worktree builds are not promotion inputs.

## One-time SOPS bootstrap

Enter the pinned shell and initialize canonical policy with an operator-controlled age identity:

```sh
nix develop --no-write-lock-file
just env-init
```

`ores-sops init` creates exact `dev` and `prod` rules. Its initial local recipient is only a bootstrap default. Before production reliance, review `.sops.yaml`, assign separate dev/prod recipients, give humans individual identities, establish an independently controlled recovery recipient, and prefer workload identity or protected CI identities for automation. Run `sops updatekeys` and rotate the data key after recipient changes.

Only these ciphertext files may be committed:

```text
env/enc/dev.env.enc
env/enc/prod.env.enc
```

Never commit `env/dec`, `.env`, private age identities, generated runtime secret files, or decrypted output.

## Populate production values

Use `config/runtime-env.example` only as a key inventory. Edit production ciphertext directly:

```sh
just env-edit prod
just env-verify
```

Required names are `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GH_TOKEN`, `LINEAR_API_KEY`, `META_AGENT_AUTH_TOKEN`, `META_AGENT_CREDENTIAL_EXPIRES_AT`, `META_AGENT_RELEASE_SHA`, `META_AGENT_CONTROL_PLANE_IMAGE`, and `META_AGENT_RUNNER_IMAGE`. Copy the release SHA and both canonical `@sha256:` references from the retained release-manifest artifact; never reconstruct them from the mutable `:main` tag. The shared control-plane token should be a fresh random value of at least 32 bytes. GitHub and Linear credentials should be bounded service credentials with only the repositories and projects required by the dispatcher. Keep `META_AGENT_RESTART_POLICY=unless-stopped` for a normal deployment; CI sets it to `no` so a failed process cannot be hidden by a restart loop.

Do not paste provider, GitHub, Linear, SOPS, or control-plane credentials into chat, issues, pull requests, CI output, shell history, or artifacts. Insert them through `just env-edit prod` on a trusted host or through the protected deployment secret store.

Run materialization as a dedicated deployment account or under a dedicated private deployment group. The materializer preserves the account's effective group on each mode-0640 secret file and writes that numeric GID, not a credential, into the mode-0600 generated Compose env. Compose adds only that GID to the non-root services that consume mounted secrets.

## Preflight, provider certification, and safe startup

```sh
just production-preflight prod
just production-doctor prod
just production-up prod
just production-status prod
```

`production-preflight` decrypts only on the trusted host, validates canonical SOPS policy, materializes five group-restricted files below `env/dec/runtime-secrets/prod`, and generates a mode-0600 `compose.env` containing paths, the deployment GID, and non-secret tuning only. The production Compose stack mounts those files as Docker secrets. The control-plane adapter imports the shared token only inside the container process immediately before `exec`; its value is absent from the Compose model and container configuration metadata.

The preflight pulls the exact digest-addressed images and rejects a noncanonical registry path, mutable reference, or OCI revision-label mismatch before the provider doctors run. The provider doctors make real, sanitized API capability requests with `--no-deps`; they never start the control plane as a side effect and never print a key or full model inventory. A missing, expired, invalid, unauthorized, quota-exhausted, or rate-limited provider is not admitted to work. `production-up` reruns the doctor gate and starts only the already-verified authenticated control plane with builds and implicit pulls disabled. Both provider runners are profile-gated because their persistent queues can resume repository-changing work even while the dispatcher is stopped.

To start the provider workers without issue discovery, use the separate literal acknowledgment:

```sh
just production-workers-up prod ENABLE_PROVIDER_WORKERS
```

This can resume work already present in either persistent queue. Treat it as a mutation-capable operation, inspect queue state first, and do not use it for a container smoke test.

The real GitHub/Linear dispatcher is assigned to the `production-mutation` profile and is excluded from routine startup. Both provider runners share that profile so admitting the dispatcher starts its declared dependencies without a hidden second step. After every exact-head public check, paired-org test, SOPS review, and live provider doctor is green, admit mutation with the literal acknowledgment:

```sh
just production-admit prod ENABLE_REAL_PRODUCTION_MUTATION
```

Do not use that command merely to test container startup. It begins real issue discovery and can enqueue repository-changing work in the configured allowlist.

## Paired test-organization gate

`meta-agents-demo-test` is the release proving ground, not a source of production secrets. Its fixture workflow must:

- check out the exact production candidate SHA;
- generate an ephemeral age identity and obviously synthetic dotenv values at runtime;
- run `just env-ci` and an SOPS encrypt/decrypt/no-key negative journey;
- render `compose.agents.yaml` followed by `compose.production.yaml` using generated synthetic secret files;
- build and start only `control-plane`, then verify health, non-root execution, supplementary deployment-GID access, read-only root, dropped capabilities, and token-protected reads;
- prove the real dispatcher remains behind the `production-mutation` profile;
- prove both provider runners are absent without a profile, present under `production-workers`, and present with the dispatcher under `production-mutation`;
- upload no decrypted dotenv, identity, secret file, Compose env file, ciphertext body, or raw container log;
- report only candidate SHA, workflow/run identifiers, pass/fail state, and bounded non-secret evidence.

The GitHub App must be installed on `meta-agents-demo-test` with contents, pull-request, issue, and Actions access before this gate can be executed or verified. Until then, public exact-head CI is useful evidence but not a substitute for the paired-org promotion gate.

The source repository owns the reusable contract so the test organization cannot silently drift from production. The `secure-env-e2e` caller must pin both the workflow reference and `candidate_sha` to the same reviewed 40-character commit:

```yaml
jobs:
  candidate:
    uses: meta-agents-demo/meta-agent-control-plane.rs/.github/workflows/production-compose.yml@REVIEWED_40_CHARACTER_SHA
    with:
      candidate_sha: REVIEWED_40_CHARACTER_SHA
      runner_label: meta-agents-demo-test-linux
```

Use the approved self-hosted Linux label while private GitHub-hosted jobs are refused by the account spending limit. The reusable workflow rejects a non-SHA candidate and verifies the checkout before any contract step runs.

## Stop and remove plaintext

```sh
just production-down prod
just env-lock
```

The shutdown path includes the mutation profile so it stops the dispatcher when admitted, then attempts every runtime-secret cleanup and `ores-sops lock` step even if container shutdown fails. It preserves a nonzero exit status when any step fails, so an operator cannot mistake partial cleanup for success. Confirm `git status --short` contains no secret material before leaving the trusted host.

## Rotation and rollback

For compromise, offboarding, or expiry:

1. stop admission and the fleet;
2. revoke provider, GitHub, and Linear credentials at their issuers;
3. update SOPS recipients and run `sops updatekeys`;
4. rotate the SOPS data key and every affected application credential;
5. rerun the complete paired-org candidate gate;
6. redeploy the last known-good image digest or candidate SHA if the new release fails.

A rollback never reuses a revoked credential merely because an older image expected it.
