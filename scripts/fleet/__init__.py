"""Public contract surface for the verified real provider-agent fleet."""

from .cli import enqueue
from .common import (
    MAX_TASK_BYTES, Job, atomic_write_json, classify_provider_error,
    validate_admitted_job,
)
from .introspection import (
    MCPServerConfig, MCPServerSnapshot, ProviderDiscoveryError, ProviderSnapshot,
    discover_mcp_servers, discover_provider, load_mcp_server_configs, probe_mcp_server,
)
from .observability import (
    EventClient, EventIdentity, ObservationError, ObservationSummary,
    public_text, read_observation_summary,
)
from .provider import (
    enforce_credential_expiry, isolated_child_environment, provider_api_key,
    provider_environment,
)
from .supervisor import FleetRunner
from .workspace import DeliveryEvidence, missing_delivery_requirements

__all__ = [
    "MAX_TASK_BYTES", "DeliveryEvidence", "EventClient", "EventIdentity", "FleetRunner",
    "Job", "MCPServerConfig", "MCPServerSnapshot", "ObservationError", "ObservationSummary",
    "ProviderDiscoveryError", "ProviderSnapshot", "atomic_write_json", "classify_provider_error",
    "discover_mcp_servers", "discover_provider", "enqueue", "enforce_credential_expiry",
    "isolated_child_environment", "load_mcp_server_configs", "missing_delivery_requirements",
    "probe_mcp_server", "provider_api_key", "provider_environment", "public_text",
    "read_observation_summary", "validate_admitted_job",
]
