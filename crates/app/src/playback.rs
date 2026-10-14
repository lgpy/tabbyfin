//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio and publishes a [`NowPlaying`] entity that the player page draws and controls.

use std::time::Duration;

use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, MediaSourceInfo, MediaStream, MediaStreamType};
use jellyfin::{Client, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::libmpv::VideoOutput;
use player::{EndReason, LoadRequest, Player, PlayerEvent};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::changes::Changes;
use crate::config::Config;
use crate::runtime;
use crate::session::describe_error;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_VOLUME: f64 = 100.;

pub enum PlaybackEvent {
    /// Playback started; show the player.
    Started(Entity<NowPlaying>),
    /// The session ended, normally or not. Watched state and resume points may have changed.
    Ended,
    Failed(SharedString),
}

/// What to play. Stream indexes are Jellyfin's `MediaStream.Index`; `None` leaves the choice
/// to mpv (and the user's mpv config).
#[derive(Debug, Clone, Default)]
pub struct PlayOptions {
    pub start: Option<Ticks>,
    pub media_source_id: Option<String>,
    pub audio_stream_index: Option<i32>,
    pub subtitle: SubtitleChoice,
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
    },
    /// Fetched after playback starts.
    Details(Box<Details>),
    Player(PlayerEvent),
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
}

/// State of the embedded player, for the player page.
pub struct NowPlaying {
    pub item: BaseItemDto,
    pub source: MediaSourceInfo,
    pub video: VideoOutput,
    pub position: Duration,
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
    commands: mpsc::UnboundedSender<UiCommand>,
}

impl NowPlaying {
    pub fn send(&self, command: UiCommand) {
        let _ = self.commands.send(command);
    }

