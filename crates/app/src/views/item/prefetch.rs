//! Starts loading an item's page while the pointer rests on its card, so a click lands on data
//! that's there, or at least on its way.
//!
//! A page reports the card under the pointer each frame ([`Prefetch::hover`]). Once the pointer
//! has stayed for [`DWELL`], the same load the page would run starts here, in the services, since
//! the page doesn't exist yet. Nothing polls that [`Task`]: its result waits inside it until the
//! page takes the task over ([`Prefetch::take`]), so a click in the middle of the request reuses
//! it instead of starting again, and the page draws at once if it's finished.
//!
//! A load costs the server up to half a dozen requests, so a pointer crossing a shelf starts
//! none, and at most [`KEPT`] are held, the oldest cancelled for the next. What's fetched is only
//! good for the `user_data` it started at: anything that changes watch data discards it, and
//! [`FRESH_FOR`] bounds the rest (a title edited on the server, say).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use jellyfin::Client;

use super::{ItemData, load};
use crate::nav::Route;
use crate::task::{Deadline, Task};

/// How long the pointer has to stay on a card before its page starts loading: long enough to
/// skip the cards it crosses, short enough to run ahead of most clicks.
const DWELL: Duration = Duration::from_millis(150);
/// How many loads are held at a time.
const KEPT: usize = 4;
/// How long a page may open on a load started earlier.
const FRESH_FOR: Duration = Duration::from_secs(60);

/// A page's load, started ahead of the page.
struct Fetch {
    item_id: String,
    /// The `services.user_data` it started at.
    user_data: u64,
    started: Instant,
    task: Task<jellyfin::Result<ItemData>>,
}

impl Fetch {
    fn fresh(&self, user_data: u64) -> bool {
        self.user_data == user_data && self.started.elapsed() < FRESH_FOR
    }
}

#[derive(Default)]
pub struct Prefetch {
    /// The item whose card the pointer is on.
    hovered: Option<String>,
    /// A page reported [`Prefetch::hovered`] in the frame since the last [`Prefetch::tick`];
    /// when none did, the pointer has left.
    reported: bool,
    dwell: Deadline,
    /// Oldest first.
    fetches: VecDeque<Fetch>,
}

impl Prefetch {
    /// The pointer is on a card leading to `route`, this frame. Only item pages are loaded ahead.
    pub fn hover(&mut self, route: Route) {
        let Route::Item { id, .. } = route else {
            return;
        };
        self.reported = true;
        if self.hovered.as_ref() != Some(&id) {
            self.hovered = Some(id);
            self.dwell.set(DWELL);
        }
    }

    /// Forgets a card the last frame didn't report, and starts the one the pointer stayed on.
    pub fn tick(&mut self, ctx: &egui::Context, client: &Client, user_data: u64) {
        if !std::mem::take(&mut self.reported) {
            self.hovered = None;
            self.dwell.cancel();
            return;
        }
        if self.dwell.fire(ctx)
            && let Some(item_id) = self.hovered.clone()
        {
            self.start(item_id, ctx, client, user_data);
        }
    }

    /// Starts `item_id`'s load, unless a good one is already held.
    fn start(&mut self, item_id: String, ctx: &egui::Context, client: &Client, user_data: u64) {
        if self
            .fetches
            .iter()
            .any(|fetch| fetch.item_id == item_id && fetch.fresh(user_data))
        {
            return;
        }
        self.fetches
            .retain(|fetch| fetch.item_id != item_id && fetch.fresh(user_data));
        if self.fetches.len() == KEPT {
            self.fetches.pop_front();
        }
        let task = Task::spawn(ctx, load(client.clone(), item_id.clone()));
        self.fetches.push_back(Fetch {
            item_id,
            user_data,
            started: Instant::now(),
            task,
        });
    }

    /// `item_id`'s load, for its page to take over, if a good one is held.
    pub(super) fn take(&mut self, item_id: &str, user_data: u64) -> Option<Task<jellyfin::Result<ItemData>>> {
        let index = self.fetches.iter().position(|fetch| fetch.item_id == item_id)?;
        let fetch = self.fetches.remove(index)?;
        fetch.fresh(user_data).then_some(fetch.task)
    }
}

#[cfg(test)]
impl Prefetch {
    /// Starts `item_id`'s load straight away, for a test that's not about the dwell.
    pub(crate) fn start_now(&mut self, item_id: &str, ctx: &egui::Context, client: &Client, user_data: u64) {
        self.start(item_id.to_owned(), ctx, client, user_data);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jellyfin::Client;

    use super::{FRESH_FOR, KEPT, Prefetch};
    use crate::session;

    /// A client whose requests are never answered: these tests are about what's held.
    fn client() -> Client {
        Client::new("http://127.0.0.1:9", session::client_info("prefetch-tests")).expect("the URL")
    }

    fn held(prefetch: &Prefetch) -> Vec<&str> {
        prefetch.fetches.iter().map(|fetch| fetch.item_id.as_str()).collect()
    }

    #[test]
    fn a_load_is_handed_over_once() {
        let ctx = egui::Context::default();
        let mut prefetch = Prefetch::default();
        prefetch.start_now("a", &ctx, &client(), 0);
        assert!(prefetch.take("b", 0).is_none());
        assert!(prefetch.take("a", 0).is_some());
        assert!(prefetch.take("a", 0).is_none());
    }

    #[test]
    fn changed_watch_data_discards_a_load() {
        let ctx = egui::Context::default();
        let mut prefetch = Prefetch::default();
        prefetch.start_now("a", &ctx, &client(), 0);
        assert!(prefetch.take("a", 1).is_none());
        assert!(held(&prefetch).is_empty());
    }

    #[test]
    fn an_old_load_is_not_used() {
        let ctx = egui::Context::default();
        let mut prefetch = Prefetch::default();
        prefetch.start_now("a", &ctx, &client(), 0);
        prefetch.fetches[0].started -= FRESH_FOR + Duration::from_secs(1);
        assert!(prefetch.take("a", 0).is_none());
    }

    #[test]
    fn the_same_item_loads_once_and_the_oldest_makes_room() {
        let ctx = egui::Context::default();
        let mut prefetch = Prefetch::default();
        prefetch.start_now("first", &ctx, &client(), 0);
        prefetch.start_now("first", &ctx, &client(), 0);
        assert_eq!(held(&prefetch), ["first"]);
        for id in ["b", "c", "d", "e"] {
            prefetch.start_now(id, &ctx, &client(), 0);
        }
        assert_eq!(prefetch.fetches.len(), KEPT);
        assert_eq!(held(&prefetch), ["b", "c", "d", "e"]);
    }
}
