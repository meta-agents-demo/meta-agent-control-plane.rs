# Scoped issue dispatcher

The durable provider fleet admits real work from exactly four product portfolios by default:

- `apostille-me` / Linear `github.com/apostille-me`
- `embedded-alerts` / Linear `github.com/embedded-alerts`
- `evento-globolo` / Linear `github.com/evento-globolo`
- `hacker-house-medellin` / Linear `github.com/hacker-house-medellin`

`task-dispatcher` runs separately from both provider runners. It receives the GitHub and Linear task-source credentials but **no OpenAI or Anthropic key**. Provider containers continue to receive only their own provider secret plus the GitHub credential needed for branch/PR delivery.

## Admission rules

### GitHub issues

The dispatcher searches open issues across every repository in each allowlisted organization. It skips durable routing cards and explicit superseded-repository markers, but otherwise an open issue is a real executable input because the target repository is intrinsic to the issue URL.

A changed issue version produces a deterministic new job version. One provider is selected deterministically from the issue identity so OpenAI and Claude do not race to mutate the same branch/repository task.

### Linear issues

Only active issues in the four exact project names are fetched. A Linear issue becomes repository work only when exactly one target repository can be resolved from its title/description:

1. an explicit `https://github.com/<org>/<repo>` reference to an existing non-archived repository, or
2. exactly one existing repository name mentioned as a standalone token.

Zero matches means **no job**. Multiple matches means **no job**. The dispatcher logs the ambiguity and does not guess a repository. This is essential for organization-level planning tickets that legitimately span several repos.

If a Linear issue already points at a concrete repository, its URL/title/description become execution context for the durable provider job. The provider must inspect current repository and issue state before changing anything.

## Delivery contract

Every admitted job inherits the verified fleet contract:

- real checkout and assigned branch;
- public progress observations;
- at least one evidence-backed reflection;
- relevant validation when applicable;
- focused commit;
- pushed branch;
- open or updated pull request with independently verified head;
- partial/blocker status instead of invented success when delivery cannot be completed safely.

The provider process exit code alone never proves completion.

## Dedupe and fairness

The dispatcher keeps a private version ledger in its own durable volume. The ledger stores source identifiers and source versions, not provider prompts or transcripts. Provider selection is a stable hash of the source identity, distributing work without allowing two mutation-capable providers to work the same version concurrently.

## Secrets

Configure owner-readable files outside the repository:

```sh
install -d -m 700 "$HOME/.config/meta-agent/secrets"
printf '%s' "$GH_TOKEN" > "$HOME/.config/meta-agent/secrets/github-token"
printf '%s' "$LINEAR_API_KEY" > "$HOME/.config/meta-agent/secrets/linear-api-key"
chmod 600 "$HOME/.config/meta-agent/secrets/github-token" \
          "$HOME/.config/meta-agent/secrets/linear-api-key"
```

Provider credentials and the control-plane token follow the existing `docs/agent-fleet.md` setup. Do not put any value in Docker image layers, Compose YAML, Git, task events, or browser state.

## Start

```sh
set -a
. ./.env.agent-runner
set +a

docker compose -f compose.yaml -f compose.agents.yaml up --build
```

The dispatcher polls on a bounded interval (`META_AGENT_DISPATCH_INTERVAL_SECONDS`, minimum five minutes). A webhook/event-driven Linear intake is preferable for a mature deployment; the polling implementation is deliberately bounded and deduplicated for the current self-contained fleet.

## Relationship to `metagents`

`meta-agent-control-plane.rs` owns **mutation and verified delivery**. `metagents` owns the Mixpanel-style analytics/reflection dashboard and can independently ask both providers for bounded public analysis of the same real issue. Keeping those lanes separate prevents an analytics reflection from being mistaken for a commit or pull request.
