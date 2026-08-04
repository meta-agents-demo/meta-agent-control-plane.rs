use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde_json::json;

use crate::{
    auth::bearer_token,
    coordination::{CoordinationPlan, PlanningPolicy, build_plan_with_policy},
    coordination_ui,
    http::AppState,
    metacognition::AnalysisPolicy,
};

const MAX_ASSIGNMENTS_LIMIT: usize = 256;
const MAX_ASSIGNMENTS_PER_AGENT_LIMIT: usize = 32;
const MAX_INTERVENTIONS_LIMIT: usize = 512;
const MAX_HOLDS_LIMIT: usize = 1_024;

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(String),
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
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_planning_policy",
                    "message": message
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PlanningQuery {
    max_assignments: Option<usize>,
    max_assignments_per_agent: Option<usize>,
    max_interventions: Option<usize>,
    max_holds: Option<usize>,
}

impl PlanningQuery {
    fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let Some(raw_query) = raw_query.filter(|query| !query.is_empty()) else {
            return Ok(Self::default());
        };

        let mut query = Self::default();
        let mut seen = BTreeSet::new();
        for parameter in raw_query.split('&') {
            let (name, raw_value) = parameter.split_once('=').ok_or_else(|| {
                ApiError::BadRequest("planning parameters must use name=value syntax".to_owned())
            })?;
            if !seen.insert(name) {
                return Err(ApiError::BadRequest(format!(
                    "planning parameter {name} may be provided only once"
                )));
            }
            let value = raw_value.parse::<usize>().map_err(|_| {
                ApiError::BadRequest(format!(
                    "planning parameter {name} must be a positive base-10 integer"
                ))
            })?;
            match name {
                "max_assignments" => {
                    query.max_assignments = Some(bounded(
                        name,
                        value,
                        MAX_ASSIGNMENTS_LIMIT,
                    )?);
                }
                "max_assignments_per_agent" => {
                    query.max_assignments_per_agent = Some(bounded(
                        name,
                        value,
                        MAX_ASSIGNMENTS_PER_AGENT_LIMIT,
                    )?);
                }
                "max_interventions" => {
                    query.max_interventions = Some(bounded(
                        name,
                        value,
                        MAX_INTERVENTIONS_LIMIT,
                    )?);
                }
                "max_holds" => {
                    query.max_holds = Some(bounded(name, value, MAX_HOLDS_LIMIT)?);
                }
                _ => {
                    return Err(ApiError::BadRequest(format!(
                        "unknown planning parameter {name}"
                    )));
                }
            }
        }
        Ok(query)
    }

    fn policy(self) -> PlanningPolicy {
        let defaults = PlanningPolicy::default();
        PlanningPolicy {
            max_assignments: self.max_assignments.unwrap_or(defaults.max_assignments),
            max_assignments_per_agent: self
                .max_assignments_per_agent
                .unwrap_or(defaults.max_assignments_per_agent),
            max_interventions: self
                .max_interventions
                .unwrap_or(defaults.max_interventions),
            max_holds: self.max_holds.unwrap_or(defaults.max_holds),
        }
    }
}

fn bounded(name: &str, value: usize, maximum: usize) -> Result<usize, ApiError> {
    if value == 0 || value > maximum {
        return Err(ApiError::BadRequest(format!(
            "planning parameter {name} must be between 1 and {maximum}"
        )));
    }
    Ok(value)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/coordination", get(page))
        .route("/api/v1/coordination", get(plan))
        .with_state(state)
}

async fn page(State(state): State<AppState>) -> Html<String> {
    Html(coordination_ui::dashboard(state.auth.reads_are_protected()))
}

async fn plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<CoordinationPlan>, ApiError> {
    state
        .auth
        .authorize_read(bearer_token(&headers))
        .map_err(|_| ApiError::Unauthorized)?;
    let planning_policy = PlanningQuery::parse(raw_query.as_deref())?.policy();
    let snapshot = state.store.snapshot().await;
    build_plan_with_policy(&snapshot, AnalysisPolicy::default(), planning_policy)
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

    fn authorized_request(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer test-token-at-least-16-bytes")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn page_is_served_without_exposing_protected_plan_data() {
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
        assert!(body.contains("Coordination plan"));
        assert!(body.contains("Read token"));
        assert!(body.contains("advisory and read-only"));
    }

    #[tokio::test]
    async fn plan_requires_the_read_token_before_parsing_policy() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/coordination?max_assignments=not-a-number")
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
            .oneshot(authorized_request("/api/v1/coordination"))
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

    #[tokio::test]
    async fn authorized_plan_applies_custom_bounded_policy() {
        let response = router(test_state())
            .oneshot(authorized_request(
                "/api/v1/coordination?max_assignments=7&max_assignments_per_agent=3&max_interventions=9&max_holds=11",
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let plan: CoordinationPlan = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            plan.planning_policy,
            PlanningPolicy {
                max_assignments: 7,
                max_assignments_per_agent: 3,
                max_interventions: 9,
                max_holds: 11,
            }
        );
    }

    #[tokio::test]
    async fn policy_rejects_zero_excess_duplicate_and_unknown_values() {
        for uri in [
            "/api/v1/coordination?max_assignments=0",
            "/api/v1/coordination?max_assignments=257",
            "/api/v1/coordination?max_assignments=2&max_assignments=3",
            "/api/v1/coordination?max_agents=3",
            "/api/v1/coordination?max_holds=abc",
        ] {
            let response = router(test_state())
                .oneshot(authorized_request(uri))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "uri {uri}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"], "invalid_planning_policy");
        }
    }
}
