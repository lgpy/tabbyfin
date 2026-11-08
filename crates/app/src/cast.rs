//! Casting to a Chromecast or Google TV running Jellyfin's own receiver app (docs/chromecast.md).
//!
//! The receiver does the Jellyfin work itself: it negotiates the stream, reports progress to
//! the server with our token and plays the next episode. This app is only its remote: it finds
//! devices, launches the receiver, sends it commands and draws what it reports.
//!
//! [`Cast`] (`services.cast`) is the part that needs nothing else: the devices found, and the
//! [`CastSession`] while connected, whose connection runs on tokio ([`drive`]) and reports
//! through a [`task::channel`] drained in [`Services::tick`]. What involves playback is on
//! [`Services`], below: while casting, `play`, `enqueue`, `control` and the rest go to the TV
//! instead of the local player, the way a SyncPlay group takes them over with `in_group()`.
//!
//! The receiver can only be driven over the cast channel: it opens no websocket, so the
//! server's remote control can't reach it.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use cast::jellyfin::{self as receiver_app, Command, Credentials, ItemStub, MediaStatus, PlayRequest, ReceiverEvent};
use cast::{Channel, ChannelConfig, Device, DiscoveryEvent, NS_MEDIA, NS_RECEIVER, ReceiverStatus, receiver};
use jellyfin::models::{BaseItemDto, ItemField};
use jellyfin::{Client, Ticks};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::playback::{PlayOptions, SubtitleChoice, UiCommand};
use crate::runtime;
use crate::services::{AppEvent, Services};
use crate::session::Session;
use crate::task::{self, Deadline, Task};
use crate::toasts::Toast;

/// How long after a `playbackstop` with nothing new playing the player closes. The receiver
/// stops one item before it starts the next, so the gap between episodes mustn't close it.
const STOPPED_FOR: Duration = Duration::from_secs(5);
/// How long a reload waits after the receiver reported its stop, for the receiver's own stop
/// handling (which can show its idle screen) to be over before the new `PlayNow`.
const RELOAD_GAP: Duration = Duration::from_secs(1);
/// How long a reload waits for the receiver to report its stop before going ahead anyway.
const RELOAD_STOP_WAIT: Duration = Duration::from_secs(4);

/// A reload between its `Stop` and its `PlayNow`.
#[derive(Debug)]
struct Reload {
    request: PlayRequest,
    /// The receiver reported the stop; `due` is then the end of [`RELOAD_GAP`].
    stopped: bool,
    due: Deadline,
}
/// How long the custom messages may go quiet before `MEDIA_STATUS` is followed instead: another
/// sender cast to the TV, and the receiver now answers that one only. Tests use a short one, so
/// they needn't wait seconds for the switch.
fn follow_media_after() -> Duration {
    if cfg!(test) {
        Duration::from_millis(300)
    } else {
        const FOLLOW_MEDIA_AFTER: Duration = Duration::from_secs(3);
        FOLLOW_MEDIA_AFTER
    }
}
/// How long quitting waits for the TV to hear that we're leaving.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// How a dropped connection is retried: `GET_STATUS`, and if the receiver still runs, `CONNECT`
/// and `Identify`, which answers with what it plays (docs/chromecast.md, *Staying connected*).
#[derive(Debug, Clone)]
pub struct Reconnect {
    /// The wait before the first attempt; each later one doubles it.
    pub initial: Duration,
    /// The longest wait between attempts.
    pub max: Duration,
    /// How many attempts before the cast is given up. Stopping or disconnecting ends it sooner.
    pub attempts: u32,
}

impl Default for Reconnect {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(16),
            attempts: 5,
        }
    }
}

impl Reconnect {
    /// The wait before attempt `attempt`, from 1: 1 s, 2 s, 4 s… up to `max`.
    fn delay(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(8);
        (self.initial * (1 << doublings)).min(self.max)
    }
}

/// The policy the app runs with. Tests retry in milliseconds, so giving up is quick enough to
/// watch.
fn reconnect_policy() -> Reconnect {
    if cfg!(test) {
        Reconnect {
            initial: Duration::from_millis(250),
            max: Duration::from_millis(500),
            attempts: 3,
        }
    } else {
        Reconnect::default()
    }
}

/// The cast part of [`Services`]: the devices on the network and the cast in progress.
pub struct Cast {
    ctx: egui::Context,
    client: Client,
    /// Whether the session is online (`Services::online`): an offline one can't discover or
    /// cast, since the receiver would need the server.
    online: bool,
    /// Devices found on this network, the one cast to last first.
    pub devices: Vec<Device>,
    /// The mDNS browse, while signed in. Dropping it stops discovering.
    discovery: Option<cast::Discovery>,
    found: task::Receiver<DiscoveryEvent>,
    found_from: task::Sender<DiscoveryEvent>,
    /// The device cast to last ([`Config::cast_device`]).
    pub last_device: Option<String>,
    /// The cast in progress, from connecting until it ends.
    pub session: Option<CastSession>,
    /// The receiver app to launch: the user's choice on the server, or Jellyfin's stable one.
    app_id: String,
    /// What every message to the receiver carries, but its name (the device's).
    credentials: Credentials,
    /// How a dropped connection is retried.
    reconnect: Reconnect,
}

/// A cast in progress: the device, the connection and what it reports.
pub struct CastSession {
    pub device: Device,
    /// The connection's commands, in the order they're made.
    commands: mpsc::UnboundedSender<Outgoing>,
    updates: task::Receiver<Update>,
    /// Signalled once the connection has ended, for quitting to wait on.
    done: std::sync::mpsc::Receiver<()>,
    /// The receiver is launched and listening.
    pub connected: bool,
    /// What the TV plays, as it reported last.
    pub casting: Casting,
    /// What was playing here, and its request, handed over once the local session has
    /// reported its stop.
    handover: Option<(BaseItemDto, PlayRequest)>,
    /// `Identify`'s answer carries the current state, and can be a `playbackstop` (an idle
    /// Chromecast sends a `volumechange` with empty ids): the first one after connecting doesn't
    /// mean playback stopped.
    identifying: bool,
    /// The receiver stopped and nothing new started yet; closes the player when it fires.
    stopped: Deadline,
    /// Something was played from here and hasn't started on the TV yet. The receiver stops
    /// what it played first, and loading the new one (a reload for other tracks, a transcode)
    /// can take longer than [`STOPPED_FOR`]: that stop mustn't close the player.
    pub(crate) starting: bool,
    /// A reload waiting for the receiver to finish stopping (see `Services::cast_reload`).
    reload: Option<Reload>,
    /// The connection dropped and is being retried: which attempt is next, from 1. The player
    /// and the now-casting bar say so while it's set.
    pub reconnecting: Option<u32>,
    /// When the receiver's own messages last came, or ours last went out (a command makes it
    /// answer us again), to tell when another sender took over.
    pub(crate) last_event: Option<Instant>,
    /// The last standard `MEDIA_STATUS`, and when it came: kept so the switch to following
    /// another sender happens even if no newer one arrives after the custom messages go quiet.
    media: Option<(Instant, MediaStatus)>,
    /// The standard status is being followed: the receiver answers another sender now.
    following: bool,
    /// The playing item's details (versions, chapters, images), fetched as it changes.
    fetching: Option<(String, Task<jellyfin::Result<BaseItemDto>>)>,
}

/// What the TV is playing, for the player page and the now-casting bar.
#[derive(Debug, Clone, Default)]
pub struct Casting {
    /// The item the receiver reported last; `None` until it plays something.
    pub item_id: Option<String>,
    /// Its name and series, as the receiver reports them (before `item` arrives).
    pub name: String,
    pub series_name: Option<String>,
    /// The item from the server, with its versions and chapters, once fetched.
    pub item: Option<BaseItemDto>,
    pub position: Duration,
    /// When `position` was reported, to extrapolate between the receiver's reports.
    pub position_at: Option<Instant>,
    pub duration: Option<Duration>,
    pub paused: bool,
    /// Something plays: between a `playbackstart` and its `playbackstop`.
    pub playing: bool,
    pub can_seek: bool,
    pub media_source_id: Option<String>,
    pub audio: Option<i32>,
    /// The subtitle stream; `-1` is off.
    pub subtitle: Option<i32>,
    /// The TV's volume, 0 to 100.
    pub volume: f64,
    pub muted: bool,
}

