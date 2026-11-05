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
/// How long the custom messages may go quiet before `MEDIA_STATUS` is followed instead: another
/// sender cast to the TV, and the receiver now answers that one only.
const FOLLOW_MEDIA_AFTER: Duration = Duration::from_secs(3);
/// How long quitting waits for the TV to hear that we're leaving.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// The cast part of [`Services`]: the devices on the network and the cast in progress.
pub struct Cast {
    ctx: egui::Context,
    client: Client,
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
    /// `Identify`'s answer is a `playbackstop` that carries the current state: the first one
    /// after connecting doesn't mean playback stopped.
    identifying: bool,
    /// The receiver stopped and nothing new started yet; closes the player when it fires.
    stopped: Deadline,
    /// When the receiver's own messages last came, to tell when another sender took over.
    last_event: Option<Instant>,
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
        if let Some(item) = &data.now_playing_item {
            let id = item.id.clone().or_else(|| data.item_id.clone());
            if let Some(id) = id
                && self.item_id.as_ref() != Some(&id)
            {
                self.follow_item(id);
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
    pub fn new(session: &Session, ctx: &egui::Context) -> Self {
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
            devices: Vec::new(),
            discovery: None,
            found,
            found_from,
            last_device: config.cast_device,
            session: None,
            app_id,
            credentials,
        };
        // The UI tests have no network beyond loopback; they add their devices by hand.
        if !cfg!(test) {
            cast.discover();
        }
        cast
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
        };
        // Not a `Task`: leaving (`CLOSE`) or stopping the receiver has to reach the TV after
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
            last_event: None,
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
    fn send(&self, command: Command) {
        if let Some(session) = &self.session {
            let _ = session.commands.send(Outgoing::Command(command));
        }
    }

    fn send_outgoing(&self, outgoing: Outgoing) {
        if let Some(session) = &self.session {
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
        UiCommand::Audio(index) => Outgoing::Command(Command::SetAudioStreamIndex(index)),
        UiCommand::Subtitle(SubtitleChoice::Stream(index)) => Outgoing::Command(Command::SetSubtitleStreamIndex(index)),
        UiCommand::Subtitle(SubtitleChoice::Off) => Outgoing::Command(Command::SetSubtitleStreamIndex(-1)),
        UiCommand::Stop => Outgoing::Command(Command::Stop),
        UiCommand::Subtitle(SubtitleChoice::Auto)
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
            (state.item.clone(), play_request(&items, &options, &server_id))
        });
        if handover.is_some() {
            // The receiver plays it once the local session has reported its stop (`tick_cast`).
            self.stop_playback();
            self.playback.clear_queue();
        }
        tracing::info!(device = %device.name, handover = handover.is_some(), "cast: connecting");
        self.cast.connect(device, handover);
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
        if items.is_empty() {
            return;
        }
        let request = play_request(items, options, &self.cast.credentials.server_id);
        self.cast.send(Command::PlayNow(request));
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
        }
        self.events.push(AppEvent::PlaybackStarted);
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
                UiCommand::Audio(index) => casting.audio = Some(index),
                UiCommand::Subtitle(SubtitleChoice::Stream(index)) => casting.subtitle = Some(index),
                UiCommand::Subtitle(SubtitleChoice::Off) => casting.subtitle = Some(-1),
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

