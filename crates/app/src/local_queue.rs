//! The local play queue: what plays after the item playing now, what already played from it,
//! and the repeat and shuffle modes that shape it. It's the solo player's; a SyncPlay group or a
//! cast keeps its queue on the server or the TV (`syncplay.rs`, `cast.rs`).
//!
//! It holds items and nothing else, so its rules are tested without a session. `playback.rs`
//! decides when to call it.

use std::collections::VecDeque;

use jellyfin::models::BaseItemDto;
use jellyfin::{PlaybackOrder, RepeatMode};

use crate::views::item::shuffle;

#[derive(Default)]
pub struct LocalQueue {
    /// What plays after the current item, in order.
    upcoming: VecDeque<BaseItemDto>,
    /// What played from this queue before the current item, oldest first: where repeating the
    /// whole queue starts over from.
    played: Vec<BaseItemDto>,
    pub repeat: RepeatMode,
    shuffled: bool,
    /// The ids of `upcoming` in the order they had before shuffling, to put them back in it.
    unshuffled: Vec<String>,
}

impl LocalQueue {
    /// What plays after the current item, in order.
    pub fn upcoming(&self) -> impl ExactSizeIterator<Item = &BaseItemDto> {
        self.upcoming.iter()
    }

    pub fn front(&self) -> Option<&BaseItemDto> {
        self.upcoming.front()
    }

    pub fn is_empty(&self) -> bool {
        self.upcoming.is_empty()
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffled
    }

    /// The modes as a progress report names them.
    pub fn modes(&self) -> (RepeatMode, PlaybackOrder) {
        let order = if self.shuffled {
            PlaybackOrder::Shuffle
        } else {
            PlaybackOrder::Default
        };
        (self.repeat, order)
    }

    /// Forgets the items. The modes stay: they're the user's setting for the player, not for one
    /// queue.
    pub fn clear(&mut self) {
        self.upcoming.clear();
        self.played.clear();
        self.unshuffled.clear();
    }

    /// Starts a new queue of `items` (Play all). Shuffled if shuffle is on.
    pub fn start(&mut self, items: VecDeque<BaseItemDto>) {
        self.clear();
        self.upcoming = items;
        if self.shuffled {
            self.shuffle_upcoming();
        }
    }

    /// Puts `items` first, in their order: they play right after the current item.
    pub fn push_next(&mut self, items: Vec<BaseItemDto>) {
        if self.shuffled {
            self.unshuffled.splice(0..0, items.iter().map(|item| item.id.clone()));
        }
        for item in items.into_iter().rev() {
            self.upcoming.push_front(item);
        }
    }

    /// Puts `items` last.
    pub fn push_last(&mut self, items: Vec<BaseItemDto>) {
        if self.shuffled {
            self.unshuffled.extend(items.iter().map(|item| item.id.clone()));
        }
        self.upcoming.extend(items);
    }

    /// `started` is playing now, after `previous`. When it's the front of the queue, the queue
    /// moves on and `previous` joins what played; returns whether it was.
    pub fn advance(&mut self, started: &BaseItemDto, previous: Option<&BaseItemDto>) -> bool {
        if !self.upcoming.front().is_some_and(|next| next.id == started.id) {
            return false;
        }
        self.upcoming.pop_front();
        self.played.extend(previous.cloned());
        true
    }

    /// Takes the entry at `index` to play now: `previous`, the item playing, and the entries
    /// before it join what played. `None` past the end.
    pub fn jump(&mut self, index: usize, previous: Option<&BaseItemDto>) -> Option<BaseItemDto> {
        if index >= self.upcoming.len() {
            return None;
        }
        self.played.extend(previous.cloned());
        self.played.extend(self.upcoming.drain(..index));
        self.upcoming.pop_front()
    }

