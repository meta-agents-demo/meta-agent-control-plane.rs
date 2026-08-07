"""Sanitized live provider and MCP capability discovery."""

from __future__ import annotations

import contextlib
import dataclasses
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping
from typing import Any

from .common import env_bool, env_int, utc_now

URLOpen = Callable[..., Any]


class ProviderDiscoveryError(RuntimeError):
    def __init__(self, kind: str, message: str, *, recoverable: bool) -> None:
        super().__init__(message)
        self.kind = kind
        self.recoverable = recoverable


@dataclasses.dataclass(frozen=True)
class ProviderSnapshot:
    provider: str
    credential_loaded: bool
    api_status: str
    checked_at: str
    model_count: int
    model_ids: tuple[str, ...]
    capability_labels: tuple[str, ...]
    managed_agents_status: str | None = None
    managed_agents_count: int | None = None

    def public_metadata(self) -> dict[str, str]:
        values = {
            "credential_loaded": "true" if self.credential_loaded else "false",
            "credential_source": "docker_secret_or_runtime_environment",
            "provider_api_status": self.api_status,
            "provider_model_count": str(self.model_count),
            "provider_checked_at": self.checked_at,
        }
        if self.managed_agents_status is not None:
            values["managed_agents_status"] = self.managed_agents_status
        if self.managed_agents_count is not None:
            values["managed_agents_count"] = str(self.managed_agents_count)
        return values


@dataclasses.dataclass(frozen=True)
class MCPServerConfig:
    name: str
    url: str


@dataclasses.dataclass(frozen=True)
class MCPServerSnapshot:
    name: str
    url: str
    status: str
    protocol_mode: str
    server_name: str | None = None
    server_version: str | None = None
    capability_labels: tuple[str, ...] = ()
    tool_count: int | None = None
    resource_count: int | None = None
    resource_template_count: int | None = None
    prompt_count: int | None = None
    checked_at: str = ""

    def public_metadata(self) -> dict[str, str]:
        prefix = f"mcp_{_metadata_key(self.name)}"
        values = {f"{prefix}_status": self.status, f"{prefix}_protocol": self.protocol_mode}
        for suffix, value in (
            ("tools", self.tool_count), ("resources", self.resource_count),
            ("resource_templates", self.resource_template_count), ("prompts", self.prompt_count),
        ):
            if value is not None:
                values[f"{prefix}_{suffix}"] = str(value)
        return values


def _metadata_key(value: str) -> str:
    return "".join(char.lower() if char.isalnum() else "_" for char in value).strip("_")[:48] or "server"


def _bounded_string(value: Any, maximum: int = 256) -> str | None:
    if not isinstance(value, str):
        return None
    candidate = value.strip()
    return candidate[:maximum] if candidate else None


def _request_json(
    url: str,
    headers: Mapping[str, str],
    *,
    timeout: int,
    urlopen: URLOpen,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], Any]:
    body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode("utf-8")
    request_headers = {"Accept": "application/json", "User-Agent": "meta-agent-control-plane/capability-discovery", **headers}
    if body is not None:
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=body, headers=request_headers, method=method)
    with urlopen(request, timeout=timeout) as response:
        raw = response.read(2 * 1024 * 1024)
        if not raw:
            return {}, response.headers
        value = json.loads(raw.decode("utf-8"))
        if not isinstance(value, dict):
            raise ValueError("remote endpoint returned a non-object JSON payload")
        return value, response.headers


def _provider_error(provider: str, error: BaseException) -> ProviderDiscoveryError:
    if isinstance(error, urllib.error.HTTPError):
        status = int(error.code)
        if status in {401, 403}:
            return ProviderDiscoveryError("authentication_failed", f"{provider} capability discovery rejected the configured credential", recoverable=False)
        if status == 429:
            return ProviderDiscoveryError("rate_limited", f"{provider} capability discovery was rate limited", recoverable=True)
        if status >= 500:
            return ProviderDiscoveryError("provider_unavailable", f"{provider} capability discovery is temporarily unavailable", recoverable=True)
        return ProviderDiscoveryError("provider_api_error", f"{provider} capability discovery returned HTTP {status}", recoverable=False)
    if isinstance(error, (urllib.error.URLError, TimeoutError, OSError)):
        return ProviderDiscoveryError("network_unavailable", f"{provider} capability discovery could not reach the provider API", recoverable=True)
    return ProviderDiscoveryError("invalid_provider_response", f"{provider} capability discovery returned an invalid response", recoverable=False)