impl Casting {
    /// The position now, extrapolated from the last report while playing.
    pub fn position(&self) -> Duration {
        match self.position_at {
            Some(at) if self.playing && !self.paused => {
                let position = self.position + at.elapsed();
                self.duration.map_or(position, |duration| position.min(duration))
            }
            _ => self.position,
        }
    }

    /// The subtitle choice as the player's menus show it.
    pub fn subtitle_choice(&self) -> SubtitleChoice {
        match self.subtitle {
            Some(index) if index >= 0 => SubtitleChoice::Stream(index),
            Some(_) => SubtitleChoice::Off,
            None => SubtitleChoice::Auto,
        }
    }

    /// Takes in what the receiver reported. Returns whether the item changed.
    fn apply(&mut self, event: &receiver_app::Event) -> bool {
        let data = &event.data;
        let mut changed = false;
        if let Some(item) = &data.now_playing_item
            && let Some(id) = reported_item(event)
        {
            if self.item_id.as_deref() != Some(id) {
                self.follow_item(id.to_owned());
                changed = true;
            }
            if let Some(name) = &item.name {
                self.name = name.clone();
            }
            self.series_name = item.series_name.clone();
            if let Some(ticks) = item.run_time_ticks {
                self.duration = Some(Ticks(ticks).into());
            }
        }
        if let Some(state) = &data.play_state {
            if let Some(ticks) = state.position_ticks {
                self.position = Ticks(ticks.max(0)).into();
                self.position_at = Some(Instant::now());
            }
            if let Some(paused) = state.is_paused {
                self.paused = paused;
            }
            if let Some(can_seek) = state.can_seek {
                self.can_seek = can_seek;
            }
            if state.media_source_id.is_some() {
                self.media_source_id = state.media_source_id.clone();
            }
            self.audio = state.audio_stream_index.or(self.audio);
            self.subtitle = state.subtitle_stream_index.or(self.subtitle);
        }
        changed
    }

    /// Follows the standard media status, for when another sender gets the receiver's own
    /// messages. Returns whether the item changed.
    fn apply_media(&mut self, status: &MediaStatus) -> bool {
        let Some(entry) = status.entry() else {
            return false;
        };
        let mut changed = false;
        if let Some(id) = entry.item_id()
            && self.item_id.as_deref() != Some(id)
        {
            self.follow_item(id.to_owned());
            changed = true;
        }
        if let Some(seconds) = entry.position_seconds() {
            self.position = Duration::from_secs_f64(seconds.max(0.));
            self.position_at = Some(Instant::now());
        }
        if let Some(duration) = entry.media.as_ref().and_then(|media| media.duration) {
            self.duration = Some(Duration::from_secs_f64(duration.max(0.)));
        }
        match entry.player_state.as_deref() {
            Some("PAUSED") => self.paused = true,
            Some("PLAYING" | "BUFFERING") => self.paused = false,
            _ => {}
        }
        self.playing = !matches!(entry.player_state.as_deref(), Some("IDLE"));
        if let Some(data) = entry.media.as_ref().and_then(|media| media.custom_data.as_ref()) {
            self.media_source_id = data.media_source_id.clone().or(self.media_source_id.take());
            self.audio = data.audio_stream_index.or(self.audio);
            self.subtitle = data.subtitle_stream_index.or(self.subtitle);
            if let Some(can_seek) = data.can_client_seek {
                self.can_seek = can_seek;
            }
        }
        changed
    }

    /// The receiver plays `id` now. The item held stays if it's that one (the one just played
    /// from here), until the server's copy with its versions and chapters replaces it.
    fn follow_item(&mut self, id: String) {
        if self.item.as_ref().is_some_and(|item| item.id != id) {
            self.item = None;
        }
        self.item_id = Some(id);
    }

    fn apply_volume(&mut self, status: &ReceiverStatus) {
        if let Some(volume) = &status.volume {
            if let Some(level) = volume.level {
                self.volume = f64::from(level * 100.).clamp(0., 100.);
            }
            if let Some(muted) = volume.muted {
                self.muted = muted;
            }
        }
    }
}

impl Cast {
    /// The cast part of `session`'s services. `online` decides whether devices are looked for:
    /// offline there's no server for the receiver to play from.
    pub fn new(session: &Session, ctx: &egui::Context, online: bool) -> Self {
        let (found_from, found) = task::channel(ctx);
        let config = Config::load();
        let client = session.client.clone();
        let app_id = session
            .user
            .configuration
            .as_ref()
            .and_then(|config| config.cast_receiver_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| receiver_app::STABLE_APP_ID.to_owned());
        let credentials = Credentials {
            server_address: server_address(client.base_url(), session.server.local_address.as_deref()),
            access_token: client.token().unwrap_or_default().to_owned(),
            user_id: session.user.id.clone(),
            device_id: client.info().device_id.clone(),
            server_id: session.server.id.clone().unwrap_or_default(),
            server_version: session.server.version.clone().unwrap_or_default(),
            receiver_name: String::new(),
            max_bitrate: config.cast_max_bitrate,
            subtitle_appearance: None,
        };
        let mut cast = Self {
            ctx: ctx.clone(),
            client,
            online,
            devices: Vec::new(),
            discovery: None,
            found,
            found_from,
            last_device: config.cast_device,
            session: None,
            app_id,
            credentials,
            reconnect: reconnect_policy(),
        };
        // The UI tests have no network beyond loopback; they add their devices by hand.
        if online && !cfg!(test) {
            cast.discover();
        }
        cast
    }

    /// Whether this session can find devices and cast: an offline one can't, since the receiver
    /// plays from the server.
    pub fn is_online(&self) -> bool {
        self.online
    }

    /// Starts browsing for devices, forwarding what's found to the UI.
    fn discover(&mut self) {
        let (discovery, mut events) = match cast::Discovery::start() {
            Ok(started) => started,
            Err(err) => {
                tracing::warn!("cast: discovery didn't start: {err}");
                return;
            }
        };
        self.discovery = Some(discovery);
        let found = self.found_from.clone();
        runtime::detach(async move {
            while let Some(event) = events.recv().await {
                if found.send(event).is_err() {
                    break;
                }
            }
        });
    }

    /// Adds a device as if it had been found, for tests against the fake device.
    #[cfg(test)]
    pub(crate) fn add_device(&mut self, device: Device) {
        let _ = self.found_from.send(DiscoveryEvent::Found(device));
    }

    /// Whether a cast is in progress (connecting counts).
    pub fn is_casting(&self) -> bool {
        self.session.is_some()
    }

    /// What the TV plays, while casting.
    pub fn casting(&self) -> Option<&Casting> {
        self.session.as_ref().map(|session| &session.casting)
    }

    /// The cast button shows once there's something to cast to, or a cast to control.
    /// Whether what played here is waiting to go to the TV (see `Services::start_cast`).
    pub fn is_handing_over(&self) -> bool {
        self.session.as_ref().is_some_and(|session| session.handover.is_some())
    }

    pub fn has_devices(&self) -> bool {
        !self.devices.is_empty() || self.session.is_some()
    }

    /// Sets the bitrate sent to the receiver from the next message on (the settings page).
    pub fn set_max_bitrate(&mut self, bitrate: Option<u64>) {
        self.credentials.max_bitrate = bitrate;
        if let Some(session) = &self.session {
            // The receiver keeps a bitrate once it's sent; a no-op `Identify` carries the new one.
            let _ = session
                .commands
                .send(Outgoing::Credentials(self.credentials_for(&session.device)));
        }
    }

