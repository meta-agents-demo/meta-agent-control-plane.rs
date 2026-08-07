#!/usr/bin/env python3
from pathlib import Path

path = Path("deploy/k8s/fleet.yaml")
value = path.read_text(encoding="utf-8")

mount = "{ name: runtime-secrets, mountPath: /run/secrets, readOnly: true }"
if value.count(mount) != 3:
    raise SystemExit(f"expected three broad secret mounts, found {value.count(mount)}")
value = value.replace(
    mount,
    "{ name: openai-secrets, mountPath: /run/secrets, readOnly: true }",
    1,
)
value = value.replace(
    mount,
    "{ name: anthropic-secrets, mountPath: /run/secrets, readOnly: true }",
    1,
)
value = value.replace(
    mount,
    "{ name: dispatcher-secrets, mountPath: /run/secrets, readOnly: true }",
    1,
)

old = """        - name: runtime-secrets
          secret:
            secretName: meta-agent-runtime-secrets
            defaultMode: 0400
"""
if value.count(old) != 1:
    raise SystemExit("broad runtime Secret volume definition drifted")
new = """        - name: openai-secrets
          secret:
            secretName: meta-agent-runtime-secrets
            defaultMode: 0400
            items:
              - { key: openai_api_key, path: openai_api_key }
              - { key: github_token, path: github_token }
              - { key: control_plane_token, path: control_plane_token }
        - name: anthropic-secrets
          secret:
            secretName: meta-agent-runtime-secrets
            defaultMode: 0400
            items:
              - { key: anthropic_api_key, path: anthropic_api_key }
              - { key: github_token, path: github_token }
              - { key: control_plane_token, path: control_plane_token }
        - name: dispatcher-secrets
          secret:
            secretName: meta-agent-runtime-secrets
            defaultMode: 0400
            items:
              - { key: github_token, path: github_token }
"""
path.write_text(value.replace(old, new, 1), encoding="utf-8")
