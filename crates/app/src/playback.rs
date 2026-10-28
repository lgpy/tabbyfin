//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio and sends its updates through a [`task::channel`] that [`Services::tick`] drains into
//! the [`NowPlaying`] state the player page draws and controls.
//!
//! [`PlaybackManager`] is the part that needs nothing else: the sessions, the queue, the volume
//! and adjustments. What also involves SyncPlay (playing, the controls, switching tracks) is on
//! [`Services`], below, and asks SyncPlay through the seam in `syncplay.rs`.

use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use jellyfin::models::{
    BaseItemDto, BaseItemKind, ItemField, MediaSourceInfo, MediaStream, MediaStreamType, TrickplayInfo,
};
use jellyfin::{Client, PlayMethod, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::libmpv::{VideoImport, VideoOutput};
use player::{EndReason, LoadRequest, Player, PlayerEvent};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::config::{Config, SubtitleColor, SubtitleStyle, UnfinishedPlayback};
use crate::markers::{self, Chapter, Segment, SegmentKind};
use crate::runtime;
use crate::services::{AppEvent, Services};
use crate::session::describe_error;
use crate::syncplay::GroupRequest;
use crate::task::{self, Deadline};
use crate::toasts::Toast;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// How long after a volume or mute change its progress report goes out. A wheel spin or held
/// key changes the volume many times a second; one report covers them all.
const VOLUME_REPORT_AFTER: Duration = Duration::from_secs(1);
const DEFAULT_VOLUME: f64 = 100.;
/// How long quitting the app waits for a running session to report its stop. The next launch
/// reports it instead if this runs out (`report_unfinished`).
const EXIT_WAIT: Duration = Duration::from_secs(3);

/// What to play. Stream indexes are Jellyfin's `MediaStream.Index`; `None` leaves the choice
/// to mpv (and the user's mpv config).
#[derive(Debug, Clone, Default)]
pub struct PlayOptions {
    pub start: Option<Ticks>,
    pub media_source_id: Option<String>,
    pub audio_stream_index: Option<i32>,
    pub subtitle: SubtitleChoice,
    /// Load paused (SyncPlay starts playback with a scheduled command).
    pub start_paused: bool,
    /// The SyncPlay queue entry this plays, when the group controls playback.
    pub group_item: Option<String>,
    /// `start` is the item's resume point, taken without asking (next episode, autoplay), so
    /// the player offers to start over.
    pub resumed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubtitleChoice {
    #[default]
    Auto,
    Off,
    Stream(i32),
}

/// Tweaks from the player's settings menu. Speed and subtitle size carry over to the next
/// item; the delays belong to one item's files and reset when another item starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustments {
    /// 1.0 is normal.
    pub speed: f64,
    /// Seconds; positive shows subtitles later.
    pub subtitle_delay: f64,
    /// Seconds; positive plays the audio later.
    pub audio_delay: f64,
    /// 1.0 is the subtitles' own size.
    pub subtitle_scale: f64,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            speed: 1.,
            subtitle_delay: 0.,
            audio_delay: 0.,
            subtitle_scale: 1.,
        }
    }
}

impl Adjustments {
    pub const SPEEDS: [f64; 7] = [0.5, 0.75, 1., 1.25, 1.5, 1.75, 2.];
    pub const DELAY_STEP: f64 = 0.1;
    pub const SCALE_STEP: f64 = 0.1;
    const MAX_DELAY: f64 = 30.;
    const SCALE_RANGE: (f64, f64) = (0.5, 3.);

    /// The next speed in [`Self::SPEEDS`] above (`up`) or below the current one.
    pub fn step_speed(self, up: bool) -> f64 {
        let (first, last) = (Self::SPEEDS[0], Self::SPEEDS[Self::SPEEDS.len() - 1]);
        if up {
            Self::SPEEDS
                .into_iter()
                .find(|&speed| speed > self.speed + 1e-6)
                .unwrap_or(last)
        } else {
            Self::SPEEDS
                .into_iter()
                .rev()
                .find(|&speed| speed < self.speed - 1e-6)
                .unwrap_or(first)
        }
    }

    /// Within the ranges the menu offers, rounded to its steps.
    fn clamped(self) -> Self {
        let tenths = |value: f64| (value * 10.).round() / 10.;
        Self {
            speed: self.speed.clamp(Self::SPEEDS[0], Self::SPEEDS[Self::SPEEDS.len() - 1]),
            subtitle_delay: tenths(self.subtitle_delay.clamp(-Self::MAX_DELAY, Self::MAX_DELAY)),
            audio_delay: tenths(self.audio_delay.clamp(-Self::MAX_DELAY, Self::MAX_DELAY)),
            subtitle_scale: tenths(self.subtitle_scale.clamp(Self::SCALE_RANGE.0, Self::SCALE_RANGE.1)),
        }
    }

    /// For the next item: the same speed and subtitle size, no delays.
    fn for_next_item(self) -> Self {
        Self {
            speed: self.speed,
            subtitle_scale: self.subtitle_scale,
            ..Self::default()
        }
    }
}

/// Requests from the player UI to the running session.
#[derive(Debug, Clone)]
pub enum UiCommand {
    TogglePause,
    SetPause(bool),
    /// Playback speed, 1.0 is normal. SyncPlay's drift correction.
    SetSpeed(f64),
    SeekBy(f64),
    SeekTo(Duration),
    /// Percent, 0 to 100.
    SetVolume(f64),
    SetMute(bool),
    /// Seconds.
    SetSubtitleDelay(f64),
    /// Seconds.
    SetAudioDelay(f64),
    SetSubtitleScale(f64),
    ToggleMute,
    Audio(i32),
    Subtitle(SubtitleChoice),
    Stop,
}

/// Session to UI messages.
enum SessionUpdate {
    Started {
        source: Box<MediaSourceInfo>,
        video: VideoOutput,
        audio: Option<i32>,
        subtitle: SubtitleChoice,
        stream: StreamInfo,
    },
    /// Fetched after playback starts.
    Details(Box<Details>),
    Player(PlayerEvent),
    /// `run_session` returned: the last message of a session. A panic in it is raised again on
    /// the UI thread, as a [`crate::task::Task`]'s is.
    Finished(std::thread::Result<Result<Ended, String>>),
}

/// How the session's stream was set up, for switching tracks.
#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    pub method: PlayMethod,
    /// Subtitles added to mpv as files (Jellyfin stream indexes), in mpv's order after the
    /// tracks embedded in the stream.
    pub subtitle_files: Vec<i32>,
    /// A subtitle the transcoder burns into the video.
    pub burned_subtitle: Option<i32>,
}

/// Another version (media source) of the playing item.
#[derive(Debug, Clone)]
pub struct Version {
    pub id: String,
    pub name: String,
}

#[derive(Default)]
struct Details {
    versions: Vec<Version>,
    trickplay: TrickplayTiles,
    previous: Option<BaseItemDto>,
    next: Option<BaseItemDto>,
    /// `next` is the following movie of a collection, which is offered but not autoplayed.
    next_in_collection: bool,
    chapters: Vec<Chapter>,
    segments: Vec<Segment>,
}

/// Trickplay tile sets by media source id, then thumbnail width (see [`crate::trickplay`]).
pub type TrickplayTiles = HashMap<String, HashMap<String, TrickplayInfo>>;

/// How long into playback from a resume point the player offers to start over.
const RESUME_PROMPT_FOR: Duration = Duration::from_secs(10);

/// When the up-next card shows without an Outro segment: this long before the end.
const UP_NEXT_BEFORE_END: Duration = Duration::from_secs(20);

/// State of the embedded player, for the player page.
pub struct NowPlaying {
    /// The session this is the state of (the manager's generation when it started), so SyncPlay
    /// can tell a new session from the one it watches.
    pub session: u64,
    pub item: BaseItemDto,
    pub source: MediaSourceInfo,
    pub video: VideoOutput,
    pub position: Duration,
    /// When `position` was reported, to extrapolate between mpv's (once a second) updates.
    pub position_at: Instant,
    pub duration: Option<Duration>,
    pub paused: bool,
    /// Until the file has loaded.
    pub loading: bool,
    pub audio: Option<i32>,
    pub subtitle: SubtitleChoice,
    pub volume: f64,
    pub muted: bool,
    pub adjustments: Adjustments,
    /// All versions of the item, once known; empty or one entry means there's nothing to
    /// switch to.
    pub versions: Vec<Version>,
    /// Seek-bar preview tiles, once known; empty if the server has none.
    pub trickplay: TrickplayTiles,
    /// The item before this one: the previous episode, or the previous movie of its
    /// collection.
    pub previous: Option<BaseItemDto>,
    /// What plays next: the queue's next item, the following episode, or the following movie
    /// of the item's collection.
    pub next: Option<BaseItemDto>,
    /// The user dismissed the up-next card: don't play the next episode at the end.
    pub next_cancelled: bool,
    /// Play the next episode at the end (the setting, at session start).
    pub autoplay: bool,
    pub chapters: Vec<Chapter>,
    /// Intro, credits and similar, once known.
    pub segments: Vec<Segment>,
    pub stream: StreamInfo,
    /// The SyncPlay queue entry, while a group controls playback.
    pub group_item: Option<String>,
    /// Where playback resumed without asking, while the player may still offer to start over.
    pub resumed_from: Option<Duration>,
    /// Counts [`PlayerEvent::Restarted`]: playback resumed after loading or seeking.
    pub restarts: u64,
    commands: mpsc::UnboundedSender<UiCommand>,
}

