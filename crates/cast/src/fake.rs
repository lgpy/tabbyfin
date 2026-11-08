//! A scripted CASTV2 device, for tests.
//!
//! It is a TLS server with a self-signed certificate (what a real device has) that speaks the
//! connection, heartbeat and receiver namespaces and records the Jellyfin messages it's sent, so
//! the sender can be tested end to end on loopback with no hardware. `crates/cast/tests/` drives
//! it; the app's UI tests can too, through the `fake-device` feature.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};
use tokio_rustls::TlsAcceptor;

use crate::channel::{NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_RECEIVER};
use crate::codec::{self, CastMessage};
use crate::jellyfin::{NAMESPACE as APP_NAMESPACE, STABLE_APP_ID, UNSTABLE_APP_ID};
use crate::{Error, Result};

/// The `transportId` the fake app reports, and where the app's messages go.
pub const APP_TRANSPORT_ID: &str = "fake-transport";
/// The `sessionId` the fake app reports.
pub const APP_SESSION_ID: &str = "fake-session";
/// What the fake app calls itself.
pub const APP_DISPLAY_NAME: &str = "Jellyfin (fake)";

/// A message the fake device received, already parsed.
#[derive(Debug, Clone)]
pub struct FakeMessage {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: Value,
}

/// A device on loopback. Dropping it closes the connection and stops listening.
pub struct FakeDevice {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    control: Arc<Mutex<Option<mpsc::UnboundedSender<Control>>>>,
    shutdown: Option<oneshot::Sender<()>>,
    accept: tokio::task::JoinHandle<()>,
}

