# Paired `*-test` organization canary

Before the mutation-capable dispatcher is pointed at production portfolios, run a non-mutating canary across the paired test organizations:

| Production | Canary |
|---|---|
| `apostille-me` | `apostille-me-test` |
| `embedded-alerts` | `embedded-alerts-test` |
| `evento-globolo` | `evento-globolo-test` |
| `hacker-house-medellin` | `hacker-house-medellin-test` |

Matching Linear project names are `github.com/<test-org>`.

## What the canary proves

`scripts/fleet/canary.py` reuses the production dispatcher's authenticated discovery, exact repository inventory, Linear repository resolution, deterministic provider selection, branch/job construction, and `Job` admission validation. It then emits a bounded report.

It does **not**:

- write a provider queue;
- write the production dispatch ledger;
- start Codex or Claude;
- clone or edit a repository;
- push a branch;
- open a pull request;
- change a GitHub or Linear issue;
- deploy anything.

The report omits the source body and private execution task. It contains only source identity, source URL/version, resolved repository, selected provider, job/branch identity, validation flags, aggregate counts, and bounded source errors.

## Canary admission

Only open issues whose title begins with:

```text
[meta-agent-canary]
```

are admitted. Permanent routing cards, superseded-repository markers, and ordinary test-fleet maintenance remain visible in their systems but are never selected by this run.

This protects existing `*-test` work while still exercising real authenticated source discovery.

## Current canary sources

- `embedded-alerts-test/web-ui-e2e#7`
- `evento-globolo-test/api-contract-e2e#5`

At the time the canary was introduced, `apostille-me-test` and `hacker-house-medellin-test` existed but had no repositories. Their zero-repository state is represented explicitly in the report rather than silently treated as success. Bootstrap tickets should create one reviewable test harness in each before requiring one canary from every org.

## Run with runtime secret files

```sh
install -d -m 700 "$HOME/.config/meta-agent/secrets"
# Populate owner-readable GitHub and Linear files outside Git.
chmod 600 \
  "$HOME/.config/meta-agent/secrets/github-token" \
  "$HOME/.config/meta-agent/secrets/linear-api-key"

GH_TOKEN_FILE="$HOME/.config/meta-agent/secrets/github-token" \
LINEAR_API_KEY_FILE="$HOME/.config/meta-agent/secrets/linear-api-key" \
docker compose -f compose.canary.yaml run --rm paired-test-org-canary
```

No provider API key or control-plane token is mounted into this service.

The command exits nonzero when authenticated source errors occur or fewer than `META_AGENT_CANARY_MIN_JOBS` validated jobs are found. The default minimum is one so the two populated test fleets can provide evidence while the two empty test orgs are bootstrapped.

## Promotion gate

Production dispatch remains disabled until all of the following evidence exists:

1. the canary report contains no source errors;
2. every reported job validates;
3. queue and ledger writes remain zero;
4. repositories resolve only inside allowlisted `*-test` organizations;
5. no provider credential is mounted into the canary service;
6. independent tests verify ambiguous Linear issues are reported rather than guessed;
7. the canary issue threads link the exact report or CI evidence;
8. an operator reviews the production allowlist separately from the test allowlist.

A passing canary demonstrates routing and schema safety. It does not itself authorize production mutation or prove provider execution quality.
