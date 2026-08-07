#!/usr/bin/env python3
"""Fast offline checks for protocol/documentation drift.

This complements Rust tests and deliberately uses only the Python standard library.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODEL = (ROOT / "src/model.rs").read_text()
OPENAPI = json.loads((ROOT / "docs/openapi.json").read_text())
CARGO = tomllib.loads((ROOT / "Cargo.toml").read_text())

match = re.search(r"pub const EVENT_KINDS: &\[&str\] = &\[(.*?)\];", MODEL, re.S)
if not match:
    raise SystemExit("could not locate EVENT_KINDS in src/model.rs")
RUST_KINDS = re.findall(r'"([a-z_]+)"', match.group(1))
OPENAPI_KINDS = OPENAPI["x-meta-agent-event-kinds"]
udp_match = re.search(r"pub const UDP_EVENT_KINDS: &\[&str\] = &\[(.*?)\];", MODEL, re.S)
if not udp_match:
    raise SystemExit("could not locate UDP_EVENT_KINDS in src/model.rs")
RUST_UDP_KINDS = re.findall(r'"([a-z_]+)"', udp_match.group(1))
OPENAPI_UDP_KINDS = OPENAPI["x-meta-agent-udp-event-kinds"]

errors: list[str] = []
if RUST_KINDS != OPENAPI_KINDS:
    errors.append(f"event-kind drift: Rust={RUST_KINDS!r}, OpenAPI={OPENAPI_KINDS!r}")
if RUST_UDP_KINDS != OPENAPI_UDP_KINDS:
    errors.append(
        f"UDP policy drift: Rust={RUST_UDP_KINDS!r}, OpenAPI={OPENAPI_UDP_KINDS!r}"
    )

required_paths = {
    "/api/v1/coordination",
    "/api/v1/events",
    "/api/v1/explorer",
    "/api/v1/metacognition",
    "/api/v1/snapshot",
    "/api/v1/metacognition",
    "/api/v1/coordination",
    "/healthz",
    "/readyz",
    "/metrics",
    "/ws/agent",
    "/ws/ui",
}
missing_paths = required_paths - set(OPENAPI["paths"])
if missing_paths:
    errors.append(f"OpenAPI is missing paths: {sorted(missing_paths)!r}")

expected_coordination_parameters = {
    "max_assignments": (1, 256, 16),
    "max_assignments_per_agent": (1, 32, 2),
    "max_interventions": (1, 512, 32),
    "max_holds": (1, 1024, 64),
}
expected_explorer_parameters = {
    "timeline_limit": (1, 250, 100),
    "session_limit": (1, 250, 100),
    "lesson_limit": (1, 1000, 250),
}


def query_parameter_contract(path: str) -> dict[str, tuple[object, object, object]]:
    parameters = OPENAPI["paths"][path]["get"].get("parameters", [])
    return {
        parameter.get("name"): (
            parameter.get("schema", {}).get("minimum"),
            parameter.get("schema", {}).get("maximum"),
            parameter.get("schema", {}).get("default"),
        )
        for parameter in parameters
        if parameter.get("in") == "query"
    }


observed_coordination_parameters = query_parameter_contract("/api/v1/coordination")
if observed_coordination_parameters != expected_coordination_parameters:
    errors.append(
        "coordination policy parameter drift: "
        f"expected={expected_coordination_parameters!r}, "
        f"observed={observed_coordination_parameters!r}"
    )

observed_explorer_parameters = query_parameter_contract("/api/v1/explorer")
if observed_explorer_parameters != expected_explorer_parameters:
    errors.append(
        "explorer policy parameter drift: "
        f"expected={expected_explorer_parameters!r}, "
        f"observed={observed_explorer_parameters!r}"
    )

if CARGO["package"]["name"] != "meta-agent-control-plane":
    errors.append("Cargo package name changed without updating contract tooling")

for fixture in sorted((ROOT / "fixtures").glob("*.json")):
    try:
        json.loads(fixture.read_text())
    except json.JSONDecodeError as error:
        errors.append(f"{fixture.relative_to(ROOT)} is not valid JSON: {error}")

script = (ROOT / "scripts/dashboard.js").read_text().strip()
ui = (ROOT / "src/ui.rs").read_text()
if script not in ui:
    errors.append("scripts/dashboard.js drifted from the inline Leptos dashboard script")

coordination_ui = (ROOT / "src/coordination_ui.rs").read_text()
coordination_api = (ROOT / "src/coordination_api.rs").read_text()
if 'include_str!("../scripts/coordination-dashboard.js")' not in coordination_ui:
    errors.append("coordination UI is not wired to scripts/coordination-dashboard.js")
for route in ('"/coordination"', '"/api/v1/coordination"'):
    if route not in coordination_api:
        errors.append(f"coordination router is missing {route}")

explorer_ui = (ROOT / "src/explorer_ui.rs").read_text()
explorer_api = (ROOT / "src/explorer_api.rs").read_text()
if 'include_str!("../scripts/explorer-dashboard.js")' not in explorer_ui:
    errors.append("explorer UI is not wired to scripts/explorer-dashboard.js")
for route in ('"/explorer"', '"/api/v1/explorer"'):
    if route not in explorer_api:
        errors.append(f"explorer router is missing {route}")
if "authorize_read" not in explorer_api:
    errors.append("explorer API is not protected by the read authorization boundary")

conflict_pattern = re.compile(r"^(<<<<<<<|=======|>>>>>>>)", re.M)
for path in ROOT.rglob("*"):
    if path.is_file() and ".git" not in path.parts and path.suffix not in {".png", ".jpg", ".jpeg"}:
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        if conflict_pattern.search(text):
            errors.append(f"unresolved conflict marker in {path.relative_to(ROOT)}")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print(
    f"contract OK: {len(RUST_KINDS)} event kinds, "
    f"{len(RUST_UDP_KINDS)} UDP kinds, {len(required_paths)} required paths, "
    f"{len(expected_coordination_parameters)} coordination parameters, "
    f"{len(expected_explorer_parameters)} explorer parameters"
)