impl FakeDevice {
    pub async fn start() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config()?));
        let state = Arc::new(Mutex::new(State::default()));
        let control: Arc<Mutex<Option<mpsc::UnboundedSender<Control>>>> = Arc::default();
        let (shutdown, mut shutdown_rx) = oneshot::channel();

        let accept = tokio::spawn({
            let state = state.clone();
            let control = control.clone();
            async move {
                loop {
                    let accepted = tokio::select! {
                        _ = &mut shutdown_rx => break,
                        accepted = listener.accept() => accepted,
                    };
                    let Ok((tcp, _)) = accepted else { break };
                    // A client that fails the handshake is that client's problem, not the
                    // device's: keep listening.
                    let Ok(stream) = acceptor.accept(tcp).await else {
                        continue;
                    };
                    state.lock().unwrap().connections += 1;
                    tokio::spawn(handle_connection(stream, state.clone(), control.clone()));
                }
            }
        });

        Ok(Self {
            addr,
            state,
            control,
            shutdown: Some(shutdown),
            accept,
        })
    }

    /// Where the fake device listens.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Everything received, in order.
    pub fn messages(&self) -> Vec<FakeMessage> {
        self.state.lock().unwrap().messages.clone()
    }

    /// The parsed payloads of the messages received in the Jellyfin namespace.
    pub fn envelopes(&self) -> Vec<Value> {
        self.state.lock().unwrap().envelopes.clone()
    }

    /// PINGs received from the client.
    pub fn pings(&self) -> usize {
        self.state.lock().unwrap().pings
    }

    /// PONGs received from the client.
    pub fn pongs(&self) -> usize {
        self.state.lock().unwrap().pongs
    }

    /// How many clients connected (TLS completed).
    pub fn connections(&self) -> usize {
        self.state.lock().unwrap().connections
    }

    /// How many connections have ended, from either side.
    pub fn disconnects(&self) -> usize {
        self.state.lock().unwrap().disconnects
    }

    /// Whether the receiver app is running: launched, and neither stopped nor left by its sender
    /// with a `CLOSE`. It stays running while a sender only drops the connection, as a real
    /// receiver does, which is what a reconnect finds.
    pub fn is_running(&self) -> bool {
        self.state.lock().unwrap().running.is_some()
    }

    /// Sends a PING to the client, the way a device checks the sender is alive.
    pub fn send_ping(&self) {
        self.control(Control::Ping);
    }

    /// Sends `event` from the receiver app, as when something happens on the TV.
    pub fn send_app_event(&self, event: Value) {
        self.control(Control::AppEvent(event));
    }

    /// The device's own `MEDIA_STATUS` in the media namespace, as it broadcasts when a sender
    /// loads, pauses or seeks: what another sender casting to the TV looks like from here
    /// (docs/chromecast.md, *Staying connected*).
    pub fn send_media_status(&self, item_id: &str, position_seconds: f64, paused: bool) {
        self.control(Control::Media(json!({
            "type": "MEDIA_STATUS",
            "requestId": 0,
            "status": [{
                "currentTime": position_seconds,
                "playerState": if paused { "PAUSED" } else { "PLAYING" },
                "media": {
                    "duration": 1_440.,
                    "customData": {
                        "itemId": item_id,
                        "mediaSourceId": "another-source",
                        "audioStreamIndex": 1,
                        "subtitleStreamIndex": -1,
                        "canClientSeek": true
                    }
                }
            }]
        })));
    }

    /// Closes the current connection.
    pub fn close(&self) {
        self.control(Control::Close);
    }

    /// Takes the device off the network: it stops listening, and the current connection goes
    /// with it. A dropped connection it can't be reached on again is what gives up the cast.
    pub fn stop(&mut self) {
        if let Some(control) = self.control.lock().unwrap().as_ref() {
            let _ = control.send(Control::Close);
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.accept.abort();
    }

    /// Stops answering the client's PINGs, so the client's idle timeout can be tested.
    pub fn silence(&self) {
        self.state.lock().unwrap().answer_pings = false;
    }

    /// Delays the `LAUNCH` reply, so a test can see replies matched by `requestId` rather than by
    /// arrival order.
    pub fn set_launch_delay(&self, delay: Duration) {
        self.state.lock().unwrap().launch_delay = delay;
    }

    fn control(&self, command: Control) {
        if let Some(control) = self.control.lock().unwrap().as_ref() {
            let _ = control.send(command);
        }
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        if let Some(control) = self.control.lock().unwrap().as_ref() {
            let _ = control.send(Control::Close);
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.accept.abort();
    }
}

enum Control {
    Ping,
    AppEvent(Value),
    Media(Value),
    Close,
}

struct State {
    messages: Vec<FakeMessage>,
    envelopes: Vec<Value>,
    pings: usize,
    pongs: usize,
    connections: usize,
    disconnects: usize,
    /// The client's sender id, the destination of anything the device sends unprompted.
    sender_id: Option<String>,
    level: f32,
    muted: bool,
    answer_pings: bool,
    launch_delay: Duration,
    /// The app that's running, if any, and what it plays.
    running: Option<Running>,
}

/// The fake receiver app. It keeps what it plays so that an `Identify` after a reconnect answers
/// as a real receiver does: a `playbackstop`-typed message carrying the current state.
struct Running {
    app_id: String,
    playing: Option<Playing>,
}

struct Playing {
    id: String,
    name: String,
    position_ticks: i64,
    media_source_id: Value,
    audio: Value,
    subtitle: Value,
}

impl Default for State {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            envelopes: Vec::new(),
            pings: 0,
            pongs: 0,
            connections: 0,
            disconnects: 0,
            sender_id: None,
            level: 0.5,
            muted: false,
            answer_pings: true,
            launch_delay: Duration::ZERO,
            running: None,
        }
    }
}

fn server_config() -> Result<rustls::ServerConfig> {
    let certified = rcgen::generate_simple_self_signed(vec!["fake-device".to_string()])
        .map_err(|error| Error::Certificate(error.to_string()))?;
    let certificate = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()));
    // The ring provider, explicitly, as the client does: no aws-lc-rs anywhere.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)?;
    Ok(config)
}

async fn handle_connection(
    stream: tokio_rustls::server::TlsStream<TcpStream>,
    state: Arc<Mutex<State>>,
    control: Arc<Mutex<Option<mpsc::UnboundedSender<Control>>>>,
) {
    let (tx, mut commands) = mpsc::unbounded_channel();
    *control.lock().unwrap() = Some(tx);
    serve(stream, &state, &mut commands).await;
    *control.lock().unwrap() = None;
    state.lock().unwrap().disconnects += 1;
}

