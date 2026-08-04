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
    http::AppState,
    timeline::{
        MAX_TIMELINE_PAGE_LIMIT, TimelineCursor, TimelineError, TimelinePage, TimelinePolicy,
        build_timeline_page,
    },
};

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(String),
    RevisionChanged { requested: u64, current: u64 },
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
                    "error": "invalid_timeline_query",
                    "message": message
                })),
            )
                .into_response(),
            Self::RevisionChanged { requested, current } => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "timeline_revision_changed",
                    "message": "The retained timeline changed; restart pagination from the first page",
                    "requested_revision": requested,
                    "current_revision": current
                })),
            )
                .into_response(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TimelineQuery {
    limit: Option<usize>,
    cursor: Option<TimelineCursor>,
}

impl TimelineQuery {
    fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let Some(raw_query) = raw_query.filter(|query| !query.is_empty()) else {
            return Ok(Self::default());
        };

        let mut query = Self::default();
        let mut seen = BTreeSet::new();
        for parameter in raw_query.split('&') {
            let (name, raw_value) = parameter.split_once('=').ok_or_else(|| {
                ApiError::BadRequest("timeline parameters must use name=value syntax".to_owned())
            })?;
            if !seen.insert(name) {
                return Err(ApiError::BadRequest(format!(
                    "timeline parameter {name} may be provided only once"
                )));
            }
            match name {
                "limit" => {
                    let value = raw_value.parse::<usize>().map_err(|_| {
                        ApiError::BadRequest(
                            "timeline parameter limit must be a positive base-10 integer"
                                .to_owned(),
                        )
                    })?;
                    if value == 0 || value > MAX_TIMELINE_PAGE_LIMIT {
                        return Err(ApiError::BadRequest(format!(
                            "timeline parameter limit must be between 1 and {MAX_TIMELINE_PAGE_LIMIT}"
                        )));
                    }
                    query.limit = Some(value);
                }
                "cursor" => {
                    query.cursor = Some(TimelineCursor::parse(raw_value).map_err(|_| {
                        ApiError::BadRequest("timeline cursor is invalid".to_owned())
                    })?);
                }
                _ => {
                    return Err(ApiError::BadRequest(format!(
                        "unknown timeline parameter {name}"
                    )));
                }
            }
        }
        Ok(query)
    }

    fn policy(&self) -> TimelinePolicy {
        TimelinePolicy {
            limit: self.limit.unwrap_or(TimelinePolicy::default().limit),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/timeline", get(timeline))
        .with_state(state)
}

async fn timeline(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<TimelinePage>, ApiError> {
    state
        .auth
        .authorize_read(bearer_token(&headers))
        .map_err(|_| ApiError::Unauthorized)?;
    let query = TimelineQuery::parse(raw_query.as_deref())?;
    let snapshot = state.store.snapshot().await;
    build_timeline_page(&snapshot, query.policy(), query.cursor)
        .map(Json)
        .map_err(|error| match error {
            TimelineError::InvalidLimit | TimelineError::InvalidCursor => {
                ApiError::BadRequest(error.to_string())
            }
            TimelineError::RevisionChanged { requested, current } => {
                ApiError::RevisionChanged { requested, current }
            }
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use chrono::{Duration, TimeZone, Utc};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        auth::AuthPolicy,
        config::Config,
        daemon::BoundAddresses,
        model::{EventEnvelope, Transport},
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
                    .uri("/api/v1/timeline?limit=not-a-number")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn first_page_uses_defaults_and_current_snapshot_revision() {
        let response = router(test_state())
            .oneshot(authorized_request("/api/v1/timeline"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let page: TimelinePage = serde_json::from_slice(&body).unwrap();
        assert_eq!(page.revision, 0);
        assert_eq!(page.policy, TimelinePolicy::default());
        assert_eq!(page.returned, 0);
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn custom_limit_is_reflected_without_mutating_defaults() {
        let response = router(test_state())
            .oneshot(authorized_request("/api/v1/timeline?limit=7"))
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let page: TimelinePage = serde_json::from_slice(&body).unwrap();
        assert_eq!(page.policy, TimelinePolicy { limit: 7 });

        let response = router(test_state())
            .oneshot(authorized_request("/api/v1/timeline"))
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let page: TimelinePage = serde_json::from_slice(&body).unwrap();
        assert_eq!(page.policy, TimelinePolicy::default());
    }

    #[tokio::test]
    async fn malformed_duplicate_unknown_and_excess_values_fail_closed() {
        for uri in [
            "/api/v1/timeline?limit=0",
            "/api/v1/timeline?limit=101",
            "/api/v1/timeline?limit=2&limit=3",
            "/api/v1/timeline?offset=3",
            "/api/v1/timeline?limit=abc",
            "/api/v1/timeline?cursor=v1.invalid",
            "/api/v1/timeline?limit",
        ] {
            let response = router(test_state())
                .oneshot(authorized_request(uri))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "uri {uri}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"], "invalid_timeline_query");
        }
    }

    #[tokio::test]
    async fn stale_cursor_returns_a_bounded_revision_conflict() {
        let state = test_state();
        let event: EventEnvelope =
            serde_json::from_str(include_str!("../fixtures/progress-updated.json")).unwrap();
        state.store.ingest(event, Transport::Http).await.unwrap();
        let cursor = TimelineCursor {
            revision: 0,
            occurred_at: Utc.with_ymd_and_hms(2026, 7, 30, 20, 0, 0).unwrap()
                + Duration::nanoseconds(1),
            event_id: Uuid::nil(),
        }
        .encode();

        let response = router(state)
            .oneshot(authorized_request(&format!(
                "/api/v1/timeline?cursor={cursor}"
            )))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"], "timeline_revision_changed");
        assert_eq!(body["requested_revision"], 0);
        assert_eq!(body["current_revision"], 1);
    }
}
