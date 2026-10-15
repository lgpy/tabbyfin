//! Remote control from other clients: the web dashboard or a phone app plays items here,
//! pauses, seeks, changes the volume or tracks, or shows a message. `live.rs` hands over the
//! commands from the session websocket.
//!
//! Transport controls go through [`NowPlaying::control`], so in a SyncPlay group they become
//! group requests like the player's own buttons; playing items goes through the
//! [`PlaybackManager`], which hands them to the group's queue there.

use std::time::Duration;

use gpui_kit::*;
use jellyfin::models::BaseItemDto;
use jellyfin::{Client, GeneralCommand, ItemsQuery, PlayCommand, PlayRequest, PlaystateCommand, PlaystateRequest};

use crate::playback::{NowPlaying, PlayOptions, PlaybackManager, SubtitleChoice, UiCommand};
use crate::runtime;
use crate::views::item::shuffle;
use crate::views::player::{SEEK_STEP, VOLUME_STEP};

/// A remote "previous" goes back to the start of the item unless it's this close to it.
const RESTART_WITHIN: Duration = Duration::from_secs(5);

pub enum RemoteEvent {
    /// A message sent from another client, to show as a notification.
    Message(SharedString),
    ToggleFullscreen,
}

pub struct Remote {
    client: Client,
}

struct GlobalRemote(Entity<Remote>);

impl Global for GlobalRemote {}

impl EventEmitter<RemoteEvent> for Remote {}

/// How a play request places its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Now,
    Shuffled,
    Next,
    Last,
}

impl Remote {
    /// Installs the handler for a signed-in session.
    pub fn init(client: Client, cx: &mut App) {
        let remote = cx.new(|_| Remote { client });
        cx.set_global(GlobalRemote(remote));
    }

    pub fn global(cx: &App) -> Entity<Remote> {
        cx.global::<GlobalRemote>().0.clone()
    }

    /// Plays or queues the requested items. The server sends ids only, so the items are
    /// fetched first.
    pub fn handle_play(&mut self, request: PlayRequest, cx: &mut Context<Self>) {
        tracing::info!(?request, "remote play");
        let placement = match request.play_command {
            PlayCommand::PlayNow => Placement::Now,
            PlayCommand::PlayShuffle => Placement::Shuffled,
            PlayCommand::PlayNext => Placement::Next,
            PlayCommand::PlayLast => Placement::Last,
            // Instant mixes are for music.
            PlayCommand::PlayInstantMix | PlayCommand::Other => return,
        };
        let mut ids = request.item_ids;
        ids.drain(..request.start_index.unwrap_or(0).min(ids.len()));
        if ids.is_empty() {
            return;
        }
        let options = PlayOptions {
            start: request.start_position_ticks,
            media_source_id: request.media_source_id,
            audio_stream_index: request.audio_stream_index,
            subtitle: match request.subtitle_stream_index {
                Some(-1) => SubtitleChoice::Off,
                Some(index) => SubtitleChoice::Stream(index),
                None => SubtitleChoice::Auto,
            },
            ..Default::default()
        };
        let client = self.client.clone();
        cx.spawn(async move |_, cx| {
            let query = ItemsQuery {
                ids: ids.clone(),
                ..Default::default()
            };
            let fetched = runtime::spawn(async move { client.items(&query).await }).await;
            let mut items = match fetched {
                Ok(result) => in_order(&ids, result.items),
                Err(err) => {
                    tracing::warn!("couldn't fetch the items to play remotely: {err}");
                    return;
                }
            };
            if items.is_empty() {
                return;
            }
            cx.update(|cx| {
                PlaybackManager::global(cx).update(cx, |playback, cx| match placement {
                    Placement::Now => playback.play_all_with(items, options, cx),
                    Placement::Shuffled => {
                        shuffle(&mut items);
                        playback.play_all(items, cx);
                    }
                    Placement::Next => playback.enqueue(items, true, cx),
                    Placement::Last => playback.enqueue(items, false, cx),
                })
            });
        })
        .detach();
    }