impl NowPlaying {
    /// Sends a command straight to this session's player, bypassing a SyncPlay group (SyncPlay's
    /// own scheduled commands). Controls from the UI go through [`Services::control`].
    pub fn send(&self, command: UiCommand) {
        let _ = self.commands.send(command);
    }

    /// Where [`NowPlaying::send`] sends, for code that sends later from tokio: SyncPlay's
    /// scheduled commands, which mustn't wait for the UI's next frame. Sends to a session that
    /// has ended go nowhere.
    pub(crate) fn commands(&self) -> mpsc::UnboundedSender<UiCommand> {
        self.commands.clone()
    }

    /// The segment the skip button would skip, if one is playing.
    pub fn skippable(&self) -> Option<Segment> {
        markers::skippable(&self.segments, self.position)
    }

    /// Whether to show the up-next card: from the credits (or near the end) of an episode
    /// with a next one, unless dismissed.
    pub fn show_up_next(&self) -> bool {
        if self.next.is_none() || self.next_cancelled || self.loading {
            return false;
        }
        let credits = self
            .segments
            .iter()
            .find(|s| s.kind == SegmentKind::Outro)
            .map(|s| s.start);
        let near_end = self.duration.map(|d| d.saturating_sub(UP_NEXT_BEFORE_END));
        credits.or(near_end).is_some_and(|from| self.position >= from)
    }

    /// The resume point to offer starting over from: shown for the first [`RESUME_PROMPT_FOR`]
    /// of playback from it, and gone after a seek elsewhere.
    pub fn resume_prompt(&self) -> Option<Duration> {
        let from = self.resumed_from.filter(|_| !self.loading)?;
        let window = from.saturating_sub(Duration::from_secs(2))..from + RESUME_PROMPT_FOR;
        window.contains(&self.position).then_some(from)
    }

    /// What plays after this item ends by itself.
    fn autoplay_next(&self) -> Option<BaseItemDto> {
        self.next.clone().filter(|_| self.autoplay && !self.next_cancelled)
    }

    pub fn is_transcoding(&self) -> bool {
        self.stream.method == PlayMethod::Transcode
    }

    /// The SyncPlay request a control becomes while a group controls playback; `None` for the
    /// ones that stay local (volume, tracks, delays).
    fn group_request(&self, command: &UiCommand) -> Option<GroupRequest> {
        match *command {
            UiCommand::TogglePause => Some(GroupRequest::SetPause(!self.paused)),
            UiCommand::SetPause(paused) => Some(GroupRequest::SetPause(paused)),
            UiCommand::SeekBy(seconds) => {
                let target = (self.position.as_secs_f64() + seconds).max(0.);
                Some(GroupRequest::Seek(Duration::from_secs_f64(target)))
            }
            UiCommand::SeekTo(position) => Some(GroupRequest::Seek(position)),
            UiCommand::Stop => Some(GroupRequest::Stop),
            _ => None,
        }
    }

    fn apply(&mut self, event: &PlayerEvent) {
        match event {
            PlayerEvent::FileLoaded => self.loading = false,
            PlayerEvent::Position(position) => {
                self.position = *position;
                self.position_at = Instant::now();
            }
            PlayerEvent::Duration(duration) => self.duration = Some(*duration),
            PlayerEvent::Paused(paused) => self.paused = *paused,
            PlayerEvent::Volume(volume) => self.volume = *volume,
            PlayerEvent::Muted(muted) => self.muted = *muted,
            PlayerEvent::Restarted => self.restarts += 1,
            _ => {}
        }
    }

    /// Takes the details fetched after the start. `queued`: the play queue has items, which take
    /// the place of the next episode.
    fn apply_details(&mut self, details: Details, queued: bool) {
        self.versions = details.versions;
        self.trickplay = details.trickplay;
        self.previous = details.previous;
        if !queued {
            self.next = details.next;
            // A sequel isn't started without asking; the card offers it.
            self.autoplay &= !details.next_in_collection;
        }
        self.chapters = details.chapters;
        self.segments = details.segments;
    }
}

/// The playback part of [`Services`]: the sessions, the local play queue, and what carries over
/// from one item to the next (volume, speed, subtitle size).
pub struct PlaybackManager {
    /// Wakes the UI when a session sends something, and for a reconnect that's due.
    ctx: egui::Context,
    client: Client,
    /// The signed-in account ([`crate::config::SavedServer::credential_account`]).
    account: String,
    /// Stops the running session when sent or dropped.
    stop: Option<oneshot::Sender<()>>,
    /// Counts sessions, so the end of a replaced session doesn't close its successor's player.
    generation: u64,
    /// A session is running (from `play` until its end is handled).
    active: bool,
    /// Sessions whose end hasn't been handled yet: the current one, and replaced ones still
    /// reporting their stop.
    sessions: Vec<Running>,
    /// A reconnect waiting to start after a lost stream. Stopping cancels it.
    reconnect: Option<Reconnect>,
    /// What plays after the current item ("Play all" on a playlist or collection). Takes the
    /// place of the next episode for the up-next card and autoplay.
    queue: VecDeque<BaseItemDto>,
    /// The running session's state, once it has started. A replaced session's stays until its
    /// successor starts, so the player has something to show meanwhile.
    now_playing: Option<NowPlaying>,
    volume: f64,
    muted: bool,
    adjustments: Adjustments,
    /// The item `adjustments` were made for; another one starts without the delays.
    adjusted_item: Option<String>,
    /// What the window's renderer can draw, which picks the player's video output. Pixels until
    /// the app sets it ([`PlaybackManager::set_video_import`]).
    video_import: VideoImport,
}

/// A session from its start until its end is handled.
struct Running {
    generation: u64,
    item: BaseItemDto,
    /// Counts reconnects after a lost stream.
    attempt: usize,
    /// What it was started with, to reconnect with.
    options: PlayOptions,
    updates: task::Receiver<SessionUpdate>,
    commands: mpsc::UnboundedSender<UiCommand>,
    /// What its [`NowPlaying`] starts with.
    volume: f64,
    muted: bool,
    adjustments: Adjustments,
    autoplay: bool,
    resumed_from: Option<Duration>,
}

/// A session to start again after its stream was lost, once `due` fires.
struct Reconnect {
    due: Deadline,
    item: BaseItemDto,
    options: PlayOptions,
    attempt: usize,
}

impl PlaybackManager {
    /// The manager for a signed-in session. Also reports a playback the last run didn't finish.
    pub fn new(ctx: &egui::Context, client: Client, account: String) -> Self {
        report_unfinished(&client, &account);
        let config = Config::load();
        let volume = config.volume.unwrap_or(DEFAULT_VOLUME);
        let adjustments = Adjustments {
            subtitle_scale: config.subtitle_scale.unwrap_or(1.),
            ..Adjustments::default()
        }
        .clamped();
        Self {
            ctx: ctx.clone(),
            client,
            account,
            stop: None,
            generation: 0,
            active: false,
            sessions: Vec::new(),
            reconnect: None,
            queue: VecDeque::new(),
            now_playing: None,
            volume,
            muted: false,
            adjustments,
            adjusted_item: None,
            video_import: VideoImport::Pixels,
        }
    }

