#!/usr/bin/env python3
"""Translate native agent lifecycle hooks into the safe runtime hook contract.

The adapter deliberately emits only lifecycle names, tool names, resource
counters, and fixed summaries. It never forwards prompts, tool arguments,
tool results, model text, browser data, or hidden reasoning.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from typing import Any, MutableMapping

MAX_STDIN_BYTES = 1_048_576
REQUEST_TIMEOUT_SECONDS = 4
PROVIDER_PATTERNS = {
    "claude": ("claude", "anthropic"),
    "gemini": ("gemini",),
    "codex": ("codex", "chatgpt", "openai"),
}
IGNORED_PARENT_NAMES = {
    "bash",
    "cmd",
    "cmd.exe",
    "dash",
    "fish",
    "powershell",
    "powershell.exe",
    "pwsh",
    "pwsh.exe",
    "python",
    "python3",
    "python.exe",
    "sh",
    "zsh",
}
SENSITIVE_CODEX_ITEM_TYPES = {
    "agentmessage",
    "reasoning",
    "usermessage",
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def text(value: Any, maximum: int = 256) -> str | None:
    if value is None or isinstance(value, (dict, list, tuple, set)):
        return None
    rendered = str(value).strip()
    if not rendered:
        return None
    return rendered[:maximum]


def first(payload: dict[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in payload and payload[key] is not None:
            return payload[key]
    return None


def nested_dict(payload: dict[str, Any], *keys: str) -> dict[str, Any] | None:
    value = first(payload, *keys)
    return value if isinstance(value, dict) else None


def identifier(value: Any, fallback: str) -> str:
    candidate = text(value, 256)
    if candidate is None:
        return fallback
    safe = "".join(
        character
        for character in candidate
        if character.isalnum() or character in "._:/-"
    )
    return safe[:256] or fallback


def runtime_url(path: str) -> str:
    base = os.environ.get("META_AGENT_RUNTIME_URL", "http://127.0.0.1:8787").rstrip("/")
    return f"{base}{path}"


def runtime_token() -> str:
    value = os.environ.get("META_AGENT_AUTH_TOKEN", "")
    if len(value) < 16:
        raise RuntimeError("META_AGENT_AUTH_TOKEN must contain at least 16 bytes")
    return value


def post_json(path: str, payload: dict[str, Any]) -> Any:
    request = urllib.request.Request(
        runtime_url(path),
        data=json.dumps(payload, separators=(",", ":")).encode(),
        headers={
            "Authorization": f"Bearer {runtime_token()}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
        return json.load(response)


def post_hook(envelope: dict[str, Any]) -> None:
    post_json("/api/v1/runtime/hooks", envelope)


def read_hook_input() -> dict[str, Any]:
    raw = sys.stdin.buffer.read(MAX_STDIN_BYTES + 1)
    if len(raw) > MAX_STDIN_BYTES:
        raise ValueError("hook payload exceeds one MiB")
    payload = json.loads(raw or b"{}")
    if not isinstance(payload, dict):
        raise ValueError("hook payload must be a JSON object")
    return payload


def powershell() -> str | None:
    return shutil.which("pwsh") or shutil.which("powershell")


def process_identity_posix(pid: int) -> tuple[int, str, str] | None:
    completed = subprocess.run(
        ["ps", "-o", "ppid=,comm=,args=", "-p", str(pid)],
        check=False,
        capture_output=True,
        text=True,
        timeout=2,
    )
    line = completed.stdout.strip()
    if completed.returncode != 0 or not line:
        return None
    fields = line.split(None, 2)
    if len(fields) < 2:
        return None
    try:
        parent_pid = int(fields[0])
    except ValueError:
        return None
    return parent_pid, fields[1], fields[2] if len(fields) > 2 else ""


def process_identity_windows(pid: int) -> tuple[int, str, str] | None:
    shell = powershell()
    if shell is None:
        return None
    script = (
        f"$p=Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\" "
        "-ErrorAction SilentlyContinue;"
        "if($null -ne $p){"
        "[pscustomobject]@{parent=[int]$p.ParentProcessId;"
        "name=[string]$p.Name;command=[string]$p.CommandLine}"
        "|ConvertTo-Json -Compress}"
    )
    completed = subprocess.run(
        [shell, "-NoProfile", "-NonInteractive", "-Command", script],
        check=False,
        capture_output=True,
        text=True,
        timeout=4,
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        return None
    try:
        payload = json.loads(completed.stdout)
        return (
            int(payload.get("parent", 0)),
            str(payload.get("name", "")),
            str(payload.get("command", "")),
        )
    except (TypeError, ValueError, json.JSONDecodeError):
        return None


def process_identity(pid: int) -> tuple[int, str, str] | None:
    if os.name == "nt":
        return process_identity_windows(pid)
    return process_identity_posix(pid)


def explicit_pid(value: int | None) -> int | None:
    if value is not None and value > 0:
        return value
    configured = os.environ.get("META_AGENT_TARGET_PID", "").strip()
    if not configured:
        return None
    try:
        parsed = int(configured)
    except ValueError:
        return None
    return parsed if parsed > 0 else None


def discover_agent_pid(provider: str, configured_pid: int | None = None) -> int | None:
    configured = explicit_pid(configured_pid)
    if configured is not None:
        return configured

    current = os.getppid()
    patterns = PROVIDER_PATTERNS[provider]
    for _ in range(10):
        identity = process_identity(current)
        if identity is None:
            return None
        parent_pid, name, command = identity
        normalized_name = os.path.basename(name).lower()
        haystack = f"{name} {command}".lower()
        if (
            normalized_name not in IGNORED_PARENT_NAMES
            and any(pattern in haystack for pattern in patterns)
        ):
            return current
        if parent_pid <= 1 or parent_pid == current:
            break
        current = parent_pid
    return None


def finite_number(value: Any) -> float | None:
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    if parsed != parsed or parsed in (float("inf"), float("-inf")):
        return None
    return parsed


def sample_process_posix(pid: int) -> dict[str, float | int]:
    completed = subprocess.run(
        ["ps", "-o", "%cpu=,rss=,%mem=", "-p", str(pid)],
        check=False,
        capture_output=True,
        text=True,
        timeout=2,
    )
    fields = completed.stdout.strip().split()
    if completed.returncode != 0 or len(fields) < 3:
        return {}
    cpu = finite_number(fields[0])
    rss_kib = finite_number(fields[1])
    memory = finite_number(fields[2])
    result: dict[str, float | int] = {}
    if cpu is not None and cpu >= 0:
        result["cpu_percent"] = cpu
    if rss_kib is not None and rss_kib >= 0:
        result["rss_bytes"] = int(rss_kib * 1024)
    if memory is not None and 0 <= memory <= 100:
        result["memory_percent"] = memory
    return result


def sample_process_windows(pid: int) -> dict[str, float | int]:
    shell = powershell()
    if shell is None:
        return {}
    script = (
        f"$pidValue={pid};"
        "$proc=Get-Process -Id $pidValue -ErrorAction SilentlyContinue;"
        "if($null -ne $proc){"
        "$perf=Get-CimInstance Win32_PerfFormattedData_PerfProc_Process "
        "-Filter \"IDProcess = $pidValue\" -ErrorAction SilentlyContinue;"
        "$total=(Get-CimInstance Win32_ComputerSystem "
        "-ErrorAction SilentlyContinue).TotalPhysicalMemory;"
        "$cpu=$null;if($null -ne $perf){$cpu=[double]$perf.PercentProcessorTime};"
        "$mem=$null;if($total -gt 0){$mem=([double]$proc.WorkingSet64/[double]$total)*100};"
        "[pscustomobject]@{cpu_percent=$cpu;"
        "rss_bytes=[int64]$proc.WorkingSet64;memory_percent=$mem}"
        "|ConvertTo-Json -Compress}"
    )
    completed = subprocess.run(
        [shell, "-NoProfile", "-NonInteractive", "-Command", script],
        check=False,
        capture_output=True,
        text=True,
        timeout=6,
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        return {}
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return {}
    result: dict[str, float | int] = {}
    cpu = finite_number(payload.get("cpu_percent"))
    rss = finite_number(payload.get("rss_bytes"))
    memory = finite_number(payload.get("memory_percent"))
    if cpu is not None and cpu >= 0:
        result["cpu_percent"] = cpu
    if rss is not None and rss >= 0:
        result["rss_bytes"] = int(rss)
    if memory is not None and 0 <= memory <= 100:
        result["memory_percent"] = memory
    return result


def sample_process(pid: int | None) -> dict[str, float | int]:
    if pid is None or pid <= 0:
        return {}
    try:
        if os.name == "nt":
            return sample_process_windows(pid)
        return sample_process_posix(pid)
    except (OSError, subprocess.SubprocessError):
        return {}


def base_envelope(
    *,
    provider: str,
    agent_id: str,
    model: str,
    session_id: str | None,
    instance_id: str | None,
    pid: int | None,
    kind: str,
    summary: str,
    native_event: str,
    tool_name: str | None = None,
    event_id: str | None = None,
    resources: dict[str, float | int] | None = None,
    input_tokens_delta: int = 0,
    output_tokens_delta: int = 0,
    control_capable: bool = False,
) -> dict[str, Any]:
    envelope: dict[str, Any] = {
        "protocol_version": "v1",
        "event_id": event_id or str(uuid.uuid4()),
        "occurred_at": utc_now(),
        "agent": {
            "agent_id": identifier(agent_id, f"{provider}:unidentified"),
            "provider": provider,
            "model": identifier(model, "unreported"),
        },
        "kind": kind,
        "summary": summary[:4096],
        "control_capable": control_capable,
        "input_tokens_delta": max(0, int(input_tokens_delta)),
        "output_tokens_delta": max(0, int(output_tokens_delta)),
        "metadata": {
            "adapter": f"{provider}-native",
            "native_event": identifier(native_event, "unknown"),
            "host_os": identifier(platform.system().lower(), "unknown"),
        },
    }
    if instance_id:
        envelope["agent"]["instance_id"] = identifier(instance_id, "unreported")
    if session_id:
        envelope["session_id"] = identifier(session_id, "unreported")
    if pid is not None and pid > 0:
        envelope["pid"] = pid
    if tool_name:
        envelope["tool_name"] = identifier(tool_name, "unknown-tool")
    for key, value in (resources or {}).items():
        if key in {"cpu_percent", "rss_bytes", "memory_percent"}:
            envelope[key] = value
    return envelope


def override_agent_id(
    prefix: str, session_id: str | None, override: str | None
) -> str:
    configured = override or os.environ.get("META_AGENT_ID")
    if configured:
        return identifier(configured, f"{prefix}:host")
    return f"{prefix}:{identifier(session_id, 'host')}"


def override_model(payload_model: Any, override: str | None) -> str:
    configured = override or os.environ.get("META_AGENT_MODEL")
    return identifier(configured or payload_model, "unreported")


def map_claude(
    payload: dict[str, Any],
    *,
    agent_id_override: str | None = None,
    model_override: str | None = None,
    pid: int | None = None,
    resources: dict[str, float | int] | None = None,
) -> dict[str, Any] | None:
    event = identifier(
        first(payload, "hook_event_name", "hookEventName", "event_name", "eventName"),
        "Unknown",
    )
    session_id = text(first(payload, "session_id", "sessionId"))
    subagent_id = text(first(payload, "agent_id", "agentId"))
    instance_id = subagent_id or text(first(payload, "agent_type", "agentType"))
    agent_id = override_agent_id("claude", session_id, agent_id_override)
    if subagent_id and not (agent_id_override or os.environ.get("META_AGENT_ID")):
        agent_id = f"{agent_id}:{identifier(subagent_id, 'subagent')}"
    model = override_model(first(payload, "model", "model_name", "modelName"), model_override)
    tool_name = text(first(payload, "tool_name", "toolName"))
    detail = identifier(
        first(payload, "source", "reason", "notification_type", "notificationType"),
        "unspecified",
    )

    mapping = {
        "SessionStart": ("session_started", f"Session started: {detail}"),
        "UserPromptSubmit": ("activity", "User prompt submitted"),
        "PreToolUse": ("tool_started", f"Tool started: {identifier(tool_name, 'unknown')}"),
        "PermissionRequest": (
            "activity",
            f"Permission requested for tool: {identifier(tool_name, 'unknown')}",
        ),
        "PostToolUse": ("tool_finished", f"Tool finished: {identifier(tool_name, 'unknown')}"),
        "PostToolUseFailure": (
            "error_observed",
            f"Tool failed: {identifier(tool_name, 'unknown')}",
        ),
        "Notification": ("activity", f"Agent notification: {detail}"),
        "SubagentStart": ("session_started", "Subagent started"),
        "SubagentStop": ("session_finished", "Subagent stopped"),
        "Stop": ("model_response", "Agent turn completed"),
        "StopFailure": ("error_observed", "Agent stop hook failed"),
        "TeammateIdle": ("activity", "Agent teammate became idle"),
        "TaskCompleted": ("activity", "Agent task completed"),
        "PreCompact": ("activity", "Context compaction started"),
        "SessionEnd": ("session_finished", f"Session ended: {detail}"),
    }
    kind, summary = mapping.get(event, ("activity", f"Claude hook: {event}"))
    return base_envelope(
        provider="anthropic",
        agent_id=agent_id,
        model=model,
        session_id=session_id,
        instance_id=instance_id,
        pid=pid,
        kind=kind,
        summary=summary,
        native_event=event,
        tool_name=tool_name if kind in {"tool_started", "tool_finished"} else None,
        resources=resources,
    )


def map_gemini(
    payload: dict[str, Any],
    *,
    agent_id_override: str | None = None,
    model_override: str | None = None,
    pid: int | None = None,
    resources: dict[str, float | int] | None = None,
) -> dict[str, Any] | None:
    event = identifier(
        first(
            payload,
            "hook_event_name",
            "hookEventName",
            "event_name",
            "eventName",
            "event",
        ),
        "Unknown",
    )
    session_id = text(first(payload, "session_id", "sessionId"))
    agent_id = override_agent_id("gemini", session_id, agent_id_override)
    model = override_model(first(payload, "model", "model_name", "modelName"), model_override)
    tool_name = text(first(payload, "tool_name", "toolName"))
    detail = identifier(
        first(payload, "source", "reason", "notification_type", "notificationType"),
        "unspecified",
    )
    mapping = {
        "SessionStart": ("session_started", f"Session started: {detail}"),
        "BeforeAgent": ("activity", "Agent loop started"),
        "BeforeModel": ("activity", "Model request started"),
        "AfterModel": ("model_response", "Model response completed"),
        "BeforeTool": ("tool_started", f"Tool started: {identifier(tool_name, 'unknown')}"),
        "AfterTool": ("tool_finished", f"Tool finished: {identifier(tool_name, 'unknown')}"),
        "AfterAgent": ("model_response", "Agent loop completed"),
        "Notification": ("activity", f"Agent notification: {detail}"),
        "PreCompress": ("activity", "Context compression started"),
        "SessionEnd": ("session_finished", f"Session ended: {detail}"),
    }
    kind, summary = mapping.get(event, ("activity", f"Gemini hook: {event}"))
    return base_envelope(
        provider="google",
        agent_id=agent_id,
        model=model,
        session_id=session_id,
        instance_id=None,
        pid=pid,
        kind=kind,
        summary=summary,
        native_event=event,
        tool_name=tool_name if kind in {"tool_started", "tool_finished"} else None,
        resources=resources,
    )


def numeric_field(payload: dict[str, Any], *keys: str) -> int | None:
    value = first(payload, *keys)
    parsed = finite_number(value)
    if parsed is None or parsed < 0:
        return None
    return int(parsed)


def codex_token_totals(params: dict[str, Any]) -> tuple[int, int] | None:
    usage = nested_dict(params, "tokenUsage", "token_usage", "usage")
    if usage is None:
        return None
    totals = nested_dict(
        usage,
        "total",
        "totalUsage",
        "total_usage",
        "totalTokenUsage",
        "total_token_usage",
    )
    selected = totals or usage
    input_tokens = numeric_field(
        selected,
        "inputTokens",
        "input_tokens",
        "promptTokens",
        "prompt_tokens",
    )
    output_tokens = numeric_field(
        selected,
        "outputTokens",
        "output_tokens",
        "completionTokens",
        "completion_tokens",
    )
    if input_tokens is None and output_tokens is None:
        return None
    return input_tokens or 0, output_tokens or 0


def codex_context(params: dict[str, Any]) -> tuple[str | None, str | None, dict[str, Any]]:
    thread = nested_dict(params, "thread") or {}
    turn = nested_dict(params, "turn") or {}
    thread_id = text(first(params, "threadId", "thread_id")) or text(
        first(thread, "id", "threadId", "thread_id")
    )
    turn_id = text(first(params, "turnId", "turn_id")) or text(
        first(turn, "id", "turnId", "turn_id")
    )
    return thread_id, turn_id, turn


def codex_event_id(
    method: str,
    thread_id: str | None,
    turn_id: str | None,
    item_id: str | None,
    suffix: str | None = None,
) -> str:
    stable_parts = [method, thread_id or "", turn_id or "", item_id or "", suffix or ""]
    if not any(stable_parts[1:]):
        return str(uuid.uuid4())
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "meta-agent:" + "|".join(stable_parts)))


def map_codex_notification(
    payload: dict[str, Any],
    *,
    token_state: MutableMapping[str, tuple[int, int]],
    agent_id_override: str | None = None,
    model_override: str | None = None,
    pid: int | None = None,
    resources: dict[str, float | int] | None = None,
) -> dict[str, Any] | None:
    method = text(payload.get("method"))
    params = payload.get("params")
    if method is None or not isinstance(params, dict):
        return None

    thread_id, turn_id, turn = codex_context(params)
    agent_id = override_agent_id("codex", thread_id, agent_id_override)
    thread = nested_dict(params, "thread") or {}
    model = override_model(
        first(params, "model", "modelName")
        or first(thread, "model", "modelName")
        or first(turn, "model", "modelName"),
        model_override,
    )
    item = nested_dict(params, "item") or {}
    item_id = text(first(item, "id", "itemId", "item_id"))
    item_type = identifier(first(item, "type", "kind"), "unknown")
    status = identifier(
        first(params, "status") or first(turn, "status") or first(thread, "status"),
        "unspecified",
    )

    if method == "thread/tokenUsage/updated":
        totals = codex_token_totals(params)
        if totals is None:
            return None
        key = thread_id or "host"
        previous_input, previous_output = token_state.get(key, (0, 0))
        token_state[key] = totals
        input_delta = max(0, totals[0] - previous_input)
        output_delta = max(0, totals[1] - previous_output)
        event_id = codex_event_id(
            method,
            thread_id,
            turn_id,
            None,
            f"{totals[0]}:{totals[1]}",
        )
        return base_envelope(
            provider="openai",
            agent_id=agent_id,
            model=model,
            session_id=thread_id,
            instance_id=None,
            pid=pid,
            kind="heartbeat",
            summary="Token usage updated",
            native_event=method,
            event_id=event_id,
            resources=resources,
            input_tokens_delta=input_delta,
            output_tokens_delta=output_delta,
        )

    kind: str
    summary: str
    tool_name: str | None = None
    if method == "thread/started":
        kind, summary = "session_started", "Thread started"
    elif method in {"thread/closed", "thread/archived"}:
        kind, summary = "session_finished", f"Thread lifecycle: {method.split('/')[-1]}"
    elif method == "thread/status/changed":
        kind = "error_observed" if status in {"systemError", "failed"} else "activity"
        summary = f"Thread status changed: {status}"
    elif method == "turn/started":
        kind, summary = "activity", "Turn started"
    elif method == "turn/completed":
        kind = (
            "error_observed"
            if status.lower() in {"failed", "systemerror", "error"}
            else "model_response"
        )
        summary = f"Turn completed: {status}"
    elif method in {"item/started", "item/completed"}:
        if item_type.lower() in SENSITIVE_CODEX_ITEM_TYPES:
            return None
        kind = "tool_started" if method.endswith("started") else "tool_finished"
        tool_name = identifier(
            first(item, "tool", "toolName", "name") or item_type,
            "unknown-tool",
        )
        summary = (
            f"Work item started: {item_type}"
            if kind == "tool_started"
            else f"Work item finished: {item_type}"
        )
    else:
        return None

    return base_envelope(
        provider="openai",
        agent_id=agent_id,
        model=model,
        session_id=thread_id,
        instance_id=None,
        pid=pid,
        kind=kind,
        summary=summary,
        native_event=method,
        tool_name=tool_name,
        event_id=codex_event_id(method, thread_id, turn_id, item_id, status),
        resources=resources,
    )


def strict_mode() -> bool:
    return os.environ.get("META_AGENT_HOOK_STRICT", "").strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


def hook_main(args: argparse.Namespace) -> int:
    try:
        payload = read_hook_input()
        pid = discover_agent_pid(args.provider, args.pid)
        resources = sample_process(pid)
        if args.provider == "claude":
            envelope = map_claude(
                payload,
                agent_id_override=args.agent_id,
                model_override=args.model,
                pid=pid,
                resources=resources,
            )
        else:
            envelope = map_gemini(
                payload,
                agent_id_override=args.agent_id,
                model_override=args.model,
                pid=pid,
                resources=resources,
            )
        if envelope is not None:
            post_hook(envelope)
    except (OSError, ValueError, RuntimeError, urllib.error.URLError, json.JSONDecodeError) as error:
        print(f"meta-agent hook adapter: {type(error).__name__}", file=sys.stderr)
        if strict_mode():
            print("{}", flush=True)
            return 1
    print("{}", flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    root.add_argument("provider", choices=["claude", "gemini"])
    root.add_argument("--agent-id")
    root.add_argument("--model")
    root.add_argument("--pid", type=int)
    return root


def main() -> int:
    return hook_main(parser().parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
