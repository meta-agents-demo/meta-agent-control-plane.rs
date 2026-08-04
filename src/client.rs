use std::{
    fmt,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use axum::http::Uri;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header::AUTHORIZATION},
    },
};
use tokio_util::codec::{Framed, LinesCodec};

use crate::{
    model::{EventEnvelope, Transport, TransportFrame},
    store::IngestAck,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_CLIENT_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_UDP_DATAGRAM_BYTES: usize = 65_507;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientTransport {
    Http { endpoint: String },
    WebSocket { endpoint: String },
    Tcp { address: SocketAddr },
    Udp { address: SocketAddr },
}

impl ClientTransport {
    pub const fn protocol_transport(&self) -> Transport {
        match self {
            Self::Http { .. } => Transport::Http,
            Self::WebSocket { .. } => Transport::WebSocket,
            Self::Tcp { .. } => Transport::Tcp,
            Self::Udp { .. } => Transport::Udp,
        }
    }
}

#[derive(Clone)]
pub struct ClientConfig {
    pub transport: ClientTransport,
    pub token: Option<String>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientConfig")
            .field("transport", &self.transport)
            .field("token_configured", &self.token.is_some())
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

impl ClientConfig {
    pub fn new(transport: ClientTransport, token: Option<String>) -> Self {
        Self {
            transport,
            token,
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub const fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    pub fn validate(&self) -> Result<(), ClientError> {
        if self.timeout.is_zero() {
            return Err(ClientError::InvalidConfig("timeout must be greater than zero"));
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_CLIENT_RESPONSE_BYTES {
            return Err(ClientError::InvalidConfig(
                "maximum response bytes must be between 1 and 16777216",
            ));
        }
        if self.token.as_deref().is_some_and(|token| {
            token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace())
        }) {
            return Err(ClientError::InvalidConfig(
                "authentication tokens must be non-empty and contain no whitespace",
            ));
        }
        match &self.transport {
            ClientTransport::Http { endpoint } => {
                let parsed = parse_http_endpoint(endpoint)?;
                if parsed.path.is_empty() {
                    return Err(ClientError::InvalidEndpoint(
                        "HTTP endpoint must include an ingestion path".to_owned(),
                    ));
                }
            }
            ClientTransport::WebSocket { endpoint } => {
                if !endpoint.starts_with("ws://") {
                    return Err(ClientError::InvalidEndpoint(
                        "embedded WebSocket client currently requires a ws:// endpoint; use a local TLS terminator for wss://".to_owned(),
                    ));
                }
                endpoint
                    .as_str()
                    .into_client_request()
                    .map_err(|error| ClientError::InvalidEndpoint(error.to_string()))?;
            }
            ClientTransport::Tcp { .. } | ClientTransport::Udp { .. } => {}
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct EventClient {
    config: ClientConfig,
}

impl fmt::Debug for EventClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventClient")
            .field("config", &self.config)
            .finish()
    }
}

impl EventClient {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub const fn config(&self) -> &ClientConfig {
        &self.config
    }

    pub async fn send(&self, event: &EventEnvelope) -> Result<IngestAck, ClientError> {
        event
            .validate()
            .map_err(|error| ClientError::InvalidEvent(error.to_string()))?;
        let expected_transport = self.config.transport.protocol_transport();
        let ack = match &self.config.transport {
            ClientTransport::Http { endpoint } => self.send_http(endpoint, event).await?,
            ClientTransport::WebSocket { endpoint } => {
                self.send_websocket(endpoint, event).await?
            }
            ClientTransport::Tcp { address } => self.send_tcp(*address, event).await?,
            ClientTransport::Udp { address } => self.send_udp(*address, event).await?,
        };
        validate_ack(&ack, event, expected_transport)?;
        Ok(ack)
    }

    async fn send_http(
        &self,
        endpoint: &str,
        event: &EventEnvelope,
    ) -> Result<IngestAck, ClientError> {
        let endpoint = parse_http_endpoint(endpoint)?;
        let body = serde_json::to_vec(event)?;
        let mut request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
            endpoint.path,
            endpoint.host_header,
            body.len()
        );
        if let Some(token) = self.config.token.as_deref() {
            request.push_str("Authorization: Bearer ");
            request.push_str(token);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");

        let mut stream = timed(
            self.config.timeout,
            TcpStream::connect(endpoint.connect_address),
        )
        .await??;
        timed(self.config.timeout, stream.write_all(request.as_bytes())).await??;
        timed(self.config.timeout, stream.write_all(&body)).await??;
        timed(self.config.timeout, stream.flush()).await??;

        let total_limit = self
            .config
            .max_response_bytes
            .saturating_add(MAX_HTTP_HEADER_BYTES)
            .saturating_add(1);
        let mut response = Vec::new();
        let mut limited = stream.take(total_limit as u64);
        timed(self.config.timeout, limited.read_to_end(&mut response)).await??;
        if response.len() >= total_limit {
            return Err(ClientError::ResponseTooLarge {
                maximum: self.config.max_response_bytes,
            });
        }
        let separator = find_subsequence(&response, b"\r\n\r\n")
            .ok_or_else(|| ClientError::InvalidResponse("HTTP headers were incomplete".to_owned()))?;
        if separator > MAX_HTTP_HEADER_BYTES {
            return Err(ClientError::InvalidResponse(
                "HTTP response headers exceeded the client limit".to_owned(),
            ));
        }
        let header_bytes = &response[..separator];
        let response_body = &response[separator + 4..];
        if response_body.len() > self.config.max_response_bytes {
            return Err(ClientError::ResponseTooLarge {
                maximum: self.config.max_response_bytes,
            });
        }
        let headers = std::str::from_utf8(header_bytes)
            .map_err(|_| ClientError::InvalidResponse("HTTP headers were not UTF-8".to_owned()))?;
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| ClientError::InvalidResponse("HTTP status line was invalid".to_owned()))?;
        if !(200..300).contains(&status) {
            return Err(decode_remote_error(response_body, Some(status)));
        }
        decode_ack(response_body)
    }

    async fn send_websocket(
        &self,
        endpoint: &str,
        event: &EventEnvelope,
    ) -> Result<IngestAck, ClientError> {
        let mut request = endpoint
            .into_client_request()
            .map_err(|error| ClientError::InvalidEndpoint(error.to_string()))?;
        if let Some(token) = self.config.token.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| ClientError::InvalidConfig("authentication token is not a valid header value"))?;
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        let (mut socket, _) = timed(self.config.timeout, connect_async(request))
            .await?
            .map_err(|error| ClientError::WebSocket(error.to_string()))?;
        let payload = serde_json::to_string(event)?;
        timed(
            self.config.timeout,
            socket.send(Message::Text(payload.into())),
        )
        .await?
        .map_err(|error| ClientError::WebSocket(error.to_string()))?;

        loop {
            let message = timed(self.config.timeout, socket.next())
                .await?
                .ok_or(ClientError::ConnectionClosed)?
                .map_err(|error| ClientError::WebSocket(error.to_string()))?;
            match message {
                Message::Text(text) => {
                    let bytes = text.as_str().as_bytes();
                    ensure_response_size(bytes.len(), self.config.max_response_bytes)?;
                    return decode_ack(bytes);
                }
                Message::Binary(bytes) => {
                    ensure_response_size(bytes.len(), self.config.max_response_bytes)?;
                    return decode_ack(bytes.as_ref());
                }
                Message::Ping(bytes) => {
                    timed(self.config.timeout, socket.send(Message::Pong(bytes)))
                        .await?
                        .map_err(|error| ClientError::WebSocket(error.to_string()))?;
                }
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Close(_) => return Err(ClientError::ConnectionClosed),
            }
        }
    }

    async fn send_tcp(
        &self,
        address: SocketAddr,
        event: &EventEnvelope,
    ) -> Result<IngestAck, ClientError> {
        let stream = timed(self.config.timeout, TcpStream::connect(address)).await??;
        stream.set_nodelay(true)?;
        let mut framed = Framed::new(
            stream,
            LinesCodec::new_with_max_length(self.config.max_response_bytes),
        );
        let payload = serde_json::to_string(&TransportFrame {
            token: self.config.token.clone(),
            event: event.clone(),
        })?;
        timed(self.config.timeout, framed.send(payload))
            .await?
            .map_err(|error| ClientError::Framing(error.to_string()))?;
        let response = timed(self.config.timeout, framed.next())
            .await?
            .ok_or(ClientError::ConnectionClosed)?
            .map_err(|error| ClientError::Framing(error.to_string()))?;
        decode_ack(response.as_bytes())
    }

    async fn send_udp(
        &self,
        address: SocketAddr,
        event: &EventEnvelope,
    ) -> Result<IngestAck, ClientError> {
        if !event.event.allowed_over_udp() {
            return Err(ClientError::UdpPolicy {
                kind: event.kind().to_owned(),
            });
        }
        let bind_address = match address.ip() {
            IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
        };
        let socket = UdpSocket::bind(bind_address).await?;
        let payload = serde_json::to_vec(&TransportFrame {
            token: self.config.token.clone(),
            event: event.clone(),
        })?;
        if payload.len() > MAX_UDP_DATAGRAM_BYTES {
            return Err(ClientError::InvalidConfig(
                "serialized UDP event exceeds the protocol datagram ceiling",
            ));
        }
        timed(self.config.timeout, socket.send_to(&payload, address)).await??;
        let response_limit = self.config.max_response_bytes.min(MAX_UDP_DATAGRAM_BYTES);
        let mut response = vec![0_u8; response_limit];
        let (length, peer) = timed(self.config.timeout, socket.recv_from(&mut response)).await??;
        if peer != address {
            return Err(ClientError::InvalidResponse(
                "UDP acknowledgement came from an unexpected peer".to_owned(),
            ));
        }
        decode_ack(&response[..length])
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid client configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("event validation failed: {0}")]
    InvalidEvent(String),
    #[error("client operation timed out")]
    Timeout,
    #[error("connection closed before an acknowledgement arrived")]
    ConnectionClosed,
    #[error("remote server rejected the event with {code}: {message}")]
    RemoteRejected { code: String, message: String },
    #[error("HTTP server returned status {0}")]
    HttpStatus(u16),
    #[error("response exceeded the configured {maximum}-byte limit")]
    ResponseTooLarge { maximum: usize },
    #[error("invalid server response: {0}")]
    InvalidResponse(String),
    #[error("unexpected acknowledgement: {0}")]
    UnexpectedAcknowledgement(String),
    #[error("event kind {kind} is not permitted over UDP")]
    UdpPolicy { kind: String },
    #[error("WebSocket transport failed: {0}")]
    WebSocket(String),
    #[error("line framing failed: {0}")]
    Framing(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
struct HttpEndpoint {
    connect_address: String,
    host_header: String,
    path: String,
}

fn parse_http_endpoint(endpoint: &str) -> Result<HttpEndpoint, ClientError> {
    let uri = Uri::from_str(endpoint)
        .map_err(|error| ClientError::InvalidEndpoint(error.to_string()))?;
    if uri.scheme_str() != Some("http") {
        return Err(ClientError::InvalidEndpoint(
            "embedded HTTP client currently requires http://; use a local TLS terminator for https://"
                .to_owned(),
        ));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| ClientError::InvalidEndpoint("HTTP endpoint has no authority".to_owned()))?;
    let host = authority.host();
    if host.is_empty() {
        return Err(ClientError::InvalidEndpoint(
            "HTTP endpoint has no host".to_owned(),
        ));
    }
    let port = authority.port_u16().unwrap_or(80);
    let connect_address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let path = uri
        .path_and_query()
        .map_or_else(|| "/".to_owned(), ToString::to_string);
    Ok(HttpEndpoint {
        connect_address,
        host_header: authority.as_str().to_owned(),
        path,
    })
}

async fn timed<T, F>(duration: Duration, future: F) -> Result<T, ClientError>
where
    F: Future<Output = T>,
{
    timeout(duration, future)
        .await
        .map_err(|_| ClientError::Timeout)
}

fn ensure_response_size(length: usize, maximum: usize) -> Result<(), ClientError> {
    if length > maximum {
        Err(ClientError::ResponseTooLarge { maximum })
    } else {
        Ok(())
    }
}

fn validate_ack(
    ack: &IngestAck,
    event: &EventEnvelope,
    expected_transport: Transport,
) -> Result<(), ClientError> {
    if !ack.accepted {
        return Err(ClientError::UnexpectedAcknowledgement(
            "server returned accepted=false".to_owned(),
        ));
    }
    if ack.event_id != event.event_id {
        return Err(ClientError::UnexpectedAcknowledgement(
            "event ID did not match the submitted event".to_owned(),
        ));
    }
    if ack.transport != expected_transport {
        return Err(ClientError::UnexpectedAcknowledgement(format!(
            "transport was {}, expected {}",
            ack.transport, expected_transport
        )));
    }
    Ok(())
}

fn decode_ack(bytes: &[u8]) -> Result<IngestAck, ClientError> {
    match serde_json::from_slice::<IngestAck>(bytes) {
        Ok(ack) => Ok(ack),
        Err(error) => match serde_json::from_slice::<RemoteError>(bytes) {
            Ok(remote) => Err(ClientError::RemoteRejected {
                code: remote.error,
                message: remote.message.unwrap_or_else(|| "request rejected".to_owned()),
            }),
            Err(_) => Err(ClientError::InvalidResponse(error.to_string())),
        },
    }
}

fn decode_remote_error(bytes: &[u8], status: Option<u16>) -> ClientError {
    if let Ok(remote) = serde_json::from_slice::<RemoteError>(bytes) {
        ClientError::RemoteRejected {
            code: remote.error,
            message: remote.message.unwrap_or_else(|| "request rejected".to_owned()),
        }
    } else if let Some(status) = status {
        ClientError::HttpStatus(status)
    } else {
        ClientError::InvalidResponse("remote error was not valid JSON".to_owned())
    }
}

#[derive(Debug, Deserialize)]
struct RemoteError {
    error: String,
    #[serde(default)]
    message: Option<String>,
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::model::{AgentEvent, AgentRef, ProgressUpdate};

    use super::*;

    fn event() -> EventEnvelope {
        EventEnvelope::new(
            AgentRef {
                agent_id: "client-test".to_owned(),
                provider: "test".to_owned(),
                model: "fixture".to_owned(),
                instance_id: None,
            },
            AgentEvent::ProgressUpdated(ProgressUpdate {
                task_id: "task-1".to_owned(),
                progress: 0.5,
                summary: "Observable progress".to_owned(),
                blocker: None,
                next_action: Some("Verify acknowledgement".to_owned()),
            }),
        )
    }

    #[test]
    fn debug_output_redacts_the_token() {
        let config = ClientConfig::new(
            ClientTransport::Tcp {
                address: "127.0.0.1:8788".parse().unwrap(),
            },
            Some("do-not-print-this-token".to_owned()),
        );
        let debug = format!("{config:?}");
        assert!(debug.contains("token_configured: true"));
        assert!(!debug.contains("do-not-print-this-token"));
    }

    #[test]
    fn rejects_secret_bearing_or_empty_client_tokens() {
        for token in ["", "contains space", "contains\nnewline"] {
            let config = ClientConfig::new(
                ClientTransport::Tcp {
                    address: "127.0.0.1:8788".parse().unwrap(),
                },
                Some(token.to_owned()),
            );
            assert!(config.validate().is_err(), "accepted token {token:?}");
        }
    }

    #[test]
    fn parses_http_endpoints_without_putting_credentials_in_urls() {
        let endpoint = parse_http_endpoint("http://127.0.0.1:8787/api/v1/events?batch=false")
            .unwrap();
        assert_eq!(endpoint.connect_address, "127.0.0.1:8787");
        assert_eq!(endpoint.host_header, "127.0.0.1:8787");
        assert_eq!(endpoint.path, "/api/v1/events?batch=false");
    }

    #[test]
    fn rejects_non_plaintext_embedded_endpoints_explicitly() {
        let config = ClientConfig::new(
            ClientTransport::Http {
                endpoint: "https://example.invalid/api/v1/events".to_owned(),
            },
            None,
        );
        assert!(matches!(
            config.validate(),
            Err(ClientError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn validates_ack_identity_and_transport() {
        let event = event();
        let ack = IngestAck {
            accepted: true,
            duplicate: false,
            event_id: event.event_id,
            revision: 1,
            transport: Transport::Http,
            received_at: Utc::now(),
        };
        assert!(validate_ack(&ack, &event, Transport::Http).is_ok());
        assert!(validate_ack(&ack, &event, Transport::Tcp).is_err());
    }
}