    /// What the window's renderer can draw (`widgets::video::import`), for the sessions started
    /// from now on.
    pub fn set_video_import(&mut self, import: VideoImport) {
        self.video_import = import;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sets the subtitle size for the next items (the settings page; the player's menu goes
    /// through [`Services::adjust`]).
    pub fn set_subtitle_scale(&mut self, scale: f64) {
        self.adjustments = Adjustments {
            subtitle_scale: scale,
            ..self.adjustments
        }
        .clamped();
    }

    /// The signed-in session's client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The running session's state; `None` while loading, between sessions or reconnecting.
    pub fn now_playing(&self) -> Option<&NowPlaying> {
        self.now_playing.as_ref()
    }

    /// The running session's state, for the player page to change what's its own (dismissing
    /// the up-next card).
    pub fn now_playing_mut(&mut self) -> Option<&mut NowPlaying> {
        self.now_playing.as_mut()
    }

    /// The state of the current session, not of one being replaced: commands sent to it reach
    /// a session that's still going.
    fn current(&self) -> Option<&NowPlaying> {
        self.now_playing
            .as_ref()
            .filter(|state| state.session == self.generation)
    }

    /// The state of the session numbered `generation`, if it's the one held.
    fn state_of(&mut self, generation: u64) -> Option<&mut NowPlaying> {
        self.now_playing.as_mut().filter(|state| state.session == generation)
    }

    /// Stops the current session, if any. Returns whether that cancelled a pending reconnect,
    /// which closes the player (a running session closes it when it ends).
    fn stop(&mut self) -> bool {
        self.stop = None;
        self.reconnect.take().is_some()
    }

    /// Takes the queue's front when `item` is it, moving the queue on; playing anything else
    /// drops the queue, unless a SyncPlay group plays it.
    fn advance_queue(&mut self, item: &BaseItemDto, options: &PlayOptions) {
        if self.queue.front().is_some_and(|next| next.id == item.id) {
            self.queue.pop_front();
        } else if options.group_item.is_none() {
            self.queue.clear();
        }
    }

    /// Starts a session; `attempt` counts reconnects after a lost stream.
    fn start_session(&mut self, item: &BaseItemDto, options: PlayOptions, attempt: usize) {
        self.reconnect = None;
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);
        self.generation += 1;
        self.active = true;
        let (volume, muted, import) = (self.volume, self.muted, self.video_import);
        if self.adjusted_item.as_ref() != Some(&item.id) {
            self.adjustments = self.adjustments.for_next_item();
            self.adjusted_item = Some(item.id.clone());
        }
        let mut adjustments = self.adjustments;
        if options.group_item.is_some() {
            adjustments.speed = 1.;
        }
        let autoplay = Config::load().autoplay_next_episode.unwrap_or(true);
        let resumed_from = options.start.filter(|_| options.resumed).map(Duration::from);
        let (updates_tx, updates) = task::channel(&self.ctx);
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();

        {
            let (client, item, account) = (self.client.clone(), item.clone(), self.account.clone());
            let options = options.clone();
            // Not a `Task`: a session outlives the services that started it (signing out stops
            // it), so it can still report its stop.
            runtime::detach(async move {
                let channels = Channels {
                    stop: stop_rx,
                    updates: updates_tx.clone(),
                    commands: commands_rx,
                };
                let session = run_session(
                    client,
                    item,
                    options,
                    (volume, muted),
                    adjustments,
                    account,
                    import,
                    channels,
                );
                let result = AssertUnwindSafe(session).catch_unwind().await;
                let _ = updates_tx.send(SessionUpdate::Finished(result));
            });
        }

        self.sessions.push(Running {
            generation: self.generation,
            item: item.clone(),
            attempt,
            options,
            updates,
            commands: commands_tx,
            volume,
            muted,
            adjustments,
            autoplay,
            resumed_from,
        });
    }

    /// Everything the sessions sent since the last call, each with its session's generation.
    fn drain(&mut self) -> Vec<(u64, SessionUpdate)> {
        let mut updates = Vec::new();
        for session in &mut self.sessions {
            updates.extend(session.updates.drain().map(|update| (session.generation, update)));
        }
        updates
    }

    /// The reconnect to start now, once its delay is over.
    fn reconnect_due(&mut self) -> Option<Reconnect> {
        let reconnect = self.reconnect.as_mut()?;
        if reconnect.due.fire(&self.ctx) {
            self.reconnect.take()
        } else {
            None
        }
    }

    /// Remembers the volume and subtitle size for the next launch.
    fn save_preferences(&self) {
        save_preferences(self.volume, self.adjustments);
    }

    /// Stops every session and lets them finish on tokio, reporting their stop, then saves the
    /// volume. Signals on the returned channel when they're all done.
    fn finish(&mut self) -> std::sync::mpsc::Receiver<()> {
        self.stop = None;
        self.reconnect = None;
        let (done_tx, done) = std::sync::mpsc::channel();
        let sessions = std::mem::take(&mut self.sessions);
        if sessions.is_empty() {
            let _ = done_tx.send(());
            return done;
        }
        let (current, mut volume, adjustments) = (self.generation, self.volume, self.adjustments);
        runtime::detach(async move {
            for mut session in sessions {
                while let Some(update) = session.updates.recv().await {
                    match update {
                        SessionUpdate::Player(PlayerEvent::Volume(level)) if session.generation == current => {
                            volume = level;
                        }
                        SessionUpdate::Finished(_) => break,
                        _ => {}
                    }
                }
            }
            save_preferences(volume, adjustments);
            let _ = done_tx.send(());
        });
        done
    }

    /// Quitting the app: stops playback, letting the sessions report it in the background.
    /// Returns what [`wait_for_exit`] waits on, if a session was running.
    pub fn stop_for_exit(&mut self) -> Option<std::sync::mpsc::Receiver<()>> {
        let running = !self.sessions.is_empty();
        let done = self.finish();
        running.then_some(done)
    }
}

/// Waits (a little, [`EXIT_WAIT`] in all) for the sessions [`PlaybackManager::stop_for_exit`]
/// stopped to report it, since the process ends right after.
pub fn wait_for_exit(stopping: Vec<std::sync::mpsc::Receiver<()>>) {
    if stopping.is_empty() {
        return;
    }
    tracing::info!("stopping playback before quitting");
    let deadline = Instant::now() + EXIT_WAIT;
    for done in stopping {
        if done
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_err()
        {
            tracing::warn!("playback didn't stop in time; the next launch reports it");
            return;
        }
    }
}

/// Dropped with the signed-in session (after `Services::end` on sign-out, an expired session or
/// switching accounts): the running sessions stop, if `end` didn't already, and report it in
/// the background.
impl Drop for PlaybackManager {
    fn drop(&mut self) {
        drop(self.finish());
    }
}

/// Remembers the volume and subtitle size for the next launch.
fn save_preferences(volume: f64, adjustments: Adjustments) {
    let mut config = Config::load();
    let scale = Some(adjustments.subtitle_scale).filter(|&scale| scale != 1.);
    if config.volume != Some(volume) || config.subtitle_scale != scale {
        config.volume = Some(volume);
        config.subtitle_scale = scale;
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
    }
}

/// Playing, and the controls: these may go to a SyncPlay group instead of the local player.
impl Services {
    /// A control from the player UI (or remote control). In a SyncPlay group, pause, seek and
    /// stop are requests to the group, which then commands every member. Stop with no session
    /// to send it to (waiting to reconnect, or the next one still loading) stops that instead.
    pub fn control(&mut self, command: UiCommand) {
        let Some(state) = self.playback.current() else {
            if matches!(command, UiCommand::Stop) {
                self.stop_playback();
            }
            return;
        };
        if state.group_item.is_some()
            && let Some(request) = state.group_request(&command)
        {
            self.group_request(request);
            return;
        }
        state.send(command);
    }

    /// Stops the current session, if any, or a pending reconnect (closing the player).
    pub fn stop_playback(&mut self) {
        if self.playback.stop() {
            self.events.push(AppEvent::PlaybackEnded);
        }
    }

    pub fn play(&mut self, item: &BaseItemDto, options: PlayOptions) {
        // Playing the queue's next item moves the queue on; playing anything else drops it.
        self.playback.advance_queue(item, &options);
        self.play_now(item, options);
    }

    /// Plays an item from its resume point, with default tracks (e.g. the next episode).
    pub fn play_from_resume(&mut self, item: &BaseItemDto) {
        let start = item
            .user_data
            .as_ref()
            .map(|d| d.playback_position_ticks)
            .filter(|t| t.0 > 0);
        self.play(
            item,
            PlayOptions {
                start,
                resumed: start.is_some(),
                ..Default::default()
            },
        );
    }

    /// Plays `items` in order: the first now, the rest as a queue.
    pub fn play_all(&mut self, items: Vec<BaseItemDto>) {
        self.play_all_with(items, PlayOptions::default());
    }

    /// Plays `items` in order, the first with `options`. A SyncPlay group gets them as its
    /// new queue instead, from the start.
    pub fn play_all_with(&mut self, items: Vec<BaseItemDto>, options: PlayOptions) {
        let mut items = VecDeque::from(items);
        let Some(first) = items.pop_front() else { return };
        if self.in_group() {
            let mut all = vec![first];
            all.extend(items);
            self.group_start_items(&all);
            return;
        }
        self.playback.queue = items;
        self.play_now(&first, options);
    }

