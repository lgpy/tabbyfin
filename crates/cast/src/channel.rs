//! The TLS connection to a device.
//!
//! One tokio task owns the socket. It starts the connection (`CONNECT` to `receiver-0`), sends the
//! heartbeat and answers the device's own pings, writes what callers hand it, and matches replies
//! to requests by their `requestId`. Whatever it doesn't consume — the device's status updates and
//! the Jellyfin app's messages — arrives on the [`Message`] stream.
//!
//! The task also decides the connection is dead when nothing has arrived for `idle_timeout`: a
//! device that stops answering is indistinguishable from one that was switched off, and the caller
//! should reconnect rather than hang (see docs/chromecast.md, "Staying connected").

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsConnector;
use tracing::debug;

use crate::codec::{self, CastMessage};
use crate::{Error, Result};

/// The namespaces both sides use for the transport itself. Anything else is an app's.
pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";

/// The device's own id. Apps get a `transportId` from `LAUNCH`.
pub const DEVICE_ID: &str = "receiver-0";

/// How the connection behaves. The defaults are the device's; tests shorten them.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// How often we send `PING`. Device guidance is 5 s.
    pub ping_interval: Duration,
    /// How long the device can go without sending anything before we call the connection dead.
    pub idle_timeout: Duration,
    /// How long [`Channel::request`] waits for its reply.
    pub request_timeout: Duration,
    /// How long connecting (TCP and TLS) may take.
    pub connect_timeout: Duration,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// A message no part of the channel consumed: an app's namespace, or an unsolicited
/// `RECEIVER_STATUS` / `MEDIA_STATUS`. The payload is the parsed JSON; a payload that isn't JSON
/// (only the app namespaces ever are) is kept as a string rather than dropped.
#[derive(Debug, Clone)]
pub struct Message {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: Value,
}

/// What callers ask the connection task to write.
enum Outgoing {
    Send {
        namespace: String,
        destination: String,
        payload: Value,
    },
    Request {
        namespace: String,
        destination: String,
        payload: Value,
        request_id: u64,
        reply: oneshot::Sender<Value>,
    },
    ConnectApp(String),
    CloseApp(String),
    /// The caller timed out; forget its pending reply.
    Cancel(u64),
}

/// The connection to one device. Dropping it closes the connection (and, on the device, the
/// session we opened); [`Channel::close_app`] leaves the app running instead.
pub struct Channel {
    sender_id: String,
    out: mpsc::Sender<Outgoing>,
    next_request_id: AtomicU64,
    request_timeout: Duration,
    /// The connection task, aborted (dropping the socket) when the channel is dropped.
    task: AbortOnDrop,
}

impl Channel {
    /// Connects with the default [`ChannelConfig`].
    pub async fn connect(addr: SocketAddr) -> Result<(Self, mpsc::UnboundedReceiver<Message>)> {
        Self::connect_with(addr, ChannelConfig::default()).await
    }

    /// Connects to `addr` and returns the channel and the stream of messages the channel doesn't
    /// consume. The stream ends when the connection does. It's unbounded so that a caller busy
    /// awaiting a [`Channel::request`] never stalls the connection: a full stream would stop it
    /// reading that request's reply and the device's heartbeat.
    pub async fn connect_with(
        addr: SocketAddr,
        config: ChannelConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Message>)> {
        let tcp = tokio::time::timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Timeout)??;
        let connector = TlsConnector::from(tls_client_config()?);
        // Devices have self-signed certificates and no usable name, so the IP is the server name
        // and verification is skipped (docs/chromecast.md, "Transport").
        let server_name = ServerName::IpAddress(addr.ip().into());
        let stream = tokio::time::timeout(config.connect_timeout, connector.connect(server_name, tcp))
            .await
            .map_err(|_| Error::Timeout)??;
        debug!(device = %addr, "cast connection established");

        let sender_id = format!("sender-{}", uuid::Uuid::new_v4());
        let (out, out_rx) = mpsc::channel(32);
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(supervise(stream, sender_id.clone(), config.clone(), out_rx, in_tx));

