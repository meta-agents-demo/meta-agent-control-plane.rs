use axum::{
    Extension, Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::bearer_token,
    http::AppState,
    runtime::{
        ControlCommand, ControlCommandAck, ControlCommandRequest, RuntimeError,
        RuntimeHookEnvelope, RuntimeMonitor, RuntimeSnapshot,
    },
    runtime_ui,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(&'static str),
    AgentNotHookBacked,
    CommandAgentMismatch,
    NotFound,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "message": "Authentication failed"
                })),
            )
                .into_response(),
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_runtime_request",
                    "message": message
                })),
            )
                .into_response(),
            Self::AgentNotHookBacked => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "runtime_agent_not_hook_backed",
                    "message": "Agent has no cooperative runtime hook channel"
                })),
            )
                .into_response(),
            Self::CommandAgentMismatch => (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "runtime_command_agent_mismatch",
                    "message": "Control command belongs to a different agent"
                })),
            )
                .into_response(),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "runtime_command_not_found",
                    "message": "Control command was not found"
                })),
            )
                .into_response(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionControl {
    enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct CollectionControlResponse {
    enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandPollRequest {
    agent_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct HookAck {
    event_id: Uuid,
    accepted: bool,
    duplicate: bool,
}

pub fn router(state: AppState, runtime: RuntimeMonitor) -> Router {
    Router::new()
        .route("/runtime", get(page))
        .route("/api/v1/runtime/snapshot", get(snapshot))
        .route("/api/v1/runtime/hooks", post(ingest_hook))
        .route("/api/v1/runtime/collection", post(set_collection))
        .route("/api/v1/runtime/commands", post(enqueue_command))
        .route("/api/v1/runtime/commands/poll", post(pending_commands))
        .route("/api/v1/runtime/commands/ack", post(acknowledge_command))
        .layer(Extension(runtime))
        .with_state(state)
}

async fn page(State(state): State<AppState>) -> Html<String> {
    Html(runtime_ui::dashboard(state.auth.reads_are_protected()))
}

async fn snapshot(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
) -> Result<Json<RuntimeSnapshot>, ApiError> {
    authorize_read(&state, &headers)?;
    Ok(Json(runtime.snapshot().await))
}

async fn ingest_hook(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
    payload: Result<Json<RuntimeHookEnvelope>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(hook) = payload.map_err(|_| ApiError::BadRequest("Invalid runtime hook JSON"))?;
    let event_id = hook.event_id;
    match runtime.ingest_hook(hook).await {
        Ok(()) => Ok((
            StatusCode::ACCEPTED,
            Json(HookAck {
                event_id,
                accepted: true,
                duplicate: false,
            }),
        )),
        Err(RuntimeError::DuplicateHook) => Ok((
            StatusCode::OK,
            Json(HookAck {
                event_id,
                accepted: true,
                duplicate: true,
            }),
        )),
        Err(_) => Err(ApiError::BadRequest("Runtime hook failed validation")),
    }
}

async fn set_collection(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
    payload: Result<Json<CollectionControl>, JsonRejection>,
) -> Result<Json<CollectionControlResponse>, ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(control) =
        payload.map_err(|_| ApiError::BadRequest("Invalid collection control JSON"))?;
    runtime.set_collection_enabled(control.enabled);
    Ok(Json(CollectionControlResponse {
        enabled: runtime.collection_enabled(),
    }))
}

async fn enqueue_command(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
    payload: Result<Json<ControlCommandRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ControlCommand>), ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(request) =
        payload.map_err(|_| ApiError::BadRequest("Invalid control command JSON"))?;
    match runtime.enqueue_command(request).await {
        Ok(command) => Ok((StatusCode::ACCEPTED, Json(command))),
        Err(RuntimeError::AgentNotHookBacked) => Err(ApiError::AgentNotHookBacked),
        Err(_) => Err(ApiError::BadRequest("Control command failed validation")),
    }
}

async fn pending_commands(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
    payload: Result<Json<CommandPollRequest>, JsonRejection>,
) -> Result<Json<Vec<ControlCommand>>, ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(request) = payload.map_err(|_| ApiError::BadRequest("Invalid command poll JSON"))?;
    runtime
        .pending_commands(&request.agent_id)
        .await
        .map(Json)
        .map_err(|_| ApiError::BadRequest("Command poll failed validation"))
}

async fn acknowledge_command(
    State(state): State<AppState>,
    Extension(runtime): Extension<RuntimeMonitor>,
    headers: HeaderMap,
    payload: Result<Json<ControlCommandAck>, JsonRejection>,
) -> Result<Json<ControlCommand>, ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(ack) =
        payload.map_err(|_| ApiError::BadRequest("Invalid command acknowledgement JSON"))?;
    match runtime.acknowledge_command(ack).await {
        Ok(command) => Ok(Json(command)),
        Err(RuntimeError::CommandNotFound) => Err(ApiError::NotFound),
        Err(RuntimeError::CommandAgentMismatch) => Err(ApiError::CommandAgentMismatch),
        Err(_) => Err(ApiError::BadRequest(
            "Command acknowledgement failed validation",
        )),
    }
}