    fn credentials_for(&self, device: &Device) -> Credentials {
        Credentials {
            receiver_name: device.name.clone(),
            ..self.credentials.clone()
        }
    }

    /// Connects to `device` and launches the receiver; `handover` is played once it's up.
    fn connect(&mut self, device: Device, handover: Option<(BaseItemDto, PlayRequest)>) {
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates) = task::channel(&self.ctx);
        let (done_tx, done) = std::sync::mpsc::channel();
        let target = Target {
            addr: device.socket_addr(),
            app_id: self.app_id.clone(),
            credentials: self.credentials_for(&device),
            config: ChannelConfig::default(),
            reconnect: self.reconnect.clone(),
        };
        // Not a `Task`: leaving or stopping the receiver has to reach the TV after
        // the services that started it are gone.
        runtime::detach(async move {
            let end = run(target, commands_rx, &updates_tx).await;
            tracing::info!(?end, "cast: ended");
            let _ = updates_tx.send(Update::Ended(end));
            let _ = done_tx.send(());
        });
        self.remember(&device);
        self.session = Some(CastSession {
            device,
            commands,
            updates,
            done,
            connected: false,
            casting: Casting {
                volume: 100.,
                ..Casting::default()
            },
            handover,
            identifying: true,
            stopped: Deadline::default(),
            starting: false,
            reload: None,
            reconnecting: None,
            last_event: None,
            media: None,
            following: false,
            fetching: None,
        });
    }

    /// Remembers `device` as the one cast to last, and lists it first.
    fn remember(&mut self, device: &Device) {
        self.last_device = Some(device.id.clone());
        sort_devices(&mut self.devices, self.last_device.as_deref());
        let mut config = Config::load();
        if config.cast_device.as_ref() != Some(&device.id) {
            config.cast_device = Some(device.id.clone());
            if let Err(err) = config.save() {
                tracing::warn!("failed to save config: {err:#}");
            }
        }
    }

    /// Sends `command` to the receiver, once it's connected.
    fn send(&mut self, command: Command) {
        self.send_outgoing(Outgoing::Command(command));
    }

    fn send_outgoing(&mut self, outgoing: Outgoing) {
        if let Some(session) = &mut self.session {
            // The receiver answers the sender of the last custom message, so this counts as
            // our own traffic: a `MEDIA_STATUS` in hand is older than what we just asked for.
            session.last_event = Some(Instant::now());
            session.following = false;
            session.media = None;
            let _ = session.commands.send(outgoing);
        }
    }

    /// Takes in what discovery found since the last frame.
    fn tick_discovery(&mut self) {
        let mut changed = false;
        for event in self.found.drain() {
            changed = true;
            match event {
                DiscoveryEvent::Found(device) | DiscoveryEvent::Updated(device) => {
                    match self.devices.iter_mut().find(|known| known.id == device.id) {
                        Some(known) => *known = device,
                        None => self.devices.push(device),
                    }
                }
                DiscoveryEvent::Lost(id) => self.devices.retain(|device| device.id != id),
            }
        }
        if changed {
            sort_devices(&mut self.devices, self.last_device.as_deref());
        }
    }

    /// Quitting the app: leaves the receiver playing, as the web client does. Returns what to
    /// wait on for the TV to hear it.
    pub(crate) fn disconnect_for_exit(&mut self) -> Option<std::sync::mpsc::Receiver<()>> {
        let session = self.session.take()?;
        let _ = session.commands.send(Outgoing::Disconnect);
        Some(session.done)
    }
}

/// The item an event is about. The receiver sends empty ids when nothing plays (its answer to
/// `Identify` on an idle TV is a `volumechange` like that), which isn't an item.
fn reported_item(event: &receiver_app::Event) -> Option<&str> {
    let data = &event.data;
    let item = data.now_playing_item.as_ref()?;
    item.id
        .as_deref()
        .filter(|id| !id.is_empty())
        .or_else(|| data.item_id.as_deref().filter(|id| !id.is_empty()))
}

