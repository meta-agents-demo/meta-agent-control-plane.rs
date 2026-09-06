# Encrypted environment bundles

`meta-agents-demo/meta-agent-control-plane.rs` follows the canonical `ORESoftware/ores-sops` dotenv
convention (sops + age + just + nix), the same contract used across the fleet
(fiducia-cloud, shared-auth, benefactor-cc, 3FA-app, zed-pkg, file-tunnel).

```text
env/enc/dev.env.enc     tracked ciphertext — dev profile
env/enc/prod.env.enc    tracked ciphertext — protected-operator profile
env/dec/dev.env         ignored plaintext, mode 0600
env/dec/prod.env        ignored plaintext, mode 0600
.env                    relative managed symlink -> env/dec/<dev|prod>.env
```

Only the two ciphertext files above may be committed. `env/dec` and the root
`.env` symlink are local-only; `.gitignore` and `ores-sops verify` enforce it.

`dev.env.enc` was seeded from `.env.example`: the variable names are right and
every value is still the placeholder from the example. `prod.env.enc` starts
empty and is encrypted to the protected-operator recipient set in `.sops.yaml`.

## Day to day

```sh
nix develop            # provides sops, age, just, ores-sops
just env-use dev       # or: ores-sops use dev   →  .env -> env/dec/dev.env
just env-edit dev      # edit ciphertext in $EDITOR, plaintext never lands on disk
just env-encrypt dev   # fold env/dec/dev.env edits back into the ciphertext
just env-verify        # keyless policy audit (safe in CI)
just env-lock          # wipe decrypted material
```

(Repos whose justfile predates the `env-` prefix expose the same recipes as
`just use|edit|encrypt|status|diff|lock`.)

Recipient changes: edit only the public `age1…` values in `.sops.yaml`, run
`sops updatekeys env/enc/<dev|prod>.env.enc` from a host that can decrypt, commit
the re-keyed ciphertext, and rotate the underlying credentials whenever a
recipient is removed. Every rule keeps at least two recipients — sops has no
backdoor, so a single lost key would otherwise be permanent data loss.
