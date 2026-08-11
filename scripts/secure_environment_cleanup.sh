#!/bin/sh
# Best-effort shutdown and plaintext cleanup that preserves a failing status.
# No secret value is read or printed by this script.

set -u

with_containers=false
if test "${1:-}" = "--with-containers"; then
  with_containers=true
  shift
fi

if test "$#" -eq 0; then
  echo "usage: $0 [--with-containers] dev|prod [...]" >&2
  exit 64
fi

failed=0
record_failure() {
  echo "secure cleanup step failed: $1" >&2
  failed=1
}

for profile in "$@"; do
  case "$profile" in
    dev|prod) ;;
    *) echo "profile must be dev or prod" >&2; exit 64 ;;
  esac
done

if test "$with_containers" = true; then
  if test "$#" -ne 1; then
    echo "container cleanup accepts exactly one profile" >&2
    exit 64
  fi
  profile=$1
  compose_env="env/dec/runtime-secrets/$profile/compose.env"
  if test -f "$compose_env"; then
    docker compose \
      --profile production-workers \
      --profile production-mutation \
      --env-file "$compose_env" \
      -f compose.agents.yaml \
      -f compose.production.yaml \
      down --remove-orphans || record_failure "container shutdown"
  else
    record_failure "container shutdown metadata is missing"
  fi
fi

for profile in "$@"; do
  python3 scripts/materialize_runtime_secrets.py \
    --profile "$profile" --clean || record_failure "$profile runtime-secret cleanup"
done

ores-sops lock || record_failure "ores-sops lock"
mkdir -p env/dec || record_failure "env/dec creation"
chmod 700 env/dec || record_failure "env/dec permissions"

exit "$failed"