def _supported_paths(value: Any, prefix: str = "") -> set[str]:
    paths: set[str] = set()
    if not isinstance(value, dict):
        return paths
    if value.get("supported") is True and prefix:
        paths.add(prefix)
    for key, child in value.items():
        if key != "supported":
            paths.update(_supported_paths(child, f"{prefix}.{key}" if prefix else str(key)))
    return paths


def discover_provider(provider: str, api_key: str | None, *, urlopen: URLOpen = urllib.request.urlopen) -> ProviderSnapshot:
    if not api_key:
        raise ProviderDiscoveryError("missing_credential", f"{provider} capability discovery requires a configured credential", recoverable=False)
    timeout = env_int("META_AGENT_DISCOVERY_TIMEOUT_SECONDS", 10, 2, 60)
    provider = provider.lower()
    if provider == "openai":
        base = os.getenv("META_AGENT_OPENAI_API_BASE", "https://api.openai.com").rstrip("/")
        url = f"{base}/v1/models"
        headers = {"Authorization": f"Bearer {api_key}"}
    elif provider == "anthropic":
        base = os.getenv("META_AGENT_ANTHROPIC_API_BASE", "https://api.anthropic.com").rstrip("/")
        url = f"{base}/v1/models?limit=1000"
        headers = {"X-Api-Key": api_key, "anthropic-version": "2023-06-01"}
    else:
        raise ProviderDiscoveryError("unsupported_provider", f"unsupported provider: {provider}", recoverable=False)
    try:
        payload, _response_headers = _request_json(url, headers, timeout=timeout, urlopen=urlopen)
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, ValueError, json.JSONDecodeError) as error:
        raise _provider_error(provider, error) from error
    raw_models = payload.get("data", [])
    if not isinstance(raw_models, list):
        raise ProviderDiscoveryError("invalid_provider_response", f"{provider} model inventory did not contain a data list", recoverable=False)
    model_ids: list[str] = []
    capability_paths: set[str] = set()
    for item in raw_models[:1000]:
        if not isinstance(item, dict):
            continue
        model_id = _bounded_string(item.get("id"), 256)
        if model_id and len(model_ids) < 256:
            model_ids.append(model_id)
        if provider == "anthropic":
            capability_paths.update(_supported_paths(item.get("capabilities")))
    labels = {f"{provider}.models.list"}
    labels.update(f"{provider}.model_capability.{path}" for path in sorted(capability_paths)[:96])
    managed_status: str | None = None
    managed_count: int | None = None
    if provider == "anthropic" and env_bool("META_AGENT_ANTHROPIC_DISCOVER_MANAGED_AGENTS", True):
        managed_status, managed_count = _discover_anthropic_managed_agents(base, headers, timeout, urlopen)
        if managed_status == "ready":
            labels.add("anthropic.managed_agents.list")
    return ProviderSnapshot(
        provider=provider,
        credential_loaded=True,
        api_status="ready",
        checked_at=utc_now(),
        model_count=len(raw_models),
        model_ids=tuple(model_ids),
        capability_labels=tuple(sorted(labels)[:128]),
        managed_agents_status=managed_status,
        managed_agents_count=managed_count,
    )


def _discover_anthropic_managed_agents(base: str, base_headers: Mapping[str, str], timeout: int, urlopen: URLOpen) -> tuple[str, int | None]:
    headers = {**base_headers, "anthropic-beta": "managed-agents-2026-04-01"}
    total = 0
    page: str | None = None
    for _ in range(5):
        query = {"limit": "100"}
        if page:
            query["page"] = page
        try:
            payload, _response_headers = _request_json(
                f"{base}/v1/agents?{urllib.parse.urlencode(query)}", headers, timeout=timeout, urlopen=urlopen
            )
        except urllib.error.HTTPError as error:
            if error.code in {401, 403}:
                return "forbidden", None
            if error.code == 404:
                return "not_available", None
            if error.code == 429:
                return "rate_limited", None
            return "provider_error", None
        except (urllib.error.URLError, TimeoutError, OSError):
            return "unreachable", None
        except (ValueError, json.JSONDecodeError):
            return "invalid_response", None
        data = payload.get("data", [])
        if not isinstance(data, list):
            return "invalid_response", None
        total += len(data)
        next_page = _bounded_string(payload.get("next_page"), 2_048)
        if not next_page:
            return "ready", total
        page = next_page
    return "ready_truncated", total


