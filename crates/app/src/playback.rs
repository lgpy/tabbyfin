//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio and publishes a [`NowPlaying`] entity that the player page draws and controls.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, MediaSourceInfo, MediaStream, MediaStreamType};
use jellyfin::{Client, PlayMethod, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::libmpv::VideoOutput;
use player::{EndReason, LoadRequest, Player, PlayerEvent};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::changes::Changes;
use crate::config::{Config, UnfinishedPlayback};
use crate::markers::{self, Chapter, Segment, SegmentKind};
use crate::runtime;
use crate::session::describe_error;
use crate::syncplay::{GroupRequest, SyncPlay};

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubtitleChoice {
    #[default]
    Auto,
    Off,
    Stream(i32),
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
    next: Option<BaseItemDto>,
    chapters: Vec<Chapter>,
    segments: Vec<Segment>,
}

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
    /// All versions of the item, once known; empty or one entry means there's nothing to
    /// switch to.
    pub versions: Vec<Version>,
    /// The following episode, for episodes.
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

    /// Plays the same version again from the current position with the current tracks.
    fn restart(&self, cx: &mut App) {
        let options = PlayOptions {
            start: Some(Ticks::from(self.position)),
            media_source_id: Some(self.source.id.clone()),
            audio_stream_index: self.audio,
            subtitle: self.subtitle,
            group_item: self.group_item.clone(),
            ..Default::default()
        };
        let item = self.item.clone();
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
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
    volume: f64,
    muted: bool,
}

struct GlobalPlayback(Entity<PlaybackManager>);

impl Global for GlobalPlayback {}

impl EventEmitter<PlaybackEvent> for PlaybackManager {}

impl PlaybackManager {
    /// Installs the manager for a signed-in session, replacing (and stopping) any previous one.
    pub fn init(client: Client, cx: &mut App) {
        let volume = Config::load().volume.unwrap_or(DEFAULT_VOLUME);
        let manager = cx.new(|_| PlaybackManager {
            client,
            stop: None,
            generation: 0,
            active: false,
            reconnect: None,
            queue: VecDeque::new(),
            volume,
            muted: false,
        });
        cx.set_global(GlobalPlayback(manager));
    }

    pub fn global(cx: &App) -> Entity<PlaybackManager> {
        cx.global::<GlobalPlayback>().0.clone()
    }

    pub fn is_active(&self) -> bool {
        self.active
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
                ..Default::default()
            },
            cx,
        );
    }

    /// Plays `items` in order: the first now, the rest as a queue.
    pub fn play_all(&mut self, items: Vec<BaseItemDto>, cx: &mut Context<Self>) {
        let mut items = VecDeque::from(items);
        let Some(first) = items.pop_front() else { return };
        if let Some(syncplay) = SyncPlay::try_global(cx).filter(|s| s.read(cx).group.is_some()) {
            let mut all = vec![first];
            all.extend(items);
            syncplay.update(cx, |syncplay, cx| syncplay.start_items(&all, cx));
            return;
        }
        self.queue = items;
        self.play_now(&first, PlayOptions::default(), cx);
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
        let autoplay = Config::load().autoplay_next_episode.unwrap_or(true);
        let group_item = options.group_item.clone();
        let retry_options = options.clone();
        let queued = self.queue.front().cloned();
        let (updates_tx, mut updates) = mpsc::unbounded_channel();
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();

        let session = {
            let (client, item) = (self.client.clone(), item.clone());
            runtime::spawn(async move {
                let channels = Channels {
                    stop: stop_rx,
                    updates: updates_tx,
                    commands: commands_rx,
                };
                run_session(client, item, options, (volume, muted), channels).await
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
                            versions: Vec::new(),
                            next: queued.clone(),
                            next_cancelled: false,
                            autoplay,
                            chapters: Vec::new(),
                            segments: Vec::new(),
                            stream,
                            group_item: group_item.clone(),
                            restarts: 0,
                            commands,
                        });
                        this.update(cx, |_, cx| cx.emit(PlaybackEvent::Started(entity.clone())))
                            .ok();
                        now_playing = Some(entity);
                    }
                    SessionUpdate::Details(details) => {
                        if let Some(entity) = &now_playing {
                            entity.update(cx, |state, cx| {
                                state.versions = details.versions;
                                if queued.is_none() {
                                    state.next = details.next;
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
                this.save_volume();
                Changes::notify_user_data(cx);
                // A newer session took over (another item, version or episode); its player is
                // already showing.
                if this.generation != generation {
                    return;
                }
                this.active = false;
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

    fn save_volume(&self) {
        let mut config = Config::load();
        if config.volume != Some(self.volume) {
            config.volume = Some(self.volume);
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
                "libmpv-2.dll is missing next to jelly-gpui.exe; reinstall the app."
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
    let next = async {
        match (item.kind, &item.series_id) {
            (BaseItemKind::Episode, Some(series_id)) => client.next_episode(series_id, &item.id).await,
            _ => Ok(None),
        }
    };
    let (full, segments, next) = tokio::join!(
        client.item(&item.id, &[ItemField::MediaSources, ItemField::Chapters]),
        client.media_segments(&item.id),
        next,
    );
    let full = full
        .inspect_err(|err| tracing::warn!("couldn't load versions and chapters: {err}"))
        .ok();
    let next = next
        .inspect_err(|err| tracing::warn!("couldn't find the next episode: {err}"))
        .ok()
        .flatten();
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
        next,
        chapters,
        segments,
    }
}

/// Runs one playback from negotiation to the final stop report. Returns a user-facing error.
async fn run_session(
    client: Client,
    item: BaseItemDto,
    options: PlayOptions,
    (volume, muted): (f64, bool),
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
                max_streaming_bitrate: Config::load().max_streaming_bitrate,
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
    let mut started = false;
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
                match event {
                    PlayerEvent::FileLoaded => {
                        for (stream, url) in &subtitle_files {
                            let title = stream.display_title.as_deref().unwrap_or("External");
                            let select = file_selected == Some(stream.index);
                            if let Err(err) = player.add_subtitle(url.as_str(), title, stream.language.as_deref(), select).await {
                                tracing::warn!("couldn't add subtitle: {err}");
                            }
                        }
                        report_or_log(client.report_playback_start(&report).await, "start");
                        save_unfinished(Some(&report));
                        started = true;
                    }
                    PlayerEvent::Position(position) => report.position = Ticks::from(position),
                    PlayerEvent::Duration(length) => duration = Some(length),
                    PlayerEvent::Volume(volume) => report.volume_level = volume_level(volume),
                    PlayerEvent::Muted(muted) => report.is_muted = muted,
                    PlayerEvent::Paused(paused) => {
                        report.is_paused = paused;
                        if started {
                            report_or_log(client.report_playback_progress(&report).await, "progress");
                        }
                    }
                    PlayerEvent::Seeked if started => {
                        report_or_log(client.report_playback_progress(&report).await, "progress");
                    }
                    PlayerEvent::EndFile(EndReason::Eof) if !stopping => finished = true,
                    PlayerEvent::EndFile(EndReason::Error(message)) => {
                        error = Some(format!("mpv couldn't play this file: {message}"));
                    }
                    PlayerEvent::Exited => break,
                    _ => {}
                }
            }
            Some(command) = channels.commands.recv() => {
                tracing::debug!(?command, "player command");
                if let Err(err) = apply_command(&player, &source, &stream, &mut report, command).await {
                    tracing::warn!("player command failed: {err}");
                }
                if started {
                    report_or_log(client.report_playback_progress(&report).await, "progress");
                }
            }
            _ = ticker.tick(), if started => {
                report_or_log(client.report_playback_progress(&report).await, "progress");
                save_unfinished(Some(&report));
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
    if started {
        report_or_log(client.report_playback_stopped(&report).await, "stop");
        save_unfinished(None);
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
        UiCommand::SetVolume(volume) => {
            // mpv's event confirms this right after, but the report the command triggers goes
            // out before it arrives.
            report.volume_level = volume_level(volume);
            player.set_property("volume", volume.clamp(0., 100.)).await
        }
        UiCommand::ToggleMute => {
            // Mirrors mpv's `cycle mute`; the event that follows confirms it.
            report.is_muted = !report.is_muted;
            player.command(json!(["cycle", "mute"])).await
        }
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
fn save_unfinished(report: Option<&PlaybackReport>) {
    let mut config = Config::load();
    config.unfinished_playback = report.map(|report| UnfinishedPlayback {
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
/// so the resume point is right and the session doesn't look like it's still playing.
pub fn report_unfinished(client: &Client) {
    let mut config = Config::load();
    let Some(unfinished) = config.unfinished_playback.take() else {
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

    use super::{mpv_subtitle_id, mpv_track_id, volume_level};

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
}
