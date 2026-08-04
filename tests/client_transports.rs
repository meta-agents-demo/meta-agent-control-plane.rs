use std::time::Duration;

use meta_agent_control_plane::{
    Config, Daemon,
    client::{ClientConfig, ClientError, ClientTransport, EventClient},
    model::{AgentEvent, AgentRef, EventEnvelope, ProgressUpdate, TaskSpec, Transport},
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "test-token-at-least-16-bytes";

fn progress_event(agent_id: &str, task_id: &str, progress: f32) -> EventEnvelope {
    EventEnvelope::new(
        AgentRef {
            agent_id: agent_id.to_owned(),
            provider: "client-integration".to_owned(),
            model: "fixture".to_owned(),
            instance_id: Some(format!("{agent_id}-instance")),
        },
        AgentEvent::ProgressUpdated(ProgressUpdate {
            task_id: task_id.to_owned(),
            progress,
            summary: format!("{agent_id} reported observable progress."),
            blocker: None,
            next_action: Some("Verify the transport-specific acknowledgement.".to_owned()),
        }),
    )
}

fn client(transport: ClientTransport) -> EventClient {
    EventClient::new(
        ClientConfig::new(transport, Some(TOKEN.to_owned()))
            .with_timeout(Duration::from_secs(5))
            .with_max_response_bytes(64 * 1024),
    )
    .unwrap()
}

#[tokio::test]
async fn reliable_transports_and_udp_telemetry_reach_one_store() {
    let daemon = Daemon::bind(Config::local_test()).await.unwrap();
    let addresses = daemon.addresses();
    let store = daemon.store();
    let cancellation = CancellationToken::new();
    let server_cancellation = cancellation.clone();
    let server = tokio::spawn(async move {
        daemon.serve(server_cancellation).await.unwrap();
    });
    tokio::task::yield_now().await;

    let cases = [
        (
            client(ClientTransport::Http {
                endpoint: format!("http://{}/api/v1/events", addresses.http),
            }),
            progress_event("http-agent", "http-task", 0.25),
            Transport::Http,
        ),
        (
            client(ClientTransport::WebSocket {
                endpoint: format!("ws://{}/ws/agent", addresses.http),
            }),
            progress_event("websocket-agent", "websocket-task", 0.5),
            Transport::WebSocket,
        ),
        (
            client(ClientTransport::Tcp {
                address: addresses.tcp,
            }),
            progress_event("tcp-agent", "tcp-task", 0.75),
            Transport::Tcp,
        ),
        (
            client(ClientTransport::Udp {
                address: addresses.udp,
            }),
            progress_event("udp-agent", "udp-task", 1.0),
            Transport::Udp,
        ),
    ];

    for (client, event, expected_transport) in cases {
        let acknowledgement = client.send(&event).await.unwrap();
        assert!(acknowledgement.accepted);
        assert!(!acknowledgement.duplicate);
        assert_eq!(acknowledgement.event_id, event.event_id);
        assert_eq!(acknowledgement.transport, expected_transport);
    }

    let snapshot = store.snapshot().await;
    assert_eq!(snapshot.tasks.len(), 4);
    for transport in ["http", "websocket", "tcp", "udp"] {
        assert_eq!(
            snapshot.counters.accepted_by_transport.get(transport),
            Some(&1)
        );
    }

    cancellation.cancel();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn client_rejects_udp_control_events_before_network_mutation() {
    let daemon = Daemon::bind(Config::local_test()).await.unwrap();
    let addresses = daemon.addresses();
    let store = daemon.store();
    let cancellation = CancellationToken::new();
    let server_cancellation = cancellation.clone();
    let server = tokio::spawn(async move {
        daemon.serve(server_cancellation).await.unwrap();
    });
    tokio::task::yield_now().await;

    let event = EventEnvelope::new(
        AgentRef {
            agent_id: "udp-control-test".to_owned(),
            provider: "client-integration".to_owned(),
            model: "fixture".to_owned(),
            instance_id: None,
        },
        AgentEvent::TaskCreated(TaskSpec {
            task_id: "must-not-cross-udp".to_owned(),
            title: "Privileged task declaration".to_owned(),
            goal_id: None,
            depends_on: Vec::new(),
            tags: Vec::new(),
            expected_outcome: None,
        }),
    );
    let error = client(ClientTransport::Udp {
        address: addresses.udp,
    })
    .send(&event)
    .await
    .expect_err("task declarations must not cross UDP");
    assert!(matches!(error, ClientError::UdpPolicy { ref kind } if kind == "task_created"));
    assert_eq!(store.revision().await, 0);

    cancellation.cancel();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn wrong_http_credentials_return_a_bounded_remote_error() {
    let daemon = Daemon::bind(Config::local_test()).await.unwrap();
    let addresses = daemon.addresses();
    let cancellation = CancellationToken::new();
    let server_cancellation = cancellation.clone();
    let server = tokio::spawn(async move {
        daemon.serve(server_cancellation).await.unwrap();
    });
    tokio::task::yield_now().await;

    let client = EventClient::new(
        ClientConfig::new(
            ClientTransport::Http {
                endpoint: format!("http://{}/api/v1/events", addresses.http),
            },
            Some("wrong-token-at-least-16-bytes".to_owned()),
        )
        .with_timeout(Duration::from_secs(5)),
    )
    .unwrap();
    let error = client
        .send(&progress_event("unauthorized-agent", "task", 0.1))
        .await
        .expect_err("wrong credentials must be rejected");
    assert!(matches!(
        error,
        ClientError::RemoteRejected { ref code, .. } if code == "unauthorized"
    ));

    cancellation.cancel();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}