def load_mcp_server_configs() -> tuple[MCPServerConfig, ...]:
    raw = os.getenv("META_AGENT_MCP_SERVERS_JSON", "[]").strip() or "[]"
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ValueError("META_AGENT_MCP_SERVERS_JSON must be valid JSON") from error
    if not isinstance(value, list) or len(value) > 16:
        raise ValueError("META_AGENT_MCP_SERVERS_JSON must be a list of at most 16 servers")
    allowed_hosts = {item.strip().lower() for item in os.getenv("META_AGENT_MCP_ALLOWED_HOSTS", "developers.openai.com").split(",") if item.strip()}
    configs: list[MCPServerConfig] = []
    seen_names: set[str] = set()
    for item in value:
        if not isinstance(item, dict) or set(item) != {"name", "url"}:
            raise ValueError("each MCP server must contain only name and url")
        name = _bounded_string(item.get("name"), 64)
        url = _bounded_string(item.get("url"), 2_048)
        if not name or not url or name in seen_names:
            raise ValueError("MCP server names and URLs must be non-empty and names must be unique")
        parsed = urllib.parse.urlsplit(url)
        host = (parsed.hostname or "").lower()
        if parsed.scheme != "https" or not host or host not in allowed_hosts:
            raise ValueError(f"MCP server {name} must use HTTPS and an explicitly allowed host")
        configs.append(MCPServerConfig(name, url))
        seen_names.add(name)
    return tuple(configs)


def discover_mcp_servers(*, urlopen: URLOpen = urllib.request.urlopen) -> tuple[MCPServerSnapshot, ...]:
    return tuple(probe_mcp_server(config, urlopen=urlopen) for config in load_mcp_server_configs())


def _decode_mcp_payload(raw: bytes, content_type: str) -> dict[str, Any]:
    if not raw:
        return {}
    text = raw.decode("utf-8")
    if "text/event-stream" in content_type:
        for line in text.splitlines():
            if line.startswith("data:") and line[5:].strip():
                value = json.loads(line[5:].strip())
                if isinstance(value, dict):
                    return value
        raise ValueError("MCP event stream contained no JSON data event")
    value = json.loads(text)
    if not isinstance(value, dict):
        raise ValueError("MCP endpoint returned non-object JSON")
    return value


def _mcp_request_meta(protocol_version: str) -> dict[str, Any]:
    return {
        "io.modelcontextprotocol/protocolVersion": protocol_version,
        "io.modelcontextprotocol/clientInfo": {"name": "meta-agent-control-plane", "version": "0.1.0"},
        "io.modelcontextprotocol/clientCapabilities": {},
    }


