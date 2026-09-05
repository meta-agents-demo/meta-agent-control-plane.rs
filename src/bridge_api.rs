use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State, WebSocketUpgrade, rejection::JsonRejection, ws},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    auth::{AuthPolicy, bearer_token},
    bridge::{
        BridgeError, BridgeMessageInput, BridgeParticipantInput, BridgeRoomInput, BridgeSnapshot,
    },
    bridge_ui,
    http::{AppState, websocket_origin_allowed},
    model::Transport,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Forbidden,
    BadRequest(String),
    NotFound,
    Conflict(String),
}

impl From<BridgeError> for ApiError {
    fn from(error: BridgeError) -> Self {
        match error {
            BridgeError::RoomNotFound => Self::NotFound,
            BridgeError::RoomCapacity | BridgeError::MemberCapacity => {
                Self::Conflict(error.to_string())
            }
            _ => Self::BadRequest(error.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed".to_owned(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "Cross-origin WebSocket upgrade rejected".to_owned(),
            ),
            Self::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "invalid_bridge_request", message)
            }
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "bridge_room_not_found",
                "Bridge room was not found".to_owned(),
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, "bridge_capacity", message),
        };
        (status, Json(json!({ "error": error, "message": message }))).into_response()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinRequest {
    participant: BridgeParticipantInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Auth { token: String },
    Join { participant: BridgeParticipantInput },
    Message { message: BridgeMessageInput },
    Snapshot,
    Ping,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Authenticated,
    Joined {
        participant: crate::bridge::BridgeParticipant,
    },
    MessageAccepted {
        ack: crate::bridge::BridgeAck,
    },
    Snapshot {
        snapshot: BridgeSnapshot,
    },
    Update {
        update: crate::bridge::BridgeUpdate,
    },
    Pong,
    Error {
        error: String,
        message: String,
    },
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/bridge", get(page))
        .route("/api/v1/bridge/rooms", get(list_rooms).post(create_room))
        .route("/api/v1/bridge/rooms/{room_slug}", get(snapshot))
        .route("/api/v1/bridge/rooms/{room_slug}/join", post(join_room))
        .route(
            "/api/v1/bridge/rooms/{room_slug}/messages",
            get(messages).post(post_message),
        )
        .route("/ws/bridge/{room_slug}", get(bridge_websocket))
        .with_state(state)
}

async fn page(State(state): State<AppState>) -> Html<String> {
    Html(bridge_ui::dashboard(
        state.auth.reads_are_protected(),
        state.addresses.tcp,
    ))
}

async fn list_rooms(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::bridge::BridgeRoomSummary>>, ApiError> {
    authorize_read(&state, &headers)?;
    Ok(Json(state.store.bridge().list_rooms().await))
}

async fn create_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<BridgeRoomInput>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::bridge::BridgeRoom>), ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(input) =
        payload.map_err(|_| ApiError::BadRequest("Invalid bridge room JSON".to_owned()))?;
    let room = state.store.bridge().create_room(input).await?;
    Ok((StatusCode::CREATED, Json(room)))
}

