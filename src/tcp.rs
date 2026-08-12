use std::{collections::HashSet, io, sync::Arc};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream, tcp::OwnedWriteHalf},
    sync::Semaphore,
    task::JoinSet,
};
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};
use tracing::{debug, warn};

use crate::{
    auth::AuthPolicy,
    bridge::BridgeTcpFrame,
    model::{Transport, TransportPayload},
    store::Store,
};

pub async fn serve(
    listener: TcpListener,
    store: Store,
    auth: AuthPolicy,
    max_payload_bytes: usize,
    max_connections: usize,
    cancellation: CancellationToken,
) -> io::Result<()> {
    let limiter = Arc::new(Semaphore::new(max_connections));
    let mut connections = JoinSet::new();

    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let Ok(permit) = Arc::clone(&limiter).try_acquire_owned() else {
                    warn!(%peer, "rejecting TCP agent connection because the connection limit is full");
                    drop(stream);
                    continue;
                };
                let connection_store = store.clone();
                let connection_auth = auth.clone();
                let connection_cancellation = cancellation.child_token();
                connections.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(
                        stream,
                        connection_store,
                        connection_auth,
                        max_payload_bytes,
                        connection_cancellation,
                    )
                    .await
                    {
                        debug!(%peer, %error, "TCP agent connection ended");
                    }
                });
            }
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = result {
                    warn!(%error, "TCP agent connection task failed");
                }
            }
        }
    }

    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            warn!(%error, "TCP agent connection task failed during shutdown");
        }
    }
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    store: Store,
    auth: AuthPolicy,
    max_payload_bytes: usize,
    cancellation: CancellationToken,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let (read_half, mut write_half) = stream.into_split();
    let mut frames = FramedRead::new(
        read_half,
        LinesCodec::new_with_max_length(max_payload_bytes),
    );
    let mut joined_participants = HashSet::<(String, String)>::new();

    loop {
        let frame = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            frame = frames.next() => frame,
        };
        let Some(frame) = frame else {
            return Ok(());
        };

        let line = match frame {
            Ok(line) => line,
            Err(error) => {
                store.record_rejection(Transport::Tcp).await;
                write_json_line(
                    &mut write_half,
                    &json!({ "error": "invalid_frame", "message": error.to_string() }),
                )
                .await?;
                continue;
            }
        };

        let value = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(error) => {
                store.record_rejection(Transport::Tcp).await;
                write_json_line(
                    &mut write_half,
                    &json!({ "error": "invalid_json", "message": error.to_string() }),
                )
                .await?;
                continue;
            }
        };

        if value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("bridge_"))
        {
            let frame = match serde_json::from_value::<BridgeTcpFrame>(value) {
                Ok(frame) => frame,
                Err(error) => {
                    store.bridge().record_rejection(Transport::Tcp).await;
                    write_json_line(
                        &mut write_half,
                        &json!({ "error": "invalid_bridge_frame", "message": error.to_string() }),
                    )
                    .await?;
                    continue;
                }
            };
            handle_bridge_frame(
                &mut write_half,
                &store,
                &auth,
                &mut joined_participants,
                frame,
            )
            .await?;
            continue;
        }

        let payload = match serde_json::from_value::<TransportPayload>(value) {
            Ok(payload) => payload,
            Err(error) => {
                store.record_rejection(Transport::Tcp).await;
                write_json_line(
                    &mut write_half,
                    &json!({ "error": "invalid_json", "message": error.to_string() }),
                )
                .await?;
                continue;
            }
        };
        let (token, event) = payload.into_parts();

        if let Err(error) = auth.authorize_ingest(token.as_deref()) {
            store.record_rejection(Transport::Tcp).await;
            write_json_line(
                &mut write_half,
                &json!({ "error": "unauthorized", "message": error.to_string() }),
            )
            .await?;
            continue;
        }

        match store.ingest(event, Transport::Tcp).await {
            Ok(ack) => write_json_line(&mut write_half, &ack).await?,
            Err(error) => {
                write_json_line(
                    &mut write_half,
                    &json!({ "error": "invalid_event", "message": error.to_string() }),
                )
                .await?;
            }
        }
    }
}

