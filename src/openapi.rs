use serde_json::{Value, json};

use crate::{
    daemon::BoundAddresses,
    model::{EVENT_KINDS, UDP_EVENT_KINDS},
};

pub fn document(
    addresses: BoundAddresses,
    ingestion_protected: bool,
    reads_protected: bool,
) -> Value {
    let ingestion_security = if ingestion_protected {
        json!([{ "bearerAuth": [] }])
    } else {
        json!([])
    };
    let read_security = if reads_protected {
        json!([{ "bearerAuth": [] }])
    } else {
        json!([])
    };

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Meta-Agent Control Plane API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Provider-neutral telemetry for observable agent goals, tasks, progress, explicit reflection, and learned lessons. The protocol intentionally excludes hidden chain-of-thought."
        },
        "servers": [{ "url": "/", "description": "Same-origin daemon API" }],
        "paths": {
            "/api/v1/bridge/rooms": {
                "get": {
                    "summary": "List bounded bridge rooms",
                    "security": read_security.clone(),
                    "responses": {
                        "200": { "description": "Bridge room summaries" },
                        "401": { "description": "Read API authentication failed" }
                    }
                },
                "post": {
                    "summary": "Create an idempotent shared bridge room",
                    "security": ingestion_security.clone(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BridgeRoomInput" } } } },
                    "responses": {
                        "201": { "description": "Room created or returned" },
                        "400": { "description": "Invalid room or secret-like visible text" },
                        "401": { "description": "Authentication failed" }
                    }
                }
            },
            "/api/v1/bridge/rooms/{room_slug}": {
                "get": {
                    "summary": "Read a bridge room snapshot",
                    "security": read_security.clone(),
                    "parameters": [{ "name": "room_slug", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Room, participants, messages, contacts, and transport counters" },
                        "401": { "description": "Authentication failed" },
                        "404": { "description": "Room not found" }
                    }
                }
            },
            "/api/v1/bridge/rooms/{room_slug}/join": {
                "post": {
                    "summary": "Declare a participant in a bridge room",
                    "description": "Participant identity is scoped to the authenticated local control-plane boundary; it is not a provider-issued identity assertion.",
                    "security": ingestion_security.clone(),
                    "parameters": [{ "name": "room_slug", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Participant joined" },
                        "400": { "description": "Invalid participant" },
                        "401": { "description": "Authentication failed" }
                    }
                }
            },
            "/api/v1/bridge/rooms/{room_slug}/messages": {
                "get": {
                    "summary": "Read retained visible bridge summaries",
                    "security": read_security.clone(),
                    "parameters": [{ "name": "room_slug", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "Bounded visible message summaries" }, "401": { "description": "Authentication failed" } }
                },
                "post": {
                    "summary": "Post a joined participant message over HTTP",
                    "description": "Accepts explicit visible summaries only. Credential-like text and participant identity mismatches are rejected.",
                    "security": ingestion_security.clone(),
                    "parameters": [{ "name": "room_slug", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BridgeMessageInput" } } } },
                    "responses": { "202": { "description": "Message accepted" }, "400": { "description": "Invalid bridge message" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/events": {
                "post": {
                    "summary": "Ingest one agent event",
                    "security": ingestion_security.clone(),
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/EventEnvelope" }
                            }
                        }
                    },
                    "responses": {
                        "202": {
                            "description": "Event accepted",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/IngestAck" }
                                }
                            }
                        },
                        "400": { "description": "Malformed or invalid event" },
                        "401": { "description": "Authentication failed" }
                    }
                }
            },
            "/api/v1/coordination": {
                "get": {
                    "summary": "Build the current deterministic coordination plan",
                    "description": "Returns bounded dependency-safe assignments, operator interventions, and held tasks derived from the same visible retained snapshot and metacognition rules. The endpoint is read-only and never dispatches or mutates agent work.",
                    "security": read_security.clone(),
                    "parameters": [
                        {
                            "name": "max_assignments",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 256, "default": 16 }
                        },
                        {
                            "name": "max_assignments_per_agent",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 32, "default": 2 }
                        },
                        {
                            "name": "max_interventions",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 512, "default": 32 }
                        },
                        {
                            "name": "max_holds",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 1024, "default": 64 }
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Current coordination plan",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/CoordinationPlan" }
                                }
                            }
                        },
                        "400": { "description": "Planning query was malformed, unknown, duplicated, zero, or above its server cap" },
                        "401": { "description": "Read API authentication failed" },
                        "500": { "description": "The configured planning policy could not produce a plan" }
                    }
                }
            },
            "/api/v1/explorer": {
                "get": {
                    "summary": "Read the bounded operator explorer projection",
                    "description": "Returns sorted agents, retained sessions, recent timeline events, lessons, cache pressure, counters, and explicit retention omissions from one coherent snapshot. The endpoint is read-only.",
                    "security": read_security.clone(),
                    "parameters": [
                        {
                            "name": "timeline_limit",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 250, "default": 100 }
                        },
                        {
                            "name": "session_limit",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 250, "default": 100 }
                        },
                        {
                            "name": "lesson_limit",
                            "in": "query",
                            "required": false,
                            "schema": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 250 }
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Current operator explorer projection",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/ExplorerSnapshot" }
                                }
                            }
                        },
                        "400": { "description": "Explorer query was malformed, unknown, duplicated, zero, or above its server cap" },
                        "401": { "description": "Read API authentication failed" },
                        "500": { "description": "The explorer projection could not be produced" }
                    }
                }
            },
            "/api/v1/metacognition": {
                "get": {
                    "summary": "Read the current explainable metacognition projection",
                    "description": "Returns deterministic diagnostics, evidence-backed progress, critical paths, stalls, retry loops, and recommended actions derived only from visible retained state.",
                    "security": read_security.clone(),
                    "responses": {
                        "200": {
                            "description": "Current metacognition projection",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/MetacognitionSnapshot" }
                                }
                            }
                        },
                        "401": { "description": "Read API authentication failed" }
                    }
                }
            },
            "/api/v1/snapshot": {
                "get": {
                    "summary": "Read the current bounded in-memory projection",
                    "security": read_security.clone(),
                    "responses": {
                        "200": {
                            "description": "Current state",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/Snapshot" }
                                }
                            }
                        },
                        "401": { "description": "Read API authentication failed" }
                    }
                }
            },
            "/api/v1/runtime/snapshot": {
                "get": {
                    "summary": "Read real process and explicit hook telemetry",
                    "security": read_security.clone(),
                    "responses": { "200": { "description": "Current runtime snapshot" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/hooks": {
                "post": {
                    "summary": "Ingest an explicit provider or wrapper runtime hook",
                    "security": ingestion_security.clone(),
                    "responses": { "202": { "description": "Hook accepted" }, "400": { "description": "Invalid or privacy-unsafe hook" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/host-observations": {
                "post": {
                    "summary": "Ingest a privacy-minimized native host process sample",
                    "description": "Accepts fixed process identity, lineage, CPU, and RSS fields. Command arguments, environments, prompts, and provider credentials are outside this contract.",
                    "security": ingestion_security.clone(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/HostProcessObservationEnvelope" } } } },
                    "responses": { "202": { "description": "Host observation accepted" }, "400": { "description": "Invalid, future, duplicate-PID, or out-of-order observation" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/collection": {
                "post": {
                    "summary": "Enable or pause the local Linux process collector",
                    "security": ingestion_security.clone(),
                    "responses": { "200": { "description": "Collector state changed" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/commands": {
                "post": {
                    "summary": "Queue a cooperative command for a hook-capable agent",
                    "security": ingestion_security.clone(),
                    "responses": { "202": { "description": "Command queued" }, "409": { "description": "Agent is not control capable" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/commands/poll": {
                "post": {
                    "summary": "Poll pending cooperative commands",
                    "security": ingestion_security.clone(),
                    "responses": { "200": { "description": "Pending commands" }, "401": { "description": "Authentication failed" } }
                }
            },
            "/api/v1/runtime/commands/ack": {
                "post": {
                    "summary": "Acknowledge a cooperative command",
                    "security": ingestion_security.clone(),
                    "responses": { "200": { "description": "Command acknowledgement recorded" }, "401": { "description": "Authentication failed" }, "404": { "description": "Command not found" } }
                }
            },
            "/ws/bridge/{room_slug}": {
                "get": {
                    "summary": "Authenticated bridge room WebSocket",
                    "description": "Send authentication first, join a declared participant, then exchange visible bridge summaries and room updates.",
                    "security": ingestion_security.clone()
                }
            },
            "/ws/agent": {
                "get": {
                    "summary": "WebSocket agent ingestion",
                    "description": "Authenticate during the WebSocket upgrade with a Bearer header or token query parameter, then send EventEnvelope JSON messages.",
                    "security": ingestion_security.clone()
                }
            },
            "/ws/ui": {
                "get": {
                    "summary": "Live projection invalidation stream",
                    "description": "Emits an authenticated handshake followed by StoreUpdate messages; clients refetch the snapshot after each update. When read protection is enabled, the first client message must be JSON with a token field."
                }
            },
            "/healthz": {
                "get": {
                    "summary": "Liveness probe",
                    "responses": { "200": { "description": "Alive" } }
                }
            },
            "/readyz": {
                "get": {
                    "summary": "Readiness probe",
                    "responses": { "200": { "description": "Ready" } }
                }
            },
            "/metrics": {
                "get": {
                    "summary": "Prometheus text exposition",
                    "security": read_security,
                    "responses": { "200": { "description": "Metrics" } }
                }
            }
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "opaque"
                }
            },
            "schemas": {
                "AgentRef": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["agent_id", "provider", "model"],
                    "properties": {
                        "agent_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "provider": { "type": "string", "minLength": 1, "maxLength": 128 },
                        "model": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "instance_id": { "type": ["string", "null"], "maxLength": 256 }
                    }
                },
                "EventEnvelope": {
                    "type": "object",
                    "additionalProperties": true,
                    "required": ["protocol_version", "event_id", "occurred_at", "agent", "kind", "data"],
                    "properties": {
                        "protocol_version": { "const": "v1" },
                        "event_id": { "type": "string", "format": "uuid" },
                        "occurred_at": { "type": "string", "format": "date-time" },
                        "agent": { "$ref": "#/components/schemas/AgentRef" },
                        "session_id": { "type": ["string", "null"] },
                        "correlation_id": { "type": ["string", "null"] },
                        "sequence": { "type": ["integer", "null"], "minimum": 0 },
                        "kind": { "type": "string", "enum": EVENT_KINDS },
                        "data": {
                            "type": "object",
                            "description": "Payload selected by kind. See docs/protocol.md for every event shape."
                        }
                    }
                },
                "TransportFrame": {
                    "type": "object",
                    "required": ["event"],
                    "properties": {
                        "token": { "type": ["string", "null"], "writeOnly": true },
                        "event": { "$ref": "#/components/schemas/EventEnvelope" }
                    }
                },
                "IngestAck": {
                    "type": "object",
                    "required": ["accepted", "duplicate", "event_id", "revision", "transport", "received_at"],
                    "properties": {
                        "accepted": { "type": "boolean" },
                        "duplicate": { "type": "boolean" },
                        "event_id": { "type": "string", "format": "uuid" },
                        "revision": { "type": "integer", "minimum": 0 },
                        "transport": { "type": "string", "enum": ["http", "websocket", "tcp", "udp"] },
                        "received_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Snapshot": {
                    "type": "object",
                    "description": "Bounded LRU projection containing agents, goals, tasks, lessons, recent events, an independent idempotency window, counters, and cache pressure."
                },
                "ExplorerSnapshot": {
                    "type": "object",
                    "description": "Read-only bounded operator projection containing sorted agents, retained session summaries, timeline events, lessons, system pressure, counters, policy, and explicit omission counts."
                },
                "MetacognitionSnapshot": {
                    "type": "object",
                    "description": "Deterministic read-only projection of visible progress, evidence, dependency, retry, stall, and consistency diagnostics."
                },
                "CoordinationPlan": {
                    "type": "object",
                    "description": "Deterministic read-only plan containing bounded assignments, interventions, held tasks, planning policies, and source-event provenance."
                },
                "BridgeRoomInput": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["slug", "title", "objective"],
                    "properties": {
                        "slug": { "type": "string", "pattern": "^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$" },
                        "title": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "objective": { "type": "string", "minLength": 1, "maxLength": 2048 }
                    }
                },
                "BridgeMessageInput": {
                    "type": "object",
                    "description": "A credential-filtered visible summary from an already joined participant. Hidden reasoning and raw prompts are outside the contract."
                },
                "HostProcessObservationEnvelope": {
                    "type": "object",
                    "description": "A bounded native process sample containing no command arguments, environment variables, prompts, or credentials."
                }
            }
        },
        "x-meta-agent-transports": {
            "http": "/api/v1/events",
            "websocket": "/ws/agent",
            "tcp_ndjson": format!("<daemon-host>:{}", addresses.tcp.port()),
            "udp_json": format!("<daemon-host>:{}", addresses.udp.port()),
            "udp_policy": "heartbeat, progress_updated, reflection_recorded, error_observed, and agent_status_changed only",
            "bound_addresses": {
                "http": addresses.http.to_string(),
                "tcp": addresses.tcp.to_string(),
                "udp": addresses.udp.to_string()
            }
        },
        "x-meta-agent-event-kinds": EVENT_KINDS,
        "x-meta-agent-udp-event-kinds": UDP_EVENT_KINDS
    })
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;

    #[test]
    fn publishes_all_protocol_event_kinds() {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);
        let document = document(
            BoundAddresses {
                http: address,
                tcp: address,
                udp: address,
            },
            true,
            true,
        );
        let kinds = document["x-meta-agent-event-kinds"].as_array().unwrap();
        assert_eq!(kinds.len(), EVENT_KINDS.len());
        assert_eq!(
            document["x-meta-agent-udp-event-kinds"]
                .as_array()
                .map(Vec::len),
            Some(UDP_EVENT_KINDS.len())
        );
        assert_eq!(document["servers"][0]["url"], "/");
        assert!(document["paths"]["/metrics"]["get"]["security"].is_array());
        assert_eq!(
            document["paths"]["/api/v1/coordination"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/CoordinationPlan"
        );
        assert_eq!(
            document["paths"]["/api/v1/coordination"]["get"]["parameters"]
                .as_array()
                .map(Vec::len),
            Some(4)
        );
        assert_eq!(
            document["paths"]["/api/v1/coordination"]["get"]["parameters"][0]["schema"]["maximum"],
            256
        );
        assert_eq!(
            document["paths"]["/api/v1/explorer"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/ExplorerSnapshot"
        );
        assert_eq!(
            document["paths"]["/api/v1/explorer"]["get"]["parameters"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );
        assert_eq!(
            document["paths"]["/api/v1/metacognition"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/MetacognitionSnapshot"
        );
    }

    #[test]
    fn checked_in_openapi_tracks_runtime_protocol_extensions() {
        let checked_in: Value = serde_json::from_str(include_str!("../docs/openapi.json"))
            .expect("checked-in OpenAPI must be valid JSON");
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);
        let runtime = document(
            BoundAddresses {
                http: address,
                tcp: address,
                udp: address,
            },
            true,
            true,
        );

        assert_eq!(
            checked_in["x-meta-agent-event-kinds"],
            runtime["x-meta-agent-event-kinds"]
        );
        assert_eq!(
            checked_in["x-meta-agent-udp-event-kinds"],
            runtime["x-meta-agent-udp-event-kinds"]
        );
        assert_eq!(
            checked_in["paths"]
                .as_object()
                .map(|paths| paths.keys().collect::<Vec<_>>()),
            runtime["paths"]
                .as_object()
                .map(|paths| paths.keys().collect::<Vec<_>>())
        );
        assert_eq!(
            checked_in["paths"]["/api/v1/coordination"]["get"]["parameters"],
            runtime["paths"]["/api/v1/coordination"]["get"]["parameters"]
        );
        assert_eq!(
            checked_in["paths"]["/api/v1/explorer"]["get"]["parameters"],
            runtime["paths"]["/api/v1/explorer"]["get"]["parameters"]
        );
    }
}
