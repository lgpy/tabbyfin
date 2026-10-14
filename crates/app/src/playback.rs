//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio; the GPUI side only hears when it ends, so pages can refresh watched state.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, MediaSourceInfo, MediaStream, MediaStreamType};
use jellyfin::{Client, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::{EndReason, LoadRequest, Player, PlayerEvent, SpawnOptions};
use tokio::sync::oneshot;

use crate::changes::Changes;
use crate::config::{Config, PlayerSettings};
use crate::runtime;
use crate::session::describe_error;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

pub enum PlaybackEvent {
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

pub struct PlaybackManager {
    client: Client,
    /// Stops the running session when sent or dropped.
    stop: Option<oneshot::Sender<()>>,
}

struct GlobalPlayback(Entity<PlaybackManager>);

impl Global for GlobalPlayback {}

impl EventEmitter<PlaybackEvent> for PlaybackManager {}

impl PlaybackManager {
    /// Installs the manager for a signed-in session, replacing (and stopping) any previous one.
    pub fn init(client: Client, cx: &mut App) {
        let manager = cx.new(|_| PlaybackManager { client, stop: None });
        cx.set_global(GlobalPlayback(manager));
    }

    pub fn global(cx: &App) -> Entity<PlaybackManager> {
        cx.global::<GlobalPlayback>().0.clone()
    }

    /// Stops the current session, if any.
    pub fn stop(&mut self) {
        self.stop = None;
    }

    pub fn play(&mut self, item: &BaseItemDto, options: PlayOptions, cx: &mut Context<Self>) {
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);

        let client = self.client.clone();
        let item = item.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move {
                let settings = Config::load().player;
                run_session(client, item, options, settings, stop_rx).await
            })
            .await;
            this.update(cx, |_, cx| {
                if let Err(message) = result {
                    tracing::warn!("playback failed: {message}");
                    cx.emit(PlaybackEvent::Failed(message.into()));
                }
                Changes::notify_user_data(cx);
            })
            .ok();
        })
        .detach();
    }
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

/// Runs one playback from negotiation to the final stop report. Returns a user-facing error.
async fn run_session(
    client: Client,
    item: BaseItemDto,
    options: PlayOptions,
    settings: PlayerSettings,
    mut stop: oneshot::Receiver<()>,
) -> Result<(), String> {
    let subtitle_index = match options.subtitle {
        SubtitleChoice::Stream(index) => Some(index),
        SubtitleChoice::Off => Some(-1),
        SubtitleChoice::Auto => None,
    };
    let info = client
        .playback_info(
            &item.id,
            &PlaybackInfoRequest {
                media_source_id: options.media_source_id.clone(),
                start_position: options.start,
                audio_stream_index: options.audio_stream_index,
                subtitle_stream_index: subtitle_index,
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

    let spawn_options = SpawnOptions {
        mpv_path: settings.mpv_path.map(PathBuf::from),
        extra_args: if settings.use_mpv_config { Vec::new() } else { vec!["--no-config".into()] },
    };
    let (player, mut events) = Player::spawn(spawn_options).await.map_err(|err| match err {
        player::Error::NotFound(path) => format!(
            "mpv wasn't found ({}). Install mpv, or set its location in Settings.",
            path.display()
        ),
        err => format!("Couldn't start mpv: {err}"),
    })?;

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

    let external_subtitles: Vec<_> = source.media_streams.iter().filter(|s| s.is_external).cloned().collect();
    let mut report = PlaybackReport {
        item_id: item.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: info.play_session_id.clone(),
        position: options.start.unwrap_or_default(),
        audio_stream_index: options.audio_stream_index,
        subtitle_stream_index: subtitle_index,
        ..Default::default()
    };
    let mut started = false;
    let mut stopping = false;
    let mut error = None;
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
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
                        if tracing::enabled!(tracing::Level::DEBUG) {
                            let aid = player.command(serde_json::json!(["get_property", "aid"])).await;
                            let sid = player.command(serde_json::json!(["get_property", "sid"])).await;
                            tracing::debug!(?aid, ?sid, "mpv tracks after load");
                        }
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
                    PlayerEvent::EndFile(EndReason::Error(message)) => {
                        error = Some(format!("mpv couldn't play this file: {message}"));
                    }
                    PlayerEvent::Exited => break,
                    _ => {}
                }
            }
            _ = ticker.tick(), if started => {
                report_or_log(client.report_playback_progress(&report).await, "progress");
            }
            _ = &mut stop, if !stopping => {
                // Replaced by a new session or signed out: ask mpv to quit and keep going until
                // it exits, so the final position gets reported.
                stopping = true;
                let _ = player.quit().await;
            }
        }
    }

    if started {
        report_or_log(client.report_playback_stopped(&report).await, "stop");
    }
    tracing::info!(item = %item.name, position = report.position.as_secs_f64(), "playback ended");
    error.map_or(Ok(()), Err)
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

    use super::mpv_track_id;

    fn stream(index: i32, kind: MediaStreamType, is_external: bool) -> MediaStream {
        serde_json::from_value(serde_json::json!({
            "Index": index,
            "Type": kind.as_str(),
            "IsExternal": is_external,
        }))
        .unwrap()
    }

    #[test]
    fn maps_jellyfin_indexes_to_mpv_track_ids() {
        let source: MediaSourceInfo = serde_json::from_value(serde_json::json!({ "Id": "s" })).unwrap();
        let source = MediaSourceInfo {
            media_streams: vec![
                stream(0, MediaStreamType::Subtitle, true),
                stream(1, MediaStreamType::Video, false),
                stream(2, MediaStreamType::Audio, false),
                stream(3, MediaStreamType::Audio, false),
                stream(4, MediaStreamType::Subtitle, false),
            ],
            ..source
        };
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 2), Some(1));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 3), Some(2));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 4), Some(1));
        // External streams aren't in the file.
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 0), None);
    }
}