    /// Adds `items` to the queue: right after the playing item with `next`, otherwise at the
    /// end. With nothing playing they play now.
    pub fn enqueue(&mut self, items: Vec<BaseItemDto>, next: bool) {
        if self.in_group() {
            self.group_enqueue(&items, next);
            return;
        }
        let playback = &mut self.playback;
        if !playback.active {
            self.play_all(items);
            return;
        }
        if next {
            for item in items.into_iter().rev() {
                playback.queue.push_front(item);
            }
        } else {
            playback.queue.extend(items);
        }
        // The queue takes the place of the next episode for the up-next card and autoplay.
        if let (Some(state), Some(front)) = (&mut playback.now_playing, playback.queue.front()) {
            state.next = Some(front.clone());
        }
    }

    fn play_now(&mut self, item: &BaseItemDto, options: PlayOptions) {
        // In a SyncPlay group, starting something means giving the group a new queue; the
        // server then has every member (this one included) load it.
        if options.group_item.is_none() && self.in_group() {
            let start = options.start.unwrap_or_default();
            self.group_start_queue(item, start);
            return;
        }
        self.playback.start_session(item, options, 0);
    }

    /// Plays the next episode (or queued item, or movie of the collection) now.
    pub fn play_next(&mut self) {
        if let Some(next) = self.playback.now_playing().and_then(|state| state.next.clone()) {
            self.play_from_resume(&next);
        }
    }

    /// Plays the previous episode (or movie of the collection) from its resume point.
    pub fn play_previous(&mut self) {
        if let Some(previous) = self.playback.now_playing().and_then(|state| state.previous.clone()) {
            self.play_from_resume(&previous);
        }
    }

    /// Plays from the beginning instead of the resume point.
    pub fn start_over(&mut self) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        state.resumed_from = None;
        self.control(UiCommand::SeekTo(Duration::ZERO));
    }

    /// Switches the audio track. A transcode carries only the chosen track, so it restarts
    /// at the current position.
    pub fn change_audio(&mut self, index: i32) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if state.audio == Some(index) {
            return;
        }
        state.audio = Some(index);
        if state.is_transcoding() {
            self.restart();
        } else {
            state.send(UiCommand::Audio(index));
        }
    }

    /// Switches subtitles. A transcode restarts if a burned-in subtitle has to go, or the new
    /// one has to be burned in.
    pub fn change_subtitle(&mut self, choice: SubtitleChoice) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if state.subtitle == choice {
            return;
        }
        state.subtitle = choice;
        let as_file = match choice {
            SubtitleChoice::Stream(index) => state.stream.subtitle_files.contains(&index),
            SubtitleChoice::Off | SubtitleChoice::Auto => true,
        };
        if state.is_transcoding() && (state.stream.burned_subtitle.is_some() || !as_file) {
            self.restart();
        } else {
            state.send(UiCommand::Subtitle(choice));
        }
    }

    /// Applies changes from the settings menu. Speed stays at normal in a SyncPlay group,
    /// whose drift correction uses it.
    pub fn adjust(&mut self, adjustments: Adjustments) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        let mut new = adjustments.clamped();
        if state.group_item.is_some() {
            new.speed = 1.;
        }
        let old = state.adjustments;
        if new == old {
            return;
        }
        state.adjustments = new;
        let changes = [
            (new.speed != old.speed).then_some(UiCommand::SetSpeed(new.speed)),
            (new.subtitle_delay != old.subtitle_delay).then_some(UiCommand::SetSubtitleDelay(new.subtitle_delay)),
            (new.audio_delay != old.audio_delay).then_some(UiCommand::SetAudioDelay(new.audio_delay)),
            (new.subtitle_scale != old.subtitle_scale).then_some(UiCommand::SetSubtitleScale(new.subtitle_scale)),
        ];
        for command in changes.into_iter().flatten() {
            state.send(command);
        }
        self.playback.adjustments = new;
    }

    /// Plays the same version again from the current position with the current tracks.
    fn restart(&mut self) {
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        let options = PlayOptions {
            start: Some(Ticks::from(state.position)),
            media_source_id: Some(state.source.id.clone()),
            audio_stream_index: state.audio,
            subtitle: state.subtitle,
            ..Default::default()
        };
        self.reload(options);
    }

    /// Loads the playing item again here only, e.g. in another version: at the current
    /// position, or `options.start`. In a SyncPlay group the others wait meanwhile
    /// (`SyncPlay::reload`), rather than the group getting a new queue as when something new is
    /// played.
    pub fn reload(&mut self, mut options: PlayOptions) {
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        options.start = options.start.or(Some(Ticks::from(state.position)));
        let item = state.item.clone();
        if state.group_item.is_some() {
            self.group_reload(&item, options);
        } else {
            self.play(&item, options);
        }
    }

    /// Takes in what the sessions sent, and starts a reconnect that's due. From
    /// [`Services::tick`], so it goes on while the window is hidden.
    pub(crate) fn tick_playback(&mut self) {
        for (generation, update) in self.playback.drain() {
            self.on_session_update(generation, update);
        }
        if let Some(reconnect) = self.playback.reconnect_due() {
            let Reconnect {
                item, options, attempt, ..
            } = reconnect;
            self.playback.start_session(&item, options, attempt);
        }
    }

    fn on_session_update(&mut self, generation: u64, update: SessionUpdate) {
        let playback = &mut self.playback;
        match update {
            SessionUpdate::Started {
                source,
                video,
                audio,
                subtitle,
                stream,
            } => {
                // A replaced session that got this far is stopping; its player never shows.
                if generation != playback.generation {
                    return;
                }
                let Some(running) = playback.sessions.iter().find(|s| s.generation == generation) else {
                    return;
                };
                playback.now_playing = Some(NowPlaying {
                    session: generation,
                    item: running.item.clone(),
                    source: *source,
                    video,
                    position: Duration::ZERO,
                    position_at: Instant::now(),
                    duration: None,
                    paused: false,
                    loading: true,
                    audio,
                    subtitle,
                    volume: running.volume,
                    muted: running.muted,
                    adjustments: running.adjustments,
                    versions: Vec::new(),
                    trickplay: TrickplayTiles::new(),
                    previous: None,
                    // Read when the session starts, not when it was asked for: a remote play
                    // next or last queued while it was loading counts too.
                    next: playback.queue.front().cloned(),
                    next_cancelled: false,
                    autoplay: running.autoplay,
                    chapters: Vec::new(),
                    segments: Vec::new(),
                    stream,
                    group_item: running.options.group_item.clone(),
                    resumed_from: running.resumed_from,
                    restarts: 0,
                    commands: running.commands.clone(),
                });
                self.events.push(AppEvent::PlaybackStarted);
                self.group_playback_started();
            }
            SessionUpdate::Details(details) => {
                // Items queued since the start (remote control) count too.
                let queued = !playback.queue.is_empty();
                if let Some(state) = playback.state_of(generation) {
                    state.apply_details(*details, queued);
                }
            }
            SessionUpdate::Player(event) => {
                if generation == playback.generation {
                    match event {
                        PlayerEvent::Volume(volume) => playback.volume = volume,
                        PlayerEvent::Muted(muted) => playback.muted = muted,
                        _ => {}
                    }
                }
                if let Some(state) = playback.state_of(generation) {
                    state.apply(&event);
                }
            }
            SessionUpdate::Finished(result) => {
                let result = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                let Some(index) = playback.sessions.iter().position(|s| s.generation == generation) else {
                    return;
                };
                let running = playback.sessions.remove(index);
                self.session_ended(running, result);
            }
        }
    }

    fn session_ended(&mut self, running: Running, result: Result<Ended, String>) {
        self.playback.save_preferences();
        // Watched state and resume points may have changed.
        self.user_data += 1;
        // A newer session took over (another item, version or episode); its player is
        // already showing.
        if running.generation != self.playback.generation {
            return;
        }
        let playback = &mut self.playback;
        playback.active = false;
        let next = playback
            .now_playing
            .take()
            .filter(|state| state.session == running.generation)
            .and_then(|state| state.autoplay_next());
        let group_item = running.options.group_item.clone();
        match result {
            // In a group, every member reports the end and the server moves the queue on
            // (once); the next item then comes as a queue update.
            Ok(Ended::Finished) if let Some(group_item) = &group_item => {
                self.group_item_ended(group_item);
                self.events.push(AppEvent::PlaybackEnded);
            }
            Ok(Ended::Finished) if let Some(next) = next => {
                tracing::info!(next = %next.name, "playing the next episode");
                self.play_from_resume(&next);
            }
            // Keep the player up and try again from where it stopped.
            Ok(Ended::Lost {
                position,
                message,
                crashed,
            }) if group_item.is_none() && running.attempt < RECONNECT_DELAYS.len() => {
                let delay = RECONNECT_DELAYS[running.attempt];
                tracing::warn!(attempt = running.attempt, ?delay, "stream lost: {message}");
                let notice = if crashed {
                    "The player stopped unexpectedly. Restarting it…"
                } else {
                    "Lost the connection to the server. Reconnecting…"
                };
                self.toast(Toast::info(notice));
                let mut due = Deadline::default();
                due.set(delay);
                self.playback.reconnect = Some(Reconnect {
                    due,
                    item: running.item,
                    options: PlayOptions {
                        start: Some(position),
                        resumed: false,
                        ..running.options
                    },
                    attempt: running.attempt + 1,
                });
            }
            Ok(Ended::Lost { message, crashed, .. }) => {
                tracing::warn!("stream lost for good: {message}");
                let error = if crashed {
                    "The player keeps crashing on this item. The log may say why."
                } else {
                    "Lost the connection to the server. Try again when it's back."
                };
                self.toast(Toast::error(error));
                self.events.push(AppEvent::PlaybackEnded);
            }
            Ok(_) => self.events.push(AppEvent::PlaybackEnded),
            Err(message) => {
                tracing::warn!("playback failed: {message}");
                self.toast(Toast::error(message));
                self.events.push(AppEvent::PlaybackEnded);
            }
        }
    }
}