    fn apply(&mut self, event: &PlayerEvent) {
        match event {
            PlayerEvent::FileLoaded => self.loading = false,
            PlayerEvent::Position(position) => self.position = *position,
            PlayerEvent::Duration(duration) => self.duration = Some(*duration),
            PlayerEvent::Paused(paused) => self.paused = *paused,
            PlayerEvent::Volume(volume) => self.volume = *volume,
            PlayerEvent::Muted(muted) => self.muted = *muted,
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
            volume,
            muted: false,
        });
        cx.set_global(GlobalPlayback(manager));
    }

    pub fn global(cx: &App) -> Entity<PlaybackManager> {
        cx.global::<GlobalPlayback>().0.clone()
    }

    /// Stops the current session, if any.
    pub fn stop(&mut self) {
        self.stop = None;
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

    pub fn play(&mut self, item: &BaseItemDto, options: PlayOptions, cx: &mut Context<Self>) {
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);
        self.generation += 1;
        let generation = self.generation;
        let (volume, muted) = (self.volume, self.muted);
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
                    } => {
                        let commands = commands_tx.clone();
                        let item = item.clone();
                        let entity = cx.new(|_| NowPlaying {
                            item,
                            source: *source,
                            video,
                            position: Duration::ZERO,
                            duration: None,
                            paused: false,
                            loading: true,
                            audio,
                            subtitle,
                            volume,
                            muted,
                            versions: Vec::new(),
                            next: None,
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
                                state.next = details.next;
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
            let next = now_playing.and_then(|entity| entity.read_with(cx, |state, _| state.next.clone()));
            this.update(cx, |this, cx| {
                this.save_volume();
                Changes::notify_user_data(cx);
                // A newer session took over (another item, version or episode); its player is
                // already showing.
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Ended::Finished) if let Some(next) = next => {
                        tracing::info!(next = %next.name, "playing the next episode");
                        this.play_from_resume(&next, cx);
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
}

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

/// Versions and the next episode, for the player's controls. Failures only lose those controls.
async fn details(client: &Client, item: &BaseItemDto) -> Details {
    let sources = if item.media_sources.is_empty() {
        match client.item(&item.id, &[ItemField::MediaSources]).await {
            Ok(full) => full.media_sources,
            Err(err) => {
                tracing::warn!("couldn't load versions: {err}");
                Vec::new()
            }
        }
    } else {
        item.media_sources.clone()
    };
    let versions = sources
        .iter()
        .enumerate()
        .map(|(i, source)| Version {
            id: source.id.clone(),
            name: source.name.clone().unwrap_or_else(|| format!("Version {}", i + 1)),
        })
        .collect();
    let next = match (item.kind, &item.series_id) {
        (BaseItemKind::Episode, Some(series_id)) => {
            client.next_episode(series_id, &item.id).await.unwrap_or_else(|err| {
                tracing::warn!("couldn't find the next episode: {err}");
                None
            })
        }
        _ => None,
    };
    Details { versions, next }
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

    let (player, mut events, video) = start_player().await?;

    // Track choices are options, so they're set before loading the file.
    if let Some(aid) = options
        .audio_stream_index
        .and_then(|index| mpv_track_id(&source, MediaStreamType::Audio, index))
    {
        player.set_property("aid", aid).await.map_err(|err| err.to_string())?;
    }
    let external_selected = match options.subtitle {
        SubtitleChoice::Auto => None,
        SubtitleChoice::Off => {
            player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
            None
        }
        SubtitleChoice::Stream(index) => match mpv_track_id(&source, MediaStreamType::Subtitle, index) {
            Some(sid) => {
                player.set_property("sid", sid).await.map_err(|err| err.to_string())?;
                None
            }
            // External: no embedded track until it's added and selected after loading.
            None => {
                player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
                Some(index)
            }
        },
    };

    player
        .set_property("volume", volume)
        .await
        .map_err(|err| err.to_string())?;
    player
        .set_property("mute", muted)
        .await
        .map_err(|err| err.to_string())?;

    let url = client.stream_url(&item.id, &source.id, info.play_session_id.as_deref());
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
        audio: options.audio_stream_index.or(source.default_audio_stream_index),
        subtitle: options.subtitle,
    });
    let details_task = {
        let (client, item, updates) = (client.clone(), item.clone(), channels.updates.clone());
        tokio::spawn(async move {
            let details = details(&client, &item).await;
            let _ = updates.send(SessionUpdate::Details(Box::new(details)));
        })
    };

    let external_subtitles: Vec<_> = source.media_streams.iter().filter(|s| s.is_external).cloned().collect();
    let mut report = PlaybackReport {
        item_id: item.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: info.play_session_id.clone(),
        position: options.start.unwrap_or_default(),
        audio_stream_index: options.audio_stream_index,
        subtitle_stream_index: subtitle_index(options.subtitle),
        ..Default::default()
    };
    let mut started = false;
    let mut stopping = false;
    let mut finished = false;
    let mut error = None;
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                let _ = channels.updates.send(SessionUpdate::Player(event.clone()));
                match event {
                    PlayerEvent::FileLoaded => {
                        for stream in &external_subtitles {
                            let url = client.subtitle_url(&item.id, &source.id, stream);
                            let title = stream.display_title.as_deref().unwrap_or("External");
                            let select = external_selected == Some(stream.index);
                            if let Err(err) = player.add_subtitle(url.as_str(), title, stream.language.as_deref(), select).await {
                                tracing::warn!("couldn't add subtitle: {err}");
                            }
                        }
                        report_or_log(client.report_playback_start(&report).await, "start");
                        started = true;
                    }
                    PlayerEvent::Position(position) => report.position = Ticks::from(position),
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
                if let Err(err) = apply_command(&player, &source, &mut report, command).await {
                    tracing::warn!("player command failed: {err}");
                }
                if started {
                    report_or_log(client.report_playback_progress(&report).await, "progress");
                }
            }
            _ = ticker.tick(), if started => {
                report_or_log(client.report_playback_progress(&report).await, "progress");
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
    }
    tracing::info!(item = %item.name, position = report.position.as_secs_f64(), "playback ended");
    match error {
        Some(error) => Err(error),
        None if finished => Ok(Ended::Finished),
        None => Ok(Ended::Stopped),
    }
}

async fn apply_command(
    player: &Player,
    source: &MediaSourceInfo,
    report: &mut PlaybackReport,
    command: UiCommand,
) -> player::Result<()> {
    match command {
        UiCommand::TogglePause => player.command(json!(["cycle", "pause"])).await,
        UiCommand::SeekBy(seconds) => player.command(json!(["seek", seconds, "relative"])).await,
        UiCommand::SeekTo(position) => {
            player
                .command(json!(["seek", position.as_secs_f64(), "absolute"]))
                .await
        }
        UiCommand::SetVolume(volume) => player.set_property("volume", volume.clamp(0., 100.)).await,
        UiCommand::ToggleMute => player.command(json!(["cycle", "mute"])).await,
        UiCommand::Audio(index) => {
            if let Some(aid) = mpv_track_id(source, MediaStreamType::Audio, index) {
                player.set_property("aid", aid).await?;
                report.audio_stream_index = Some(index);
            }
            Ok(())
        }
        UiCommand::Subtitle(choice) => {
            let sid = match choice {
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

fn report_or_log(result: jellyfin::Result<()>, what: &str) {
    if let Err(err) = result {
        tracing::warn!("playback {what} report failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that glob brings in GPUI's `test` attribute, which shadows the std one.
    use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

    use super::{mpv_subtitle_id, mpv_track_id};

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
}