        Ok((
            Self {
                sender_id,
                out,
                next_request_id: AtomicU64::new(1),
                request_timeout: config.request_timeout,
                task: AbortOnDrop(Some(task)),
            },
            in_rx,
        ))
    }

    /// This connection's sender id (`sender-<uuid>`), the `source_id` of everything we send.
    pub fn sender_id(&self) -> &str {
        &self.sender_id
    }

    /// Refuses a message too big for one frame here, with [`Error::FrameTooLarge`]: written, it
    /// would fail in the connection task and end the connection.
    fn check_size(&self, namespace: &str, destination: &str, payload: &Value) -> Result<()> {
        message_json(&self.sender_id, destination, namespace, payload)
            .encode()
            .map(drop)
    }

    /// Sends `payload` to `destination` in `namespace` without waiting for anything. Replies that
    /// carry no matching `requestId` (or none at all) come back on the message stream.
    pub async fn send(&self, namespace: &str, destination: &str, payload: Value) -> Result<()> {
        self.check_size(namespace, destination, &payload)?;
        self.out
            .send(Outgoing::Send {
                namespace: namespace.to_string(),
                destination: destination.to_string(),
                payload,
            })
            .await
            .map_err(|_| Error::Closed)
    }

    /// Sends `payload` with a fresh `requestId` and waits for the device's reply with the same id.
    /// Times out after [`ChannelConfig::request_timeout`].
    pub async fn request(&self, namespace: &str, destination: &str, mut payload: Value) -> Result<Value> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        payload["requestId"] = json!(request_id);
        self.check_size(namespace, destination, &payload)?;
        let (reply, reply_rx) = oneshot::channel();
        self.out
            .send(Outgoing::Request {
                namespace: namespace.to_string(),
                destination: destination.to_string(),
                payload,
                request_id,
                reply,
            })
            .await
            .map_err(|_| Error::Closed)?;
        match tokio::time::timeout(self.request_timeout, reply_rx).await {
            Ok(Ok(reply)) => Ok(reply),
            // The connection task ended before the device answered.
            Ok(Err(_)) => Err(Error::Closed),
            Err(_) => {
                // A device that answers nothing would otherwise leave the entry pending forever.
                let _ = self.out.try_send(Outgoing::Cancel(request_id));
                Err(Error::Timeout)
            }
        }
    }

    /// `CONNECT` to a launched app's `transportId`, so its messages can flow.
    pub async fn connect_app(&self, transport_id: &str) -> Result<()> {
        self.out
            .send(Outgoing::ConnectApp(transport_id.to_string()))
            .await
            .map_err(|_| Error::Closed)
    }

    /// `CLOSE` the connection to an app's `transportId`. This tells the app its sender left on
    /// purpose, and Jellyfin's receiver quits when its last sender does: to leave it playing, drop
    /// the connection instead ([`Channel::shutdown`]).
    pub async fn close_app(&self, transport_id: &str) -> Result<()> {
        self.out
            .send(Outgoing::CloseApp(transport_id.to_string()))
            .await
            .map_err(|_| Error::Closed)
    }

    /// Closes the connection once everything sent so far is written, waiting up to `wait`.
    /// Dropping the channel instead stops at once, and may lose a `CLOSE` still queued.
    pub async fn shutdown(self, wait: Duration) {
        let Self { out, mut task, .. } = self;
        // The task writes what's queued, then sees the channel close and shuts the socket.
        drop(out);
        if let Some(task) = task.0.take() {
            let _ = tokio::time::timeout(wait, task).await;
        }
    }
}

/// Owns the connection task and aborts it when the [`Channel`] goes away, which drops the socket.
/// [`Channel::shutdown`] takes the handle out to wait for the task instead.
struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

