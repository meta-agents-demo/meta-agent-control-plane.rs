use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;

use crate::{
    auth::bearer_token,
    coordination::{CoordinationPlan, build_plan},
    http::AppState,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    PlanningFailed(String),
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
            Self::PlanningFailed(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "planning_failed",
                    "message": message
                })),
            )
                .into_response(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/coordination", get(plan))
        .with_state(state)
}

async fn plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CoordinationPlan>, ApiError> {
    state
        .auth
        .authorize_read(bearer_token(&headers))
        .map_err(|_| ApiError::Unauthorized)?;
    let snapshot = state.store.snapshot().await;
    build_plan(&snapshot)
        .map(Json)
        .map_err(|error| ApiError::PlanningFailed(error.to_string()))
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

    #[tokio::test]
    async fn plan_requires_the_read_token() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/coordination")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authorized_plan_uses_the_current_store_revision_and_bounded_defaults() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/coordination")
                    .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let plan: CoordinationPlan = serde_json::from_slice(&body).unwrap();
        assert_eq!(plan.revision, 0);
        assert_eq!(plan.summary.total_tasks, 0);
        assert_eq!(plan.summary.assignments, 0);
        assert_eq!(plan.planning_policy.max_assignments, 16);
        assert_eq!(plan.planning_policy.max_assignments_per_agent, 2);
    }
}