/// How a session ended without an error.
enum Ended {
    /// Played to the end.
    Finished,
    Stopped,
    /// The stream failed (or ended early) after playback had started, at `position`: likely
    /// the network, so worth retrying.
    Lost {
        position: Ticks,
        message: String,
        /// The player process crashed, rather than the stream failing.
        crashed: bool,
    },
}

/// Waits before each reconnect attempt after a lost stream.
const RECONNECT_DELAYS: [Duration; 3] = [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)];
/// An end of file this long before the duration is a dropped stream, not the end.
const EARLY_END: Duration = Duration::from_secs(10);

struct Channels {
    stop: oneshot::Receiver<()>,
    updates: task::Sender<SessionUpdate>,
    commands: mpsc::UnboundedReceiver<UiCommand>,
}

/// The item as mpv's window title and the shell's now-playing line show it.
pub fn title(item: &BaseItemDto) -> String {
    match (item.kind, &item.series_name) {
        (BaseItemKind::Episode, Some(series)) => {
            let label = crate::widgets::text::episode_label(item);
            format!("{series} {label} - {}", item.name)
        }
        _ => item.name.clone(),
    }
}

/// mpv numbers the tracks of each type in a file from 1, in stream order. External subtitles
/// are added separately, so only embedded streams count.
fn mpv_track_id(source: &MediaSourceInfo, kind: MediaStreamType, index: i32) -> Option<usize> {
    let mut embedded: Vec<&MediaStream> = source
        .media_streams
        .iter()
        .filter(|s| s.kind == kind && !s.is_external)
        .collect();
    embedded.sort_by_key(|s| s.index);
    embedded.iter().position(|s| s.index == index).map(|i| i + 1)
}

/// mpv's id for a subtitle once the file is loaded and the external ones were added (after
/// the embedded tracks, in `media_streams` order).
fn mpv_subtitle_id(source: &MediaSourceInfo, index: i32) -> Option<usize> {
    if let Some(id) = mpv_track_id(source, MediaStreamType::Subtitle, index) {
        return Some(id);
    }
    let subtitles = |external| {
        source
            .media_streams
            .iter()
            .filter(move |s| s.kind == MediaStreamType::Subtitle && s.is_external == external)
    };
    let position = subtitles(true).position(|s| s.index == index)?;
    Some(subtitles(false).count() + position + 1)
}

/// The original file when the server allows direct play, else its HLS transcode.
fn stream_url(
    client: &Client,
    item_id: &str,
    source: &MediaSourceInfo,
    play_session_id: Option<&str>,
) -> Result<(Url, PlayMethod), String> {
    if source.supports_direct_play || source.supports_direct_stream {
        return Ok((
            client.stream_url(item_id, &source.id, play_session_id),
            PlayMethod::DirectPlay,
        ));
    }
    let relative = source
        .transcoding_url
        .as_deref()
        .ok_or("The server can't stream this item to this app.")?;
    let url = client.transcode_url(relative);
    let reasons = query_param(&url, "TranscodeReasons").unwrap_or_default();
    tracing::info!(%reasons, "the server is transcoding");
    Ok((url, PlayMethod::Transcode))
}

fn query_param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Subtitles mpv loads as separate files: external ones when direct playing; when
/// transcoding, those the server sends as files (embedded text subtitles included).
fn subtitle_files(
    client: &Client,
    item_id: &str,
    source: &MediaSourceInfo,
    method: PlayMethod,
) -> Vec<(MediaStream, Url)> {
    source
        .media_streams
        .iter()
        .filter(|s| s.kind == MediaStreamType::Subtitle)
        .filter_map(|stream| {
            let url = match method {
                PlayMethod::DirectPlay if stream.is_external => client.subtitle_url(item_id, &source.id, stream),
                PlayMethod::Transcode if stream.delivery_method.as_deref() == Some("External") => {
                    client.transcode_url(stream.delivery_url.as_deref()?)
                }
                _ => return None,
            };
            Some((stream.clone(), url))
        })
        .collect()
}

fn subtitle_index(choice: SubtitleChoice) -> Option<i32> {
    match choice {
        SubtitleChoice::Stream(index) => Some(index),
        SubtitleChoice::Off => Some(-1),
        SubtitleChoice::Auto => None,
    }
}

async fn start_player(
    import: VideoImport,
) -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput), String> {
    Player::spawn(import).await.map_err(|err| match err {
        player::Error::LibmpvUnavailable(reason) => {
            tracing::warn!("libmpv unavailable: {reason}");
            let hint = if cfg!(target_os = "macos") {
                "Install mpv with Homebrew (brew install mpv)."
            } else if cfg!(windows) {
                "libmpv-2.dll is missing next to tabbyfin.exe; reinstall the app."
            } else {
                "Install libmpv (the libmpv2 package on Debian and Ubuntu)."
            };
            format!("Video playback needs libmpv, which wasn't found. {hint}")
        }
        err => format!("Couldn't start the player: {err}"),
    })
}

/// Versions, chapters, segments and the next episode, for the player's controls. Failures
/// only lose those controls.
async fn details(client: &Client, item: &BaseItemDto) -> Details {
    let (full, segments, neighbours) = tokio::join!(
        client.item(
            &item.id,
            &[ItemField::MediaSources, ItemField::Chapters, ItemField::Trickplay]
        ),
        client.media_segments(&item.id),
        neighbours(client, item),
    );
    let full = full
        .inspect_err(|err| tracing::warn!("couldn't load versions and chapters: {err}"))
        .ok();
    let neighbours = neighbours
        .inspect_err(|err| tracing::warn!("couldn't find the previous and next items: {err}"))
        .unwrap_or_default();
    let sources = full.as_ref().map(|f| f.media_sources.clone()).unwrap_or_default();
    let chapters = full
        .as_ref()
        .map(|f| markers::chapters(&f.chapters))
        .unwrap_or_default();
    let segments = match segments {
        Ok(segments) if !segments.is_empty() => markers::from_server(&segments),
        Ok(_) => markers::from_chapters(&chapters, item.run_time_ticks.map(Duration::from)),
        Err(err) => {
            tracing::warn!("couldn't load media segments: {err}");
            markers::from_chapters(&chapters, item.run_time_ticks.map(Duration::from))
        }
    };
    let versions = sources
        .iter()
        .enumerate()
        .map(|(i, source)| Version {
            id: source.id.clone(),
            name: source.name.clone().unwrap_or_else(|| format!("Version {}", i + 1)),
        })
        .collect();
    Details {
        versions,
        trickplay: full.map(|full| full.trickplay).unwrap_or_default(),
        previous: neighbours.previous,
        next: neighbours.next,
        next_in_collection: neighbours.in_collection,
        chapters,
        segments,
    }
}

#[derive(Default)]
struct Neighbours {
    previous: Option<BaseItemDto>,
    next: Option<BaseItemDto>,
    /// They're movies of the item's collection, not episodes.
    in_collection: bool,
}

/// The episodes around an episode, across seasons; or for a movie, the movies around it in
/// its first collection, in the collection page's order.
async fn neighbours(client: &Client, item: &BaseItemDto) -> jellyfin::Result<Neighbours> {
    match (item.kind, &item.series_id) {
        (BaseItemKind::Episode, Some(series_id)) => {
            let episodes = client.episodes(series_id, None, &[]).await?.items;
            let (previous, next) = adjacent(episodes, &item.id);
            Ok(Neighbours {
                previous,
                next,
                in_collection: false,
            })
        }
        (BaseItemKind::Movie | BaseItemKind::Video, _) => {
            let Some(collection) = client.item_collections(&item.id).await?.into_iter().next() else {
                return Ok(Neighbours::default());
            };
            let mut movies = crate::views::item::collection_items(client, &collection.id, &[]).await?;
            movies.retain(|movie| matches!(movie.kind, BaseItemKind::Movie | BaseItemKind::Video));
            let (previous, next) = adjacent(movies, &item.id);
            Ok(Neighbours {
                previous,
                next,
                in_collection: true,
            })
        }
        _ => Ok(Neighbours::default()),
    }
}