async fn serve(
    stream: tokio_rustls::server::TlsStream<TcpStream>,
    state: &Arc<Mutex<State>>,
    commands: &mut mpsc::UnboundedReceiver<Control>,
) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut outbox: Vec<(Instant, CastMessage)> = Vec::new();

    loop {
        let next = outbox.first().map(|(due, _)| *due);
        tokio::select! {
            frame = codec::read_frame(&mut reader) => {
                let Ok(frame) = frame else { break };
                for (due, message) in handle(frame, state) {
                    match due {
                        Some(due) => outbox.push((due, message)),
                        None => {
                            if codec::write_frame(&mut writer, &message).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                outbox.sort_by_key(|(due, _)| *due);
            }
            command = commands.recv() => match command {
                Some(Control::Ping) => {
                    let Some(ping) = device_ping(state) else { continue };
                    if codec::write_frame(&mut writer, &ping).await.is_err() {
                        break;
                    }
                }
                Some(Control::AppEvent(event)) => {
                    let Some(sender) = state.lock().unwrap().sender_id.clone() else { continue };
                    let message = CastMessage::json(APP_TRANSPORT_ID, &sender, APP_NAMESPACE, &event.to_string());
                    if codec::write_frame(&mut writer, &message).await.is_err() {
                        break;
                    }
                }
                Some(Control::Media(status)) => {
                    let Some(sender) = state.lock().unwrap().sender_id.clone() else { continue };
                    let message = CastMessage::json("receiver-0", &sender, NS_MEDIA, &status.to_string());
                    if codec::write_frame(&mut writer, &message).await.is_err() {
                        break;
                    }
                }
                Some(Control::Close) | None => break,
            },
            _ = sleep_until_due(next) => {
                let now = Instant::now();
                while outbox.first().is_some_and(|(due, _)| *due <= now) {
                    let (_, message) = outbox.remove(0);
                    if codec::write_frame(&mut writer, &message).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

async fn sleep_until_due(due: Option<Instant>) {
    match due {
        Some(due) => sleep_until(due).await,
        None => std::future::pending().await,
    }
}

fn device_ping(state: &Arc<Mutex<State>>) -> Option<CastMessage> {
    let state = state.lock().unwrap();
    let sender = state.sender_id.as_deref()?;
    Some(CastMessage::json(
        "receiver-0",
        sender,
        NS_HEARTBEAT,
        "{\"type\":\"PING\"}",
    ))
}

/// Answers one message. Replies with a due time are held until then.
fn handle(frame: CastMessage, state: &Arc<Mutex<State>>) -> Vec<(Option<Instant>, CastMessage)> {
    let payload: Value = frame
        .payload_utf8
        .as_deref()
        .and_then(|payload| serde_json::from_str(payload).ok())
        .unwrap_or(Value::Null);
    let sender = frame.source_id.clone();
    let mut replies = Vec::new();

    {
        let mut state = state.lock().unwrap();
        state.sender_id = Some(sender.clone());
        state.messages.push(FakeMessage {
            source: frame.source_id.clone(),
            destination: frame.destination_id.clone(),
            namespace: frame.namespace.clone(),
            payload: payload.clone(),
        });
        if frame.namespace == APP_NAMESPACE {
            state.envelopes.push(payload.clone());
        }

        let request_id = payload.get("requestId").cloned().unwrap_or(Value::Null);
        match frame.namespace.as_str() {
            NS_HEARTBEAT => match payload.get("type").and_then(Value::as_str) {
                Some("PING") => {
                    state.pings += 1;
                    if state.answer_pings {
                        replies.push((None, pong(&sender)));
                    }
                }
                Some("PONG") => state.pongs += 1,
                _ => {}
            },
            NS_RECEIVER => match payload.get("type").and_then(Value::as_str) {
                Some("GET_STATUS") => {
                    let status = status_of(&state);
                    replies.push((None, receiver_reply(&sender, "RECEIVER_STATUS", request_id, status)));
                }
                Some("LAUNCH") => {
                    let app_id = payload.get("appId").and_then(Value::as_str).unwrap_or_default();
                    let launched = app_id == STABLE_APP_ID || app_id == UNSTABLE_APP_ID;
                    let reply = if launched {
                        state.running = Some(Running {
                            app_id: app_id.to_string(),
                            playing: None,
                        });
                        let status = status_of(&state);
                        receiver_reply(&sender, "RECEIVER_STATUS", request_id, status)
                    } else {
                        CastMessage::json(
                            "receiver-0",
                            &sender,
                            NS_RECEIVER,
                            &json!({"type": "LAUNCH_ERROR", "requestId": request_id, "reason": "NOT_FOUND"})
                                .to_string(),
                        )
                    };
                    // A real launch takes a moment; delaying it makes reply matching observable.
                    let due = (state.launch_delay > Duration::ZERO).then(|| Instant::now() + state.launch_delay);
                    replies.push((due, reply));
                    if launched {
                        // A real TV follows a launch with a broadcast that has the volume but no
                        // `applications`: that isn't the app going away.
                        let volume = json!({"volume": {"level": state.level, "muted": state.muted}});
                        replies.push((due, receiver_reply(&sender, "RECEIVER_STATUS", json!(0), volume)));
                    }
                }
                Some("STOP") => {
                    state.running = None;
                    let status = status_of(&state);
                    replies.push((None, receiver_reply(&sender, "RECEIVER_STATUS", request_id, status)));
                }
                Some("SET_VOLUME") => {
                    let volume = &payload["volume"];
                    if let Some(level) = volume.get("level").and_then(Value::as_f64) {
                        state.level = level as f32;
                    }
                    if let Some(muted) = volume.get("muted").and_then(Value::as_bool) {
                        state.muted = muted;
                    }
                    let status = status_of(&state);
                    replies.push((None, receiver_reply(&sender, "RECEIVER_STATUS", request_id, status)));
                }
                _ => {}
            },
            // Jellyfin's receiver quits when its last sender leaves with a `CLOSE`, as a real one
            // does; a sender that only drops the connection leaves it playing.
            NS_CONNECTION
                if frame.destination_id == APP_TRANSPORT_ID
                    && payload.get("type").and_then(Value::as_str) == Some("CLOSE") =>
            {
                state.running = None;
            }
            APP_NAMESPACE => {
                replies.extend(app_events(&payload, &sender, &mut state));
            }
            _ => {}
        }
    }

    replies
}

/// What the fake app answers: `Identify` with the current state, `PlayNow` with a few progress
/// events. Anything else just gets recorded.
fn app_events(payload: &Value, sender: &str, state: &mut State) -> Vec<(Option<Instant>, CastMessage)> {
    let event = |body: Value| {
        let message = CastMessage::json(APP_TRANSPORT_ID, sender, APP_NAMESPACE, &body.to_string());
        (None, message)
    };
    match payload.get("command").and_then(Value::as_str) {
        // What a real receiver answers: `playbackstop`-typed with the state while something
        // plays, and a `volumechange` about an item with empty ids when it's idle (seen on a
        // Chromecast). A reconnect's `Identify` is how the state comes back.
        Some("Identify") => match state.running.as_ref().and_then(|running| running.playing.as_ref()) {
            Some(playing) => vec![event(json!({
                "type": "playbackstop",
                "data": {
                    "ItemId": playing.id,
                    "NowPlayingItem": {
                        "Id": playing.id,
                        "Name": playing.name,
                        "Type": "Episode",
                        "RunTimeTicks": 1_440_000_000i64,
                        "MediaType": "Video"
                    },
                    "PlayState": {
                        "PositionTicks": playing.position_ticks,
                        "IsPaused": false,
                        "IsMuted": state.muted,
                        "VolumeLevel": (state.level * 100.0).round(),
                        "AudioStreamIndex": playing.audio,
                        "SubtitleStreamIndex": playing.subtitle,
                        "MediaSourceId": playing.media_source_id,
                        "PlayMethod": "Transcode",
                        "CanSeek": true
                    }
                }
            }))],
            None => vec![event(json!({
                "type": "volumechange",
                "data": {
                    "ItemId": "",
                    "NowPlayingItem": {"Id": "", "RunTimeTicks": 0},
                    "PlayState": {
                        "PositionTicks": 0,
                        "IsPaused": false,
                        "IsMuted": state.muted,
                        "VolumeLevel": (state.level * 100.0).round(),
                        "CanSeek": false,
                        "ItemId": "",
                        "MediaSourceId": "",
                        "PlaySessionId": "",
                        "RepeatMode": "RepeatNone"
                    }
                }
            }))],
        },
        // A real receiver reports the stop of what played (here, nothing in particular), and
        // keeps running.
        Some("Stop") => {
            if let Some(running) = &mut state.running {
                running.playing = None;
            }
            vec![event(json!({
                "type": "playbackstop",
                "data": {
                    "ItemId": "",
                    "NowPlayingItem": {"Id": ""},
                    "PlayState": {"PositionTicks": 0, "IsPaused": false}
                }
            }))]
        }
        Some("PlayNow") => {
            let options = &payload["options"];
            let item_id = options["items"][0]
                .get("Id")
                .and_then(Value::as_str)
                .unwrap_or("unknown-item");
            let media_source_id = options.get("mediaSourceId").cloned().unwrap_or(Value::Null);
            let audio = options.get("audioStreamIndex").cloned().unwrap_or(Value::Null);
            let subtitle = options.get("subtitleStreamIndex").cloned().unwrap_or(json!(-1));
            if let Some(running) = &mut state.running {
                running.playing = Some(Playing {
                    id: item_id.to_owned(),
                    name: "Fake item".into(),
                    // Where the scripted progress leaves it (2 s), which a later `Identify`
                    // reports.
                    position_ticks: 20_000_000,
                    media_source_id: media_source_id.clone(),
                    audio: audio.clone(),
                    subtitle: subtitle.clone(),
                });
            }
            (0..3)
                .map(|step| {
                    event(json!({
                        "type": "playbackprogress",
                        "data": {
                            "ItemId": item_id,
                            "PlayState": {
                                "PositionTicks": step * 10_000_000,
                                "IsPaused": false,
                                "IsMuted": state.muted,
                                "VolumeLevel": (state.level * 100.0).round(),
                                "AudioStreamIndex": audio,
                                "SubtitleStreamIndex": subtitle,
                                "MediaSourceId": media_source_id,
                                "PlayMethod": "Transcode",
                                "CanSeek": true
                            },
                            "NowPlayingItem": {
                                "Id": item_id,
                                "Name": "Fake item",
                                "Type": "Episode",
                                "RunTimeTicks": 1_440_000_000i64,
                                "MediaType": "Video"
                            }
                        }
                    }))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn status_of(state: &State) -> Value {
    let applications = match &state.running {
        Some(running) => json!([{
            "appId": running.app_id,
            "displayName": APP_DISPLAY_NAME,
            "namespaces": [
                {"name": APP_NAMESPACE},
                {"name": crate::channel::NS_MEDIA}
            ],
            "sessionId": APP_SESSION_ID,
            "statusText": "Ready To Cast",
            "transportId": APP_TRANSPORT_ID
        }]),
        None => json!([]),
    };
    json!({
        "applications": applications,
        "volume": {"level": state.level, "muted": state.muted}
    })
}

fn receiver_reply(sender: &str, kind: &str, request_id: Value, status: Value) -> CastMessage {
    CastMessage::json(
        "receiver-0",
        sender,
        NS_RECEIVER,
        &json!({"type": kind, "requestId": request_id, "status": status}).to_string(),
    )
}

fn pong(sender: &str) -> CastMessage {
    CastMessage::json("receiver-0", sender, NS_HEARTBEAT, &json!({"type": "PONG"}).to_string())
}
