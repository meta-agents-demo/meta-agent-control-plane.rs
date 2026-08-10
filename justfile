set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_default:
    @just --list --unsorted

# Verify that the development shell exposes the exact organization-approved
# secure-environment tool. This intentionally does not create recipients,
# decrypt ciphertext, or invent repository-specific secret policy.
env-toolchain:
    test "$(ores-sops --version)" = "ores-sops 0.3.1"

# Evaluate the repository flake without writing an unreviewed lockfile.
env-ci: env-toolchain
    nix flake check --no-write-lock-file -L