/// The items before and after the one with `id`, if it's there.
fn adjacent(mut items: Vec<BaseItemDto>, id: &str) -> (Option<BaseItemDto>, Option<BaseItemDto>) {
    let Some(index) = items.iter().position(|item| item.id == id) else {
        return (None, None);
    };
    let next = (index + 1 < items.len()).then(|| items.remove(index + 1));
    let previous = index.checked_sub(1).map(|i| items.swap_remove(i));
    (previous, next)
}

/// How long a connection measurement is reused.
const BITRATE_TEST_TTL: Duration = Duration::from_secs(10 * 60);
/// The share of the measured speed to stream at, leaving room for other traffic and dips.
const BITRATE_HEADROOM: f64 = 0.8;
/// Measured above this, the connection isn't the limit: no cap.
const BITRATE_NO_LIMIT: u64 = 200_000_000;

/// The last connection measurement: when, and the cap it gave (`None`: no limit).
static MEASURED_BITRATE: std::sync::Mutex<Option<(Instant, Option<u64>)>> = std::sync::Mutex::new(None);

/// The bitrate cap for the next playback: the setting, or with *Automatic* a measurement of
/// the connection (reused for a while).
async fn max_bitrate(client: &Client) -> Option<u64> {
    let config = Config::load();
    if !config.automatic_bitrate.unwrap_or(false) {
        return config.max_streaming_bitrate;
    }
    if let Some((at, cap)) = *MEASURED_BITRATE.lock().unwrap()
        && at.elapsed() < BITRATE_TEST_TTL
    {
        return cap;
    }
    let cap = match measure_bitrate(client).await {
        Ok(bits) => {
            let cap = (bits < BITRATE_NO_LIMIT).then_some((bits as f64 * BITRATE_HEADROOM) as u64);
            tracing::info!(measured_mbps = bits / 1_000_000, cap_mbps = ?cap.map(|c| c / 1_000_000), "connection measured");
            cap
        }
        Err(err) => {
            // Without a measurement, fall back to the fixed setting.
            tracing::warn!("couldn't measure the connection: {err}");
            return config.max_streaming_bitrate;
        }
    };
    *MEASURED_BITRATE.lock().unwrap() = Some((Instant::now(), cap));
    cap
}

/// Like the web client: a small download first, and a bigger one when that was quick, so a
/// fast connection isn't judged by its latency.
async fn measure_bitrate(client: &Client) -> jellyfin::Result<u64> {
    let small = client.bitrate_test(500_000).await?;
    if small < 10_000_000 {
        return Ok(small);
    }
    client.bitrate_test(3_000_000).await
}

/// Runs one playback from negotiation to the final stop report. Returns a user-facing error.
#[allow(clippy::too_many_arguments)]
async fn run_session(
    client: Client,
    item: BaseItemDto,
    options: PlayOptions,
    (volume, muted): (f64, bool),
    adjustments: Adjustments,
    account: String,
    import: VideoImport,
    mut channels: Channels,
) -> Result<Ended, String> {
    let info = client
        .playback_info(
            &item.id,
            &PlaybackInfoRequest {
                media_source_id: options.media_source_id.clone(),
                start_position: options.start,
                audio_stream_index: options.audio_stream_index,
                subtitle_stream_index: subtitle_index(options.subtitle),
                max_streaming_bitrate: max_bitrate(&client).await,
                prefer_hevc: Config::load().prefer_hevc.unwrap_or(false),
            },
        )
        .await
        .map_err(|err| describe_error(&err))?;
    if let Some(code) = info.error_code {
        return Err(format!("The server won't play this item ({code})."));
    }
    let source = match &options.media_source_id {
        Some(id) => info.media_sources.into_iter().find(|s| &s.id == id),
        None => info.media_sources.into_iter().next(),
    }
    .ok_or("The server returned no playable media.")?;
    let (url, method) = stream_url(&client, &item.id, &source, info.play_session_id.as_deref())?;
    let subtitle_files = subtitle_files(&client, &item.id, &source, method);
    // Tracks not chosen explicitly (autoplay, the next episode, SyncPlay) follow the server's
    // defaults, which apply the user's language preferences and subtitle mode; mpv on its own
    // would go by the file's default flags. A transcode names its audio track in the URL.
    let audio = options.audio_stream_index.or_else(|| match method {
        PlayMethod::Transcode => query_param(&url, "AudioStreamIndex").and_then(|i| i.parse().ok()),
        PlayMethod::DirectPlay => source.default_audio_stream_index,
    });
    let subtitle = match (options.subtitle, source.default_subtitle_stream_index) {
        (SubtitleChoice::Auto, Some(-1)) => SubtitleChoice::Off,
        (SubtitleChoice::Auto, Some(index)) => SubtitleChoice::Stream(index),
        (choice, _) => choice,
    };
    let stream = StreamInfo {
        method,
        subtitle_files: subtitle_files.iter().map(|(stream, _)| stream.index).collect(),
        burned_subtitle: (method == PlayMethod::Transcode)
            .then(|| {
                source
                    .media_streams
                    .iter()
                    .find(|s| s.delivery_method.as_deref() == Some("Encode"))
            })
            .flatten()
            .map(|s| s.index),
    };

    let (player, mut events, video) = start_player(import).await?;

    // Track choices are options, so they're set before loading the file. A transcode has
    // just the chosen audio track and no embedded subtitles.
    let direct = method == PlayMethod::DirectPlay;
    if let Some(aid) = audio
        .filter(|_| direct)
        .and_then(|index| mpv_track_id(&source, MediaStreamType::Audio, index))
    {
        player.set_property("aid", aid).await.map_err(|err| err.to_string())?;
    }
    let file_selected = match subtitle {
        // The server's default, when it sends that one as a file.
        SubtitleChoice::Auto if !direct => source
            .default_subtitle_stream_index
            .filter(|index| stream.subtitle_files.contains(index)),
        SubtitleChoice::Auto => None,
        SubtitleChoice::Off => {
            player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
            None
        }
        SubtitleChoice::Stream(index) => match mpv_track_id(&source, MediaStreamType::Subtitle, index) {
            Some(sid) if direct => {
                player.set_property("sid", sid).await.map_err(|err| err.to_string())?;
                None
            }
            // A file: no embedded track until it's added and selected after loading.
            _ => {
                player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
                Some(index)
            }
        },
    };

    if options.start_paused {
        player
            .set_property("pause", true)
            .await
            .map_err(|err| err.to_string())?;
    }
    player
        .set_property("volume", volume)
        .await
        .map_err(|err| err.to_string())?;
    player
        .set_property("mute", muted)
        .await
        .map_err(|err| err.to_string())?;
    // Cosmetic, so a property this mpv doesn't have is skipped: before 0.38 there's no
    // `sub-border-style`, and a visible `sub-back-color` draws the box on its own.
    let config = Config::load();
    for (name, value) in subtitle_look(
        config.subtitle_color.unwrap_or_default(),
        config.subtitle_style.unwrap_or_default(),
    ) {
        if let Err(err) = player.set_property(name, value).await {
            tracing::debug!("mpv didn't take {name}={value}: {err}");
        }
    }
    for (name, value) in [
        ("speed", adjustments.speed),
        ("sub-delay", adjustments.subtitle_delay),
        ("audio-delay", adjustments.audio_delay),
        ("sub-scale", adjustments.subtitle_scale),
    ] {
        player.set_property(name, value).await.map_err(|err| err.to_string())?;
    }

    player
        .load(LoadRequest {
            url: url.to_string(),
            title: Some(title(&item)),
            start: options.start.map(Duration::from),
            http_headers: vec![format!("Authorization: {}", client.authorization_header())],
        })
        .await
        .map_err(|err| format!("mpv couldn't open the stream: {err}"))?;

    let _ = channels.updates.send(SessionUpdate::Started {
        source: Box::new(source.clone()),
        video,
        audio,
        subtitle,
        stream: stream.clone(),
    });
    let details_task = {
        let (client, item, updates) = (client.clone(), item.clone(), channels.updates.clone());
        tokio::spawn(async move {
            let details = details(&client, &item).await;
            let _ = updates.send(SessionUpdate::Details(Box::new(details)));
        })
    };

    let mut report = PlaybackReport {
        item_id: item.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: info.play_session_id.clone(),
        position: options.start.unwrap_or_default(),
        audio_stream_index: audio,
        subtitle_stream_index: subtitle_index(subtitle),
        play_method: method,
        volume_level: volume_level(volume),
        is_muted: muted,
        ..Default::default()
    };
    let mut reporting = Reporting::default();
    let mut stopping = false;
    let mut finished = false;
    let mut error = None;
    let mut duration = None;
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                tracing::trace!(?event, "player event");
                let _ = channels.updates.send(SessionUpdate::Player(event.clone()));
                match &event {
                    // External subtitles go in before the start report.
                    PlayerEvent::FileLoaded => {
                        for (stream, url) in &subtitle_files {
                            let title = stream.display_title.as_deref().unwrap_or("External");
                            let select = file_selected == Some(stream.index);
                            if let Err(err) = player.add_subtitle(url.as_str(), title, stream.language.as_deref(), select).await {
                                tracing::warn!("couldn't add subtitle: {err}");
                            }
                        }
                    }
                    PlayerEvent::Duration(length) => duration = Some(*length),
                    PlayerEvent::EndFile(EndReason::Eof) if !stopping => finished = true,
                    PlayerEvent::EndFile(EndReason::Error(message)) => {
                        error = Some(format!("mpv couldn't play this file: {message}"));
                    }
                    PlayerEvent::Exited => break,
                    _ => {}
                }
                match reporting.on_event(&event, &mut report, tokio::time::Instant::now()) {
                    Outgoing::Start => {
                        report_or_log(client.report_playback_start(&report).await, "start");
                        save_unfinished(Some(&report), &account);
                    }
                    Outgoing::Progress => report_or_log(client.report_playback_progress(&report).await, "progress"),
                    Outgoing::Nothing => {}
                }
            }
            Some(command) = channels.commands.recv() => {
                tracing::debug!(?command, "player command");
                let send = reporting.on_command(&command);
                if let Err(err) = apply_command(&player, &source, &stream, &mut report, command).await {
                    tracing::warn!("player command failed: {err}");
                }
                if send == Outgoing::Progress {
                    report_or_log(client.report_playback_progress(&report).await, "progress");
                }
            }
            _ = tokio::time::sleep_until(reporting.volume_due.unwrap_or_else(tokio::time::Instant::now)), if reporting.volume_due.is_some() => {
                reporting.volume_reported();
                report_or_log(client.report_playback_progress(&report).await, "progress");
            }
            _ = ticker.tick(), if reporting.started => {
                reporting.volume_reported();
                report_or_log(client.report_playback_progress(&report).await, "progress");
                save_unfinished(Some(&report), &account);
            }
            _ = &mut channels.stop, if !stopping => {
                // Replaced by a new session or signed out: ask mpv to quit and keep going until
                // it exits, so the final position gets reported.
                stopping = true;
                let _ = player.quit().await;
            }
        }
    }

    details_task.abort();
    let started = reporting.started;
    if started {
        report_or_log(client.report_playback_stopped(&report).await, "stop");
        save_unfinished(None, &account);
    }
    if let (PlayMethod::Transcode, Some(id)) = (method, &info.play_session_id) {
        report_or_log(client.stop_transcoding(id).await, "transcode stop");
    }
    tracing::info!(item = %item.name, position = report.position.as_secs_f64(), "playback ended");
    let ended_early = duration.is_some_and(|d| Duration::from(report.position) + EARLY_END < d);
    match error {
        // A failure right at the end isn't worth replaying from there.
        Some(message) if started && !stopping && ended_early => Ok(Ended::Lost {
            position: report.position,
            crashed: message.ends_with(player::HOST_CRASHED),
            message,
        }),
        Some(error) => Err(error),
        None if finished && ended_early => Ok(Ended::Lost {
            position: report.position,
            message: "the stream ended early".into(),
            crashed: false,
        }),
        None if finished => Ok(Ended::Finished),
        None => Ok(Ended::Stopped),
    }
}