async fn snapshot(
    Path(room_slug): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BridgeSnapshot>, ApiError> {
    authorize_read(&state, &headers)?;
    Ok(Json(state.store.bridge().snapshot(&room_slug).await?))
}

async fn messages(
    Path(room_slug): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::bridge::BridgeMessage>>, ApiError> {
    authorize_read(&state, &headers)?;
    Ok(Json(
        state.store.bridge().snapshot(&room_slug).await?.messages,
    ))
}

async fn join_room(
    Path(room_slug): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<JoinRequest>, JsonRejection>,
) -> Result<Json<crate::bridge::BridgeParticipant>, ApiError> {
    authorize_ingest(&state, &headers)?;
    let Json(request) =
        payload.map_err(|_| ApiError::BadRequest("Invalid bridge join JSON".to_owned()))?;
    Ok(Json(
        state
            .store
            .bridge()
            .join(&room_slug, request.participant, false)
            .await?,
    ))
}

async fn post_message(
    Path(room_slug): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<BridgeMessageInput>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::bridge::BridgeAck>), ApiError> {
    if let Err(error) = authorize_ingest(&state, &headers) {
        state.store.bridge().record_rejection(Transport::Http).await;
        return Err(error);
    }
    let Json(message) =
        payload.map_err(|_| ApiError::BadRequest("Invalid bridge message JSON".to_owned()))?;
    match state
        .store
        .bridge()
        .post_message(&room_slug, message, Transport::Http)
        .await
    {
        Ok(ack) => {
            let status = if ack.duplicate {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            };
            Ok((status, Json(ack)))
        }
        Err(error) => {
            state.store.bridge().record_rejection(Transport::Http).await;
            Err(error.into())
        }
    }
}

async fn bridge_websocket(
    Path(room_slug): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !websocket_origin_allowed(&headers, state.config.cors_any) {
        return Err(ApiError::Forbidden);
    }
    let header_token = bearer_token(&headers).map(str::to_owned);
    Ok(websocket
        .on_upgrade(move |socket| handle_bridge_socket(socket, state, room_slug, header_token))
        .into_response())
}

async fn handle_bridge_socket(
    mut socket: ws::WebSocket,
    state: AppState,
    room_slug: String,
    header_token: Option<String>,
) {
    let authenticated = header_token
        .as_deref()
        .is_some_and(|token| state.auth.authorize_ingest(Some(token)).is_ok())
        || (!state.auth.ingestion_is_protected());
    if !authenticated && !authenticate_socket(&mut socket, &state.auth).await {
        state
            .store
            .bridge()
            .record_rejection(Transport::WebSocket)
            .await;
        return;
    }
    if send_frame(&mut socket, &ServerFrame::Authenticated)
        .await
        .is_err()
    {
        return;
    }

    let hub = state.store.bridge();
    let mut updates = hub.subscribe();
    let mut joined_participants = Vec::<String>::new();

    loop {
        tokio::select! {
            incoming = socket.next() => {
                let Some(incoming) = incoming else { break };
                let Ok(incoming) = incoming else { break };
                match incoming {
                    ws::Message::Text(text) => {
                        let frame = serde_json::from_str::<ClientFrame>(text.as_str());
                        let response = match frame {
                            Ok(ClientFrame::Auth { .. }) => ServerFrame::Error {
                                error: "already_authenticated".to_owned(),
                                message: "The bridge socket is already authenticated".to_owned(),
                            },
                            Ok(ClientFrame::Join { participant }) => {
                                match hub.join(&room_slug, participant, true).await {
                                    Ok(participant) => {
                                        if !joined_participants.contains(&participant.participant_id) {
                                            joined_participants.push(participant.participant_id.clone());
                                        }
                                        ServerFrame::Joined { participant }
                                    }
                                    Err(error) => error_frame(error),
                                }
                            }
                            Ok(ClientFrame::Message { message }) => {
                                if !joined_participants.contains(&message.author.participant_id) {
                                    hub.record_rejection(Transport::WebSocket).await;
                                    ServerFrame::Error {
                                        error: "participant_not_joined".to_owned(),
                                        message: "Join this participant before posting a bridge message".to_owned(),
                                    }
                                } else { match hub.post_message(&room_slug, message, Transport::WebSocket).await {
                                    Ok(ack) => ServerFrame::MessageAccepted { ack },
                                    Err(error) => {
                                        hub.record_rejection(Transport::WebSocket).await;
                                        error_frame(error)
                                    }
                                }}
                            }
                            Ok(ClientFrame::Snapshot) => match hub.snapshot(&room_slug).await {
                                Ok(snapshot) => ServerFrame::Snapshot { snapshot },
                                Err(error) => error_frame(error),
                            },
                            Ok(ClientFrame::Ping) => ServerFrame::Pong,
                            Err(error) => ServerFrame::Error {
                                error: "invalid_bridge_frame".to_owned(),
                                message: error.to_string(),
                            },
                        };
                        if send_frame(&mut socket, &response).await.is_err() {
                            break;
                        }
                    }
                    ws::Message::Close(_) => break,
                    ws::Message::Ping(value) => {
                        if socket.send(ws::Message::Pong(value)).await.is_err() {
                            break;
                        }
                    }
                    ws::Message::Binary(_) | ws::Message::Pong(_) => {}
                }
            }
            update = updates.recv() => {
                match update {
                    Ok(update) if update.room_slug == room_slug => {
                        if send_frame(&mut socket, &ServerFrame::Update { update }).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        match hub.snapshot(&room_slug).await {
                            Ok(snapshot) => {
                                if send_frame(&mut socket, &ServerFrame::Snapshot { snapshot }).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    for participant_id in joined_participants {
        hub.set_websocket_connected(&room_slug, &participant_id, false)
            .await;
    }
}

async fn authenticate_socket(socket: &mut ws::WebSocket, auth: &AuthPolicy) -> bool {
    let Ok(Some(Ok(ws::Message::Text(text)))) =
        tokio::time::timeout(Duration::from_secs(10), socket.next()).await
    else {
        return false;
    };
    let Ok(ClientFrame::Auth { token }) = serde_json::from_str(text.as_str()) else {
        let _ = send_value(
            socket,
            json!({
                "type": "error",
                "error": "authentication_required",
                "message": "Send an auth frame before joining a bridge room"
            }),
        )
        .await;
        return false;
    };
    if auth.authorize_ingest(Some(&token)).is_err() {
        let _ = send_value(
            socket,
            json!({
                "type": "error",
                "error": "unauthorized",
                "message": "Authentication failed"
            }),
        )
        .await;
        return false;
    }
    true
}

fn error_frame(error: BridgeError) -> ServerFrame {
    ServerFrame::Error {
        error: "invalid_bridge_request".to_owned(),
        message: error.to_string(),
    }
}

async fn send_frame(socket: &mut ws::WebSocket, frame: &ServerFrame) -> Result<(), ()> {
    let value = serde_json::to_value(frame).map_err(|_| ())?;
    send_value(socket, value).await
}

async fn send_value(socket: &mut ws::WebSocket, value: Value) -> Result<(), ()> {
    socket
        .send(ws::Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
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
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, header},
    };
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        auth::AuthPolicy, bridge::BRIDGE_PROTOCOL_VERSION, config::Config, daemon::BoundAddresses,
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

    fn request(method: Method, path: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.into())
            .unwrap()
    }

    fn room() -> BridgeRoomInput {
        BridgeRoomInput {
            slug: "agent-lab".to_owned(),
            title: "Agent lab".to_owned(),
            objective: "Cross-check a bounded decision".to_owned(),
        }
    }

    #[tokio::test]
    async fn human_can_create_join_and_post_to_a_protected_room() {
        let app = router(test_state());
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/bridge/rooms",
                serde_json::to_vec(&room()).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let participant = BridgeParticipantInput {
            participant_id: "human-operator".to_owned(),
            display_name: "Human operator".to_owned(),
            kind: crate::bridge::BridgeParticipantKind::Human,
            provider: None,
            model: None,
            runtime_agent_id: None,
        };
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/bridge/rooms/agent-lab/join",
                serde_json::to_vec(&JoinRequest {
                    participant: participant.clone(),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/bridge/rooms/agent-lab/messages",
                serde_json::to_vec(&BridgeMessageInput {
                    protocol_version: BRIDGE_PROTOCOL_VERSION.to_owned(),
                    message_id: Uuid::new_v4(),
                    occurred_at: Utc::now(),
                    author: participant,
                    summary: "Please compare the observed evidence.".to_owned(),
                    reply_to: None,
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let response = app
            .oneshot(request(
                Method::GET,
                "/api/v1/bridge/rooms/agent-lab",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: BridgeSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.messages.len(), 1);
    }

    #[tokio::test]
    async fn bridge_read_and_write_apis_fail_closed_without_token() {
        let app = router(test_state());
        for (method, path) in [
            (Method::GET, "/api/v1/bridge/rooms"),
            (Method::POST, "/api/v1/bridge/rooms"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn unauthorized_http_message_is_visible_in_bridge_rejection_analytics() {
        let state = test_state();
        let app = router(state.clone());
        let participant = BridgeParticipantInput {
            participant_id: "human-operator".to_owned(),
            display_name: "Human operator".to_owned(),
            kind: crate::bridge::BridgeParticipantKind::Human,
            provider: None,
            model: None,
            runtime_agent_id: None,
        };

        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/bridge/rooms",
                serde_json::to_vec(&room()).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/bridge/rooms/agent-lab/join",
                serde_json::to_vec(&JoinRequest {
                    participant: participant.clone(),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/bridge/rooms/agent-lab/messages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&BridgeMessageInput {
                            protocol_version: BRIDGE_PROTOCOL_VERSION.to_owned(),
                            message_id: Uuid::new_v4(),
                            occurred_at: Utc::now(),
                            author: participant,
                            summary: "This must not be accepted without authentication.".to_owned(),
                            reply_to: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let snapshot = state.store.bridge().snapshot("agent-lab").await.unwrap();
        assert_eq!(snapshot.messages.len(), 0);
        assert_eq!(snapshot.counters.rejected_by_transport["http"], 1);
    }
}
