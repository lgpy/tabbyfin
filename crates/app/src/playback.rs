//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio; the GPUI side only hears when it ends, so pages can refresh watched state.

use std::time::Duration;

use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{Client, PlaybackInfoRequest, PlaybackReport, Ticks};
use player::{EndReason, LoadRequest, Player, PlayerEvent, SpawnOptions};
use tokio::sync::oneshot;

use crate::runtime;
use crate::session::describe_error;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

pub enum PlaybackEvent {
    /// A session ended (normally or not). Watched state and resume points may have changed.
    Stopped,
    Failed(SharedString),
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

    /// Plays `item` from `start` (or the beginning).
    pub fn play(&mut self, item: &BaseItemDto, start: Option<Ticks>, cx: &mut Context<Self>) {
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);

        let client = self.client.clone();
        let item = item.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(run_session(client, item, start, stop_rx)).await;
            this.update(cx, |_, cx| {
                if let Err(message) = result {
                    tracing::warn!("playback failed: {message}");
                    cx.emit(PlaybackEvent::Failed(message.into()));
                }
                cx.emit(PlaybackEvent::Stopped);
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

/// Runs one playback from negotiation to the final stop report. Returns a user-facing error.
async fn run_session(
    client: Client,
    item: BaseItemDto,
    start: Option<Ticks>,
    mut stop: oneshot::Receiver<()>,
) -> Result<(), String> {
    let info = client
        .playback_info(
            &item.id,
            &PlaybackInfoRequest {
                start_position: start,
                ..Default::default()
            },
        )
        .await
        .map_err(|err| describe_error(&err))?;
    if let Some(code) = info.error_code {
        return Err(format!("The server won't play this item ({code})."));
    }
    let source = info
        .media_sources
        .into_iter()
        .next()
        .ok_or("The server returned no playable media.")?;

    let (player, mut events) = Player::spawn(SpawnOptions::default())
        .await
        .map_err(|err| match err {
            player::Error::NotFound(path) => format!(
                "mpv wasn't found ({}). Install mpv and make sure it's on your PATH.",
                path.display()
            ),
            err => format!("Couldn't start mpv: {err}"),
        })?;

    let url = client.stream_url(&item.id, &source.id, info.play_session_id.as_deref());
    player
        .load(LoadRequest {
            url: url.to_string(),
            title: Some(title(&item)),
            start: start.map(Duration::from),
            http_headers: vec![format!("Authorization: {}", client.authorization_header())],
        })
        .await
        .map_err(|err| format!("mpv couldn't open the stream: {err}"))?;

    let external_subtitles: Vec<_> = source.media_streams.iter().filter(|s| s.is_external).cloned().collect();
    let mut report = PlaybackReport {
        item_id: item.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: info.play_session_id.clone(),
        position: start.unwrap_or_default(),
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
                            if let Err(err) = player.add_subtitle(url.as_str(), title, stream.language.as_deref()).await {
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
