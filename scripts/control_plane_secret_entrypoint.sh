#!/bin/sh
set -eu

fail() {
  printf '%s\n' "meta-agent control-plane startup failed: $1" >&2
  exit 64
}

secret_file=${META_AGENT_AUTH_TOKEN_FILE:-}
if [ -n "$secret_file" ]; then
  [ -z "${META_AGENT_AUTH_TOKEN:-}" ] || fail "configure either META_AGENT_AUTH_TOKEN_FILE or META_AGENT_AUTH_TOKEN, not both"
  [ ! -L "$secret_file" ] || fail "authentication token file must not be a symlink"
  [ -f "$secret_file" ] || fail "authentication token file is not a regular file"
  [ -r "$secret_file" ] || fail "authentication token file is not readable"

  byte_count=$(wc -c < "$secret_file" | tr -d '[:space:]')
  [ "$byte_count" -le 65536 ] || fail "authentication token file exceeds 65536 bytes"

  token=$(cat -- "$secret_file")
  case "$token" in
    *"
"*|*""*) fail "authentication token must be a single line" ;;
  esac
  [ "${#token}" -ge 16 ] || fail "authentication token must contain at least 16 bytes"

  META_AGENT_AUTH_TOKEN=$token
  export META_AGENT_AUTH_TOKEN
  unset token META_AGENT_AUTH_TOKEN_FILE secret_file byte_count
fi

exec /usr/local/bin/meta-agent-control-plane "$@"
