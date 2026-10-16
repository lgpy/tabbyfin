//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio and publishes a [`NowPlaying`] entity that the player page draws and controls.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use gpui_kit::*;
use jellyfin::models::{
    BaseItemDto, BaseItemKind, ItemField, MediaSourceInfo, MediaStream, MediaStreamType, TrickplayInfo,
};
use jellyfin::{Client, PlayMethod, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::libmpv::VideoOutput;
use player::{EndReason, LoadRequest, Player, PlayerEvent};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::changes::Changes;
use crate::config::{Config, SubtitleColor, SubtitleStyle, UnfinishedPlayback};
use crate::markers::{self, Chapter, Segment, SegmentKind};
use crate::runtime;
use crate::session::describe_error;
use crate::syncplay::{GroupRequest, SyncPlay};

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// How long after a volume or mute change its progress report goes out. A wheel spin or held
/// key changes the volume many times a second; one report covers them all.
const VOLUME_REPORT_AFTER: Duration = Duration::from_secs(1);
const DEFAULT_VOLUME: f64 = 100.;

pub enum PlaybackEvent {
    /// Playback started; show the player.
    Started(Entity<NowPlaying>),
    /// The session ended, normally or not. Watched state and resume points may have changed.
    Ended,
    Failed(SharedString),
    /// Something to tell the user that isn't an error, e.g. reconnecting.
    Notice(SharedString),
}

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
    pub fn send(&self, command: UiCommand) {
        let _ = self.commands.send(command);
    }

    /// A control from the player UI. In a SyncPlay group, pause, seek and stop are requests
    /// to the group, which then commands every member.
    pub fn control(&self, command: UiCommand, cx: &mut App) {
        if self.group_item.is_some()
            && let Some(syncplay) = SyncPlay::try_global(cx)
        {
            let request = match command {
                UiCommand::TogglePause => Some(GroupRequest::SetPause(!self.paused)),
                UiCommand::SetPause(paused) => Some(GroupRequest::SetPause(paused)),
                UiCommand::SeekBy(seconds) => {
                    let target = (self.position.as_secs_f64() + seconds).max(0.);
                    Some(GroupRequest::Seek(Duration::from_secs_f64(target)))
                }
                UiCommand::SeekTo(position) => Some(GroupRequest::Seek(position)),
                UiCommand::Stop => Some(GroupRequest::Stop),
                _ => None,
            };
            if let Some(request) = request {
                syncplay.update(cx, |syncplay, cx| syncplay.request(request, cx));
                return;
            }
        }
        self.send(command);
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

    /// Plays the next episode now.
    pub fn play_next(&self, cx: &mut App) {
        if let Some(next) = self.next.clone() {
            PlaybackManager::global(cx).update(cx, |playback, cx| playback.play_from_resume(&next, cx));
        }
    }

    /// The resume point to offer starting over from: shown for the first [`RESUME_PROMPT_FOR`]
    /// of playback from it, and gone after a seek elsewhere.
    pub fn resume_prompt(&self) -> Option<Duration> {
        let from = self.resumed_from.filter(|_| !self.loading)?;
        let window = from.saturating_sub(Duration::from_secs(2))..from + RESUME_PROMPT_FOR;
        window.contains(&self.position).then_some(from)
    }

    /// Plays from the beginning instead of the resume point.
    pub fn start_over(&mut self, cx: &mut Context<Self>) {
        self.resumed_from = None;
        self.control(UiCommand::SeekTo(Duration::ZERO), cx);
        cx.notify();
    }

    /// Plays the previous episode (or movie of the collection) from its resume point.
    pub fn play_previous(&self, cx: &mut App) {
        if let Some(previous) = self.previous.clone() {
            PlaybackManager::global(cx).update(cx, |playback, cx| playback.play_from_resume(&previous, cx));
        }
    }

    /// What plays after this item ends by itself.
    fn autoplay_next(&self) -> Option<BaseItemDto> {
        self.next.clone().filter(|_| self.autoplay && !self.next_cancelled)
    }

    pub fn is_transcoding(&self) -> bool {
        self.stream.method == PlayMethod::Transcode
    }

    /// Switches the audio track. A transcode carries only the chosen track, so it restarts
    /// at the current position.
    pub fn change_audio(&mut self, index: i32, cx: &mut Context<Self>) {
        if self.audio == Some(index) {
            return;
        }
        self.audio = Some(index);
        if self.is_transcoding() {
            self.restart(cx);
        } else {
            self.send(UiCommand::Audio(index));
        }
    }

    /// Switches subtitles. A transcode restarts if a burned-in subtitle has to go, or the new
    /// one has to be burned in.
    pub fn change_subtitle(&mut self, choice: SubtitleChoice, cx: &mut Context<Self>) {
        if self.subtitle == choice {
            return;
        }
        self.subtitle = choice;
        let as_file = match choice {
            SubtitleChoice::Stream(index) => self.stream.subtitle_files.contains(&index),
            SubtitleChoice::Off | SubtitleChoice::Auto => true,
        };
        if self.is_transcoding() && (self.stream.burned_subtitle.is_some() || !as_file) {
            self.restart(cx);
        } else {
            self.send(UiCommand::Subtitle(choice));
        }
    }

    /// Applies changes from the settings menu. Speed stays at normal in a SyncPlay group,
    /// whose drift correction uses it.
    pub fn adjust(&mut self, adjustments: Adjustments, cx: &mut Context<Self>) {
        let mut new = adjustments.clamped();
        if self.group_item.is_some() {
            new.speed = 1.;
        }
        let old = self.adjustments;
        if new == old {
            return;
        }
        self.adjustments = new;
        let changes = [
            (new.speed != old.speed).then_some(UiCommand::SetSpeed(new.speed)),
            (new.subtitle_delay != old.subtitle_delay).then_some(UiCommand::SetSubtitleDelay(new.subtitle_delay)),
            (new.audio_delay != old.audio_delay).then_some(UiCommand::SetAudioDelay(new.audio_delay)),
            (new.subtitle_scale != old.subtitle_scale).then_some(UiCommand::SetSubtitleScale(new.subtitle_scale)),
        ];
        for command in changes.into_iter().flatten() {
            self.send(command);
        }
        PlaybackManager::global(cx).update(cx, |playback, _| playback.adjustments = new);
        cx.notify();
    }

    /// Plays the same version again from the current position with the current tracks.
    fn restart(&self, cx: &mut App) {
        let options = PlayOptions {
            start: Some(Ticks::from(self.position)),
            media_source_id: Some(self.source.id.clone()),
            audio_stream_index: self.audio,
            subtitle: self.subtitle,
            ..Default::default()
        };
        self.reload(options, cx);
    }

    /// Loads this item again here only, e.g. in another version: at the current position, or
    /// `options.start`. In a SyncPlay group the others wait meanwhile (`SyncPlay::reload`),
    /// rather than the group getting a new queue as when something new is played.
    pub fn reload(&self, mut options: PlayOptions, cx: &mut App) {
        options.start = options.start.or(Some(Ticks::from(self.position)));
        let item = self.item.clone();
        match (&self.group_item, SyncPlay::try_global(cx)) {
            (Some(_), Some(syncplay)) => {
                syncplay.update(cx, |syncplay, cx| syncplay.reload(&item, options, cx));
            }
            _ => PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx)),
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
}

pub struct PlaybackManager {
    client: Client,
    /// The signed-in account ([`crate::config::SavedServer::credential_account`]).
    account: String,
    /// Stops the running session when sent or dropped.
    stop: Option<oneshot::Sender<()>>,
    /// Counts sessions, so the end of a replaced session doesn't close its successor's player.
    generation: u64,
    /// A session is running (from `play` until its end is handled).
    active: bool,
    /// A reconnect waiting to start after a lost stream. Stopping cancels it.
    reconnect: Option<Task<()>>,
    /// What plays after the current item ("Play all" on a playlist or collection). Takes the
    /// place of the next episode for the up-next card and autoplay.
    queue: VecDeque<BaseItemDto>,
    /// The running session's state, once it has started.
    now_playing: Option<Entity<NowPlaying>>,
    volume: f64,
    muted: bool,
    adjustments: Adjustments,
    /// The item `adjustments` were made for; another one starts without the delays.
    adjusted_item: Option<String>,
}

struct GlobalPlayback(Entity<PlaybackManager>);

impl Global for GlobalPlayback {}

impl EventEmitter<PlaybackEvent> for PlaybackManager {}

impl PlaybackManager {
    /// Installs the manager for a signed-in session, replacing (and stopping) any previous one.
    pub fn init(client: Client, account: String, cx: &mut App) {
        let config = Config::load();
        let volume = config.volume.unwrap_or(DEFAULT_VOLUME);
        let adjustments = Adjustments {
            subtitle_scale: config.subtitle_scale.unwrap_or(1.),
            ..Adjustments::default()
        }
        .clamped();
        let manager = cx.new(|_| PlaybackManager {
            client,
            account,
            stop: None,
            generation: 0,
            active: false,
            reconnect: None,
            queue: VecDeque::new(),
            now_playing: None,
            volume,
            muted: false,
            adjustments,
            adjusted_item: None,
        });
        cx.set_global(GlobalPlayback(manager));
    }

    pub fn global(cx: &App) -> Entity<PlaybackManager> {
        cx.global::<GlobalPlayback>().0.clone()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sets the subtitle size for the next items (the settings page; the player's menu goes
    /// through [`NowPlaying::adjust`]).
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
    pub fn now_playing(&self) -> Option<Entity<NowPlaying>> {
        self.now_playing.clone()
    }

    /// Stops the current session, if any, or a pending reconnect (closing the player).
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.stop = None;
        if self.reconnect.take().is_some() {
            cx.emit(PlaybackEvent::Ended);
        }
    }

    /// Plays an item from its resume point, with default tracks (e.g. the next episode).
    pub fn play_from_resume(&mut self, item: &BaseItemDto, cx: &mut Context<Self>) {
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
            cx,
        );
    }

    /// Plays `items` in order: the first now, the rest as a queue.
    pub fn play_all(&mut self, items: Vec<BaseItemDto>, cx: &mut Context<Self>) {
        self.play_all_with(items, PlayOptions::default(), cx);
    }

    /// Plays `items` in order, the first with `options`. A SyncPlay group gets them as its
    /// new queue instead, from the start.
    pub fn play_all_with(&mut self, items: Vec<BaseItemDto>, options: PlayOptions, cx: &mut Context<Self>) {
        let mut items = VecDeque::from(items);
        let Some(first) = items.pop_front() else { return };
        if let Some(syncplay) = SyncPlay::try_global(cx).filter(|s| s.read(cx).group.is_some()) {
            let mut all = vec![first];
            all.extend(items);
            syncplay.update(cx, |syncplay, cx| syncplay.start_items(&all, cx));
            return;
        }
        self.queue = items;
        self.play_now(&first, options, cx);
    }

    /// Adds `items` to the queue: right after the playing item with `next`, otherwise at the
    /// end. With nothing playing they play now.
    pub fn enqueue(&mut self, items: Vec<BaseItemDto>, next: bool, cx: &mut Context<Self>) {
        if let Some(syncplay) = SyncPlay::try_global(cx).filter(|s| s.read(cx).group.is_some()) {
            syncplay.update(cx, |syncplay, cx| syncplay.enqueue(&items, next, cx));
            return;
        }
        if !self.active {
            self.play_all(items, cx);
            return;
        }
        if next {
            for item in items.into_iter().rev() {
                self.queue.push_front(item);
            }
        } else {
            self.queue.extend(items);
        }
        // The queue takes the place of the next episode for the up-next card and autoplay.
        if let (Some(now_playing), Some(front)) = (&self.now_playing, self.queue.front()) {
            let front = front.clone();
            now_playing.update(cx, |state, cx| {
                state.next = Some(front);
                cx.notify();
            });
        }
    }

    pub fn play(&mut self, item: &BaseItemDto, options: PlayOptions, cx: &mut Context<Self>) {
        // Playing the queue's next item moves the queue on; playing anything else drops it.
        if self.queue.front().is_some_and(|next| next.id == item.id) {
            self.queue.pop_front();
        } else if options.group_item.is_none() {
            self.queue.clear();
        }
        self.play_now(item, options, cx);
    }

    fn play_now(&mut self, item: &BaseItemDto, options: PlayOptions, cx: &mut Context<Self>) {
        // In a SyncPlay group, starting something means giving the group a new queue; the
        // server then has every member (this one included) load it.
        if options.group_item.is_none()
            && let Some(syncplay) = SyncPlay::try_global(cx).filter(|s| s.read(cx).group.is_some())
        {
            let start = options.start.unwrap_or_default();
            syncplay.update(cx, |syncplay, cx| syncplay.start_queue(item, start, cx));
            return;
        }
        self.start_session(item, options, 0, cx);
    }

    /// Starts a session; `attempt` counts reconnects after a lost stream.
    fn start_session(&mut self, item: &BaseItemDto, options: PlayOptions, attempt: usize, cx: &mut Context<Self>) {
        self.reconnect = None;
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);
        self.generation += 1;
        self.active = true;
        let generation = self.generation;
        let (volume, muted) = (self.volume, self.muted);
        if self.adjusted_item.as_ref() != Some(&item.id) {
            self.adjustments = self.adjustments.for_next_item();
            self.adjusted_item = Some(item.id.clone());
        }
        let mut adjustments = self.adjustments;
        if options.group_item.is_some() {
            adjustments.speed = 1.;
        }
        let autoplay = Config::load().autoplay_next_episode.unwrap_or(true);
        let group_item = options.group_item.clone();
        let resumed_from = options.start.filter(|_| options.resumed).map(Duration::from);
        let retry_options = options.clone();
        let queued = self.queue.front().cloned();
        let (updates_tx, mut updates) = mpsc::unbounded_channel();
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();

        let session = {
            let (client, item, account) = (self.client.clone(), item.clone(), self.account.clone());
            runtime::spawn(async move {
                let channels = Channels {
                    stop: stop_rx,
                    updates: updates_tx,
                    commands: commands_rx,
                };
                run_session(client, item, options, (volume, muted), adjustments, account, channels).await
            })
        };

        let item = item.clone();
        cx.spawn(async move |this, cx| {
            let mut now_playing: Option<Entity<NowPlaying>> = None;
            while let Some(update) = updates.recv().await {
                match update {
                    SessionUpdate::Started {
                        source,
                        video,
                        audio,
                        subtitle,
                        stream,
                    } => {
                        let commands = commands_tx.clone();
                        let item = item.clone();
                        let entity = cx.new(|_| NowPlaying {
                            item,
                            source: *source,
                            video,
                            position: Duration::ZERO,
                            position_at: Instant::now(),
                            duration: None,
                            paused: false,
                            loading: true,
                            audio,
                            subtitle,
                            volume,
                            muted,
                            adjustments,
                            versions: Vec::new(),
                            trickplay: TrickplayTiles::new(),
                            previous: None,
                            next: queued.clone(),
                            next_cancelled: false,
                            autoplay,
                            chapters: Vec::new(),
                            segments: Vec::new(),
                            stream,
                            group_item: group_item.clone(),
                            resumed_from,
                            restarts: 0,
                            commands,
                        });
                        this.update(cx, |this, cx| {
                            if this.generation == generation {
                                this.now_playing = Some(entity.clone());
                            }
                            cx.emit(PlaybackEvent::Started(entity.clone()));
                        })
                        .ok();
                        now_playing = Some(entity);
                    }
                    SessionUpdate::Details(details) => {
                        // Items queued since the start (remote control) count too.
                        let queued = this.read_with(cx, |this, _| !this.queue.is_empty()).unwrap_or(true);
                        if let Some(entity) = &now_playing {
                            entity.update(cx, |state, cx| {
                                state.versions = details.versions;
                                state.trickplay = details.trickplay;
                                state.previous = details.previous;
                                if !queued {
                                    state.next = details.next;
                                    // A sequel isn't started without asking; the card offers it.
                                    state.autoplay &= !details.next_in_collection;
                                }
                                state.chapters = details.chapters;
                                state.segments = details.segments;
                                cx.notify();
                            });
                        }
                    }
                    SessionUpdate::Player(event) => {
                        if let PlayerEvent::Volume(_) | PlayerEvent::Muted(_) = event {
                            this.update(cx, |this, _| {
                                if this.generation == generation {
                                    match event {
                                        PlayerEvent::Volume(volume) => this.volume = volume,
                                        PlayerEvent::Muted(muted) => this.muted = muted,
                                        _ => {}
                                    }
                                }
                            })
                            .ok();
                        }
                        if let Some(entity) = &now_playing {
                            entity.update(cx, |state, cx| {
                                state.apply(&event);
                                cx.notify();
                            });
                        }
                    }
                }
            }
            let result = session.await;
            let next = now_playing.and_then(|entity| entity.read_with(cx, |state, _| state.autoplay_next()));
            this.update(cx, |this, cx| {
                this.save_preferences();
                Changes::notify_user_data(cx);
                // A newer session took over (another item, version or episode); its player is
                // already showing.
                if this.generation != generation {
                    return;
                }
                this.active = false;
                this.now_playing = None;
                match result {
                    // In a group, every member reports the end and the server moves the
                    // queue on (once); the next item then comes as a queue update.
                    Ok(Ended::Finished) if let Some(group_item) = &group_item => {
                        if let Some(syncplay) = SyncPlay::try_global(cx) {
                            syncplay.update(cx, |syncplay, cx| syncplay.item_ended(group_item, cx));
                        }
                        cx.emit(PlaybackEvent::Ended);
                    }
                    Ok(Ended::Finished) if let Some(next) = next => {
                        tracing::info!(next = %next.name, "playing the next episode");
                        this.play_from_resume(&next, cx);
                    }
                    // Keep the player up and try again from where it stopped.
                    Ok(Ended::Lost {
                        position,
                        message,
                        crashed,
                    }) if group_item.is_none() && attempt < RECONNECT_DELAYS.len() => {
                        let delay = RECONNECT_DELAYS[attempt];
                        tracing::warn!(attempt, ?delay, "stream lost: {message}");
                        let notice = if crashed {
                            "The player stopped unexpectedly. Restarting it…"
                        } else {
                            "Lost the connection to the server. Reconnecting…"
                        };
                        cx.emit(PlaybackEvent::Notice(notice.into()));
                        let options = PlayOptions {
                            start: Some(position),
                            resumed: false,
                            ..retry_options
                        };
                        let item = item.clone();
                        this.reconnect = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(delay).await;
                            this.update(cx, |this, cx| this.start_session(&item, options, attempt + 1, cx))
                                .ok();
                        }));
                    }
                    Ok(Ended::Lost { message, crashed, .. }) => {
                        tracing::warn!("stream lost for good: {message}");
                        let error = if crashed {
                            "The player keeps crashing on this item. The log may say why."
                        } else {
                            "Lost the connection to the server. Try again when it's back."
                        };
                        cx.emit(PlaybackEvent::Failed(error.into()));
                        cx.emit(PlaybackEvent::Ended);
                    }
                    Ok(_) => cx.emit(PlaybackEvent::Ended),
                    Err(message) => {
                        tracing::warn!("playback failed: {message}");
                        cx.emit(PlaybackEvent::Failed(message.into()));
                        cx.emit(PlaybackEvent::Ended);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Remembers the volume and subtitle size for the next launch.
    fn save_preferences(&self) {
        let mut config = Config::load();
        let scale = Some(self.adjustments.subtitle_scale).filter(|&scale| scale != 1.);
        if config.volume != Some(self.volume) || config.subtitle_scale != scale {
            config.volume = Some(self.volume);
            config.subtitle_scale = scale;
            if let Err(err) = config.save() {
                tracing::warn!("failed to save config: {err:#}");
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
    updates: mpsc::UnboundedSender<SessionUpdate>,
    commands: mpsc::UnboundedReceiver<UiCommand>,
}

fn title(item: &BaseItemDto) -> String {
    match (item.kind, &item.series_name) {
        (BaseItemKind::Episode, Some(series)) => {
            let label = crate::components::episode_label(item);
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

async fn start_player() -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput), String> {
    Player::spawn().await.map_err(|err| match err {
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
async fn run_session(
    client: Client,
    item: BaseItemDto,
    options: PlayOptions,
    (volume, muted): (f64, bool),
    adjustments: Adjustments,
    account: String,
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

    let (player, mut events, video) = start_player().await?;

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
    // Not `super::*`: that glob brings in GPUI's `test` attribute, which shadows the std one.
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
