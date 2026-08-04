use std::{net::SocketAddr, time::Duration};

use chrono::{TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use meta_agent_control_plane::{
    Config, Daemon,
    daemon::{BoundAddresses, DaemonError},
    model::{
        AgentEvent, AgentRef, AgentStatus, EventEnvelope, Heartbeat, TaskSpec, TransportFrame,
        TransportPayload,
    },
    store::{Snapshot, Store},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Error as WebSocketError, Message,
        client::IntoClientRequest,
        http::{HeaderValue, header::AUTHORIZATION},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
enum NetworkTransport {
    Http,
    WebSocket,
    Tcp,
}

struct Harness {
    addresses: BoundAddresses,
    store: Store,
    token: String,
    cancellation: CancellationToken,
    task: JoinHandle<Result<(), DaemonError>>,
}

impl Harness {
    async fn start() -> Self {
        let config = Config::local_test();
        let token = config
            .auth_token
            .clone()
            .expect("local test configuration requires an auth token");
        let daemon = Daemon::bind(config).await.expect("bind test daemon");
        let addresses = daemon.addresses();
        let store = daemon.store();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move { daemon.serve(task_cancellation).await });
        tokio::task::yield_now().await;

        Self {
            addresses,
            store,
            token,
            cancellation,
            task,
        }
    }

    async fn stop(self) {
        self.cancellation.cancel();
        timeout(IO_TIMEOUT, self.task)
            .await
            .expect("daemon shutdown timeout")
            .expect("daemon task joined")
            .expect("daemon stopped cleanly");
    }
}

fn fixed_event(event_id: u128, event: AgentEvent) -> EventEnvelope {
    let mut envelope = EventEnvelope::new(
        AgentRef {
            agent_id: "network-agent".to_owned(),
            provider: "network-fixture".to_owned(),
            model: "network-model".to_owned(),
            instance_id: Some("loopback".to_owned()),
        },
        event,
    );
    envelope.event_id = Uuid::from_u128(event_id);
    envelope.occurred_at = Utc
        .timestamp_opt(1_810_000_000, 0)
        .single()
        .expect("fixed timestamp");
    envelope.session_id = Some("network-session".to_owned());
    envelope.correlation_id = Some("network-correlation".to_owned());
    envelope.sequence = Some(1);
    envelope
}

fn heartbeat_event(event_id: u128) -> EventEnvelope {
    fixed_event(
        event_id,
        AgentEvent::Heartbeat(Heartbeat {
            status: Some(AgentStatus::Running),
            active_task_id: None,
            load: Some(0.375),
        }),
    )
}

fn task_event(event_id: u128) -> EventEnvelope {
    fixed_event(
        event_id,
        AgentEvent::TaskCreated(TaskSpec {
            task_id: "network-task".to_owned(),
            title: "Exercise actual network transports".to_owned(),
            goal_id: None,
            depends_on: Vec::new(),
            tags: vec!["network".to_owned(), "conformance".to_owned()],
            expected_outcome: Some("Equivalent projection".to_owned()),
        }),
    )
}

fn normalized_projection(snapshot: &Snapshot) -> Value {
    json!({
        "revision": snapshot.revision,
        "agents": &snapshot.agents,
        "goals": &snapshot.goals,
        "tasks": &snapshot.tasks,
        "lessons": &snapshot.lessons,
    })
}

fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut remaining = body;
    loop {
        let line_end = remaining
            .windows(2)
            .position(|window| window == b"\r\n")
            .expect("chunk size line");
        let size = usize::from_str_radix(
            std::str::from_utf8(&remaining[..line_end]).expect("chunk size UTF-8"),
            16,
        )
        .expect("chunk size hexadecimal");
        remaining = &remaining[line_end + 2..];
        if size == 0 {
            break;
        }
        assert!(remaining.len() >= size + 2, "complete chunk payload");
        output.extend_from_slice(&remaining[..size]);
        assert_eq!(&remaining[size..size + 2], b"\r\n");
        remaining = &remaining[size + 2..];
    }
    output
}

async fn send_http(
    address: SocketAddr,
    token: &str,
    event: &EventEnvelope,
) -> (u16, Value) {
    let body = serde_json::to_vec(event).expect("serialize HTTP event");
    let request = format!(
        "POST /api/v1/events HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(address).await.expect("connect HTTP");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write HTTP headers");
    stream.write_all(&body).await.expect("write HTTP body");

    let mut response = Vec::new();
    timeout(IO_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("HTTP response timeout")
        .expect("read HTTP response");
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP header separator");
    let headers = std::str::from_utf8(&response[..separator]).expect("HTTP headers UTF-8");
    let status = headers
        .lines()
        .next()
        .expect("HTTP status line")
        .split_whitespace()
        .nth(1)
        .expect("HTTP status code")
        .parse::<u16>()
        .expect("numeric HTTP status");
    let raw_body = &response[separator + 4..];
    let decoded = if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked(raw_body)
    } else {
        raw_body.to_vec()
    };
    let payload = serde_json::from_slice(&decoded).expect("JSON HTTP response");
    (status, payload)
}

