use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    bridge_api, coordination_api, explorer_api,
    http::{self, AppState},
    metacognition_api,
    runtime::RuntimeMonitor,
    runtime_api, timeline_api,
};

pub fn router(state: AppState) -> Router {
    router_with_runtime(state, RuntimeMonitor::from_env())
}

fn router_with_runtime(state: AppState, runtime: RuntimeMonitor) -> Router {
    http::router(state.clone())
        .merge(bridge_api::router(state.clone()))
        .merge(metacognition_api::router(state.clone()))
        .merge(coordination_api::router(state.clone()))
        .merge(explorer_api::router(state.clone()))
        .merge(timeline_api::router(state.clone()))
        .merge(runtime_api::router(state, runtime))
}

pub async fn serve(
    listener: TcpListener,
    state: AppState,
    cancellation: CancellationToken,
) -> std::io::Result<()> {
    let runtime = RuntimeMonitor::from_env();
    let app = router_with_runtime(state, runtime.clone());
    let server_cancellation = cancellation.child_token();
    let collector_cancellation = cancellation.child_token();
    let server =
        axum::serve(listener, app).with_graceful_shutdown(server_cancellation.cancelled_owned());
    let collector = async move {
        runtime.run(collector_cancellation).await;
        Ok::<(), std::io::Error>(())
    };

    tokio::try_join!(server, collector)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
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
    async fn combined_router_serves_operator_pages_and_protects_read_apis() {
        let app = router(test_state());
        for path in [
            "/",
            "/metacognition",
            "/coordination",
            "/explorer",
            "/runtime",
            "/bridge",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "path {path}");
        }

        for path in [
            "/api/v1/coordination",
            "/api/v1/explorer",
            "/api/v1/timeline",
            "/api/v1/runtime/snapshot",
            "/api/v1/bridge/rooms",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "path {path}");
        }
    }
}
