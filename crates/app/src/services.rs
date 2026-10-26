//! Everything that lives as long as a signed-in session, in one place the app owns.
//!
//! On `main` these were GPUI globals and entities. Here the app owns one `Services` while
//! signed in and lends it to the shell and its pages each frame (`PageCx::services`), so
//! nothing is shared through globals or `Rc<RefCell<…>>`. Code that needs two parts at once
//! (playback asking SyncPlay, remote control driving playback) is a method here, which can
//! reach every part; each part's module adds its own `impl Services` block (playback's
//! controls are in `playback.rs`).
//!
//! Still to come: `images` (I1), `syncplay` (P2), `remote` and `live` (P3).

use crate::playback::PlaybackManager;
use crate::session::{Session, describe_error};
use crate::toasts::Toast;

pub struct Services {
    pub session: Session,
    /// Bumped when the user's watch data changed elsewhere (played state, resume points,
    /// favorites). Pages remember the value they loaded at and load again when it differs.
    pub user_data: u64,
    /// What happened since the app last looked, for the app and the shell to act on. Drained
    /// every frame.
    pub events: Vec<AppEvent>,
    /// Playback sessions, the play queue and what carries over between items. Playing and the
    /// player's controls are methods on `Services` (`playback.rs`), since SyncPlay may take
    /// them over. Dropping it (signing out) stops playback.
    pub playback: PlaybackManager,
}

/// Something signed-in code can't handle itself, for the app or the shell.
#[derive(Debug)]
pub enum AppEvent {
    Toast(Toast),
    /// The server rejected the session's token: back to sign-in.
    SessionExpired,
    /// A playback session started: show the player (`services.playback.now_playing()`). A new
    /// session replaces the player that's showing.
    PlaybackStarted,
    /// Playback ended for good (stopped, failed, or finished with nothing to play next): close
    /// the player. Not sent between an episode and the next one it autoplays, nor while waiting
    /// to reconnect.
    PlaybackEnded,
}

impl Services {
    pub fn new(ctx: &egui::Context, session: Session) -> Self {
        let playback = PlaybackManager::new(ctx, session.client.clone(), session.account_key());
        Self {
            session,
            user_data: 0,
            events: Vec::new(),
            playback,
        }
    }

    /// Work that has to go on whatever is on screen: draining the background channels, firing
    /// deadlines. Runs from `App::logic` before each frame, and also while the window is
    /// hidden, so playback keeps reporting when it's minimised.
    pub fn tick(&mut self, _ctx: &egui::Context) {
        self.tick_playback();
    }

    /// The app is quitting: stops what the session is doing on the server, waiting a little for
    /// it to be reported, since the process ends right after.
    pub fn shut_down(&mut self) {
        self.playback.shut_down();
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
