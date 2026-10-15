//! SyncPlay (watch together): group requests over HTTP, and the typed group updates and
//! playback commands the server sends back over the session websocket. See `docs/ROADMAP.md`
//! §2.4 for how the protocol works.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::client::Query;
use crate::time::{format_utc, parse_utc};
use crate::{Client, Result, Ticks};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum GroupState {
    #[default]
    Idle,
    /// Waiting for members to load or buffer.
    Waiting,
    Paused,
    Playing,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GroupInfo {
    pub group_id: String,
    pub group_name: String,
    #[serde(default)]
    pub state: GroupState,
    /// User names.
    #[serde(default)]
    pub participants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueItem {
    pub item_id: String,
    /// Identifies this entry in the queue (the same item can be queued twice).
    pub playlist_item_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayQueue {
    /// Why it changed: `NewPlaylist`, `SetCurrentItem`, `NextItem`, `Queue`…
    pub reason: String,
    #[serde(default)]
    pub playlist: Vec<QueueItem>,
    /// `-1` when nothing is selected.
    pub playing_item_index: i32,
    #[serde(default)]
    pub start_position_ticks: Ticks,
    #[serde(default)]
    pub is_playing: bool,
}

impl PlayQueue {
    pub fn current(&self) -> Option<&QueueItem> {
        usize::try_from(self.playing_item_index)
            .ok()
            .and_then(|i| self.playlist.get(i))
    }
}

#[derive(Debug, Clone)]
pub enum GroupUpdate {
    /// This session joined (or created) a group.
    GroupJoined(GroupInfo),
    GroupLeft,
    UserJoined(String),
    UserLeft(String),
    StateUpdate {
        state: GroupState,
        /// The request that caused it (`Pause`, `Seek`, `Buffer`…).
        reason: String,
    },
    PlayQueue(PlayQueue),
    NotInGroup,
    GroupDoesNotExist,
    LibraryAccessDenied,
}

impl GroupUpdate {
    /// From a `SyncPlayGroupUpdate` message's `Data`. `None` for update types we don't know.
    pub(crate) fn parse(data: serde_json::Value) -> serde_json::Result<Option<Self>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Raw {
            #[serde(rename = "Type")]
            kind: String,
            #[serde(default)]
            data: serde_json::Value,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct State {
            state: GroupState,
            #[serde(default)]
            reason: String,
        }
        let raw: Raw = serde_json::from_value(data)?;
        let text = |data: serde_json::Value| data.as_str().unwrap_or_default().to_owned();
        Ok(Some(match raw.kind.as_str() {
            "GroupJoined" => GroupUpdate::GroupJoined(serde_json::from_value(raw.data)?),
            "GroupLeft" => GroupUpdate::GroupLeft,
            "UserJoined" => GroupUpdate::UserJoined(text(raw.data)),
            "UserLeft" => GroupUpdate::UserLeft(text(raw.data)),
            "StateUpdate" => {
                let state: State = serde_json::from_value(raw.data)?;
                GroupUpdate::StateUpdate {
                    state: state.state,
                    reason: state.reason,
                }
            }
            "PlayQueue" => GroupUpdate::PlayQueue(serde_json::from_value(raw.data)?),
            "NotInGroup" => GroupUpdate::NotInGroup,
            "GroupDoesNotExist" => GroupUpdate::GroupDoesNotExist,
            "LibraryAccessDenied" => GroupUpdate::LibraryAccessDenied,
            _ => return Ok(None),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CommandKind {
    Unpause,
    Pause,
    Stop,
    Seek,
}

/// A playback command for every member, to carry out at `when` (server time).
#[derive(Debug, Clone)]
pub struct Command {
    pub kind: CommandKind,
    pub playlist_item_id: String,
    pub when: SystemTime,
    pub position: Ticks,
}

impl Command {
    /// From a `SyncPlayCommand` message's `Data`.
    pub(crate) fn parse(data: serde_json::Value) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Raw {
            command: CommandKind,
            #[serde(default)]
            playlist_item_id: String,
            when: String,
            #[serde(default)]
            position_ticks: Ticks,
        }
        let raw: Raw = serde_json::from_value(data)?;
        let when =
            parse_utc(&raw.when).ok_or_else(|| serde::de::Error::custom(format!("unreadable time {:?}", raw.when)))?;
        Ok(Self {
            kind: raw.command,
            playlist_item_id: raw.playlist_item_id,
            when,
            position: raw.position_ticks,
        })
    }
}

/// Where this member is, for `Buffering` and `Ready`.
#[derive(Debug, Clone)]
pub struct BufferReport {
    /// Server time of the report.
    pub when: SystemTime,
    pub position: Ticks,
    pub is_playing: bool,
    pub playlist_item_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct BufferBody<'a> {
    when: String,
    position_ticks: Ticks,
    is_playing: bool,
    playlist_item_id: &'a str,
}

impl BufferReport {
    fn body(&self) -> BufferBody<'_> {
        BufferBody {
            when: format_utc(self.when),
            position_ticks: self.position,
            is_playing: self.is_playing,
            playlist_item_id: &self.playlist_item_id,
        }
    }
}

impl Client {
    pub async fn syncplay_groups(&self) -> Result<Vec<GroupInfo>> {
        self.get("/SyncPlay/List", &Query::new()).await
    }

    /// Creates a group and joins it; a `GroupJoined` update follows on the websocket.
    pub async fn syncplay_new(&self, name: &str) -> Result<()> {
        self.syncplay_post("New", serde_json::json!({ "GroupName": name }))
            .await
    }

    pub async fn syncplay_join(&self, group_id: &str) -> Result<()> {
        self.syncplay_post("Join", serde_json::json!({ "GroupId": group_id }))
            .await
    }

    pub async fn syncplay_leave(&self) -> Result<()> {
        self.syncplay_post("Leave", serde_json::json!({})).await
    }

    /// Replaces the group's queue and starts `items[index]` at `start` for every member.
    pub async fn syncplay_set_queue(&self, item_ids: &[String], index: usize, start: Ticks) -> Result<()> {
        let body = serde_json::json!({
            "PlayingQueue": item_ids,
            "PlayingItemPosition": index,
            "StartPositionTicks": start,
        });
        self.syncplay_post("SetNewQueue", body).await
    }

    /// Adds items to the group's queue: right after the playing entry with `next`, otherwise
    /// at the end.
    pub async fn syncplay_queue(&self, item_ids: &[String], next: bool) -> Result<()> {
        let body = serde_json::json!({
            "ItemIds": item_ids,
            "Mode": if next { "QueueNext" } else { "Queue" },
        });
        self.syncplay_post("Queue", body).await
    }

    pub async fn syncplay_pause(&self) -> Result<()> {
        self.syncplay_post("Pause", serde_json::json!({})).await
    }

    pub async fn syncplay_unpause(&self) -> Result<()> {
        self.syncplay_post("Unpause", serde_json::json!({})).await
    }

    pub async fn syncplay_seek(&self, position: Ticks) -> Result<()> {
        self.syncplay_post("Seek", serde_json::json!({ "PositionTicks": position }))
            .await
    }

    /// Stops playback for the whole group.
    pub async fn syncplay_stop(&self) -> Result<()> {
        self.syncplay_post("Stop", serde_json::json!({})).await
    }

    /// Moves the group to the queue entry after `playlist_item_id` (when that one ended).
    pub async fn syncplay_next_item(&self, playlist_item_id: &str) -> Result<()> {
        self.syncplay_post("NextItem", serde_json::json!({ "PlaylistItemId": playlist_item_id }))
            .await
    }

    pub async fn syncplay_buffering(&self, report: &BufferReport) -> Result<()> {
        self.post_no_content("/SyncPlay/Buffering", &Query::new(), Some(&report.body()))
            .await
    }

    pub async fn syncplay_ready(&self, report: &BufferReport) -> Result<()> {
        self.post_no_content("/SyncPlay/Ready", &Query::new(), Some(&report.body()))
            .await
    }

    /// Round-trip time to the server, which it uses to schedule commands with enough lead.
    pub async fn syncplay_ping(&self, ping_ms: u64) -> Result<()> {
        self.syncplay_post("Ping", serde_json::json!({ "Ping": ping_ms })).await
    }

    async fn syncplay_post(&self, endpoint: &str, body: serde_json::Value) -> Result<()> {
        self.post_no_content(&format!("/SyncPlay/{endpoint}"), &Query::new(), Some(&body))
            .await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Command, CommandKind, GroupState, GroupUpdate};
    use crate::time::parse_utc;

    #[test]
    fn parses_group_updates() {
        let joined = json!({"GroupId": "g", "Type": "GroupJoined", "Data": {
            "GroupId": "g", "GroupName": "Movie night", "State": "Idle",
            "Participants": ["dev"], "LastUpdatedAt": "2026-10-14T19:00:00Z"}});
        match GroupUpdate::parse(joined).unwrap() {
            Some(GroupUpdate::GroupJoined(info)) => {
                assert_eq!(info.group_name, "Movie night");
                assert_eq!(info.participants, ["dev"]);
            }
            other => panic!("{other:?}"),
        }
        let queue = json!({"GroupId": "g", "Type": "PlayQueue", "Data": {
            "Reason": "NewPlaylist", "LastUpdate": "2026-10-14T19:00:00Z",
            "Playlist": [{"ItemId": "a", "PlaylistItemId": "p1"}, {"ItemId": "b", "PlaylistItemId": "p2"}],
            "PlayingItemIndex": 1, "StartPositionTicks": 50, "IsPlaying": true,
            "ShuffleMode": "Sorted", "RepeatMode": "RepeatNone"}});
        match GroupUpdate::parse(queue).unwrap() {
            Some(GroupUpdate::PlayQueue(queue)) => {
                assert_eq!(queue.current().map(|i| i.item_id.as_str()), Some("b"));
                assert!(queue.is_playing);
            }
            other => panic!("{other:?}"),
        }
        let state = json!({"GroupId": "g", "Type": "StateUpdate", "Data": {"State": "Waiting", "Reason": "Seek"}});
        assert!(matches!(
            GroupUpdate::parse(state).unwrap(),
            Some(GroupUpdate::StateUpdate {
                state: GroupState::Waiting,
                ..
            })
        ));
        let user = json!({"GroupId": "g", "Type": "UserJoined", "Data": "friend"});
        assert!(matches!(GroupUpdate::parse(user).unwrap(), Some(GroupUpdate::UserJoined(name)) if name == "friend"));
        assert!(
            GroupUpdate::parse(json!({"GroupId": "g", "Type": "Brand new"}))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn parses_commands() {
        let command = Command::parse(json!({
            "GroupId": "g", "PlaylistItemId": "p1", "When": "2026-10-14T19:00:01.5Z",
            "PositionTicks": 100000000, "Command": "Unpause", "EmittedAt": "2026-10-14T19:00:00Z"}))
        .unwrap();
        assert_eq!(command.kind, CommandKind::Unpause);
        assert_eq!(command.when, parse_utc("2026-10-14T19:00:01.5Z").unwrap());
        assert_eq!(command.position.as_secs_f64(), 10.);
    }
}
