use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde_json::json;

use crate::{
    auth::bearer_token,
    coordination::{CoordinationPlan, PlanningError, build_plan},
    coordination_ui,
    http::AppState,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Planning(PlanningError),
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
            Self::Planning(error) => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_planning_policy",
                    "message": error.to_string()
                })),
            )
                .into_response(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/coordination", get(page))
        .route("/api/v1/coordination", get(projection))
        .with_state(state)
}

async fn page(State(state): State<AppState>) -> Html<String> {
    Html(coordination_ui::dashboard(
        state.auth.reads_are_protected(),
    ))
}

async fn projection(
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
        .map_err(ApiError::Planning)
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
    async fn page_is_static_and_does_not_embed_protected_state() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/coordination")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Coordination planner"));
        assert!(body.contains("Read token"));
        assert!(body.contains("Recommendations, not leases"));
        assert!(!body.contains("test-token-at-least-16-bytes"));
    }

    #[tokio::test]
    async fn projection_requires_read_authorization() {
        let app = router(test_state());
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/coordination")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/coordination")
                    .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
        let body = to_bytes(authorized.into_body(), usize::MAX).await.unwrap();
        let plan: CoordinationPlan = serde_json::from_slice(&body).unwrap();
        assert_eq!(plan.revision, 0);
        assert_eq!(plan.summary.total_tasks, 0);
        assert!(plan.assignments.is_empty());
        assert!(plan.interventions.is_empty());
        assert!(plan.held_tasks.is_empty());
    }

    #[tokio::test]
    async fn repeated_reads_are_byte_stable_for_an_unchanged_store() {
        let app = router(test_state());
        let request = || {
            Request::builder()
                .uri("/api/v1/coordination")
                .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
                .body(Body::empty())
                .unwrap()
        };
        let first = app.clone().oneshot(request()).await.unwrap();
        let second = app.oneshot(request()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        let first = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        let second = to_bytes(second.into_body(), usize::MAX).await.unwrap();
        assert_eq!(first, second);
    }
}
