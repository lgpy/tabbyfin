//! What an item's menu and actions offer: played and favorite state, resume points, what can be
//! played, marked or added to a playlist. The menu itself is `widgets/menu.rs` and the item
//! page's own buttons are U6's.
//!
//! It also carries the item page's playlist edits to the server: the page changes its rows at
//! once and queues the change here, and [`Services`] sends them one at a time in the order they
//! were made, whatever page is on top (they used to be the page's own tasks, which stopped when
//! the page was left and only ran while it was drawn).

use std::collections::VecDeque;

use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{Client, Ticks};

use crate::services::Services;
use crate::task;
use crate::toasts::Toast;

#[derive(Debug, Clone, Copy)]
pub enum Mark {
    Played(bool),
    Favorite(bool),
}

impl Mark {
    /// Whether `item` already shows this.
    pub fn shown_on(self, item: &BaseItemDto) -> bool {
        match self {
            Mark::Played(played) => is_played(item) == played,
            Mark::Favorite(favorite) => is_favorite(item) == favorite,
        }
    }

    /// Shows this on `item`, ahead of the server. The rest of its user data (a played item's
    /// resume point) comes with the reload that follows.
    pub fn apply(self, item: &mut BaseItemDto) {
        let data = item.user_data.get_or_insert_default();
        match self {
            Mark::Played(played) => data.played = played,
            Mark::Favorite(favorite) => data.is_favorite = favorite,
        }
    }

    pub fn undone(self) -> Mark {
        match self {
            Mark::Played(played) => Mark::Played(!played),
            Mark::Favorite(favorite) => Mark::Favorite(!favorite),
        }
    }
}

pub fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

pub fn is_favorite(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.is_favorite)
}

/// Where playback would resume, if it's partway through.
pub fn resume_point(item: &BaseItemDto) -> Option<Ticks> {
    item.user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0)
}

pub fn is_playable(item: &BaseItemDto) -> bool {
    matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
    )
}

/// The episode a series' or season's Play starts, out of its episodes in order: the first
/// that isn't watched (resumed if it's partway), or the first if they all are. Specials
/// (season 0) only count when there's nothing else, as with Jellyfin's web client.
pub fn first_unwatched(episodes: &[BaseItemDto]) -> Option<&BaseItemDto> {
    let regular: Vec<_> = episodes.iter().filter(|e| e.parent_index_number != Some(0)).collect();
    let candidates = if regular.is_empty() {
        episodes.iter().collect()
    } else {
        regular
    };
    candidates
        .iter()
        .find(|e| !is_played(e))
        .or(candidates.first())
        .copied()
}

/// All of a series' episodes in order, or a season's.
pub async fn episodes_of(client: &Client, item: &BaseItemDto) -> jellyfin::Result<Vec<BaseItemDto>> {
    Ok(match item.kind {
        BaseItemKind::Series => client.episodes(&item.id, None, &[]).await?.items,
        BaseItemKind::Season => {
            let series_id = item.series_id.as_deref().unwrap_or_default();
            client.episodes(series_id, Some(&item.id), &[]).await?.items
        }
        _ => Vec::new(),
    })
}

/// Movies, episodes and what holds episodes can go in a playlist.
pub fn can_add_to_playlist(item: &BaseItemDto) -> bool {
    is_playable(item) || matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season)
}

/// What has watched and favorite state worth setting from a card.
pub fn can_mark(item: &BaseItemDto) -> bool {
    can_add_to_playlist(item) || item.kind == BaseItemKind::BoxSet
}

/// A change to a playlist's entries, by entry id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlaylistEdit {
    Move { entry_id: String, to: usize },
    Remove { entry_id: String },
}

/// One playlist edit's outcome, as its request finishes: the playlist it was for, and how it
/// went.
pub(crate) type EditDone = (String, jellyfin::Result<()>);

/// The playlist changes on their way to the server: in the order they were made, one request at
/// a time, from `Services::tick` whatever page is on top. The item page shows its edits at once
/// and asks [`Services::playlist_edits_pending`] whether its playlist still has any, instead of
/// holding the requests itself.
pub(crate) struct PlaylistEdits {
    /// Edits waiting, oldest first, with the playlist each is for.
    queue: VecDeque<(String, PlaylistEdit)>,
    /// The playlist whose edit is out now, if one is.
    sending: Option<String>,
    /// What finished requests reported, for `Services::tick` to drain.
    done: task::Receiver<EditDone>,
    done_from: task::Sender<EditDone>,
}

impl PlaylistEdits {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let (done_from, done) = task::channel(ctx);
        Self {
            queue: VecDeque::new(),
            sending: None,
            done,
            done_from,
        }
    }

    /// Queues an edit; the page has shown it already.
    pub(crate) fn push(&mut self, playlist_id: String, edit: PlaylistEdit) {
        self.queue.push_back((playlist_id, edit));
    }

    /// Whether `playlist_id` still has an edit out or waiting.
    pub(crate) fn pending(&self, playlist_id: &str) -> bool {
        self.sending.as_deref() == Some(playlist_id) || self.queue.iter().any(|(id, _)| id == playlist_id)
    }

    /// Drops what hasn't gone for `playlist_id`: the server refused one of its edits, and the
    /// page shows the server's order again.
    pub(crate) fn clear(&mut self, playlist_id: &str) {
        self.queue.retain(|(id, _)| id != playlist_id);
    }
}

impl Services {
    /// Queues a change to a playlist, which the page has shown already. A queued edit goes as
    /// soon as the one before it is done and the next [`Services::tick`] runs, so moves land in
    /// the order they were made even when the user leaves the page or minimises the window.
    pub fn edit_playlist(&mut self, playlist_id: String, edit: PlaylistEdit) {
        self.playlist_edits.push(playlist_id, edit);
        self.send_next_playlist_edit();
    }