/// The last device first, then by name.
fn sort_devices(devices: &mut [Device], last: Option<&str>) {
    devices.sort_by(|a, b| {
        let last_a = Some(a.id.as_str()) == last;
        let last_b = Some(b.id.as_str()) == last;
        last_b
            .cmp(&last_a)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// The server address to give the TV. A loopback address means this computer to the TV, so the
/// server's own LAN address (`/System/Info/Public`'s `LocalAddress`) goes instead, as the web
/// client does.
fn server_address(base: &url::Url, local_address: Option<&str>) -> String {
    let loopback = match base.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    let address = match local_address.filter(|address| !address.is_empty()) {
        Some(local) if loopback => local.to_owned(),
        _ => base.as_str().to_owned(),
    };
    address.trim_end_matches('/').to_owned()
}

/// An item as the receiver gets it: it fetches the rest itself.
fn stub(item: &BaseItemDto, server_id: &str) -> ItemStub {
    ItemStub {
        id: item.id.clone(),
        server_id: item.server_id.clone().unwrap_or_else(|| server_id.to_owned()),
        name: item.name.clone(),
        kind: item.kind.as_str().unwrap_or("Video").to_owned(),
        // The app plays video only.
        media_type: "Video".to_owned(),
        is_folder: item.is_folder,
    }
}

/// `PlayNow` for `items`, the first one played with `options`' position, version and tracks.
fn play_request(items: &[BaseItemDto], options: &PlayOptions, server_id: &str) -> PlayRequest {
    PlayRequest {
        items: items.iter().map(|item| stub(item, server_id)).collect(),
        start_index: None,
        start_position_ticks: options.start.map(|ticks| ticks.0).filter(|&ticks| ticks > 0),
        media_source_id: options.media_source_id.clone(),
        audio_stream_index: options.audio_stream_index,
        subtitle_stream_index: match options.subtitle {
            SubtitleChoice::Auto => None,
            SubtitleChoice::Off => Some(-1),
            SubtitleChoice::Stream(index) => Some(index),
        },
    }
}

/// Room the `PlayNow` envelope leaves in a frame for its addressing (sender, destination,
/// namespace) and the protobuf around it.
const FRAME_SLACK: usize = 1024;

/// Shortens the queue's tail until the `PlayNow` fits in one frame ([`cast::MAX_FRAME`]): a
/// bigger one can't be sent (a 400-episode queue was 67 KB). What plays first always stays.
fn fit_play_request(request: &mut PlayRequest, credentials: &Credentials) {
    let size = |request: &PlayRequest| {
        receiver_app::Envelope::new(credentials, Command::PlayNow(request.clone()))
            .to_value()
            .to_string()
            .len()
    };
    let budget = cast::MAX_FRAME - FRAME_SLACK;
    let total = size(request);
    if total <= budget {
        return;
    }
    let queued = request.items.len();
    // A first guess from the average item, then exact.
    let keep = (budget * queued / total).clamp(1, queued);
    request.items.truncate(keep);
    while request.items.len() > 1 && size(request) > budget {
        request.items.pop();
    }
    tracing::info!(
        queued,
        sent = request.items.len(),
        "cast: the queue is longer than one message holds"
    );
}

/// What a player control becomes on the receiver. `None` for what it has no control for (speed,
/// delays, subtitle size). Volume goes to the TV instead (`SET_VOLUME`), see [`Outgoing`].
fn outgoing_for(command: &UiCommand, casting: &Casting) -> Option<Outgoing> {
    let seek = |position: Duration| {
        let position = casting.duration.map_or(position, |duration| position.min(duration));
        Outgoing::Command(Command::Seek {
            position_seconds: position.as_secs_f64(),
        })
    };
    Some(match *command {
        UiCommand::TogglePause => Outgoing::Command(Command::PlayPause),
        UiCommand::SetPause(true) => Outgoing::Command(Command::Pause),
        UiCommand::SetPause(false) => Outgoing::Command(Command::Unpause),
        UiCommand::SeekTo(position) => seek(position),
        UiCommand::SeekBy(seconds) => seek(Duration::from_secs_f64(
            (casting.position().as_secs_f64() + seconds).max(0.),
        )),
        UiCommand::SetVolume(percent) => Outgoing::Volume((percent.clamp(0., 100.) / 100.) as f32),
        UiCommand::SetMute(muted) => Outgoing::Muted(muted),
        UiCommand::ToggleMute => Outgoing::Muted(!casting.muted),
        UiCommand::Stop => Outgoing::Command(Command::Stop),
        // Tracks are a reload (`Services::cast_reload`), not a receiver command.
        UiCommand::Audio(_)
        | UiCommand::Subtitle(_)
        | UiCommand::SetSpeed(_)
        | UiCommand::SetSubtitleDelay(_)
        | UiCommand::SetAudioDelay(_)
        | UiCommand::SetSubtitleScale(_) => return None,
    })
}

/// Casting, where it involves playback: while a cast is in progress, playing and the player's
/// controls go to the TV.
impl Services {
    /// Whether a cast is in progress, which then takes what's played and the controls.
    pub fn is_casting(&self) -> bool {
        self.cast.is_casting()
    }

    /// Casts to the device with this id. Something playing here is handed over: it stops here
    /// and plays on the TV from the same spot, with the same version, tracks and queue.
    pub fn start_cast(&mut self, device_id: &str) {
        // Offline there's no server for the TV to play from, and the cast panel isn't offered.
        if !self.cast.is_online() {
            return;
        }
        if self.in_group() {
            self.toast(Toast::info("Leave the SyncPlay group to cast to a TV."));
            return;
        }
        let Some(device) = self.cast.devices.iter().find(|device| device.id == device_id).cloned() else {
            return;
        };
        if let Some(session) = &self.cast.session {
            if session.device.id == device.id {
                return;
            }
            // Another TV: leave the first one playing what it plays.
            self.disconnect_cast();
        }
        let server_id = self.cast.credentials.server_id.clone();
        let handover = self.playback.now_playing().map(|state| {
            let mut items = vec![state.item.clone()];
            items.extend(self.playback.queued().cloned());
            let options = PlayOptions {
                start: Some(Ticks::from(state.position)),
                media_source_id: Some(state.source.id.clone()),
                audio_stream_index: state.audio,
                subtitle: state.subtitle,
                ..Default::default()
            };
            let mut request = play_request(&items, &options, &server_id);
            fit_play_request(&mut request, &self.cast.credentials);
            (state.item.clone(), request)
        });
        if handover.is_some() {
            // The receiver plays it once the local session has reported its stop (`tick_cast`).
            self.stop_playback();
            self.playback.clear_queue();
        }
        tracing::info!(device = %device.name, handover = handover.is_some(), "cast: connecting");
        let handed = handover.as_ref().map(|(item, _)| item.clone());
        self.cast.connect(device, handover);
        // The player stays up through the handover, showing what goes to the TV meanwhile.
        if let Some(item) = handed
            && let Some(session) = &mut self.cast.session
        {
            session.casting.name = item.name.clone();
            session.casting.series_name = item.series_name.clone();
            session.casting.duration = item.run_time_ticks.map(Duration::from);
            session.casting.item = Some(item);
            session.starting = true;
        }
    }

    /// Stops the receiver: the TV goes back to its idle screen.
    pub fn stop_cast(&mut self) {
        if let Some(session) = self.cast.session.take() {
            let _ = session.commands.send(Outgoing::Stop);
            self.cast_closed();
        }
    }

    /// Leaves the receiver playing and stops controlling it.
    pub fn disconnect_cast(&mut self) {
        if let Some(session) = self.cast.session.take() {
            let _ = session.commands.send(Outgoing::Disconnect);
            self.cast_closed();
        }
    }

    /// The cast ended here: the player closes, unless it's playing something of its own.
    fn cast_closed(&mut self) {
        if !self.playback.is_active() {
            self.events.push(AppEvent::PlaybackEnded);
        }
    }

    /// Plays `items` on the TV, the first with `options`, and opens the player.
    pub(crate) fn cast_play(&mut self, items: &[BaseItemDto], options: &PlayOptions) {
        if let Some(request) = self.begin_cast_play(items, options) {
            self.cast.send(Command::PlayNow(request));
        }
    }

    /// What playing `items` on the TV shows at once, before the receiver reports anything; the
    /// request to send for it.
    fn begin_cast_play(&mut self, items: &[BaseItemDto], options: &PlayOptions) -> Option<PlayRequest> {
        if items.is_empty() {
            return None;
        }
        let mut request = play_request(items, options, &self.cast.credentials.server_id);
        fit_play_request(&mut request, &self.cast.credentials);
        if let Some(session) = &mut self.cast.session {
            // What the TV played before is gone; the player waits for the new item's report.
            session.casting.item_id = None;
            session.casting.name = items[0].name.clone();
            session.casting.series_name = items[0].series_name.clone();
            session.casting.item = Some(items[0].clone());
            session.casting.position = options.start.map(Duration::from).unwrap_or_default();
            session.casting.position_at = None;
            session.casting.duration = items[0].run_time_ticks.map(Duration::from);
            session.stopped.cancel();
            session.starting = true;
            // Something new replaces a reload still waiting.
            session.reload = None;
        }
        self.events.push(AppEvent::PlaybackStarted);
        Some(request)
    }

    /// Adds `items` to the TV's queue. The receiver puts *Play next* at the end too.
    pub(crate) fn cast_enqueue(&mut self, items: &[BaseItemDto], next: bool) {
        let nothing_plays = self.cast.casting().is_none_or(|casting| casting.item_id.is_none());
        if nothing_plays {
            self.cast_play(items, &PlayOptions::default());
            return;
        }
        let server_id = self.cast.credentials.server_id.clone();
        let items = items.iter().map(|item| stub(item, &server_id)).collect();
        self.cast.send(if next {
            Command::PlayNext { items }
        } else {
            Command::PlayLast { items }
        });
    }

    /// A player control (or remote control) while casting.
    pub(crate) fn cast_control(&mut self, command: UiCommand) {
        // Other tracks play the item again (see `cast_reload`).
        match command {
            UiCommand::Audio(index) => {
                return self.cast_reload(PlayOptions {
                    audio_stream_index: Some(index),
                    ..Default::default()
                });
            }
            UiCommand::Subtitle(choice @ (SubtitleChoice::Off | SubtitleChoice::Stream(_))) => {
                return self.cast_reload(PlayOptions {
                    subtitle: choice,
                    ..Default::default()
                });
            }
            _ => {}
        }
        let Some(casting) = self.cast.casting() else {
            return;
        };
        let Some(outgoing) = outgoing_for(&command, casting) else {
            return;
        };
        // Shown at once rather than after the receiver's next report, as mpv's own controls do.
        if let Some(session) = &mut self.cast.session {
            let casting = &mut session.casting;
            match command {
                UiCommand::SeekTo(_) | UiCommand::SeekBy(_) => {
                    if let Outgoing::Command(Command::Seek { position_seconds }) = &outgoing {
                        casting.position = Duration::from_secs_f64(*position_seconds);
                        casting.position_at = Some(Instant::now());
                    }
                }
                UiCommand::SetVolume(percent) => casting.volume = percent.clamp(0., 100.),
                UiCommand::SetMute(muted) => casting.muted = muted,
                UiCommand::ToggleMute => casting.muted = !casting.muted,
                _ => {}
            }
        }
        self.cast.send_outgoing(outgoing);
    }

    /// The TV's next or previous item.
    pub(crate) fn cast_skip(&mut self, next: bool) {
        self.cast.send(if next {
            Command::NextTrack
        } else {
            Command::PreviousTrack
        });
    }

    /// Plays the casting item again from where it is: in another version, or with other tracks.
    ///
    /// Tracks change this way rather than with the receiver's `SetAudioStreamIndex` and
    /// `SetSubtitleStreamIndex`: when those reload the stream the receiver asks the server for a
    /// fractional start position and gets no stream back (`NoCompatibleStream`, seen on a
    /// Chromecast), and it never reloads to leave burned-in subtitles.
    ///
    /// Nor is it a `PlayNow` over what plays: the receiver's handling of the stop that interrupts
    /// can land after the new stream started and put its idle screen (*Ready to cast*) over it
    /// for good, while it plays on underneath (seen on a Chromecast, with Jellyfin's Android app
    /// too). So it's a `Stop`, then once the receiver has reported that stop and had
    /// [`RELOAD_GAP`] to settle, the `PlayNow`: the same path as casting from idle.
    pub(crate) fn cast_reload(&mut self, mut options: PlayOptions) {
        let Some(casting) = self.cast.casting() else {
            return;
        };
        let Some(item) = casting.item.clone() else {
            return;
        };
        // Whole ticks: the server rejects a fractional `StartTimeTicks`.
        options.start = options.start.or(Some(Ticks::from(casting.position())));
        let same_version = options
            .media_source_id
            .as_ref()
            .is_none_or(|id| casting.media_source_id.as_ref() == Some(id));
        // Track indexes belong to a version: another one starts with the server's choice.
        if same_version {
            options.media_source_id = options.media_source_id.or_else(|| casting.media_source_id.clone());
            options.audio_stream_index = options.audio_stream_index.or(casting.audio);
            if options.subtitle == SubtitleChoice::Auto {
                options.subtitle = casting.subtitle_choice();
            }
        }
        // A reload already waiting keeps its place: its `Stop` is out, only the request changes.
        let stop_sent = self
            .cast
            .session
            .as_ref()
            .and_then(|session| session.reload.as_ref())
            .map(|reload| reload.stopped);
        let Some(request) = self.begin_cast_play(&[item], &options) else {
            return;
        };
        if let Some(session) = &mut self.cast.session {
            let mut reload = Reload {
                request,
                stopped: stop_sent.unwrap_or(false),
                due: Deadline::default(),
            };
            if stop_sent.is_none() {
                let _ = session.commands.send(Outgoing::Command(Command::Stop));
                reload.due.set(RELOAD_STOP_WAIT);
            } else if reload.stopped {
                reload.due.set(RELOAD_GAP);
            } else {
                reload.due.set(RELOAD_STOP_WAIT);
            }
            session.reload = Some(reload);
            // Shown at once, as the receiver's report of them comes after the reload.
            let casting = &mut session.casting;
            casting.media_source_id = options.media_source_id.clone();
            casting.audio = options.audio_stream_index;
            casting.subtitle = match options.subtitle {
                SubtitleChoice::Auto => None,
                SubtitleChoice::Off => Some(-1),
                SubtitleChoice::Stream(index) => Some(index),
            };
        }
    }

    /// Takes in what the devices and the cast reported. From [`Services::tick`].
    pub(crate) fn tick_cast(&mut self) {
        self.cast.tick_discovery();
        let Some(session) = &mut self.cast.session else {
            return;
        };
        // The handover waits for the local session's stop report, so the server sees this
        // device stop before the TV starts.
        if session.connected
            && session.handover.is_some()
            && !self.playback.is_active()
            && let Some((item, request)) = session.handover.take()
        {
            let _ = session.commands.send(Outgoing::Command(Command::PlayNow(request)));
            session.casting.name = item.name.clone();
            session.casting.series_name = item.series_name.clone();
            session.casting.duration = item.run_time_ticks.map(Duration::from);
            session.casting.item = Some(item);
            self.events.push(AppEvent::PlaybackStarted);
        }
        let Some(session) = &mut self.cast.session else {
            return;
        };
        let updates: Vec<Update> = session.updates.drain().collect();
        let mut fetch = None;
        let mut toasts = Vec::new();
        let mut ended = None;
        for update in updates {
            match update {
                Update::Connected(status) => {
                    session.connected = true;
                    session.reconnecting = None;
                    session.casting.apply_volume(&status);
                    toasts.push(Toast::success(format!("Casting to {}", session.device.name)));
                }
                // The receiver was still running: `Identify`'s answer restores what it plays,
                // so only the connection's state changes here.
                Update::Reconnected(status) => {
                    session.connected = true;
                    session.reconnecting = None;
                    session.casting.apply_volume(&status);
                }
                Update::Reconnecting(attempt) => {
                    // A launch isn't due, and a handover has nowhere to go until it's back.
                    session.connected = false;
                    session.reconnecting = Some(attempt);
                    // The new `Identify` answers with the state, rather than a stop.
                    session.identifying = true;
                }
                Update::Status(status) => session.casting.apply_volume(&status),
                Update::Event(event) => {
                    session.last_event = Some(Instant::now());
                    // The receiver answers this sender again, so the standard status isn't
                    // what it plays any more.
                    session.following = false;
                    let identifying = std::mem::take(&mut session.identifying);
                    let casting = &mut session.casting;
                    match &*event {
                        ReceiverEvent::PlaybackStart(data) => {
                            casting.playing = true;
                            session.stopped.cancel();
                            session.starting = false;
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                        }
                        ReceiverEvent::PlaybackProgress(data)
                        | ReceiverEvent::PlayStateChange(data)
                        | ReceiverEvent::VolumeChange(data) => {
                            // An idle TV reports its volume too, about no item.
                            if reported_item(data).is_some() {
                                casting.playing = true;
                            } else if identifying && casting.item_id.is_some() {
                                // `Identify`'s answer after a reconnect, with nothing playing:
                                // what it played ended while the connection was gone.
                                casting.playing = false;
                                session.stopped.set(STOPPED_FOR);
                            }
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                        }
                        // `Identify`'s answer: what plays now, if anything.
                        ReceiverEvent::PlaybackStop(data) if identifying => {
                            casting.playing = reported_item(data).is_some();
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                            if !casting.playing && casting.item_id.is_some() {
                                session.stopped.set(STOPPED_FOR);
                            }
                        }
                        // What played before stops ahead of what was just played from here.
                        ReceiverEvent::PlaybackStop(_) if session.starting => {
                            if let Some(reload) = &mut session.reload
                                && !reload.stopped
                            {
                                reload.stopped = true;
                                reload.due.set(RELOAD_GAP);
                            }
                        }
                        ReceiverEvent::PlaybackStop(data) => {
                            casting.apply(data);
                            casting.playing = false;
                            session.stopped.set(STOPPED_FOR);
                        }
                        ReceiverEvent::PlaybackError(data) => {
                            // Nothing is starting any more: its stop closes the player.
                            session.starting = false;
                            let code = data.message.as_deref().unwrap_or("unknown error");
                            toasts.push(Toast::error(format!("The TV couldn't play this ({code}).")).title("Casting"));
                        }
                        ReceiverEvent::ConnectionError(_) => {
                            session.starting = false;
                            toasts.push(
                                Toast::error(format!(
                                    "The TV couldn't reach the server at {}. It needs an address the TV can \
                                     reach on this network.",
                                    self.cast.credentials.server_address
                                ))
                                .title("Casting"),
                            );
                        }
                        ReceiverEvent::Error(data) => {
                            let message = data.message.as_deref().unwrap_or("unknown error");
                            tracing::warn!("cast: receiver error: {message}");
                        }
                        ReceiverEvent::RepeatModeChange(_) | ReceiverEvent::Other(..) => {}
                    }
                }
                Update::Media(status) => {
                    // Another sender's item, as the device itself describes it: followed once
                    // the custom messages go quiet (`Services::tick_cast` does the switch, so
                    // a status that came while ours still flowed isn't mistaken for it).
                    if session.following && session.casting.apply_media(&status) {
                        fetch = session.casting.item_id.clone();
                    }
                    session.media = Some((Instant::now(), status));
                }
                Update::Ended(end) => ended = Some(end),
            }
        }
        // Another sender took over: the receiver answers only it now, so follow the TV's own
        // status (kept from above) until our messages flow again, which `send` also starts.
        // The status must be newer than the last custom message to be the other sender's.
        if let Some(session) = &mut self.cast.session
            && !session.following
            && session
                .last_event
                .is_none_or(|last| last.elapsed() >= follow_media_after())
            && let Some((at, status)) = &session.media
            && session.last_event.is_none_or(|last| last <= *at)
        {
            session.following = true;
            if session.casting.apply_media(status) {
                fetch = session.casting.item_id.clone();
            }
        }
        if let Some(id) = fetch {
            let client = self.cast.client.clone();
            let task = Task::spawn(&self.cast.ctx, {
                let id = id.clone();
                async move { client.item(&id, &[ItemField::MediaSources, ItemField::Chapters]).await }
            });
            if let Some(session) = &mut self.cast.session {
                session.fetching = Some((id, task));
            }
        }
        if let Some(session) = &mut self.cast.session
            && let Some((id, task)) = &mut session.fetching
            && let Some(result) = task.poll()
        {
            match result {
                Ok(item) if session.casting.item_id.as_ref() == Some(id) => session.casting.item = Some(item),
                Ok(_) => {}
                Err(err) => tracing::warn!("cast: couldn't load the casting item: {err}"),
            }
            session.fetching = None;
        }
        if let Some(session) = &mut self.cast.session
            && session
                .reload
                .as_mut()
                .is_some_and(|reload| reload.due.fire(&self.cast.ctx))
            && let Some(reload) = session.reload.take()
        {
            let _ = session
                .commands
                .send(Outgoing::Command(Command::PlayNow(reload.request)));
        }
        let stopped = self
            .cast
            .session
            .as_mut()
            .is_some_and(|session| session.stopped.fire(&self.cast.ctx));
        if stopped {
            if let Some(session) = &mut self.cast.session {
                session.casting.item_id = None;
                session.casting.item = None;
            }
            self.cast_closed();
        }
        for toast in toasts {
            self.toast(toast);
        }
        if let Some(end) = ended {
            let name = self
                .cast
                .session
                .take()
                .map(|session| session.device.name)
                .unwrap_or_default();
            match end {
                End::Failed(message) => self.toast(Toast::error(message).title("Casting")),
                End::Dropped => self.toast(Toast::error(format!("Lost the connection to {name}.")).title("Casting")),
                End::AppGone => self.toast(Toast::info(format!("{name} stopped casting."))),
                End::Stopped | End::Disconnected => {}
            }
            self.cast_closed();
        }
    }
}

/// What the UI asks of the connection.
#[derive(Debug)]
enum Outgoing {
    Command(Command),
    /// The TV's volume, 0.0 to 1.0.
    Volume(f32),
    Muted(bool),
    /// New credentials (another bitrate), sent with an `Identify`.
    Credentials(Credentials),
    /// Stop the receiver app, then close.
    Stop,
    /// Leave the receiver playing and close.
    Disconnect,
}

/// What the connection reports.
#[derive(Debug)]
enum Update {
    /// The receiver is launched and listening; the device's status at that point.
    Connected(ReceiverStatus),
    /// The connection came back and the receiver was joined again: its status, and what the
    /// `Identify` after it reports, follow as the usual updates.
    Reconnected(ReceiverStatus),
    /// The connection dropped; `attempt` is the one about to be tried, from 1.
    Reconnecting(u32),
    Status(ReceiverStatus),
    Event(Box<ReceiverEvent>),
    Media(MediaStatus),
    Ended(End),
}

/// How a cast ended.
#[derive(Debug)]
enum End {
    /// It never got going: the device couldn't be reached or the receiver didn't launch.
    Failed(String),
    /// The device dropped the connection or stopped answering, and the retries ran out.
    Dropped,
    /// The receiver app went away: stopped from the TV or another sender.
    AppGone,
    Stopped,
    Disconnected,
}

/// Where to cast and with what.
struct Target {
    addr: SocketAddr,
    app_id: String,
    credentials: Credentials,
    config: ChannelConfig,
    reconnect: Reconnect,
}

/// An attempt that got the receiver up: the connection, its messages and the app.
struct Joined {
    channel: Channel,
    incoming: mpsc::UnboundedReceiver<cast::Message>,
    app: cast::Application,
}

/// How an attempt to reach the receiver went.
enum Attempt {
    Joined(Box<Joined>),
    /// A message for the user; no retry.
    Failed(String),
    /// The receiver isn't running any more (a reconnect found it stopped).
    Gone,
    /// It answered, then dropped before it was up.
    Dropped,
    /// Nobody is watching any more (the services went): end quietly.
    Ended(End),
}

/// The connection to a device, from connecting until it ends: launches the receiver, sends what
/// the UI asks and reports what comes back. A connection that drops is retried with backoff,
/// joining the receiver again if it's still running (docs/chromecast.md, *Staying connected*).
async fn run(target: Target, mut commands: mpsc::UnboundedReceiver<Outgoing>, updates: &task::Sender<Update>) -> End {
    let Target {
        addr,
        app_id,
        mut credentials,
        config,
        reconnect,
    } = target;
    // The first attempt launches the receiver; every later one asks what the device runs and
    // joins the receiver, which is how a cast survives a dropped connection.
    let mut joined = false;
    let mut losses = 0;
    loop {
        match join(addr, &app_id, &mut credentials, &config, joined, updates).await {
            Attempt::Joined(established) => {
                joined = true;
                // The tries are per outage: a cast that came back starts its next one afresh,
                // rather than a long cast being given up on its fifth drop.
                losses = 0;
                let Joined {
                    channel,
                    mut incoming,
                    app,
                } = *established;
                match drive(&channel, &app, &mut credentials, &mut incoming, &mut commands, updates).await {
                    Exit::Stop => {
                        if let Err(err) = receiver::stop(&channel, &app.session_id).await {
                            tracing::warn!("cast: stopping the receiver failed: {err}");
                        }
                        channel.shutdown(EXIT_WAIT).await;
                        return End::Stopped;
                    }
                    // Leaving, or the services that drove this are gone (signed out, quit). Only
                    // the socket closes: Jellyfin's receiver quits when its last sender leaves
                    // with a `CLOSE` (seen on a Chromecast), while a dropped connection leaves it
                    // playing.
                    Exit::Leave => {
                        channel.shutdown(EXIT_WAIT).await;
                        return End::Disconnected;
                    }
                    Exit::Ended(End::Dropped) => {}
                    Exit::Ended(end) => return end,
                }
                drop(channel);
                losses += 1;
                if let Some(end) = wait_to_retry(losses, &reconnect, &mut commands, updates).await {
                    return end;
                }
            }
            Attempt::Failed(message) => return End::Failed(message),
            Attempt::Gone => return End::AppGone,
            Attempt::Ended(end) => return end,
            Attempt::Dropped => {
                losses += 1;
                if let Some(end) = wait_to_retry(losses, &reconnect, &mut commands, updates).await {
                    return end;
                }
            }
        }
    }
}

/// Connects to the device and gets the receiver up. The first time that's a launch; afterwards
/// it asks what the device runs and joins the receiver where it is, sending `Identify` so that
/// its answer can restore the state. `resumed` says which.
async fn join(
    addr: SocketAddr,
    app_id: &str,
    credentials: &mut Credentials,
    config: &ChannelConfig,
    resumed: bool,
    updates: &task::Sender<Update>,
) -> Attempt {
    let name = credentials.receiver_name.clone();
    let (channel, incoming) = match Channel::connect_with(addr, config.clone()).await {
        Ok(connected) => connected,
        Err(err) => {
            tracing::warn!("cast: couldn't connect to {addr}: {err}");
            return if resumed {
                Attempt::Dropped
            } else {
                Attempt::Failed(format!("Couldn't connect to {name}."))
            };
        }
    };
    let app = if resumed {
        // It may still be playing: ask before launching anything over it. Only the app still
        // being listed means the cast can carry on; anything else is the TV's idle screen.
        let status = match receiver::get_status(&channel).await {
            Ok(status) => status,
            Err(err) => {
                tracing::warn!("cast: asking {name} what it runs failed: {err}");
                return Attempt::Dropped;
            }
        };
        match status.application(app_id) {
            Some(app) if app.has_namespace(receiver_app::NAMESPACE) => app.clone(),
            _ => {
                tracing::info!("cast: {name} isn't running the receiver any more");
                return Attempt::Gone;
            }
        }
    } else {
        let app = match receiver::launch(&channel, app_id).await {
            Ok(app) => app,
            Err(err) => {
                tracing::warn!("cast: launching {app_id} failed: {err}");
                return Attempt::Failed(format!("{name} couldn't start Jellyfin."));
            }
        };
        if !app.has_namespace(receiver_app::NAMESPACE) {
            tracing::warn!(?app, "cast: the receiver doesn't listen on {}", receiver_app::NAMESPACE);
            return Attempt::Failed(format!("{name} started an app that isn't Jellyfin's receiver."));
        }
        app
    };
    let transport = app.transport_id.clone();
    let envelope = receiver_app::Envelope::new(credentials, Command::Identify).to_value();
    if channel.connect_app(&transport).await.is_err()
        || channel
            .send(receiver_app::NAMESPACE, &transport, envelope)
            .await
            .is_err()
    {
        return Attempt::Dropped;
    }
    let status = receiver::get_status(&channel).await.unwrap_or_default();
    let update = if resumed {
        Update::Reconnected(status)
    } else {
        Update::Connected(status)
    };
    if updates.send(update).is_err() {
        // Nobody listens: the services went while connecting. Leave it as `Exit::Leave` does.
        channel.shutdown(EXIT_WAIT).await;
        return Attempt::Ended(End::Disconnected);
    }
    Attempt::Joined(Box::new(Joined { channel, incoming, app }))
}

/// Waits out the backoff before attempt `attempt`, telling the UI. Returns the end instead when
/// the cast should stop: the tries ran out, or Stop or Disconnect came while waiting.
async fn wait_to_retry(
    attempt: u32,
    reconnect: &Reconnect,
    commands: &mut mpsc::UnboundedReceiver<Outgoing>,
    updates: &task::Sender<Update>,
) -> Option<End> {
    if attempt > reconnect.attempts {
        return Some(End::Dropped);
    }
    if updates.send(Update::Reconnecting(attempt)).is_err() {
        return Some(End::Disconnected);
    }
    let deadline = tokio::time::Instant::now() + reconnect.delay(attempt);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Outgoing::Stop) => return Some(End::Stopped),
                // The session went (stopped, disconnected, signed out): nothing left to keep.
                Some(Outgoing::Disconnect) | None => return Some(End::Disconnected),
                // Anything about the connection that's gone is stale: the `Identify` after the
                // reconnect answers with what the TV does now.
                Some(_) => {}
            },
            _ = tokio::time::sleep_until(deadline) => return None,
        }
    }
}