    /// Plays the casting item again in another version, from where it is.
    pub(crate) fn cast_reload(&mut self, mut options: PlayOptions) {
        let Some(casting) = self.cast.casting() else {
            return;
        };
        let Some(item) = casting.item.clone() else {
            return;
        };
        options.start = options.start.or(Some(Ticks::from(casting.position())));
        options.audio_stream_index = options.audio_stream_index.or(casting.audio);
        if options.subtitle == SubtitleChoice::Auto {
            options.subtitle = casting.subtitle_choice();
        }
        self.cast_play(&[item], &options);
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
                    session.casting.apply_volume(&status);
                    toasts.push(Toast::success(format!("Casting to {}", session.device.name)));
                }
                Update::Status(status) => session.casting.apply_volume(&status),
                Update::Event(event) => {
                    session.last_event = Some(Instant::now());
                    let identifying = std::mem::take(&mut session.identifying);
                    let casting = &mut session.casting;
                    match &*event {
                        ReceiverEvent::PlaybackStart(data) => {
                            casting.playing = true;
                            session.stopped.cancel();
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                        }
                        ReceiverEvent::PlaybackProgress(data)
                        | ReceiverEvent::PlayStateChange(data)
                        | ReceiverEvent::VolumeChange(data) => {
                            casting.playing = true;
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                        }
                        // `Identify`'s answer: what plays now, if anything.
                        ReceiverEvent::PlaybackStop(data) if identifying => {
                            casting.playing = data.data.now_playing_item.is_some();
                            if casting.apply(data) {
                                fetch = casting.item_id.clone();
                            }
                        }
                        ReceiverEvent::PlaybackStop(data) => {
                            casting.apply(data);
                            casting.playing = false;
                            session.stopped.set(STOPPED_FOR);
                        }
                        ReceiverEvent::PlaybackError(data) => {
                            let code = data.message.as_deref().unwrap_or("unknown error");
                            toasts.push(Toast::error(format!("The TV couldn't play this ({code}).")).title("Casting"));
                        }
                        ReceiverEvent::ConnectionError(_) => {
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
                    // Another sender took over: its messages go to it, so follow the TV's
                    // standard status until we send something again.
                    let quiet = session.last_event.is_none_or(|at| at.elapsed() >= FOLLOW_MEDIA_AFTER);
                    if quiet && session.casting.apply_media(&status) {
                        fetch = session.casting.item_id.clone();
                    }
                }
                Update::Ended(end) => ended = Some(end),
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
    /// The receiver is up and listening; the device's status at that point.
    Connected(ReceiverStatus),
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
    /// The device dropped the connection or stopped answering.
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
}

/// The connection to a device, from connecting until it ends: launches the receiver, sends what
/// the UI asks and reports what comes back.
async fn run(target: Target, mut commands: mpsc::UnboundedReceiver<Outgoing>, updates: &task::Sender<Update>) -> End {
    let Target {
        addr,
        app_id,
        mut credentials,
        config,
    } = target;
    let name = credentials.receiver_name.clone();
    let (channel, mut incoming) = match Channel::connect_with(addr, config).await {
        Ok(connected) => connected,
        Err(err) => {
            tracing::warn!("cast: couldn't connect to {addr}: {err}");
            return End::Failed(format!("Couldn't connect to {name}."));
        }
    };
    let app = match receiver::launch(&channel, &app_id).await {
        Ok(app) => app,
        Err(err) => {
            tracing::warn!("cast: launching {app_id} failed: {err}");
            return End::Failed(format!("{name} couldn't start Jellyfin."));
        }
    };
    if !app.has_namespace(receiver_app::NAMESPACE) {
        tracing::warn!(?app, "cast: the receiver doesn't listen on {}", receiver_app::NAMESPACE);
        return End::Failed(format!("{name} started an app that isn't Jellyfin's receiver."));
    }
    let transport = app.transport_id.clone();
    let send = async |channel: &Channel, credentials: &Credentials, command: Command| {
        let envelope = receiver_app::Envelope::new(credentials, command).to_value();
        channel.send(receiver_app::NAMESPACE, &transport, envelope).await
    };
    if channel.connect_app(&transport).await.is_err() || send(&channel, &credentials, Command::Identify).await.is_err()
    {
        return End::Dropped;
    }
    let status = receiver::get_status(&channel).await.unwrap_or_default();
    if updates.send(Update::Connected(status)).is_err() {
        // Nobody listens: the services went while connecting.
        let _ = channel.close_app(&transport).await;
        channel.shutdown(EXIT_WAIT).await;
        return End::Disconnected;
    }
    loop {
        tokio::select! {
            message = incoming.recv() => {
                let Some(message) = message else { return End::Dropped };
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
                        let status: ReceiverStatus = message
                            .payload
                            .get("status")
                            .cloned()
                            .and_then(|status| serde_json::from_value(status).ok())
                            .unwrap_or_default();
                        if !status.applications.iter().any(|running| running.session_id == app.session_id) {
                            return End::AppGone;
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
                    let _ = channel.close_app(&transport).await;
                    channel.shutdown(EXIT_WAIT).await;
                    return End::Disconnected;
                }
            }
            command = commands.recv() => {
                let result = match command {
                    Some(Outgoing::Command(command)) => send(&channel, &credentials, command).await,
                    Some(Outgoing::Credentials(new)) => {
                        credentials = new;
                        send(&channel, &credentials, Command::Identify).await
                    }
                    Some(Outgoing::Volume(level)) => {
                        report_status(receiver::set_volume_level(&channel, level).await, updates)
                    }
                    Some(Outgoing::Muted(muted)) => report_status(receiver::set_muted(&channel, muted).await, updates),
                    Some(Outgoing::Stop) => {
                        if let Err(err) = receiver::stop(&channel, &app.session_id).await {
                            tracing::warn!("cast: stopping the receiver failed: {err}");
                        }
                        channel.shutdown(EXIT_WAIT).await;
                        return End::Stopped;
                    }
                    // Leaving, or the services that drove this are gone (signed out, quit).
                    Some(Outgoing::Disconnect) | None => {
                        let _ = channel.close_app(&transport).await;
                        channel.shutdown(EXIT_WAIT).await;
                        return End::Disconnected;
                    }
                };
                if result.is_err() {
                    return End::Dropped;
                }
            }
        }
    }
}

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
        assert_eq!(command(UiCommand::Audio(2)), Some(Command::SetAudioStreamIndex(2)));
        assert_eq!(
            command(UiCommand::Subtitle(SubtitleChoice::Stream(4))),
            Some(Command::SetSubtitleStreamIndex(4))
        );
        assert_eq!(
            command(UiCommand::Subtitle(SubtitleChoice::Off)),
            Some(Command::SetSubtitleStreamIndex(-1))
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
            UiCommand::SetSpeed(1.5),
            UiCommand::SetSubtitleDelay(1.),
            UiCommand::SetAudioDelay(1.),
            UiCommand::SetSubtitleScale(1.2),
        ] {
            assert!(outgoing_for(&ignored, &casting).is_none());
        }
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