    /// Whether edits for `playlist_id` are still on their way: the page doesn't load over them
    /// until they're done (the last one bumps `user_data`, which loads it again).
    pub fn playlist_edits_pending(&self, playlist_id: &str) -> bool {
        self.playlist_edits.pending(playlist_id)
    }

    /// Sends the oldest queued edit, unless one is still out. The result comes back through the
    /// channel and [`Services::tick`] starts the next one.
    fn send_next_playlist_edit(&mut self) {
        if self.playlist_edits.sending.is_some() {
            return;
        }
        let Some((playlist_id, edit)) = self.playlist_edits.queue.pop_front() else {
            return;
        };
        self.playlist_edits.sending = Some(playlist_id.clone());
        let client = self.session.client.clone();
        let sender = self.playlist_edits.done_from.clone();
        crate::runtime::detach(async move {
            let result = match edit {
                PlaylistEdit::Move { entry_id, to } => client.move_playlist_entry(&playlist_id, &entry_id, to).await,
                PlaylistEdit::Remove { entry_id } => client.remove_from_playlist(&playlist_id, &[entry_id]).await,
            };
            let _ = sender.send((playlist_id, result));
        });
    }

    /// A finished edit: the next queued one goes, and a playlist whose edits ran dry loads
    /// again, for its new runtime and to match what the server ended up with. A refusal drops
    /// the rest of that playlist's edits, says why, and loads it too.
    pub(crate) fn tick_playlist_edits(&mut self) {
        let done: Vec<EditDone> = self.playlist_edits.done.drain().collect();
        for (playlist_id, result) in done {
            self.playlist_edits.sending = None;
            if let Err(err) = result {
                self.playlist_edits.clear(&playlist_id);
                let message = match err {
                    jellyfin::Error::Forbidden => {
                        "This playlist belongs to someone else; it can't be changed.".to_owned()
                    }
                    err => self.handle_error(&err),
                };
                self.toast(Toast::error(message));
            }
            self.send_next_playlist_edit();
            // The page waits while more of its edits are pending, so the bump reaches it once
            // its queue is dry.
            if !self.playlist_edits.pending(&playlist_id) {
                self.user_data += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::{PlaylistEdit, PlaylistEdits, first_unwatched};

    fn episode(id: &str, season: i32, played: bool) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": id,
            "Type": "Episode",
            "ParentIndexNumber": season,
            "UserData": { "Played": played, "PlaybackPositionTicks": 0, "IsFavorite": false },
        }))
        .expect("episode")
    }

    fn pick(episodes: &[BaseItemDto]) -> Option<&str> {
        first_unwatched(episodes).map(|e| e.id.as_str())
    }

    #[test]
    fn plays_the_first_unwatched_episode() {
        let episodes = [episode("a", 1, true), episode("b", 1, false), episode("c", 2, false)];
        assert_eq!(pick(&episodes), Some("b"));
    }

    #[test]
    fn starts_over_when_everything_is_watched() {
        let episodes = [episode("a", 1, true), episode("b", 1, true)];
        assert_eq!(pick(&episodes), Some("a"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn skips_specials_unless_there_is_nothing_else() {
        let episodes = [
            episode("special", 0, false),
            episode("a", 1, true),
            episode("b", 1, false),
        ];
        assert_eq!(pick(&episodes), Some("b"));
        let watched = [episode("special", 0, false), episode("a", 1, true)];
        assert_eq!(pick(&watched), Some("a"));
        assert_eq!(pick(&[episode("special", 0, false)]), Some("special"));
    }

    fn move_edit(entry_id: &str, to: usize) -> PlaylistEdit {
        PlaylistEdit::Move {
            entry_id: entry_id.into(),
            to,
        }
    }

    fn remove(entry_id: &str) -> PlaylistEdit {
        PlaylistEdit::Remove {
            entry_id: entry_id.into(),
        }
    }

    #[test]
    fn queued_edits_keep_the_order_they_were_made_in() {
        let mut edits = PlaylistEdits::new(&egui::Context::default());
        edits.push("one".into(), move_edit("a", 1));
        edits.push("two".into(), remove("b"));
        edits.push("one".into(), move_edit("c", 0));
        assert!(edits.pending("one"));
        assert!(edits.pending("two"));
        assert!(!edits.pending("three"));

        // Oldest first, whichever playlist it's for.
        let queued: Vec<(String, PlaylistEdit)> = edits.queue.drain(..).collect();
        assert_eq!(
            queued,
            vec![
                ("one".into(), move_edit("a", 1)),
                ("two".into(), remove("b")),
                ("one".into(), move_edit("c", 0)),
            ]
        );
    }

    #[test]
    fn a_refusal_drops_only_that_playlists_edits() {
        let mut edits = PlaylistEdits::new(&egui::Context::default());
        edits.push("one".into(), move_edit("a", 1));
        edits.push("two".into(), remove("b"));
        edits.push("one".into(), move_edit("c", 0));
        edits.sending = Some("two".into());

        // The refusal came for "one"; its queued edits go, the others stay, and an edit already
        // out keeps counting as pending until its result is in.
        edits.clear("one");
        assert!(!edits.pending("one"));
        assert!(edits.pending("two"));
        let queued: Vec<(String, PlaylistEdit)> = edits.queue.drain(..).collect();
        assert_eq!(queued, vec![("two".into(), remove("b"))]);

        edits.sending = Some("one".into());
        edits.clear("one");
        assert!(edits.pending("one"));
    }
}