    pub fn handle_playstate(&mut self, request: PlaystateRequest, cx: &mut Context<Self>) {
        tracing::info!(?request, "remote playstate");
        let playback = PlaybackManager::global(cx);
        let Some(now_playing) = playback.read(cx).now_playing() else {
            // A reconnect may be pending; Stop cancels it.
            if request.command == PlaystateCommand::Stop {
                playback.update(cx, |playback, cx| playback.stop(cx));
            }
            return;
        };
        let command = match request.command {
            PlaystateCommand::Stop => UiCommand::Stop,
            PlaystateCommand::Pause => UiCommand::SetPause(true),
            PlaystateCommand::Unpause => UiCommand::SetPause(false),
            PlaystateCommand::PlayPause => UiCommand::TogglePause,
            PlaystateCommand::Seek => match request.seek_position_ticks {
                Some(position) => UiCommand::SeekTo(position.into()),
                None => return,
            },
            PlaystateCommand::Rewind => UiCommand::SeekBy(-SEEK_STEP),
            PlaystateCommand::FastForward => UiCommand::SeekBy(SEEK_STEP),
            PlaystateCommand::NextTrack => {
                now_playing.update(cx, |state, cx| state.play_next(cx));
                return;
            }
            // Like a music player: back to the start, unless already near it and there's an
            // item before this one.
            PlaystateCommand::PreviousTrack => {
                let state = now_playing.read(cx);
                if state.position <= RESTART_WITHIN && state.previous.is_some() {
                    now_playing.update(cx, |state, cx| state.play_previous(cx));
                    return;
                }
                UiCommand::SeekTo(Duration::ZERO)
            }
            PlaystateCommand::Other => return,
        };
        control(&now_playing, command, cx);
    }

    pub fn handle_command(&mut self, command: GeneralCommand, cx: &mut Context<Self>) {
        tracing::info!(?command, "remote command");
        if let GeneralCommand::DisplayMessage { header, text } = command {
            let message = match header {
                Some(header) => format!("{header}: {text}"),
                None => text,
            };
            cx.emit(RemoteEvent::Message(message.into()));
            return;
        }
        // The rest act on what's playing.
        let Some(now_playing) = PlaybackManager::global(cx).read(cx).now_playing() else {
            return;
        };
        let volume = now_playing.read(cx).volume;
        let set_volume = |volume: f64| UiCommand::SetVolume(volume.clamp(0., 100.));
        let command = match command {
            GeneralCommand::VolumeUp => set_volume(volume + VOLUME_STEP),
            GeneralCommand::VolumeDown => set_volume(volume - VOLUME_STEP),
            GeneralCommand::SetVolume(volume) => set_volume(f64::from(volume)),
            GeneralCommand::Mute => UiCommand::SetMute(true),
            GeneralCommand::Unmute => UiCommand::SetMute(false),
            GeneralCommand::ToggleMute => UiCommand::ToggleMute,
            GeneralCommand::SetAudioStreamIndex(index) => {
                now_playing.update(cx, |state, cx| state.change_audio(index, cx));
                return;
            }
            GeneralCommand::SetSubtitleStreamIndex(index) => {
                let choice = if index < 0 {
                    SubtitleChoice::Off
                } else {
                    SubtitleChoice::Stream(index)
                };
                now_playing.update(cx, |state, cx| state.change_subtitle(choice, cx));
                return;
            }
            GeneralCommand::ToggleFullscreen => {
                cx.emit(RemoteEvent::ToggleFullscreen);
                return;
            }
            GeneralCommand::DisplayMessage { .. } | GeneralCommand::Other(_) => return,
        };
        control(&now_playing, command, cx);
    }
}

fn control(now_playing: &Entity<NowPlaying>, command: UiCommand, cx: &mut App) {
    now_playing.update(cx, |state, cx| state.control(command, cx));
}

/// The fetched items in the requested order (the server returns them in its own), leaving
/// out ids it didn't return.
fn in_order(ids: &[String], items: Vec<BaseItemDto>) -> Vec<BaseItemDto> {
    let mut items: Vec<Option<BaseItemDto>> = items.into_iter().map(Some).collect();
    ids.iter()
        .filter_map(|id| {
            items
                .iter_mut()
                .find(|item| item.as_ref().is_some_and(|i| &i.id == id))?
                .take()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::in_order;

    fn item(id: &str) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({ "Id": id, "Name": id })).expect("item")
    }

    #[test]
    fn items_keep_the_requested_order() {
        let ids = ["c", "a", "b", "gone"].map(String::from);
        let items = in_order(&ids, vec![item("a"), item("b"), item("c")]);
        let order: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
    }
}
