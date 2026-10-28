//! Remote control from other clients: the web dashboard or a phone app plays items here,
//! pauses, seeks, changes the volume or tracks, or shows a message. `live.rs` hands over the
//! commands from the session websocket.
//!
//! Transport controls go through [`Services::control`], so in a SyncPlay group they become group
//! requests like the player's own buttons; playing items goes through the playback service,
//! which hands them to the group's queue there.
//!
//! [`Remote`] is the part that needs nothing else: a `Play`'s items on their way from the server
//! (it sends ids only). The handlers are on [`Services`], below, since they drive playback.

use std::collections::VecDeque;
use std::time::Duration;

use jellyfin::models::BaseItemDto;
use jellyfin::{Client, GeneralCommand, ItemsQuery, PlayCommand, PlayRequest, PlaystateCommand, PlaystateRequest};

use crate::playback::{PlayOptions, SubtitleChoice, UiCommand};
use crate::services::{AppEvent, Services};
use crate::task::Task;
use crate::toasts::Toast;
use crate::views::item::shuffle;
use crate::views::player::{SEEK_STEP, VOLUME_STEP};

/// A remote "previous" goes back to the start of the item unless it's this close to it.
const RESTART_WITHIN: Duration = Duration::from_secs(5);

/// How a play request places its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Now,
    Shuffled,
    Next,
    Last,
}

/// What a play request asks for, with the items it names once they arrive.
struct Placing {
    placement: Placement,
    /// The request's ids, in its order, for [`in_order`].
    ids: Vec<String>,
    options: PlayOptions,
    items: Task<jellyfin::Result<Vec<BaseItemDto>>>,
}

/// Remote control's own part: the play requests waiting for their items, oldest first.
pub struct Remote {
    ctx: egui::Context,
    client: Client,
    /// Play requests whose items are arriving, in the order they were made. They're placed one
    /// at a time, oldest first, so a play next sent right after a play now still adds to the
    /// new session's queue, however the two fetches race.
    placing: VecDeque<Placing>,
}

impl Remote {
    pub fn new(client: Client, ctx: &egui::Context) -> Self {
        Self {
            ctx: ctx.clone(),
            client,
            placing: VecDeque::new(),
        }
    }

    /// The oldest play request whose items have arrived; later ones wait behind it.
    fn next_finished(&mut self) -> Option<(Placing, jellyfin::Result<Vec<BaseItemDto>>)> {
        let result = self.placing.front_mut()?.items.poll()?;
        Some((self.placing.pop_front()?, result))
    }
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

/// How a `Play` request places its items; `None` for the commands that aren't for this app
/// (instant mixes are for music).
fn placement(command: PlayCommand) -> Option<Placement> {
    match command {
        PlayCommand::PlayNow => Some(Placement::Now),
        PlayCommand::PlayShuffle => Some(Placement::Shuffled),
        PlayCommand::PlayNext => Some(Placement::Next),
        PlayCommand::PlayLast => Some(Placement::Last),
        PlayCommand::PlayInstantMix | PlayCommand::Other => None,
    }
}

impl Services {
    /// A `Play` from another client: play or queue the items it names. The server sends ids
    /// only, so the items are fetched first; [`Services::tick_remote`] places them when they
    /// arrive.
    pub fn on_remote_play(&mut self, request: PlayRequest) {
        tracing::info!(?request, "remote play");
        let Some(placement) = placement(request.play_command) else {
            return;
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
        let client = self.remote.client.clone();
        let query = ItemsQuery {
            ids: ids.clone(),
            ..Default::default()
        };
        let items = Task::spawn(&self.remote.ctx, async move {
            client.items(&query).await.map(|result| result.items)
        });
        self.remote.placing.push_back(Placing {
            placement,
            ids,
            options,
            items,
        });
    }

    /// Places the play requests whose items have arrived, one at a time in the order they were
    /// made, from [`Services::tick`].
    pub(crate) fn tick_remote(&mut self) {
        while let Some((placing, result)) = self.remote.next_finished() {
            let mut items = match result {
                Ok(items) => in_order(&placing.ids, items),
                Err(err) => {
                    // Only logged, as on `main`; an expired token still ends the session.
                    let message = self.handle_error(&err);
                    tracing::warn!("couldn't fetch the items to play remotely: {message}");
                    continue;
                }
            };
            if items.is_empty() {
                continue;
            }
            match placing.placement {
                Placement::Now => self.play_all_with(items, placing.options),
                // A shuffled list starts from its first shuffled item, not the request's start.
                Placement::Shuffled => {
                    shuffle(&mut items);
                    self.play_all(items);
                }
                Placement::Next => self.enqueue(items, true),
                Placement::Last => self.enqueue(items, false),
            }
        }
    }

    /// A `Playstate` from another client: pause, seek, stop and the like, through the same paths
    /// as the player's own controls (and so the group's, in a SyncPlay group).
    pub fn on_remote_playstate(&mut self, request: PlaystateRequest) {
        tracing::info!(?request, "remote playstate");
        let Some(state) = self.playback.now_playing() else {
            // A reconnect may be pending; Stop cancels it.
            if request.command == PlaystateCommand::Stop {
                self.stop_playback();
            }
            return;
        };
        let (position, previous) = (state.position, state.previous.is_some());
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
                self.play_next();
                return;
            }
            // Like a music player: back to the start, unless already near it and there's an
            // item before this one.
            PlaystateCommand::PreviousTrack if position <= RESTART_WITHIN && previous => {
                self.play_previous();
                return;
            }
            PlaystateCommand::PreviousTrack => UiCommand::SeekTo(Duration::ZERO),
            PlaystateCommand::Other => return,
        };
        self.control(command);
    }

