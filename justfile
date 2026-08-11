set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_default:
    @just --list --unsorted

# Verify the exact organization-approved secure-environment tool. This command
# never creates recipients or decrypts ciphertext.
env-toolchain:
    mkdir -p env/dec
    chmod 700 env/dec
    test "$(ores-sops --version)" = "ores-sops 0.3.1"

# Bootstrap canonical policy with the operator's local age identity. Review and
# replace the pilot recipient sets before committing production policy.
env-init: env-toolchain
    mkdir -p env/enc env/dec
    chmod 700 env/dec
    ores-sops init

# Decrypt and activate one canonical profile locally.
env-use profile="dev": env-toolchain
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    mkdir -p env/dec
    chmod 700 env/dec
    ores-sops use "{{ profile }}"

# Edit ciphertext through SOPS rather than maintaining durable plaintext.
env-edit profile="dev": env-toolchain
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    mkdir -p env/dec
    chmod 700 env/dec
    ores-sops edit "{{ profile }}"

# Encrypt the selected local plaintext after deliberate edits.
env-encrypt profile="dev": env-toolchain
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    mkdir -p env/dec
    chmod 700 env/dec
    ores-sops encrypt "{{ profile }}"

# Keyless policy verification; trusted release hosts additionally run env-use.
env-verify: env-toolchain
    mkdir -p env/dec
    chmod 700 env/dec
    ores-sops verify

# Remove all generated runtime files and decrypted dotenv material.
env-lock:
    scripts/secure_environment_cleanup.sh dev prod

# Convert an already decrypted profile into restricted Docker secret files.
runtime-secrets profile="prod": env-toolchain
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    mkdir -p env/dec
    chmod 700 env/dec
    python3 scripts/materialize_runtime_secrets.py --profile "{{ profile }}"

# Run the complete non-provider production gate. This proves SOPS policy,
# selected-profile decryption, runtime-secret generation, and Compose validity.
production-preflight profile="prod": env-toolchain
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    mkdir -p env/dec
    chmod 700 env/dec
    ores-sops verify
    ores-sops use "{{ profile }}"
    python3 scripts/materialize_runtime_secrets.py --profile "{{ profile }}"
    docker compose \
      --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      config --quiet
    docker compose \
      --profile production-workers \
      --profile production-mutation \
      --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      config --quiet

# Run live, sanitized provider capability checks. These are the only gates that
# require valid OpenAI and Anthropic credentials.
production-doctor profile="prod":
    just production-preflight "{{ profile }}"
    scripts/verify_release_images.sh "env/dec/runtime-secrets/{{ profile }}/compose.env"
    docker compose --profile production-workers --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" -f compose.agents.yaml -f compose.production.yaml run --rm --no-deps agent-runner-openai doctor --provider openai
    docker compose --profile production-workers --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" -f compose.agents.yaml -f compose.production.yaml run --rm --no-deps agent-runner-anthropic doctor --provider anthropic

# Start only the authenticated control plane. Both provider runners remain
# disabled because a persisted queue can resume repository-changing work.
production-up profile="prod":
    just production-doctor "{{ profile }}"
    docker compose \
      --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      up --detach --no-build --pull never control-plane

# Explicitly start provider workers without admitting issue discovery. The
# literal acknowledgment makes resumption of persisted queues deliberate.
production-workers-up profile="prod" acknowledgment="":
    test "{{ acknowledgment }}" = "ENABLE_PROVIDER_WORKERS" || { echo 'pass ENABLE_PROVIDER_WORKERS as the second argument' >&2; exit 64; }
    just production-doctor "{{ profile }}"
    docker compose \
      --profile production-workers \
      --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      up --detach --no-build --pull never control-plane agent-runner-openai agent-runner-anthropic

# Explicitly admit the real GitHub/Linear dispatcher after every earlier gate.
# The literal acknowledgment prevents a routine `production-up` from mutating repos.
production-admit profile="prod" acknowledgment="":
    test "{{ acknowledgment }}" = "ENABLE_REAL_PRODUCTION_MUTATION" || { echo 'pass ENABLE_REAL_PRODUCTION_MUTATION as the second argument' >&2; exit 64; }
    just production-workers-up "{{ profile }}" ENABLE_PROVIDER_WORKERS
    docker compose \
      --profile production-mutation \
      --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      up --detach --no-build --pull never task-dispatcher

production-status profile="prod":
    docker compose --profile production-workers --profile production-mutation --env-file "env/dec/runtime-secrets/{{ profile }}/compose.env" -f compose.agents.yaml -f compose.production.yaml ps

production-down profile="prod":
    case "{{ profile }}" in dev|prod) ;; *) echo 'profile must be dev or prod' >&2; exit 64;; esac
    scripts/secure_environment_cleanup.sh --with-containers "{{ profile }}"

# Credential-free checks used by public CI and the paired test-org harness.
env-ci: env-toolchain
    mkdir -p env/dec
    chmod 700 env/dec
    python3 -m unittest -v tests/test_materialize_runtime_secrets.py
    python3 -m unittest -v tests/test_secure_environment_cleanup.py
    sh -n scripts/control_plane_secret_entrypoint.sh
    sh -n scripts/secure_environment_cleanup.sh
    bash -n scripts/verify_release_images.sh
    nix flake check --no-write-lock-file -L