    /// Takes the entry at `index` out.
    pub fn remove(&mut self, index: usize) -> Option<BaseItemDto> {
        let item = self.upcoming.remove(index)?;
        if let Some(at) = self.unshuffled.iter().position(|id| *id == item.id) {
            self.unshuffled.remove(at);
        }
        Some(item)
    }

    /// Moves the entry at `index` up (`-1`) or down (`1`). Returns where it went, `None` at an
    /// end.
    pub fn move_entry(&mut self, index: usize, by: isize) -> Option<usize> {
        let to = index.checked_add_signed(by).filter(|to| *to < self.upcoming.len())?;
        let item = self.upcoming.remove(index)?;
        self.upcoming.insert(to, item);
        Some(to)
    }

    /// Turns shuffle on or off. Shuffling mixes what's coming, never the playing item (it isn't
    /// in the queue); turning it off puts the entries back in the order they had.
    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffled {
            return;
        }
        self.shuffled = on;
        if on {
            self.shuffle_upcoming();
        } else {
            self.unshuffle();
        }
    }

    fn shuffle_upcoming(&mut self) {
        self.unshuffled = self.upcoming.iter().map(|item| item.id.clone()).collect();
        shuffle(self.upcoming.make_contiguous());
    }

    fn unshuffle(&mut self) {
        let order = std::mem::take(&mut self.unshuffled);
        let mut used = vec![false; order.len()];
        let mut entries: Vec<(usize, BaseItemDto)> = self
            .upcoming
            .drain(..)
            .map(|item| {
                let place = order
                    .iter()
                    .enumerate()
                    .position(|(at, id)| !used[at] && *id == item.id);
                if let Some(place) = place {
                    used[place] = true;
                }
                (place.unwrap_or(usize::MAX), item)
            })
            .collect();
        // Stable, so what the order doesn't know keeps its place at the end.
        entries.sort_by_key(|(place, _)| *place);
        self.upcoming = entries.into_iter().map(|(_, item)| item).collect();
    }

    /// Repeating the whole queue, at its end: everything that played plus `current` again, the
    /// first of them to play now and the rest queued. `None` while entries are still waiting.
    pub fn restart(&mut self, current: &BaseItemDto) -> Option<BaseItemDto> {
        if !self.upcoming.is_empty() {
            return None;
        }
        let mut all = std::mem::take(&mut self.played);
        all.push(current.clone());
        let first = all.remove(0);
        self.upcoming = all.into();
        if self.shuffled {
            self.shuffle_upcoming();
        }
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;
    use jellyfin::{PlaybackOrder, RepeatMode};

    use super::LocalQueue;

    fn item(id: &str) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({ "Id": id, "Name": id })).expect("item")
    }

    fn items(ids: &[&str]) -> Vec<BaseItemDto> {
        ids.iter().map(|id| item(id)).collect()
    }

    fn ids(queue: &LocalQueue) -> Vec<&str> {
        queue.upcoming().map(|item| item.id.as_str()).collect()
    }

    fn queue(upcoming: &[&str]) -> LocalQueue {
        let mut queue = LocalQueue::default();
        queue.start(items(upcoming).into());
        queue
    }

    #[test]
    fn play_next_goes_before_what_is_queued_and_add_goes_after() {
        let mut queue = queue(&["b", "c"]);
        queue.push_last(items(&["d"]));
        queue.push_next(items(&["x", "y"]));
        assert_eq!(ids(&queue), ["x", "y", "b", "c", "d"]);
    }

    #[test]
    fn the_queue_moves_on_when_its_front_starts() {
        let mut queue = queue(&["b", "c"]);
        assert!(queue.advance(&item("b"), Some(&item("a"))));
        assert_eq!(ids(&queue), ["c"]);
        // Something else playing doesn't take from it.
        assert!(!queue.advance(&item("z"), Some(&item("b"))));
        assert_eq!(ids(&queue), ["c"]);
    }

    #[test]
    fn entries_are_removed_and_moved() {
        let mut queue = queue(&["a", "b", "c", "d"]);
        assert_eq!(queue.remove(1).map(|item| item.id), Some("b".to_owned()));
        assert_eq!(ids(&queue), ["a", "c", "d"]);
        assert!(queue.remove(7).is_none());

        assert_eq!(queue.move_entry(2, -1), Some(1));
        assert_eq!(ids(&queue), ["a", "d", "c"]);
        assert_eq!(queue.move_entry(1, 1), Some(2));
        assert_eq!(ids(&queue), ["a", "c", "d"]);
        // No moving past either end.
        assert_eq!(queue.move_entry(0, -1), None);
        assert_eq!(queue.move_entry(2, 1), None);
        assert_eq!(ids(&queue), ["a", "c", "d"]);
    }

    #[test]
    fn jumping_plays_an_entry_and_keeps_what_was_skipped_for_repeat() {
        let mut queue = queue(&["b", "c", "d"]);
        let jumped = queue.jump(1, Some(&item("a"))).expect("an entry");
        assert_eq!(jumped.id, "c");
        assert_eq!(ids(&queue), ["d"]);
        assert!(queue.jump(5, Some(&item("c"))).is_none());

        // Repeat all, at the end of the queue: everything again, in its order.
        queue.advance(&item("d"), Some(&item("c")));
        let first = queue.restart(&item("d")).expect("a restart");
        assert_eq!(first.id, "a");
        assert_eq!(ids(&queue), ["b", "c", "d"]);
    }

    #[test]
    fn repeating_the_queue_waits_for_its_end() {
        let mut queue = queue(&["b"]);
        queue.advance(&item("b"), Some(&item("a")));
        assert_eq!(queue.restart(&item("b")).map(|item| item.id), Some("a".to_owned()));
        assert_eq!(ids(&queue), ["b"]);
        // Entries waiting: not the end yet.
        assert!(queue.restart(&item("a")).is_none());
    }

    #[test]
    fn a_lone_item_repeats_itself() {
        let mut queue = LocalQueue::default();
        assert_eq!(queue.restart(&item("a")).map(|item| item.id), Some("a".to_owned()));
        assert!(queue.is_empty());
    }

    #[test]
    fn shuffling_keeps_every_entry_and_unshuffling_puts_them_back() {
        let ids_in = ["b", "c", "d", "e", "f", "g", "h", "i"];
        let mut queue = queue(&ids_in);
        queue.set_shuffle(true);
        assert!(queue.is_shuffled());
        let mut shuffled: Vec<&str> = ids(&queue);
        shuffled.sort_unstable();
        assert_eq!(shuffled, ids_in);

        // Added while shuffled: last, and last again once unshuffled.
        queue.push_last(items(&["z"]));
        queue.set_shuffle(false);
        assert!(!queue.is_shuffled());
        assert_eq!(ids(&queue), ["b", "c", "d", "e", "f", "g", "h", "i", "z"]);
    }

    #[test]
    fn a_shuffled_queue_keeps_the_entries_it_was_given_later() {
        let mut queue = queue(&["b", "c", "d"]);
        queue.set_shuffle(true);
        queue.push_next(items(&["x"]));
        assert_eq!(queue.front().map(|item| item.id.as_str()), Some("x"));
        queue.remove(0);
        queue.set_shuffle(false);
        assert_eq!(ids(&queue), ["b", "c", "d"]);
    }

    #[test]
    fn a_new_queue_follows_the_shuffle_setting_and_keeps_the_modes() {
        let mut queue = queue(&["b"]);
        queue.set_shuffle(true);
        queue.repeat = RepeatMode::RepeatAll;
        queue.clear();
        assert!(queue.is_empty());
        assert_eq!(queue.modes(), (RepeatMode::RepeatAll, PlaybackOrder::Shuffle));
        queue.start(items(&["a", "b", "c"]).into());
        assert_eq!(queue.upcoming().len(), 3);
    }
}
