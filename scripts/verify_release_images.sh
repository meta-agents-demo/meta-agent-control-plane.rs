#!/usr/bin/env bash
# Pull and verify the exact production images without exposing environment data.

set -euo pipefail

skip_pull=false
if [[ "${1:-}" == "--skip-pull" ]]; then
  skip_pull=true
  shift
fi

compose_env=${1:-}
if [[ -z "$compose_env" || ! -f "$compose_env" ]]; then
  echo "usage: $0 [--skip-pull] path/to/compose.env" >&2
  exit 64
fi

release_sha=$(sed -n 's/^META_AGENT_RELEASE_SHA="\([0-9a-f]\{40\}\)"$/\1/p' "$compose_env")
if [[ ! "$release_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "generated Compose environment lacks an exact release SHA" >&2
  exit 2
fi

compose=(
  docker compose
  --profile production-mutation
  --env-file "$compose_env"
  -f compose.agents.yaml
  -f compose.production.yaml
)
services=(control-plane agent-runner-openai agent-runner-anthropic task-dispatcher)

if [[ "$skip_pull" != true ]]; then
  "${compose[@]}" pull "${services[@]}"
fi

for service in "${services[@]}"; do
  image_ref=$("${compose[@]}" config --format json | python3 -c \
    'import json, sys; print(json.load(sys.stdin)["services"][sys.argv[1]]["image"])' \
    "$service")
  if [[ -z "$image_ref" ]]; then
    echo "verified image is unavailable for service: $service" >&2
    exit 2
  fi
  if [[ "$skip_pull" != true && ! "$image_ref" =~ ^ghcr\.io/meta-agents-demo/meta-agent-(control-plane|runner)@sha256:[0-9a-f]{64}$ ]]; then
    echo "production image is not an immutable canonical GHCR digest: $service" >&2
    exit 2
  fi
  revision=$(docker image inspect \
    --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}' \
    "$image_ref")
  if [[ "$revision" != "$release_sha" ]]; then
    echo "OCI revision mismatch for service: $service" >&2
    exit 2
  fi
done

echo "verified four OCI images at release ${release_sha:0:12}"