async fn send_tcp(address: SocketAddr, token: &str, event: &EventEnvelope) -> Value {
    let stream = TcpStream::connect(address).await.expect("connect TCP");
    let (read_half, mut write_half) = stream.into_split();
    let payload = TransportPayload::Frame(TransportFrame {
        token: Some(token.to_owned()),
        event: event.clone(),
    });
    let mut bytes = serde_json::to_vec(&payload).expect("serialize TCP payload");
    bytes.push(b'\n');
    write_half.write_all(&bytes).await.expect("write TCP frame");

    let mut lines = BufReader::new(read_half).lines();
    let response = timeout(IO_TIMEOUT, lines.next_line())
        .await
        .expect("TCP response timeout")
        .expect("read TCP response")
        .expect("TCP response line");
    serde_json::from_str(&response).expect("JSON TCP response")
}

async fn send_websocket(address: SocketAddr, token: &str, event: &EventEnvelope) -> Value {
    let mut request = format!("ws://{address}/ws/agent")
        .into_client_request()
        .expect("WebSocket request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization header"),
    );
    let (mut socket, response) = timeout(IO_TIMEOUT, connect_async(request))
        .await
        .expect("WebSocket handshake timeout")
        .expect("WebSocket handshake");
    assert_eq!(response.status().as_u16(), 101);

    let payload = TransportPayload::Frame(TransportFrame {
        token: None,
        event: event.clone(),
    });
    socket
        .send(Message::Text(
            serde_json::to_string(&payload)
                .expect("serialize WebSocket payload")
                .into(),
        ))
        .await
        .expect("send WebSocket frame");
    let message = timeout(IO_TIMEOUT, socket.next())
        .await
        .expect("WebSocket response timeout")
        .expect("WebSocket stream remains open")
        .expect("WebSocket response");
    let text = match message {
        Message::Text(text) => text,
        other => panic!("expected WebSocket text response, got {other:?}"),
    };
    let value = serde_json::from_str(text.as_ref()).expect("JSON WebSocket response");
    socket.close(None).await.expect("close WebSocket");
    value
}

async fn project_event(transport: NetworkTransport, event: EventEnvelope) -> Value {
    let harness = Harness::start().await;
    let response = match transport {
        NetworkTransport::Http => {
            let (status, response) = send_http(harness.addresses.http, &harness.token, &event).await;
            assert_eq!(status, 202);
            response
        }
        NetworkTransport::WebSocket => {
            send_websocket(harness.addresses.http, &harness.token, &event).await
        }
        NetworkTransport::Tcp => send_tcp(harness.addresses.tcp, &harness.token, &event).await,
    };
    assert_eq!(response["accepted"], true);
    assert_eq!(response["duplicate"], false);
    let snapshot = harness.store.snapshot().await;
    let projection = normalized_projection(&snapshot);
    harness.stop().await;
    projection
}

#[tokio::test]
async fn actual_http_websocket_and_tcp_telemetry_produce_identical_projection() {
    let mut projections = Vec::new();
    for transport in [
        NetworkTransport::Http,
        NetworkTransport::WebSocket,
        NetworkTransport::Tcp,
    ] {
        projections.push(project_event(transport, heartbeat_event(401)).await);
    }
    for projection in &projections[1..] {
        assert_eq!(projection, &projections[0]);
    }
}

#[tokio::test]
async fn actual_http_websocket_and_tcp_privileged_events_produce_identical_projection() {
    let mut projections = Vec::new();
    for transport in [
        NetworkTransport::Http,
        NetworkTransport::WebSocket,
        NetworkTransport::Tcp,
    ] {
        projections.push(project_event(transport, task_event(402)).await);
    }
    for projection in &projections[1..] {
        assert_eq!(projection, &projections[0]);
    }
}

#[tokio::test]
async fn actual_network_transports_reject_bad_credentials_without_state_mutation() {
    let harness = Harness::start().await;

    let (http_status, http_body) = send_http(
        harness.addresses.http,
        "wrong-http-token",
        &heartbeat_event(410),
    )
    .await;
    assert_eq!(http_status, 401);
    assert_eq!(http_body["error"], "unauthorized");

    let tcp_body = send_tcp(
        harness.addresses.tcp,
        "wrong-tcp-token",
        &heartbeat_event(411),
    )
    .await;
    assert_eq!(tcp_body["error"], "unauthorized");

    let mut request = format!("ws://{}/ws/agent", harness.addresses.http)
        .into_client_request()
        .expect("WebSocket request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer wrong-websocket-token"),
    );
    let error = timeout(IO_TIMEOUT, connect_async(request))
        .await
        .expect("WebSocket rejection timeout")
        .expect_err("bad WebSocket credential must fail the handshake");
    match error {
        WebSocketError::Http(response) => assert_eq!(response.status().as_u16(), 401),
        other => panic!("expected HTTP WebSocket rejection, got {other:?}"),
    }

    let snapshot = harness.store.snapshot().await;
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.counters.accepted, 0);
    assert_eq!(snapshot.counters.rejected, 3);
    assert_eq!(snapshot.counters.rejected_by_transport["http"], 1);
    assert_eq!(snapshot.counters.rejected_by_transport["websocket"], 1);
    assert_eq!(snapshot.counters.rejected_by_transport["tcp"], 1);
    assert!(snapshot.agents.is_empty());
    assert!(snapshot.tasks.is_empty());

    harness.stop().await;
}