/// A report to send to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outgoing {
    Nothing,
    Start,
    Progress,
}

/// When a session reports to the server, from the player's events and the user's commands.
/// `run_session` sends what this decides; the stop report goes out at the end if it started.
#[derive(Debug, Default)]
struct Reporting {
    /// The start report went out (mpv loaded the file). Nothing is reported before it, since
    /// the start report carries the state itself.
    started: bool,
    /// When the batched volume and mute report is due: a wheel spin or a held key sends one
    /// report, a second after the first change.
    volume_due: Option<tokio::time::Instant>,
}

impl Reporting {
    /// Updates `report` from a player event and says what to send.
    fn on_event(&mut self, event: &PlayerEvent, report: &mut PlaybackReport, now: tokio::time::Instant) -> Outgoing {
        match event {
            PlayerEvent::FileLoaded => {
                self.started = true;
                return Outgoing::Start;
            }
            PlayerEvent::Position(position) => report.position = Ticks::from(*position),
            PlayerEvent::Volume(volume) => {
                report.volume_level = volume_level(*volume);
                self.schedule_volume(now);
            }
            PlayerEvent::Muted(muted) => {
                report.is_muted = *muted;
                self.schedule_volume(now);
            }
            PlayerEvent::Paused(paused) => {
                report.is_paused = *paused;
                return self.progress();
            }
            PlayerEvent::Seeked => return self.progress(),
            _ => {}
        }
        Outgoing::Nothing
    }

    /// Commands that change what the reports carry are reported once applied. Volume and mute
    /// are reported from mpv's events instead, batched.
    fn on_command(&self, command: &UiCommand) -> Outgoing {
        match command {
            UiCommand::TogglePause
            | UiCommand::SetPause(_)
            | UiCommand::SeekBy(_)
            | UiCommand::SeekTo(_)
            | UiCommand::Audio(_)
            | UiCommand::Subtitle(_) => self.progress(),
            _ => Outgoing::Nothing,
        }
    }

    /// A progress report went out, which carries the volume too.
    fn volume_reported(&mut self) {
        self.volume_due = None;
    }

    fn schedule_volume(&mut self, now: tokio::time::Instant) {
        if self.started {
            self.volume_due.get_or_insert(now + VOLUME_REPORT_AFTER);
        }
    }

    fn progress(&self) -> Outgoing {
        if self.started {
            Outgoing::Progress
        } else {
            Outgoing::Nothing
        }
    }
}

/// mpv options for the text subtitle appearance from the settings. ASS subtitles keep their own
/// styling (mpv only scales them).
fn subtitle_look(color: SubtitleColor, style: SubtitleStyle) -> [(&'static str, &'static str); 3] {
    let color = match color {
        SubtitleColor::White => "#FFFFFFFF",
        SubtitleColor::Yellow => "#FFFFE14D",
    };
    let (border_style, back_color) = match style {
        SubtitleStyle::Outline => ("outline-and-shadow", "#00000000"),
        SubtitleStyle::Box => ("background-box", "#B0000000"),
    };
    [
        ("sub-color", color),
        ("sub-border-style", border_style),
        ("sub-back-color", back_color),
    ]
}

/// mpv's volume in percent as the whole number the server's reports expect.
fn volume_level(volume: f64) -> i32 {
    volume.clamp(0., 100.).round() as i32
}

async fn apply_command(
    player: &Player,
    source: &MediaSourceInfo,
    stream: &StreamInfo,
    report: &mut PlaybackReport,
    command: UiCommand,
) -> player::Result<()> {
    match command {
        UiCommand::TogglePause => player.command(json!(["cycle", "pause"])).await,
        UiCommand::SetPause(paused) => player.set_property("pause", paused).await,
        UiCommand::SetSpeed(speed) => player.set_property("speed", speed).await,
        UiCommand::SeekBy(seconds) => player.command(json!(["seek", seconds, "relative"])).await,
        UiCommand::SeekTo(position) => {
            player
                .command(json!(["seek", position.as_secs_f64(), "absolute"]))
                .await
        }
        UiCommand::SetVolume(volume) => player.set_property("volume", volume.clamp(0., 100.)).await,
        UiCommand::SetMute(muted) => player.set_property("mute", muted).await,
        UiCommand::SetSubtitleDelay(seconds) => player.set_property("sub-delay", seconds).await,
        UiCommand::SetAudioDelay(seconds) => player.set_property("audio-delay", seconds).await,
        UiCommand::SetSubtitleScale(scale) => player.set_property("sub-scale", scale).await,
        UiCommand::ToggleMute => player.command(json!(["cycle", "mute"])).await,
        // A transcode restarts for a new audio track instead (`NowPlaying::change_audio`).
        UiCommand::Audio(_) if stream.method == PlayMethod::Transcode => Ok(()),
        UiCommand::Audio(index) => {
            if let Some(aid) = mpv_track_id(source, MediaStreamType::Audio, index) {
                player.set_property("aid", aid).await?;
                report.audio_stream_index = Some(index);
            }
            Ok(())
        }
        UiCommand::Subtitle(choice) => {
            let sid = match choice {
                SubtitleChoice::Stream(index) if stream.method == PlayMethod::Transcode => stream
                    .subtitle_files
                    .iter()
                    .position(|&i| i == index)
                    .map(|i| json!(i + 1)),
                SubtitleChoice::Stream(index) => mpv_subtitle_id(source, index).map(|id| json!(id)),
                SubtitleChoice::Off => Some(json!("no")),
                SubtitleChoice::Auto => Some(json!("auto")),
            };
            if let Some(sid) = sid {
                player.set_property("sid", sid).await?;
                report.subtitle_stream_index = subtitle_index(choice);
            }
            Ok(())
        }
        UiCommand::Stop => player.quit().await,
    }
}

