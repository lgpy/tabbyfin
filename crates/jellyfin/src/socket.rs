//! The session websocket (`/socket`): what the server pushes to this session, such as watched
//! state changed elsewhere, SyncPlay updates and (later) remote-control commands.
//!
//! [`Client::socket`] keeps one connection open in a tokio task, answering the server's
//! keep-alive and reconnecting with backoff, and hands out typed messages.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::header;
use reqwest_websocket::{Message, RequestBuilderExt, WebSocket};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::client::Query;
use crate::{Client, Error, Result};

/// The server asks for keep-alives at this interval unless it says otherwise.
const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub enum SocketEvent {
    /// Connected, or reconnected after a drop: anything pushed in between was missed.
    Connected,
    Disconnected,
    Message(ServerMessage),
}

#[derive(Debug, Clone)]
pub enum ServerMessage {
    /// Watched state, resume points or favorites changed, e.g. on another device.
    UserDataChanged { item_ids: Vec<String> },
    /// SyncPlay group state (joined, left, members, queue…). Typed with SyncPlay itself.
    SyncPlayGroupUpdate(serde_json::Value),
    /// A scheduled SyncPlay playback command.
    SyncPlayCommand(serde_json::Value),
    Other {
        message_type: String,
        data: serde_json::Value,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawMessage {
    message_type: String,
    #[serde(default)]
    data: serde_json::Value,
}

impl ServerMessage {
    /// `None` for the socket's own housekeeping (keep-alive).
    fn parse(text: &str) -> Option<std::result::Result<Self, serde_json::Error>> {
        let raw: RawMessage = match serde_json::from_str(text) {
            Ok(raw) => raw,
            Err(err) => return Some(Err(err)),
        };
        Some(Ok(match raw.message_type.as_str() {
            "KeepAlive" | "ForceKeepAlive" => return None,
            "UserDataChanged" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "PascalCase")]
                struct Data {
                    #[serde(default)]
                    user_data_list: Vec<Entry>,
                }
                #[derive(Deserialize)]
                #[serde(rename_all = "PascalCase")]
                struct Entry {
                    item_id: String,
                }
                match serde_json::from_value::<Data>(raw.data) {
                    Ok(data) => ServerMessage::UserDataChanged {
                        item_ids: data.user_data_list.into_iter().map(|e| e.item_id).collect(),
                    },
                    Err(err) => return Some(Err(err)),
                }
            }
            "SyncPlayGroupUpdate" => ServerMessage::SyncPlayGroupUpdate(raw.data),
            "SyncPlayCommand" => ServerMessage::SyncPlayCommand(raw.data),
            _ => ServerMessage::Other {
                message_type: raw.message_type,
                data: raw.data,
            },
        }))
    }
}

impl Client {
    /// Keeps the session websocket connected and forwards what the server sends. Must be
    /// called inside a tokio runtime. Drop the receiver to disconnect.
    pub fn socket(&self) -> mpsc::UnboundedReceiver<SocketEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let client = self.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            while !tx.is_closed() {
                match client.connect_socket().await {
                    Ok(socket) => {
                        backoff = Duration::from_secs(1);
                        tracing::info!("session websocket connected");
                        if tx.send(SocketEvent::Connected).is_err() {
                            break;
                        }
                        if let Err(err) = run(socket, &tx).await {
                            tracing::warn!("session websocket dropped: {err}");
                        }
                        let _ = tx.send(SocketEvent::Disconnected);
                    }
                    Err(err) => tracing::warn!("session websocket couldn't connect: {err}"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = tx.closed() => break,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });
        rx
    }