fn authorize_read(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    state
        .auth
        .authorize_read(bearer_token(headers))
        .map_err(|_| ApiError::Unauthorized)
}

fn authorize_ingest(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    state
        .auth
        .authorize_ingest(bearer_token(headers))
        .map_err(|_| ApiError::Unauthorized)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, header},
    };
    use chrono::Utc;
    use tower::ServiceExt;

    use crate::{
        auth::AuthPolicy,
        config::Config,
        daemon::BoundAddresses,
        runtime::{RUNTIME_PROTOCOL_VERSION, RuntimeAgentRef, RuntimeConfig, RuntimeHookKind},
        store::Store,
    };

    use super::*;

    fn test_state() -> AppState {
        let config = Arc::new(Config::local_test());
        let addresses = BoundAddresses {
            http: config.http_addr,
            tcp: config.tcp_addr,
            udp: config.udp_addr,
        };
        AppState {
            store: Store::new(config.cache_config(), config.update_channel_capacity),
            auth: AuthPolicy::from_config(&config),
            config,
            addresses,
        }
    }

    fn test_runtime() -> RuntimeMonitor {
        RuntimeMonitor::new(RuntimeConfig {
            process_collection_enabled: false,
            proc_root: PathBuf::from("/not-used-in-api-tests"),
            sample_interval: Duration::from_secs(1),
            process_patterns: vec!["claude".to_owned()],
            hook_capacity: 16,
            command_capacity: 16,
        })
    }

    fn authorized_request(method: Method, uri: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    }

    fn hook() -> RuntimeHookEnvelope {
        RuntimeHookEnvelope {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            agent: RuntimeAgentRef {
                agent_id: "claude-test".to_owned(),
                provider: "anthropic".to_owned(),
                model: "claude-test-model".to_owned(),
                instance_id: None,
            },
            session_id: Some("session-1".to_owned()),
            pid: None,
            kind: RuntimeHookKind::ModelResponse,
            summary: Some("Visible response summary".to_owned()),
            tool_name: None,
            confidence: Some(0.8),
            cpu_percent: Some(18.5),
            rss_bytes: Some(64 * 1_024 * 1_024),
            memory_percent: Some(2.0),
            input_tokens_delta: 20,
            output_tokens_delta: 8,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn runtime_page_is_static_and_contains_no_credentials() {
        let response = router(test_state(), test_runtime())
            .oneshot(
                Request::builder()
                    .uri("/runtime")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Live agent runtime"));
        assert!(body.contains("panel-toggle"));
        assert!(!body.contains("test-token-at-least-16-bytes"));
    }

    #[tokio::test]
    async fn runtime_snapshot_is_read_protected() {
        let response = router(test_state(), test_runtime())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn hook_data_is_real_and_visible_in_runtime_snapshot() {
        let runtime = test_runtime();
        let app = router(test_state(), runtime);
        let event = hook();
        let response = app
            .clone()
            .oneshot(authorized_request(
                Method::POST,
                "/api/v1/runtime/hooks",
                Body::from(serde_json::to_vec(&event).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let response = app
            .oneshot(authorized_request(
                Method::GET,
                "/api/v1/runtime/snapshot",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: RuntimeSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot.agents.len(), 1);
        assert_eq!(snapshot.agents[0].reported_confidence, Some(0.8));
        assert_eq!(snapshot.agents[0].cpu_percent, Some(18.5));
        assert_eq!(snapshot.agents[0].resource_source, "hook");
        assert_eq!(snapshot.agents[0].input_tokens, 20);
    }

    #[tokio::test]
    async fn process_only_agents_cannot_receive_cooperative_commands() {
        let request = ControlCommandRequest {
            agent_id: "process:anthropic:4242".to_owned(),
            action: crate::runtime::ControlAction::Pause,
        };
        let response = router(test_state(), test_runtime())
            .oneshot(authorized_request(
                Method::POST,
                "/api/v1/runtime/commands",
                Body::from(serde_json::to_vec(&request).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn dashboard_controls_queue_commands_for_hook_aware_agents() {
        let runtime = test_runtime();
        let app = router(test_state(), runtime);
        let event = hook();
        let response = app
            .clone()
            .oneshot(authorized_request(
                Method::POST,
                "/api/v1/runtime/hooks",
                Body::from(serde_json::to_vec(&event).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let request = ControlCommandRequest {
            agent_id: "claude-test".to_owned(),
            action: crate::runtime::ControlAction::Pause,
        };
        let response = app
            .clone()
            .oneshot(authorized_request(
                Method::POST,
                "/api/v1/runtime/commands",
                Body::from(serde_json::to_vec(&request).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let command: ControlCommand = serde_json::from_slice(&body).unwrap();

        let response = app
            .oneshot(authorized_request(
                Method::POST,
                "/api/v1/runtime/commands/ack",
                Body::from(
                    serde_json::to_vec(&ControlCommandAck {
                        command_id: command.command_id,
                        agent_id: "claude-test".to_owned(),
                        accepted: true,
                        message: Some("paused".to_owned()),
                    })
                    .unwrap(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