/// Remembers the running playback in the config (or forgets it when it ends), so a crash
/// can be reported on the next launch.
fn save_unfinished(report: Option<&PlaybackReport>, account: &str) {
    let mut config = Config::load();
    config.unfinished_playback = report.map(|report| UnfinishedPlayback {
        account: Some(account.to_owned()),
        item_id: report.item_id.clone(),
        media_source_id: report.media_source_id.clone(),
        play_session_id: report.play_session_id.clone(),
        position_ticks: report.position.0,
    });
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
}

/// On launch: if the last run ended mid-playback (a crash), tell the server where it stopped,
/// so the resume point is right and the session doesn't look like it's still playing. Only
/// the account that was playing reports it; it waits for that one to sign in otherwise.
pub fn report_unfinished(client: &Client, account: &str) {
    let mut config = Config::load();
    let Some(unfinished) = config
        .unfinished_playback
        .take_if(|unfinished| unfinished.account.as_deref().is_none_or(|a| a == account))
    else {
        return;
    };
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
    let report = PlaybackReport {
        item_id: unfinished.item_id,
        media_source_id: unfinished.media_source_id,
        play_session_id: unfinished.play_session_id,
        position: Ticks(unfinished.position_ticks),
        ..Default::default()
    };
    tracing::warn!(item = %report.item_id, position = report.position.as_secs_f64(), "the last playback didn't end cleanly; reporting its stop");
    let client = client.clone();
    runtime::detach(async move { report_or_log(client.report_playback_stopped(&report).await, "stop") });
}

fn report_or_log(result: jellyfin::Result<()>, what: &str) {
    if let Err(err) = result {
        tracing::warn!("playback {what} report failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

    use std::time::Duration;

    use jellyfin::PlaybackReport;
    use player::PlayerEvent;

    use super::{
        Adjustments, Outgoing, Reporting, UiCommand, VOLUME_REPORT_AFTER, adjacent, mpv_subtitle_id, mpv_track_id,
        volume_level,
    };

    #[test]
    fn nothing_is_reported_before_the_start() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        for event in [PlayerEvent::Paused(true), PlayerEvent::Seeked, PlayerEvent::Volume(40.)] {
            assert_eq!(reporting.on_event(&event, &mut report, now), Outgoing::Nothing);
        }
        assert_eq!(reporting.on_command(&UiCommand::SeekBy(10.)), Outgoing::Nothing);
        // The start report carries what changed meanwhile, so no volume report is pending.
        assert!(report.is_paused && report.volume_level == 40);
        assert_eq!(reporting.volume_due, None);
        assert_eq!(
            reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now),
            Outgoing::Start
        );
    }

    #[test]
    fn pauses_seeks_and_track_changes_report_progress() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now);
        assert_eq!(
            reporting.on_event(&PlayerEvent::Paused(true), &mut report, now),
            Outgoing::Progress
        );
        assert!(report.is_paused);
        assert_eq!(
            reporting.on_event(&PlayerEvent::Seeked, &mut report, now),
            Outgoing::Progress
        );
        let position = PlayerEvent::Position(Duration::from_secs(5));
        assert_eq!(reporting.on_event(&position, &mut report, now), Outgoing::Nothing);
        assert_eq!(Duration::from(report.position), Duration::from_secs(5));
        assert_eq!(reporting.on_command(&UiCommand::Audio(2)), Outgoing::Progress);
        assert_eq!(reporting.on_command(&UiCommand::SetSpeed(1.5)), Outgoing::Nothing);
        assert_eq!(reporting.on_command(&UiCommand::SetVolume(50.)), Outgoing::Nothing);
    }

    #[test]
    fn volume_changes_are_batched_into_one_report() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now);
        reporting.on_event(&PlayerEvent::Volume(60.), &mut report, now);
        let later = now + Duration::from_millis(300);
        reporting.on_event(&PlayerEvent::Volume(65.), &mut report, later);
        reporting.on_event(&PlayerEvent::Muted(true), &mut report, later);
        // Due a second after the first change, carrying the last state.
        assert_eq!(reporting.volume_due, Some(now + VOLUME_REPORT_AFTER));
        assert!(report.volume_level == 65 && report.is_muted);
        reporting.volume_reported();
        assert_eq!(reporting.volume_due, None);
    }

    fn stream(index: i32, kind: MediaStreamType, is_external: bool) -> MediaStream {
        serde_json::from_value(serde_json::json!({
            "Index": index,
            "Type": kind.as_str(),
            "IsExternal": is_external,
        }))
        .unwrap()
    }

    fn source() -> MediaSourceInfo {
        let source: MediaSourceInfo = serde_json::from_value(serde_json::json!({ "Id": "s" })).unwrap();
        MediaSourceInfo {
            media_streams: vec![
                stream(0, MediaStreamType::Subtitle, true),
                stream(1, MediaStreamType::Video, false),
                stream(2, MediaStreamType::Audio, false),
                stream(3, MediaStreamType::Audio, false),
                stream(4, MediaStreamType::Subtitle, false),
                stream(5, MediaStreamType::Subtitle, true),
            ],
            ..source
        }
    }

    #[test]
    fn maps_jellyfin_indexes_to_mpv_track_ids() {
        let source = source();
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 2), Some(1));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 3), Some(2));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 4), Some(1));
        // External streams aren't in the file.
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 0), None);
    }

    #[test]
    fn external_subtitles_follow_embedded_ones() {
        let source = source();
        assert_eq!(mpv_subtitle_id(&source, 4), Some(1));
        assert_eq!(mpv_subtitle_id(&source, 0), Some(2));
        assert_eq!(mpv_subtitle_id(&source, 5), Some(3));
        assert_eq!(mpv_subtitle_id(&source, 9), None);
    }

    #[test]
    fn the_reported_volume_is_a_whole_percent() {
        assert_eq!(volume_level(54.6), 55);
        assert_eq!(volume_level(100.4), 100);
        assert_eq!(volume_level(-3.), 0);
        assert_eq!(volume_level(150.), 100);
    }

    #[test]
    fn speed_steps_through_the_menu() {
        let at = |speed| Adjustments {
            speed,
            ..Adjustments::default()
        };
        assert_eq!(at(1.).step_speed(true), 1.25);
        assert_eq!(at(1.).step_speed(false), 0.75);
        assert_eq!(at(2.).step_speed(true), 2.);
        assert_eq!(at(0.5).step_speed(false), 0.5);
        // From a speed between two steps (set some other way), to the neighbouring ones.
        assert_eq!(at(1.1).step_speed(true), 1.25);
        assert_eq!(at(1.1).step_speed(false), 1.);
    }

    #[test]
    fn adjustments_stay_in_range_and_on_their_steps() {
        let wild = Adjustments {
            speed: 9.,
            subtitle_delay: 0.1 + 0.2,
            audio_delay: -100.,
            subtitle_scale: 0.1,
        }
        .clamped();
        assert_eq!(wild.speed, 2.);
        assert_eq!(wild.subtitle_delay, 0.3);
        assert_eq!(wild.audio_delay, -30.);
        assert_eq!(wild.subtitle_scale, 0.5);
    }

    #[test]
    fn the_next_item_keeps_speed_and_size_but_not_delays() {
        let next = Adjustments {
            speed: 1.5,
            subtitle_delay: 1.2,
            audio_delay: -0.3,
            subtitle_scale: 1.4,
        }
        .for_next_item();
        assert_eq!((next.speed, next.subtitle_scale), (1.5, 1.4));
        assert_eq!((next.subtitle_delay, next.audio_delay), (0., 0.));
    }

    #[test]
    fn adjacent_items() {
        let item = |id: &str| -> jellyfin::models::BaseItemDto {
            serde_json::from_value(serde_json::json!({ "Id": id, "Name": id })).expect("item")
        };
        let ids = |pair: (
            Option<jellyfin::models::BaseItemDto>,
            Option<jellyfin::models::BaseItemDto>,
        )| { (pair.0.map(|i| i.id), pair.1.map(|i| i.id)) };
        let items = || vec![item("a"), item("b"), item("c")];
        assert_eq!(ids(adjacent(items(), "b")), (Some("a".into()), Some("c".into())));
        assert_eq!(ids(adjacent(items(), "a")), (None, Some("b".into())));
        assert_eq!(ids(adjacent(items(), "c")), (Some("b".into()), None));
        assert_eq!(ids(adjacent(items(), "x")), (None, None));
    }
}