def _mcp_post(
    config: MCPServerConfig,
    message: dict[str, Any],
    *,
    protocol_version: str,
    protocol_mode: str,
    session_id: str | None,
    timeout: int,
    urlopen: URLOpen,
) -> tuple[dict[str, Any], str | None]:
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        "MCP-Protocol-Version": protocol_version,
        "User-Agent": "meta-agent-control-plane/mcp-discovery",
    }
    if protocol_mode == "modern":
        method = _bounded_string(message.get("method"), 256)
        if method:
            headers["Mcp-Method"] = method
        params = message.get("params")
        if method in {"tools/call", "prompts/get"} and isinstance(params, dict):
            name = _bounded_string(params.get("name"), 2_048)
            if name:
                headers["Mcp-Name"] = name
        elif method == "resources/read" and isinstance(params, dict):
            uri = _bounded_string(params.get("uri"), 2_048)
            if uri:
                headers["Mcp-Name"] = uri
    elif session_id:
        headers["Mcp-Session-Id"] = session_id
    request = urllib.request.Request(
        config.url,
        data=json.dumps(message, separators=(",", ":")).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    with urlopen(request, timeout=timeout) as response:
        value = _decode_mcp_payload(response.read(2 * 1024 * 1024), response.headers.get("Content-Type", "application/json"))
        return value, response.headers.get("Mcp-Session-Id") or session_id


def _rpc_error_code(value: dict[str, Any]) -> int | None:
    error = value.get("error")
    return int(error["code"]) if isinstance(error, dict) and isinstance(error.get("code"), int) else None


def _mcp_list_count(
    config: MCPServerConfig,
    method: str,
    result_key: str,
    *,
    protocol_version: str,
    protocol_mode: str,
    session_id: str | None,
    timeout: int,
    urlopen: URLOpen,
) -> int:
    total = 0
    cursor: str | None = None
    for index in range(5):
        params: dict[str, Any] = {} if cursor is None else {"cursor": cursor}
        if protocol_mode == "modern":
            params["_meta"] = _mcp_request_meta(protocol_version)
        response, session_id = _mcp_post(
            config,
            {"jsonrpc": "2.0", "id": f"{method}-{index}", "method": method, "params": params},
            protocol_version=protocol_version,
            protocol_mode=protocol_mode,
            session_id=session_id,
            timeout=timeout,
            urlopen=urlopen,
        )
        if "error" in response:
            raise ValueError(f"MCP method {method} returned an error")
        result = response.get("result")
        if not isinstance(result, dict) or not isinstance(result.get(result_key, []), list):
            raise ValueError(f"MCP method {method} returned an invalid list")
        total += len(result.get(result_key, []))
        cursor = _bounded_string(result.get("nextCursor"), 2_048)
        if not cursor:
            return total
    return total


def _server_identity(result: Mapping[str, Any], response: Mapping[str, Any]) -> tuple[str | None, str | None]:
    server_info = result.get("serverInfo")
    if not isinstance(server_info, dict) and isinstance(result.get("_meta"), dict):
        server_info = result["_meta"].get("io.modelcontextprotocol/serverInfo")
    if not isinstance(server_info, dict) and isinstance(response.get("_meta"), dict):
        server_info = response["_meta"].get("io.modelcontextprotocol/serverInfo")
    if not isinstance(server_info, dict):
        return None, None
    return _bounded_string(server_info.get("name"), 128), _bounded_string(server_info.get("version"), 128)


def _snapshot_from_capabilities(
    config: MCPServerConfig,
    *,
    status: str,
    protocol_mode: str,
    protocol_version: str,
    capabilities: Mapping[str, Any],
    server_name: str | None,
    server_version: str | None,
    session_id: str | None,
    timeout: int,
    checked_at: str,
    urlopen: URLOpen,
) -> MCPServerSnapshot:
    labels: set[str] = {f"mcp.{config.name}.protocol.{protocol_version}"}
    counts: dict[str, int | None] = {"tools": None, "resources": None, "templates": None, "prompts": None}
    for capability, method, result_key in (
        ("tools", "tools/list", "tools"),
        ("resources", "resources/list", "resources"),
        ("templates", "resources/templates/list", "resourceTemplates"),
        ("prompts", "prompts/list", "prompts"),
    ):
        if not ((capability == "templates" and "resources" in capabilities) or capability in capabilities):
            continue
        labels.add(f"mcp.{config.name}.{method}")
        try:
            counts[capability] = _mcp_list_count(
                config,
                method,
                result_key,
                protocol_version=protocol_version,
                protocol_mode="modern" if protocol_mode == "modern_server_discover" else "legacy",
                session_id=session_id,
                timeout=timeout,
                urlopen=urlopen,
            )
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, ValueError, json.JSONDecodeError):
            counts[capability] = None
    if "completions" in capabilities:
        labels.add(f"mcp.{config.name}.completion.declared_legacy")
    if "logging" in capabilities:
        labels.add(f"mcp.{config.name}.logging.declared_legacy")
    return MCPServerSnapshot(
        config.name, config.url, status, protocol_mode,
        server_name=server_name, server_version=server_version,
        capability_labels=tuple(sorted(labels)),
        tool_count=counts["tools"], resource_count=counts["resources"],
        resource_template_count=counts["templates"], prompt_count=counts["prompts"], checked_at=checked_at,
    )


