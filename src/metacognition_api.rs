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
    http::AppState,
    metacognition::{MetacognitionSnapshot, analyze},
    metacognition_ui,
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
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
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/metacognition", get(page))
        .route("/api/v1/metacognition", get(projection))
        .with_state(state)
}

async fn page(State(state): State<AppState>) -> Html<String> {
    Html(metacognition_ui::dashboard(
        state.auth.reads_are_protected(),
    ))
}

async fn projection(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MetacognitionSnapshot>, ApiError> {
    state
        .auth
        .authorize_read(bearer_token(&headers))
        .map_err(|_| ApiError::Unauthorized)?;
    let snapshot = state.store.snapshot().await;
    Ok(Json(analyze(&snapshot)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use tower::ServiceExt;

    use crate::{
        auth::AuthPolicy,
        config::Config,
        daemon::BoundAddresses,
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

    #[tokio::test]
    async fn page_is_served_without_exposing_protected_state() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/metacognition")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Explainable metacognition"));
        assert!(body.contains("Read token"));
    }

    #[tokio::test]
    async fn projection_requires_the_read_token() {
        let app = router(test_state());
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/metacognition")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/metacognition")
                    .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
        let body = to_bytes(authorized.into_body(), usize::MAX)
            .await
            .unwrap();
        let projection: MetacognitionSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(projection.revision, 0);
        assert_eq!(projection.summary.total_tasks, 0);
    }
}
