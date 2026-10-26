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
use crate::session::{Session, describe_error};
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
}

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
        Self {
            images: ImageCache::new(session.client.clone(), ctx),
            session,
            user_data: 0,
            events: Vec::new(),
        }
    }

    /// Work that has to go on whatever is on screen: draining the background channels, firing
    /// deadlines, uploading the images that finished loading. Runs from `App::logic` before each
    /// frame, and also while the window is hidden, so playback keeps reporting when it's
    /// minimised.
    pub fn tick(&mut self, _ctx: &egui::Context) {
        self.images.tick();
    }

    /// The one hook for "this session is ending", called before another account takes over or
    /// the screen showing it is dropped: P1 stops playback here and P2 leaves a SyncPlay group.
    /// Dropping the services afterwards cancels whatever they still have running.
    pub fn end(&mut self) {}

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
