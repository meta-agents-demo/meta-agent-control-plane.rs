use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;

use crate::{
    auth::bearer_token,
    explorer::{
        ExplorerPolicy, ExplorerSnapshot, MAX_LESSON_LIMIT, MAX_SESSION_LIMIT,
        MAX_TIMELINE_LIMIT, build_explorer,
    },
    http::AppState,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(String),
    ProjectionFailed(String),
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
                    "error": "invalid_explorer_policy",
                    "message": message
                })),
            )
                .into_response(),
            Self::ProjectionFailed(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "explorer_projection_failed",
                    "message": message
                })),
            )
                .into_response(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ExplorerQuery {
    timeline_limit: Option<usize>,
    session_limit: Option<usize>,
    lesson_limit: Option<usize>,
}

impl ExplorerQuery {
    fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let Some(raw_query) = raw_query.filter(|query| !query.is_empty()) else {
            return Ok(Self::default());
        };

        let mut query = Self::default();
        let mut seen = BTreeSet::new();
        for parameter in raw_query.split('&') {
            let (name, raw_value) = parameter.split_once('=').ok_or_else(|| {
                ApiError::BadRequest("explorer parameters must use name=value syntax".to_owned())
            })?;
            if !seen.insert(name) {
                return Err(ApiError::BadRequest(format!(
                    "explorer parameter {name} may be provided only once"
                )));
            }
            let value = raw_value.parse::<usize>().map_err(|_| {
                ApiError::BadRequest(format!(
                    "explorer parameter {name} must be a positive base-10 integer"
                ))
            })?;
            match name {
                "timeline_limit" => {
                    query.timeline_limit = Some(bounded(name, value, MAX_TIMELINE_LIMIT)?);
                }
                "session_limit" => {
                    query.session_limit = Some(bounded(name, value, MAX_SESSION_LIMIT)?);
                }
                "lesson_limit" => {
                    query.lesson_limit = Some(bounded(name, value, MAX_LESSON_LIMIT)?);
                }
                _ => {
                    return Err(ApiError::BadRequest(format!(
                        "unknown explorer parameter {name}"
                    )));
                }
            }
        }
        Ok(query)
    }

    fn policy(self) -> ExplorerPolicy {
        let defaults = ExplorerPolicy::default();
        ExplorerPolicy {
            timeline_limit: self.timeline_limit.unwrap_or(defaults.timeline_limit),
            session_limit: self.session_limit.unwrap_or(defaults.session_limit),
            lesson_limit: self.lesson_limit.unwrap_or(defaults.lesson_limit),
        }
    }
}

fn bounded(name: &str, value: usize, maximum: usize) -> Result<usize, ApiError> {
    if value == 0 || value > maximum {
        return Err(ApiError::BadRequest(format!(
            "explorer parameter {name} must be between 1 and {maximum}"
        )));
    }
    Ok(value)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/explorer", get(explorer))
        .with_state(state)
}

async fn explorer(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<ExplorerSnapshot>, ApiError> {
    state
        .auth
        .authorize_read(bearer_token(&headers))
        .map_err(|_| ApiError::Unauthorized)?;
    let policy = ExplorerQuery::parse(raw_query.as_deref())?.policy();
    let snapshot = state.store.snapshot().await;
    build_explorer(&snapshot, policy)
        .map(Json)
        .map_err(|error| ApiError::ProjectionFailed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use tower::ServiceExt;

    use crate::{auth::AuthPolicy, config::Config, daemon::BoundAddresses, store::Store};

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

    fn authorized_request(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn authentication_precedes_query_parsing() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/explorer?timeline_limit=not-a-number")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn default_projection_is_bounded_and_uses_current_revision() {
        let response = router(test_state())
            .oneshot(authorized_request("/api/v1/explorer"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let explorer: ExplorerSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(explorer.revision, 0);
        assert_eq!(explorer.policy, ExplorerPolicy::default());
        assert_eq!(explorer.system.agents, 0);
        assert_eq!(explorer.retention.total_timeline_events, 0);
    }

    #[tokio::test]
    async fn custom_limits_are_reflected_without_mutating_defaults() {
        let response = router(test_state())
            .oneshot(authorized_request(
                "/api/v1/explorer?timeline_limit=7&session_limit=8&lesson_limit=9",
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let explorer: ExplorerSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            explorer.policy,
            ExplorerPolicy {
                timeline_limit: 7,
                session_limit: 8,
                lesson_limit: 9,
            }
        );

        let response = router(test_state())
            .oneshot(authorized_request("/api/v1/explorer"))
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let explorer: ExplorerSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(explorer.policy, ExplorerPolicy::default());
    }

    #[tokio::test]
    async fn rejects_zero_excess_duplicate_unknown_and_malformed_values() {
        for uri in [
            "/api/v1/explorer?timeline_limit=0",
            "/api/v1/explorer?session_limit=251",
            "/api/v1/explorer?lesson_limit=1001",
            "/api/v1/explorer?timeline_limit=2&timeline_limit=3",
            "/api/v1/explorer?agent_limit=3",
            "/api/v1/explorer?lesson_limit=abc",
            "/api/v1/explorer?timeline_limit",
        ] {
            let response = router(test_state())
                .oneshot(authorized_request(uri))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "uri {uri}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"], "invalid_explorer_policy");
        }
    }
}