/// How [`drive`] ended: what's left to do with the connection.
enum Exit {
    Stop,
    Leave,
    Ended(End),
}

/// A volume change for the TV.
#[derive(Clone, Copy)]
enum VolumeChange {
    Level(f32),
    Muted(bool),
}

/// Relays between the UI and the receiver until one of them ends it. It never waits on the TV
/// between messages: a volume request is answered while the loop goes on, and the wheel's next
/// steps wait behind it as just the latest level.
async fn drive(
    channel: &Channel,
    app: &cast::Application,
    credentials: &mut Credentials,
    incoming: &mut mpsc::UnboundedReceiver<cast::Message>,
    commands: &mut mpsc::UnboundedReceiver<Outgoing>,
    updates: &task::Sender<Update>,
) -> Exit {
    let transport = app.transport_id.as_str();
    let send = async |credentials: &Credentials, command: Command| {
        let envelope = receiver_app::Envelope::new(credentials, command).to_value();
        // What's sent, but not the credentials every envelope carries.
        tracing::debug!(command = %envelope["command"], options = %envelope["options"], "cast: to the receiver");
        channel.send(receiver_app::NAMESPACE, transport, envelope).await
    };
    let set_volume = |change: VolumeChange| -> VolumeRequest<'_> {
        match change {
            VolumeChange::Level(level) => Box::pin(receiver::set_volume_level(channel, level)),
            VolumeChange::Muted(muted) => Box::pin(receiver::set_muted(channel, muted)),
        }
    };
    let mut volume: Option<VolumeRequest<'_>> = None;
    // The level and mute asked for while a request was out; only the latest of each matters.
    let mut next_level = None;
    let mut next_muted = None;
    loop {
        tokio::select! {
            status = async { volume.as_mut().expect("guarded").await }, if volume.is_some() => {
                if report_status(status, updates).is_err() {
                    return Exit::Ended(End::Dropped);
                }
                // The finished request is replaced (or cleared): mute first, then the level.
                let next = next_muted.take().map(VolumeChange::Muted).or_else(|| next_level.take().map(VolumeChange::Level));
                volume = next.map(set_volume);
            }
            message = incoming.recv() => {
                let Some(message) = message else { return Exit::Ended(End::Dropped) };
                tracing::debug!(namespace = %message.namespace, payload = %message.payload, "cast: from the TV");
                let kind = message.payload.get("type").and_then(Value::as_str);
                let update = match message.namespace.as_str() {
                    receiver_app::NAMESPACE => {
                        // The receiver sends its JSON as a string inside the payload string at
                        // times; take either.
                        let payload = match &message.payload {
                            Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                            other => other.clone(),
                        };
                        match ReceiverEvent::from_value(&payload) {
                            Ok(event) => Update::Event(Box::new(event)),
                            Err(err) => {
                                tracing::debug!("cast: unreadable receiver message: {err}");
                                continue;
                            }
                        }
                    }
                    NS_RECEIVER if kind == Some("RECEIVER_STATUS") => {
                        let raw = message.payload.get("status");
                        // The TV sends statuses with only the volume, right after a launch and
                        // between apps: no `applications` says nothing about ours. It's gone only
                        // when the list is there without it (the idle screen's, usually).
                        let lists_apps = raw.is_some_and(|status| status.get("applications").is_some());
                        let status: ReceiverStatus = raw
                            .cloned()
                            .and_then(|status| serde_json::from_value(status).ok())
                            .unwrap_or_default();
                        if lists_apps && !status.applications.iter().any(|running| running.session_id == app.session_id) {
                            return Exit::Ended(End::AppGone);
                        }
                        Update::Status(status)
                    }
                    NS_MEDIA if kind == Some("MEDIA_STATUS") => match MediaStatus::from_value(&message.payload) {
                        Ok(status) => Update::Media(status),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                if updates.send(update).is_err() {
                    return Exit::Leave;
                }
            }
            command = commands.recv() => {
                let result = match command {
                    Some(Outgoing::Command(command)) => send(credentials, command).await,
                    Some(Outgoing::Credentials(new)) => {
                        *credentials = new;
                        send(credentials, Command::Identify).await
                    }
                    Some(Outgoing::Volume(level)) if volume.is_some() => {
                        next_level = Some(level);
                        Ok(())
                    }
                    Some(Outgoing::Muted(muted)) if volume.is_some() => {
                        next_muted = Some(muted);
                        Ok(())
                    }
                    Some(Outgoing::Volume(level)) => {
                        volume = Some(set_volume(VolumeChange::Level(level)));
                        Ok(())
                    }
                    Some(Outgoing::Muted(muted)) => {
                        volume = Some(set_volume(VolumeChange::Muted(muted)));
                        Ok(())
                    }
                    Some(Outgoing::Stop) => return Exit::Stop,
                    Some(Outgoing::Disconnect) | None => return Exit::Leave,
                };
                match result {
                    Ok(()) => {}
                    // Refused before it was sent; the connection is fine.
                    Err(err @ cast::Error::FrameTooLarge { .. }) => {
                        tracing::warn!("cast: a message too big to send: {err}");
                    }
                    Err(_) => return Exit::Ended(End::Dropped),
                }
            }
        }
    }
}

/// A `SET_VOLUME` on its way, answered with the device's status.
type VolumeRequest<'a> = std::pin::Pin<Box<dyn Future<Output = cast::Result<ReceiverStatus>> + Send + 'a>>;

/// Hands a `SET_VOLUME`'s answer on. A failed one isn't a dropped connection: the channel
/// notices that itself.
fn report_status(status: cast::Result<ReceiverStatus>, updates: &task::Sender<Update>) -> cast::Result<()> {
    match status {
        Ok(status) => {
            let _ = updates.send(Update::Status(status));
            Ok(())
        }
        Err(cast::Error::Closed) => Err(cast::Error::Closed),
        Err(err) => {
            tracing::warn!("cast: setting the volume failed: {err}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemKind;

    use super::*;

    #[test]
    fn the_backoff_doubles_up_to_its_cap() {
        let policy = Reconnect {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(8),
            attempts: 6,
        };
        let delays: Vec<u64> = (1..=6).map(|attempt| policy.delay(attempt).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 8, 8]);
    }

    #[test]
    fn a_loopback_server_is_given_as_its_lan_address() {
        let local = Some("http://192.168.1.20:8096");
        let url = |s: &str| url::Url::parse(s).unwrap();
        assert_eq!(
            server_address(&url("http://localhost:8096/"), local),
            "http://192.168.1.20:8096"
        );
        assert_eq!(
            server_address(&url("http://127.0.0.1:8096"), local),
            "http://192.168.1.20:8096"
        );
        assert_eq!(
            server_address(&url("http://[::1]:8096"), local),
            "http://192.168.1.20:8096"
        );
        assert_eq!(
            server_address(&url("http://localhost:8096"), None),
            "http://localhost:8096"
        );
        assert_eq!(
            server_address(&url("https://jf.example.com/jellyfin/"), local),
            "https://jf.example.com/jellyfin"
        );
    }

    #[test]
    fn controls_map_to_receiver_commands() {
        let casting = Casting {
            position: Duration::from_secs(100),
            duration: Some(Duration::from_secs(600)),
            muted: false,
            ..Casting::default()
        };
        let command = |ui: UiCommand| match outgoing_for(&ui, &casting) {
            Some(Outgoing::Command(command)) => Some(command),
            _ => None,
        };
        assert_eq!(command(UiCommand::TogglePause), Some(Command::PlayPause));
        assert_eq!(command(UiCommand::SetPause(true)), Some(Command::Pause));
        assert_eq!(command(UiCommand::SetPause(false)), Some(Command::Unpause));
        assert_eq!(
            command(UiCommand::SeekTo(Duration::from_secs(30))),
            Some(Command::Seek { position_seconds: 30. })
        );
        assert_eq!(
            command(UiCommand::SeekBy(-10.)),
            Some(Command::Seek { position_seconds: 90. })
        );
        assert_eq!(
            command(UiCommand::SeekBy(-1000.)),
            Some(Command::Seek { position_seconds: 0. })
        );
        assert_eq!(
            command(UiCommand::SeekTo(Duration::from_secs(9000))),
            Some(Command::Seek { position_seconds: 600. })
        );
        assert_eq!(command(UiCommand::Stop), Some(Command::Stop));
        assert!(matches!(
            outgoing_for(&UiCommand::SetVolume(40.), &casting),
            Some(Outgoing::Volume(level)) if (level - 0.4).abs() < 1e-6
        ));
        assert!(matches!(
            outgoing_for(&UiCommand::ToggleMute, &casting),
            Some(Outgoing::Muted(true))
        ));
        for ignored in [
            UiCommand::Audio(2),
            UiCommand::Subtitle(SubtitleChoice::Off),
            UiCommand::SetSpeed(1.5),
            UiCommand::SetSubtitleDelay(1.),
            UiCommand::SetAudioDelay(1.),
            UiCommand::SetSubtitleScale(1.2),
        ] {
            assert!(outgoing_for(&ignored, &casting).is_none());
        }
    }

    #[test]
    fn a_long_queue_is_cut_to_one_message() {
        let credentials = Credentials {
            server_address: "http://192.168.1.149:8096".into(),
            access_token: "0123456789abcdef0123456789abcdef".into(),
            user_id: "0123456789abcdef0123456789abcdef".into(),
            device_id: "device".into(),
            server_id: "0123456789abcdef0123456789abcdef".into(),
            server_version: "12.2.0".into(),
            receiver_name: "Quarto TV".into(),
            max_bitrate: None,
            subtitle_appearance: None,
        };
        let episodes: Vec<BaseItemDto> = (0..1000)
            .map(|index| BaseItemDto {
                id: format!("{index:032x}"),
                name: format!("Episode {index}"),
                kind: BaseItemKind::Episode,
                ..Default::default()
            })
            .collect();
        let mut request = play_request(&episodes, &PlayOptions::default(), &credentials.server_id);
        fit_play_request(&mut request, &credentials);
        let size = |request: &PlayRequest| {
            receiver_app::Envelope::new(&credentials, Command::PlayNow(request.clone()))
                .to_value()
                .to_string()
                .len()
        };
        assert!(size(&request) <= cast::MAX_FRAME - FRAME_SLACK);
        assert!(request.items.len() > 200, "as much as fits: {}", request.items.len());
        assert_eq!(request.items[0].id, episodes[0].id);
        // One more wouldn't fit.
        let mut more = request.clone();
        more.items
            .push(play_request(&episodes[request.items.len()..][..1], &PlayOptions::default(), "s").items[0].clone());
        assert!(size(&more) > cast::MAX_FRAME - FRAME_SLACK);

        // A short one is left as it is.
        let mut short = play_request(&episodes[..5], &PlayOptions::default(), &credentials.server_id);
        fit_play_request(&mut short, &credentials);
        assert_eq!(short.items.len(), 5);
    }

    #[test]
    fn play_options_become_the_play_request() {
        let item = BaseItemDto {
            id: "abc".into(),
            name: "Pilot".into(),
            kind: BaseItemKind::Episode,
            ..Default::default()
        };
        let options = PlayOptions {
            start: Some(Ticks(1_200_000_000)),
            media_source_id: Some("src".into()),
            audio_stream_index: Some(1),
            subtitle: SubtitleChoice::Off,
            ..Default::default()
        };
        let request = play_request(&[item], &options, "server");
        assert_eq!(request.items[0].id, "abc");
        assert_eq!(request.items[0].server_id, "server");
        assert_eq!(request.items[0].kind, "Episode");
        assert_eq!(request.items[0].media_type, "Video");
        assert_eq!(request.start_position_ticks, Some(1_200_000_000));
        assert_eq!(request.media_source_id.as_deref(), Some("src"));
        assert_eq!(request.audio_stream_index, Some(1));
        assert_eq!(request.subtitle_stream_index, Some(-1));
    }

    #[test]
    fn the_last_device_is_listed_first() {
        let device = |id: &str, name: &str| Device {
            id: id.into(),
            name: name.into(),
            model: "Chromecast".into(),
            address: [127, 0, 0, 1].into(),
            port: 8009,
            running_app: None,
        };
        let mut devices = vec![
            device("a", "Kitchen"),
            device("b", "bedroom"),
            device("c", "Living room"),
        ];
        sort_devices(&mut devices, Some("c"));
        let names: Vec<_> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Living room", "bedroom", "Kitchen"]);
    }
}
