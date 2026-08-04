use std::{net::SocketAddr, process::ExitCode, time::Duration};

use clap::{Parser, ValueEnum};
use meta_agent_control_plane::{
    client::{ClientConfig, ClientError, ClientTransport, EventClient},
    model::EventEnvelope,
    provider::{AdapterError, normalize_anthropic, normalize_gemini, normalize_openai},
};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{self, AsyncBufReadExt, BufReader};

const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ProviderArg {
    #[value(name = "openai")]
    OpenAi,
    Anthropic,
    Gemini,
}

impl ProviderArg {
    fn normalize(self, value: Value) -> Result<EventEnvelope, AdapterError> {
        match self {
            Self::OpenAi => normalize_openai(value),
            Self::Anthropic => normalize_anthropic(value),
            Self::Gemini => normalize_gemini(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum TransportArg {
    Http,
    WebSocket,
    Tcp,
    Udp,
}

#[derive(Debug, Parser)]
#[command(
    name = "meta-agent-sidecar",
    about = "Normalize observable provider updates and forward them to the Rust Meta Agent control plane"
)]
struct Args {
    /// Provider payload shape accepted on stdin as one JSON object per line.
    #[arg(long, value_enum)]
    provider: ProviderArg,

    /// Control-plane transport used after provider-neutral normalization.
    #[arg(long, value_enum, default_value = "http")]
    transport: TransportArg,

    /// Transport endpoint. Defaults to the matching local daemon listener.
    #[arg(long, env = "META_AGENT_ENDPOINT")]
    endpoint: Option<String>,

    /// Shared control-plane token. Never printed or added to the normalized event.
    #[arg(long, env = "META_AGENT_TOKEN")]
    token: Option<String>,

    #[arg(long, default_value_t = 10)]
    timeout_seconds: u64,

    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES)]
    max_response_bytes: usize,

    /// Normalize and print canonical events without opening a network connection.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, Error)]
enum SidecarError {
    #[error("no provider observations were supplied on stdin")]
    EmptyInput,
    #[error("line {line}: invalid JSON: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("line {line}: provider observation rejected: {source}")]
    Adapter {
        line: usize,
        #[source]
        source: AdapterError,
    },
    #[error("line {line}: control-plane delivery failed: {source}")]
    Client {
        line: usize,
        #[source]
        source: ClientError,
    },
    #[error("invalid socket address {endpoint:?}: {source}")]
    SocketAddress {
        endpoint: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("timeout must be greater than zero")]
    ZeroTimeout,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    ClientConfig(#[from] ClientError),
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("meta-agent-sidecar: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), SidecarError> {
    if args.timeout_seconds == 0 {
        return Err(SidecarError::ZeroTimeout);
    }
    let client = if args.dry_run {
        None
    } else {
        let transport = transport(&args)?;
        let config = ClientConfig::new(transport, args.token.clone())
            .with_timeout(Duration::from_secs(args.timeout_seconds))
            .with_max_response_bytes(args.max_response_bytes);
        Some(EventClient::new(config)?)
    };

    let mut lines = BufReader::new(io::stdin()).lines();
    let mut line_number = 0_usize;
    let mut observations = 0_usize;
    while let Some(line) = lines.next_line().await? {
        line_number = line_number.saturating_add(1);
        if line.trim().is_empty() {
            continue;
        }
        observations = observations.saturating_add(1);
        let value = serde_json::from_str::<Value>(&line).map_err(|source| SidecarError::Json {
            line: line_number,
            source,
        })?;
        let event = args
            .provider
            .normalize(value)
            .map_err(|source| SidecarError::Adapter {
                line: line_number,
                source,
            })?;

        if let Some(client) = client.as_ref() {
            let acknowledgement =
                client
                    .send(&event)
                    .await
                    .map_err(|source| SidecarError::Client {
                        line: line_number,
                        source,
                    })?;
            println!(
                "{}",
                serde_json::to_string(&acknowledgement).expect("ack serializes")
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&event).expect("event serializes")
            );
        }
    }

    if observations == 0 {
        return Err(SidecarError::EmptyInput);
    }
    Ok(())
}

fn transport(args: &Args) -> Result<ClientTransport, SidecarError> {
    let endpoint = args
        .endpoint
        .clone()
        .unwrap_or_else(|| match args.transport {
            TransportArg::Http => "http://127.0.0.1:8787/api/v1/events".to_owned(),
            TransportArg::WebSocket => "ws://127.0.0.1:8787/ws/agent".to_owned(),
            TransportArg::Tcp => "127.0.0.1:8788".to_owned(),
            TransportArg::Udp => "127.0.0.1:8789".to_owned(),
        });
    match args.transport {
        TransportArg::Http => Ok(ClientTransport::Http { endpoint }),
        TransportArg::WebSocket => Ok(ClientTransport::WebSocket { endpoint }),
        TransportArg::Tcp => Ok(ClientTransport::Tcp {
            address: parse_socket_address(endpoint)?,
        }),
        TransportArg::Udp => Ok(ClientTransport::Udp {
            address: parse_socket_address(endpoint)?,
        }),
    }
}

fn parse_socket_address(endpoint: String) -> Result<SocketAddr, SidecarError> {
    endpoint
        .parse()
        .map_err(|source| SidecarError::SocketAddress { endpoint, source })
}
