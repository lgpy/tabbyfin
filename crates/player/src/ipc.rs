//! JSON IPC transport: one line-delimited JSON message per line in each direction.
//! Requests carry a `request_id` that mpv echoes in its reply; everything else is an event.

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::events::EventTranslator;
use crate::{Error, PlayerEvent, Result};

pub(crate) struct Request {
    pub command: Value,
    pub reply: oneshot::Sender<Result<Value>>,
}

/// Runs the connection on a background task until mpv closes it.
pub(crate) fn start<S>(stream: S, events: mpsc::UnboundedSender<PlayerEvent>) -> mpsc::UnboundedSender<Request>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (requests_tx, requests_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(stream, requests_rx, events));
    requests_tx
}

async fn run<S>(stream: S, mut requests: mpsc::UnboundedReceiver<Request>, events: mpsc::UnboundedSender<PlayerEvent>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut pending: HashMap<u64, (String, oneshot::Sender<Result<Value>>)> = HashMap::new();
    let mut next_id = 1u64;
    let mut translator = EventTranslator::default();

    loop {
        tokio::select! {
            request = requests.recv() => {
                // The Player was dropped; the process is being killed.
                let Some(Request { command, reply }) = request else { break };
                let id = next_id;
                next_id += 1;
                let mut line = json!({ "command": command, "request_id": id }).to_string();
                line.push('\n');
                if writer.write_all(line.as_bytes()).await.is_err() {
                    let _ = reply.send(Err(Error::Closed));
                    break;
                }
                pending.insert(id, (command_name(&command), reply));
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    tracing::debug!("ignoring malformed mpv message: {line}");
                    continue;
                };
                if let Some(id) = message.get("request_id").and_then(Value::as_u64) {
                    if let Some((command, reply)) = pending.remove(&id) {
                        let _ = reply.send(response(command, &message));
                    }
                } else if let Some(event) = translator.translate(&message) {
                    let _ = events.send(event);
                }
            }
        }
    }

    for (_, (_, reply)) in pending {
        let _ = reply.send(Err(Error::Closed));
    }
    let _ = events.send(PlayerEvent::Exited);
}

fn command_name(command: &Value) -> String {
    command
        .get(0)
        .and_then(Value::as_str)
        .unwrap_or("command")
        .to_owned()
}

fn response(command: String, message: &Value) -> Result<Value> {
    match message.get("error").and_then(Value::as_str) {
        Some("success") => Ok(message.get("data").cloned().unwrap_or(Value::Null)),
        error => Err(Error::Command {
            command,
            message: error.unwrap_or("unknown error").to_owned(),
        }),
    }
}