    async fn connect_socket(&self) -> Result<WebSocket> {
        let mut url = self.url("/socket");
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).expect("ws and wss are valid schemes");
        // The upgrade needs HTTP/1.1 and no overall timeout, so not the shared client.
        let http = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(format!("{}/{}", self.info().client, self.info().version))
            .build()?;
        let response = http
            .get(url)
            .header(header::AUTHORIZATION, self.authorization_header())
            .upgrade()
            .send()
            .await
            .map_err(socket_error)?;
        response.into_websocket().await.map_err(socket_error)
    }

    /// Tells the server what this session can do. Resend after reconnecting.
    pub async fn report_capabilities(&self) -> Result<()> {
        let body = serde_json::json!({
            "PlayableMediaTypes": ["Video"],
            "SupportedCommands": [],
            "SupportsMediaControl": false,
            "SupportsPersistentIdentifier": true,
        });
        self.post_no_content("/Sessions/Capabilities/Full", &Query::new(), Some(&body))
            .await
    }
}

fn socket_error(err: reqwest_websocket::Error) -> Error {
    match err {
        reqwest_websocket::Error::Reqwest(err) => Error::Http(err),
        reqwest_websocket::Error::Handshake(err) => match err {
            reqwest_websocket::HandshakeError::UnexpectedStatusCode(status) if status.as_u16() == 401 => {
                Error::Unauthorized
            }
            err => Error::Socket(err.to_string()),
        },
        err => Error::Socket(err.to_string()),
    }
}

/// Forwards messages until the connection drops or the receiver goes away.
async fn run(mut socket: WebSocket, tx: &mpsc::UnboundedSender<SocketEvent>) -> Result<()> {
    let mut keep_alive = tokio::time::interval(DEFAULT_KEEP_ALIVE / 2);
    keep_alive.reset();
    loop {
        tokio::select! {
            message = socket.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close { .. })) | None => return Ok(()),
                    Some(Ok(_)) => continue,
                    Some(Err(err)) => return Err(socket_error(err)),
                };
                // The server says how often it wants keep-alives (in seconds).
                if let Ok(raw) = serde_json::from_str::<RawMessage>(&text)
                    && raw.message_type == "ForceKeepAlive"
                    && let Some(seconds) = raw.data.as_u64().filter(|s| *s > 0)
                {
                    keep_alive = tokio::time::interval(Duration::from_secs(seconds) / 2);
                    keep_alive.reset();
                }
                match ServerMessage::parse(&text) {
                    Some(Ok(message)) => {
                        tracing::debug!(?message, "session websocket message");
                        if tx.send(SocketEvent::Message(message)).is_err() {
                            return Ok(());
                        }
                    }
                    Some(Err(err)) => tracing::warn!("unexpected websocket message: {err}"),
                    None => {}
                }
            }
            _ = keep_alive.tick() => {
                let ping = r#"{"MessageType":"KeepAlive"}"#;
                socket.send(Message::Text(ping.into())).await.map_err(socket_error)?;
            }
            _ = tx.closed() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerMessage;

    #[test]
    fn parses_messages() {
        let text = r#"{"MessageType":"UserDataChanged","MessageId":"x","Data":{"UserId":"u","UserDataList":[{"ItemId":"a","Played":true},{"ItemId":"b"}]}}"#;
        match ServerMessage::parse(text) {
            Some(Ok(ServerMessage::UserDataChanged { item_ids })) => assert_eq!(item_ids, ["a", "b"]),
            other => panic!("{other:?}"),
        }
        assert!(ServerMessage::parse(r#"{"MessageType":"ForceKeepAlive","Data":60}"#).is_none());
        assert!(matches!(
            ServerMessage::parse(r#"{"MessageType":"SyncPlayCommand","Data":{"Command":"Unpause"}}"#),
            Some(Ok(ServerMessage::SyncPlayCommand(_)))
        ));
        assert!(matches!(
            ServerMessage::parse(r#"{"MessageType":"Play","Data":{}}"#),
            Some(Ok(ServerMessage::Other { message_type, .. })) if message_type == "Play"
        ));
        assert!(matches!(ServerMessage::parse("nope"), Some(Err(_))));
    }
}