def _probe_legacy_mcp_server(config: MCPServerConfig, *, protocol_version: str, timeout: int, checked_at: str, urlopen: URLOpen) -> MCPServerSnapshot:
    initialize = {
        "jsonrpc": "2.0", "id": "initialize", "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {},
            "clientInfo": {"name": "meta-agent-control-plane", "version": "0.1.0"},
        },
    }
    try:
        response, session_id = _mcp_post(
            config, initialize, protocol_version=protocol_version, protocol_mode="legacy",
            session_id=None, timeout=timeout, urlopen=urlopen,
        )
    except urllib.error.HTTPError as error:
        return MCPServerSnapshot(config.name, config.url, "authentication_required" if error.code in {401, 403} else "http_error", "unavailable", checked_at=checked_at)
    except (urllib.error.URLError, TimeoutError, OSError):
        return MCPServerSnapshot(config.name, config.url, "unreachable", "unavailable", checked_at=checked_at)
    except (ValueError, json.JSONDecodeError):
        return MCPServerSnapshot(config.name, config.url, "invalid_response", "unavailable", checked_at=checked_at)
    if _rpc_error_code(response) == -32601:
        return MCPServerSnapshot(config.name, config.url, "method_not_found", "legacy_initialize", checked_at=checked_at)
    result = response.get("result")
    if not isinstance(result, dict):
        return MCPServerSnapshot(config.name, config.url, "invalid_response", "legacy_initialize", checked_at=checked_at)
    capabilities = result.get("capabilities", {})
    if not isinstance(capabilities, dict):
        capabilities = {}
    negotiated = _bounded_string(result.get("protocolVersion"), 64) or protocol_version
    server_name, server_version = _server_identity(result, response)
    with contextlib.suppress(Exception):
        _mcp_post(
            config,
            {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
            protocol_version=negotiated,
            protocol_mode="legacy",
            session_id=session_id,
            timeout=timeout,
            urlopen=urlopen,
        )
    return _snapshot_from_capabilities(
        config, status="ready", protocol_mode="legacy_initialize", protocol_version=negotiated,
        capabilities=capabilities, server_name=server_name, server_version=server_version,
        session_id=session_id, timeout=timeout, checked_at=checked_at, urlopen=urlopen,
    )


def probe_mcp_server(config: MCPServerConfig, *, urlopen: URLOpen = urllib.request.urlopen) -> MCPServerSnapshot:
    timeout = env_int("META_AGENT_MCP_DISCOVERY_TIMEOUT_SECONDS", 8, 2, 60)
    modern_version = os.getenv("META_AGENT_MCP_PROTOCOL_VERSION", "2026-07-28").strip() or "2026-07-28"
    legacy_version = os.getenv("META_AGENT_MCP_LEGACY_PROTOCOL_VERSION", "2025-11-25").strip() or "2025-11-25"
    checked_at = utc_now()
    discover = {
        "jsonrpc": "2.0", "id": "server-discover", "method": "server/discover",
        "params": {"_meta": _mcp_request_meta(modern_version)},
    }
    fallback = False
    try:
        response, _session = _mcp_post(
            config, discover, protocol_version=modern_version, protocol_mode="modern",
            session_id=None, timeout=timeout, urlopen=urlopen,
        )
    except urllib.error.HTTPError as error:
        if error.code in {401, 403}:
            return MCPServerSnapshot(config.name, config.url, "authentication_required", "unavailable", checked_at=checked_at)
        if error.code in {400, 404, 405}:
            fallback = True
        else:
            return MCPServerSnapshot(config.name, config.url, "http_error", "unavailable", checked_at=checked_at)
    except (urllib.error.URLError, TimeoutError, OSError):
        return MCPServerSnapshot(config.name, config.url, "unreachable", "unavailable", checked_at=checked_at)
    except (ValueError, json.JSONDecodeError):
        return MCPServerSnapshot(config.name, config.url, "invalid_response", "unavailable", checked_at=checked_at)
    else:
        if _rpc_error_code(response) == -32601:
            fallback = True
        else:
            result = response.get("result")
            if not isinstance(result, dict):
                return MCPServerSnapshot(config.name, config.url, "invalid_response", "modern_server_discover", checked_at=checked_at)
            versions = result.get("supportedVersions")
            capabilities = result.get("capabilities")
            if not isinstance(versions, list) or not all(isinstance(value, str) for value in versions) or not isinstance(capabilities, dict):
                return MCPServerSnapshot(config.name, config.url, "invalid_response", "modern_server_discover", checked_at=checked_at)
            if modern_version not in versions:
                if legacy_version in versions or any(value.startswith("2025-") for value in versions):
                    fallback = True
                else:
                    return MCPServerSnapshot(
                        config.name, config.url, "unsupported_protocol", "modern_server_discover",
                        capability_labels=tuple(f"mcp.{config.name}.supported_version.{value}" for value in versions[:16]),
                        checked_at=checked_at,
                    )
            else:
                server_name, server_version = _server_identity(result, response)
                return _snapshot_from_capabilities(
                    config, status="ready", protocol_mode="modern_server_discover", protocol_version=modern_version,
                    capabilities=capabilities, server_name=server_name, server_version=server_version,
                    session_id=None, timeout=timeout, checked_at=checked_at, urlopen=urlopen,
                )
    if fallback:
        return _probe_legacy_mcp_server(
            config, protocol_version=legacy_version, timeout=timeout, checked_at=checked_at, urlopen=urlopen
        )
    return MCPServerSnapshot(config.name, config.url, "method_not_found", "unavailable", checked_at=checked_at)