/// The connection task: writes, reads, heartbeats and times out.
async fn supervise(
    stream: tokio_rustls::client::TlsStream<TcpStream>,
    sender_id: String,
    config: ChannelConfig,
    mut outgoing: mpsc::Receiver<Outgoing>,
    incoming: mpsc::UnboundedSender<Message>,
) {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // The reader is its own task so that waiting for a frame is never cancelled half-way through
    // `read_exact` (which would lose bytes).
    let (frame_tx, mut frames) = mpsc::channel::<Result<CastMessage>>(16);
    let reader_task = tokio::spawn(async move {
        loop {
            let frame = codec::read_frame(&mut reader).await;
            let ended = frame.is_err();
            if frame_tx.send(frame).await.is_err() || ended {
                break;
            }
        }
    });
    let _reader = AbortOnDrop(Some(reader_task));

    let mut pending: HashMap<u64, oneshot::Sender<Value>> = HashMap::new();
    let mut last_received = tokio::time::Instant::now();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + config.ping_interval, config.ping_interval);

    let connect = CastMessage::json(&sender_id, DEVICE_ID, NS_CONNECTION, "{\"type\":\"CONNECT\"}");
    if codec::write_frame(&mut writer, &connect).await.is_err() {
        return;
    }

    loop {
        let idle = tokio::time::sleep_until(last_received + config.idle_timeout);
        tokio::pin!(idle);
        tokio::select! {
            frame = frames.recv() => {
                let Some(Ok(frame)) = frame else { break };
                last_received = tokio::time::Instant::now();
                let payload = parse_payload(&frame);
                if frame.namespace == NS_HEARTBEAT {
                    // The device keeps the connection alive with PINGs of its own.
                    if payload.get("type").and_then(Value::as_str) == Some("PING") {
                        let pong = CastMessage::json(&sender_id, &frame.source_id, NS_HEARTBEAT, "{\"type\":\"PONG\"}");
                        if codec::write_frame(&mut writer, &pong).await.is_err() {
                            break;
                        }
                    }
                    continue;
                }
                if let Some(id) = payload.get("requestId").and_then(Value::as_u64)
                    && let Some(reply) = pending.remove(&id)
                {
                    // The requester may have timed out and gone; nothing to do then.
                    let _ = reply.send(payload);
                    continue;
                }
                let message = Message {
                    source: frame.source_id,
                    destination: frame.destination_id,
                    namespace: frame.namespace,
                    payload,
                };
                // A caller that doesn't read the stream (or no longer wants it) mustn't stop the
                // connection, and neither may a slow one (see `connect_with`).
                let _ = incoming.send(message);
            }
            command = outgoing.recv() => {
                match command {
                    Some(Outgoing::Send { namespace, destination, payload }) => {
                        let message = message_json(&sender_id, &destination, &namespace, &payload);
                        if codec::write_frame(&mut writer, &message).await.is_err() {
                            break;
                        }
                    }
                    Some(Outgoing::Request { namespace, destination, mut payload, request_id, reply }) => {
                        // The device matches on requestId, so it has to be in the payload.
                        payload["requestId"] = json!(request_id);
                        pending.insert(request_id, reply);
                        let message = message_json(&sender_id, &destination, &namespace, &payload);
                        if codec::write_frame(&mut writer, &message).await.is_err() {
                            break;
                        }
                    }
                    Some(Outgoing::ConnectApp(transport)) => {
                        let message = CastMessage::json(
                            &sender_id,
                            &transport,
                            NS_CONNECTION,
                            "{\"type\":\"CONNECT\"}",
                        );
                        if codec::write_frame(&mut writer, &message).await.is_err() {
                            break;
                        }
                    }
                    Some(Outgoing::Cancel(request_id)) => {
                        pending.remove(&request_id);
                    }
                    Some(Outgoing::CloseApp(transport)) => {
                        let message = CastMessage::json(
                            &sender_id,
                            &transport,
                            NS_CONNECTION,
                            "{\"type\":\"CLOSE\"}",
                        );
                        if codec::write_frame(&mut writer, &message).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = ping.tick() => {
                let ping = CastMessage::json(&sender_id, DEVICE_ID, NS_HEARTBEAT, "{\"type\":\"PING\"}");
                if codec::write_frame(&mut writer, &ping).await.is_err() {
                    break;
                }
            }
            _ = &mut idle => {
                tracing::debug!("no traffic from the device for {:?}, closing", config.idle_timeout);
                break;
            }
        }
    }

    // Fail whatever was waiting on a reply: dropping the senders turns their receivers into errors.
    drop(pending);
    let _ = writer.shutdown().await;
}

fn message_json(sender_id: &str, destination: &str, namespace: &str, payload: &Value) -> CastMessage {
    CastMessage::json(sender_id, destination, namespace, &payload.to_string())
}

fn parse_payload(frame: &CastMessage) -> Value {
    frame
        .payload_utf8
        .as_deref()
        .and_then(|payload| serde_json::from_str(payload).ok())
        .unwrap_or_else(|| Value::String(frame.payload_utf8.clone().unwrap_or_default()))
}

/// A client config that accepts the device's self-signed certificate. It is built with the ring
/// provider explicitly: relying on a process-wide default would break on a host where something
/// else installed one (and the point of ring here is to keep aws-lc-rs out of the build).
fn tls_client_config() -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Devices present a self-signed certificate with no name we could check, so nothing is verified:
/// this is the protocol's own rule (see docs/chromecast.md, "Transport"). The connection is still
/// encrypted; what it protects is the network segment, not the device's identity.
#[derive(Debug)]
struct AcceptAnyCertificate;

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