async fn handle_bridge_frame(
    writer: &mut OwnedWriteHalf,
    store: &Store,
    auth: &AuthPolicy,
    joined_participants: &mut HashSet<(String, String)>,
    frame: BridgeTcpFrame,
) -> io::Result<()> {
    let authorized = if frame.is_read() {
        auth.authorize_read(frame.token())
    } else {
        auth.authorize_ingest(frame.token())
    };
    if let Err(error) = authorized {
        store.bridge().record_rejection(Transport::Tcp).await;
        return write_json_line(
            writer,
            &json!({ "error": "unauthorized", "message": error.to_string() }),
        )
        .await;
    }

    let bridge = store.bridge();
    let result = match frame {
        BridgeTcpFrame::BridgeCreateRoom { room, .. } => bridge
            .create_room(room)
            .await
            .map(|room| json!({ "type": "bridge_room", "room": room })),
        BridgeTcpFrame::BridgeJoin {
            room_slug,
            participant,
            ..
        } => match bridge.join(&room_slug, participant, false).await {
            Ok(participant) => {
                joined_participants.insert((room_slug, participant.participant_id.clone()));
                Ok(json!({ "type": "bridge_participant", "participant": participant }))
            }
            Err(error) => Err(error),
        },
        BridgeTcpFrame::BridgeMessage {
            room_slug, message, ..
        } => {
            if !joined_participants
                .contains(&(room_slug.clone(), message.author.participant_id.clone()))
            {
                Err(crate::bridge::BridgeError::ParticipantNotJoined)
            } else {
                bridge
                    .post_message(&room_slug, message, Transport::Tcp)
                    .await
                    .map(|ack| json!({ "type": "bridge_ack", "ack": ack }))
            }
        }
        BridgeTcpFrame::BridgeSnapshot { room_slug, .. } => bridge
            .snapshot(&room_slug)
            .await
            .map(|snapshot| json!({ "type": "bridge_snapshot", "snapshot": snapshot })),
    };
    match result {
        Ok(response) => write_json_line(writer, &response).await,
        Err(error) => {
            bridge.record_rejection(Transport::Tcp).await;
            write_json_line(
                writer,
                &json!({ "error": "invalid_bridge_request", "message": error.to_string() }),
            )
            .await
        }
    }
}

async fn write_json_line<T: Serialize>(writer: &mut OwnedWriteHalf, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use uuid::Uuid;

    use crate::{
        auth::AuthPolicy,
        bridge::{
            BRIDGE_PROTOCOL_VERSION, BridgeMessageInput, BridgeParticipantInput,
            BridgeParticipantKind, BridgeRoomInput,
        },
        config::Config,
    };

    use super::*;

    async fn round_trip(
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
        frame: &BridgeTcpFrame,
    ) -> Value {
        let mut bytes = serde_json::to_vec(frame).unwrap();
        bytes.push(b'\n');
        writer.write_all(&bytes).await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn tcp_bridge_frames_create_rooms_and_record_transport_evidence() {
        let config = Config::local_test();
        let store = Store::new(config.cache_config(), config.update_channel_capacity);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let cancellation = CancellationToken::new();
        let server = tokio::spawn(serve(
            listener,
            store.clone(),
            AuthPolicy::from_config(&config),
            1_048_576,
            8,
            cancellation.clone(),
        ));

        let stream = TcpStream::connect(address).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let token = Some("test-token-at-least-16-bytes".to_owned());
        let response = round_trip(
            &mut write_half,
            &mut reader,
            &BridgeTcpFrame::BridgeCreateRoom {
                token: token.clone(),
                room: BridgeRoomInput {
                    slug: "tcp-lab".to_owned(),
                    title: "TCP lab".to_owned(),
                    objective: "Verify independent TCP bridge ingress".to_owned(),
                },
            },
        )
        .await;
        assert_eq!(response["type"], "bridge_room");

        let peer = BridgeParticipantInput {
            participant_id: "tcp-peer".to_owned(),
            display_name: "TCP peer".to_owned(),
            kind: BridgeParticipantKind::Agent,
            provider: Some("local".to_owned()),
            model: None,
            runtime_agent_id: None,
        };
        let response = round_trip(
            &mut write_half,
            &mut reader,
            &BridgeTcpFrame::BridgeJoin {
                token: token.clone(),
                room_slug: "tcp-lab".to_owned(),
                participant: peer.clone(),
            },
        )
        .await;
        assert_eq!(response["type"], "bridge_participant");

        let response = round_trip(
            &mut write_half,
            &mut reader,
            &BridgeTcpFrame::BridgeMessage {
                token: token.clone(),
                room_slug: "tcp-lab".to_owned(),
                message: BridgeMessageInput {
                    protocol_version: BRIDGE_PROTOCOL_VERSION.to_owned(),
                    message_id: Uuid::new_v4(),
                    occurred_at: Utc::now(),
                    author: peer.clone(),
                    summary: "TCP ingress reached the shared room.".to_owned(),
                    reply_to: None,
                },
            },
        )
        .await;
        assert_eq!(response["type"], "bridge_ack");
        assert_eq!(response["ack"]["accepted"], true);
        let snapshot = store.bridge().snapshot("tcp-lab").await.unwrap();
        assert_eq!(snapshot.counters.accepted_by_transport["tcp"], 1);

        let replay_stream = TcpStream::connect(address).await.unwrap();
        let (replay_read, mut replay_write) = replay_stream.into_split();
        let mut replay_reader = BufReader::new(replay_read);
        let response = round_trip(
            &mut replay_write,
            &mut replay_reader,
            &BridgeTcpFrame::BridgeMessage {
                token,
                room_slug: "tcp-lab".to_owned(),
                message: BridgeMessageInput {
                    protocol_version: BRIDGE_PROTOCOL_VERSION.to_owned(),
                    message_id: Uuid::new_v4(),
                    occurred_at: Utc::now(),
                    author: peer,
                    summary: "A new connection must join before posting.".to_owned(),
                    reply_to: None,
                },
            },
        )
        .await;
        assert_eq!(response["error"], "invalid_bridge_request");
        assert!(
            response["message"]
                .as_str()
                .unwrap()
                .contains("has not joined")
        );

        cancellation.cancel();
        server.await.unwrap().unwrap();
    }
}