    /// A `GeneralCommand` from another client: volume and mute, tracks, fullscreen, or a
    /// message. The server sends commands it wasn't told about too; those are ignored.
    pub fn on_remote_command(&mut self, command: GeneralCommand) {
        tracing::info!(?command, "remote command");
        if let GeneralCommand::DisplayMessage { header, text } = command {
            let message = match header {
                Some(header) => format!("{header}: {text}"),
                None => text,
            };
            self.toast(Toast::info(message));
            return;
        }
        // The rest act on what's playing.
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        // mpv's last reported volume: two steps in one batch move once, as on `main`.
        let volume = state.volume;
        let set_volume = |volume: f64| UiCommand::SetVolume(volume.clamp(0., 100.));
        let command = match command {
            GeneralCommand::VolumeUp => set_volume(volume + VOLUME_STEP),
            GeneralCommand::VolumeDown => set_volume(volume - VOLUME_STEP),
            GeneralCommand::SetVolume(volume) => set_volume(f64::from(volume)),
            GeneralCommand::Mute => UiCommand::SetMute(true),
            GeneralCommand::Unmute => UiCommand::SetMute(false),
            GeneralCommand::ToggleMute => UiCommand::ToggleMute,
            GeneralCommand::SetAudioStreamIndex(index) => {
                self.change_audio(index);
                return;
            }
            GeneralCommand::SetSubtitleStreamIndex(index) => {
                let choice = if index < 0 {
                    SubtitleChoice::Off
                } else {
                    SubtitleChoice::Stream(index)
                };
                self.change_subtitle(choice);
                return;
            }
            // The app owns the window, so it carries this one out.
            GeneralCommand::ToggleFullscreen => {
                self.events.push(AppEvent::ToggleFullscreen);
                return;
            }
            GeneralCommand::DisplayMessage { .. } | GeneralCommand::Other(_) => return,
        };
        self.control(command);
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::PlayCommand;
    use jellyfin::models::BaseItemDto;

    use super::{Placement, in_order, placement};

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

    #[test]
    fn play_commands_place_their_items() {
        assert_eq!(placement(PlayCommand::PlayNow), Some(Placement::Now));
        assert_eq!(placement(PlayCommand::PlayShuffle), Some(Placement::Shuffled));
        assert_eq!(placement(PlayCommand::PlayNext), Some(Placement::Next));
        assert_eq!(placement(PlayCommand::PlayLast), Some(Placement::Last));
        // Instant mixes are for music, and unknown commands aren't ours to act on.
        assert_eq!(placement(PlayCommand::PlayInstantMix), None);
        assert_eq!(placement(PlayCommand::Other), None);
    }
}
