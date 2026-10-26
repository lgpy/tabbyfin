//! Everything that lives as long as a signed-in session, in one place the app owns.
//!
//! On `main` these were GPUI globals and entities. Here the app owns one `Services` while
//! signed in and lends it to the shell and its pages each frame (`PageCx::services`), so
//! nothing is shared through globals or `Rc<RefCell<…>>`. Code that needs two parts at once
//! (playback asking SyncPlay, remote control driving playback) is a method here, which can
//! reach every part.
//!
//! Still to come: `playback` (P1), `syncplay` (P2), `remote` and `live` (P3).

use crate::config::SavedServer;
use crate::images::ImageCache;
use crate::item_menu::Mark;
use crate::session::{Session, describe_error};
use crate::task;
use crate::toasts::Toast;

pub struct Services {
    pub session: Session,
    /// Server images as egui textures (`images.rs`), with their queue and memory budget.
    pub images: ImageCache,
    /// Bumped when the user's watch data changed elsewhere (played state, resume points,
    /// favorites). Pages remember the value they loaded at and load again when it differs.
    pub user_data: u64,
    /// What happened since the app last looked, for the app and the shell to act on. Drained
    /// every frame.
    pub events: Vec<AppEvent>,
    /// Watched and favorite changes that finished on the server, for [`Services::tick`] to apply.
    marked: task::Receiver<Marked>,
    marking: task::Sender<Marked>,
}

/// A watched or favorite change, as its request finishes: what was asked for, on which item,
/// and how it went.
type Marked = (Mark, String, jellyfin::Result<()>);

/// Something signed-in code can't handle itself, for the app or the shell.
#[derive(Debug)]
pub enum AppEvent {
    Toast(Toast),
    /// The server rejected the session's token: back to sign-in.
    SessionExpired,
    /// The user signed out: revoke the token and go back to sign-in.
    SignOut,
    /// Switch to another saved account, ending this session.
    SwitchAccount(SavedServer),
    /// Sign in to another server or user, keeping this session to come back to.
    AddAccount,
}

impl Services {
    pub fn new(session: Session, ctx: &egui::Context) -> Self {
        let (marking, marked) = task::channel(ctx);
        Self {
            images: ImageCache::new(session.client.clone(), ctx),
            session,
            user_data: 0,
            events: Vec::new(),
            marked,
            marking,
        }
    }

    /// Work that has to go on whatever is on screen: draining the background channels, firing
    /// deadlines, uploading the images that finished loading. Runs from `App::logic` before each
    /// frame, and also while the window is hidden, so playback keeps reporting when it's
    /// minimised.
    pub fn tick(&mut self, _ctx: &egui::Context) {
        self.images.tick();
        self.apply_marks();
    }

    /// The one hook for "this session is ending", called before another account takes over or
    /// the screen showing it is dropped: P1 stops playback here and P2 leaves a SyncPlay group.
    /// Dropping the services afterwards cancels whatever they still have running.
    pub fn end(&mut self) {}

    /// Sets watched or favorite state on the server, from a card's menu. The pages that show it
    /// reload when it worked (`user_data`); a failure is a toast. The request runs on tokio and
    /// reports back in [`Services::tick`], so the menu doesn't have to own a task.
    pub fn mark(&mut self, item_id: String, mark: Mark) {
        let client = self.session.client.clone();
        let sender = self.marking.clone();
        crate::runtime::detach(async move {
            let result = match mark {
                Mark::Played(true) => client.mark_played(&item_id).await,
                Mark::Played(false) => client.mark_unplayed(&item_id).await,
                Mark::Favorite(true) => client.mark_favorite(&item_id).await,
                Mark::Favorite(false) => client.unmark_favorite(&item_id).await,
            };
            let _ = sender.send((mark, item_id, result));
        });
    }

    fn apply_marks(&mut self) {
        let marked: Vec<Marked> = self.marked.drain().collect();
        for (mark, item_id, result) in marked {
            match result {
                // What's shown of it (a played item's resume point, a season's unwatched count)
                // changed too: pages load again.
                Ok(()) => {
                    tracing::debug!(item = %item_id, ?mark, "marked");
                    self.user_data += 1;
                }
                Err(err) => {
                    let message = self.handle_error(&err);
                    self.toast(Toast::error(message));
                }
            }
        }
    }

    pub fn toast(&mut self, toast: Toast) {
        self.events.push(AppEvent::Toast(toast));
    }

    /// User-facing message for a failed request. A rejected token also ends the session, back
    /// to the sign-in screen.
    pub fn handle_error(&mut self, err: &jellyfin::Error) -> String {
        if matches!(err, jellyfin::Error::Unauthorized) {
            self.events.push(AppEvent::SessionExpired);
        }
        describe_error(err)
    }
}
