//! SyncPlay: group state, requests, scheduled commands at the server's time, and drift
//! correction. P2 fills it.
//!
//! Until then, this is the seam playback (`playback.rs`) calls: the `impl Services` block below
//! answers "not in a group", so playback never takes its SyncPlay branches. Each method is one
//! of `main`'s calls into the `SyncPlay` global; P2 replaces their bodies with calls into
//! `self.syncplay` and keeps the signatures.

use std::time::Duration;

use jellyfin::Ticks;
use jellyfin::models::BaseItemDto;

use crate::playback::PlayOptions;
use crate::services::Services;

/// A local control, sent to the group instead.
#[derive(Debug, Clone, Copy)]
pub enum GroupRequest {
    SetPause(bool),
    Seek(Duration),
    Stop,
}

impl Services {
    /// Whether this session is in a SyncPlay group, which then decides what plays: playing
    /// something gives the group a new queue instead. `main`: `SyncPlay.group.is_some()`.
    pub fn in_group(&self) -> bool {
        false
    }

    /// A pause, seek or stop from a group session's controls (`SyncPlay::request`).
    pub(crate) fn group_request(&mut self, request: GroupRequest) {
        not_in_group("request", &request);
    }

    /// Plays `items` in order for the whole group (`SyncPlay::start_items`).
    pub(crate) fn group_start_items(&mut self, items: &[BaseItemDto]) {
        not_in_group("start_items", &items.len());
    }

    /// Plays `item` for the whole group from `start` (`SyncPlay::start_queue`).
    pub(crate) fn group_start_queue(&mut self, item: &BaseItemDto, start: Ticks) {
        not_in_group("start_queue", &(&item.id, start));
    }

    /// Adds `items` to the group's queue (`SyncPlay::enqueue`).
    pub(crate) fn group_enqueue(&mut self, items: &[BaseItemDto], next: bool) {
        not_in_group("enqueue", &(items.len(), next));
    }

    /// Loads the group's current item again here only, another version or tracks
    /// (`SyncPlay::reload`).
    pub(crate) fn group_reload(&mut self, item: &BaseItemDto, options: PlayOptions) {
        not_in_group("reload", &(&item.id, options));
    }

    /// The group's item played to its end here (`SyncPlay::item_ended`).
    pub(crate) fn group_item_ended(&mut self, playlist_item_id: &str) {
        not_in_group("item_ended", &playlist_item_id);
    }

    /// A session started (`self.playback.now_playing()` is its state). `main`'s SyncPlay
    /// subscribed to `PlaybackEvent::Started` for this: a group session's player is then
    /// watched for restarts (`Ready`) and drift.
    pub(crate) fn group_playback_started(&mut self) {}
}

/// The group calls are only reached for a group session or while in a group, which can't
/// happen before P2.
fn not_in_group(call: &str, details: &dyn std::fmt::Debug) {
    tracing::warn!(?details, "SyncPlay {call} without SyncPlay (arrives with P2)");
}
