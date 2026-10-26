//! Everything that lives as long as a signed-in session, in one place the app owns.
//!
//! On `main` these were GPUI globals and entities. Here the app owns one `Services` while
//! signed in and lends it to the shell and its pages each frame (`PageCx::services`), so
//! nothing is shared through globals or `Rc<RefCell<…>>`. Code that needs two parts at once
//! (playback asking SyncPlay, remote control driving playback) is a method here, which can
//! reach every part; each part's module adds its own `impl Services` block (playback's
//! controls are in `playback.rs`, SyncPlay's in `syncplay.rs`).
//!
//! Still to come: `remote` (P3), and the rest of `live` (P3).

use crate::config::SavedServer;
use crate::images::ImageCache;
use crate::item_menu::Mark;
use crate::live::Live;
use crate::playback::{self, PlaybackManager};
use crate::session::{Session, describe_error};
use crate::syncplay::SyncPlay;
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
    /// Watched and favorite changes that finished on the server, for [`Services::tick`] to apply
    /// (`widgets/menu.rs` starts them).
    pub(crate) marked: task::Receiver<Marked>,
    pub(crate) marking: task::Sender<Marked>,
    /// A series' or season's next episode, fetched for the item menu's *Play*: `widgets/menu.rs`
    /// plays it (or says there's nothing to play) when it arrives.
    pub(crate) up_next: task::Receiver<UpNext>,
    pub(crate) up_next_from: task::Sender<UpNext>,
    /// Playback sessions, the play queue and what carries over between items. Playing and the
    /// player's controls are methods on `Services` (`playback.rs`), since SyncPlay may take
    /// them over.
    pub playback: PlaybackManager,
    /// The SyncPlay group this session is in, its queue, and the panel's requests. What
    /// involves playback is on `Services` (`syncplay.rs`).
    pub syncplay: SyncPlay,
    /// The session websocket, which feeds SyncPlay (and, with P3, remote control and
    /// watch-data refreshes).
    pub live: Live,
}

/// A watched or favorite change, as its request finishes: what was asked for, on which item,
/// and how it went.
pub(crate) type Marked = (Mark, String, jellyfin::Result<()>);

/// The episode a series or season plays next, once it's fetched.
pub(crate) type UpNext = jellyfin::Result<Option<jellyfin::models::BaseItemDto>>;

/// Something signed-in code can't handle itself, for the app or the shell.
#[derive(Debug)]
pub enum AppEvent {
    Toast(Toast),
    /// An appearance setting was saved: the app applies it, and starts or stops following the
    /// desktop's colour scheme with it. Settings raises this rather than calling
    /// `theme::apply` itself, since the app owns that watch.
    AppearanceChanged,
    /// The server rejected the session's token: back to sign-in.
    SessionExpired,
    /// The user signed out: revoke the token and go back to sign-in.
    SignOut,
    /// Switch to another saved account, ending this session.
    SwitchAccount(SavedServer),
    /// Sign in to another server or user, keeping this session to come back to.
    AddAccount,
    /// A playback session started: show the player (`services.playback.now_playing()`). A new
    /// session replaces the player that's showing.
    PlaybackStarted,
    /// Playback ended for good (stopped, failed, or finished with nothing to play next): close
    /// the player. Not sent between an episode and the next one it autoplays, nor while waiting
    /// to reconnect.
    PlaybackEnded,
}

impl Services {
    pub fn new(session: Session, ctx: &egui::Context) -> Self {
        let (marking, marked) = task::channel(ctx);
        let (up_next_from, up_next) = task::channel(ctx);
        Self {
            images: ImageCache::new(session.client.clone(), ctx),
            playback: PlaybackManager::new(ctx, session.client.clone(), session.account_key()),
            syncplay: SyncPlay::new(session.client.clone(), ctx),
            live: Live::connect(session.client.clone(), ctx),
            session,
            user_data: 0,
            events: Vec::new(),
            marked,
            marking,
            up_next,
            up_next_from,
        }
    }

    /// Work that has to go on whatever is on screen: draining the background channels, firing
    /// deadlines, uploading the images that finished loading. Runs from `App::logic` before each
    /// frame, and also while the window is hidden, so playback keeps reporting when it's
    /// minimised.
    pub fn tick(&mut self, _ctx: &egui::Context) {
        self.images.tick();
        self.apply_menu_requests();
        self.tick_playback();
        self.tick_live();
        self.tick_syncplay();
    }

    /// The one hook for "this session is ending", called before another account takes over or
    /// the screen showing it is dropped: stops playback and leaves a SyncPlay group. Dropping
    /// the services afterwards cancels whatever they still have running; a stopped playback
    /// session finishes in the background, reporting its stop once, and the request leaving the
    /// group still goes out.
    pub fn end(&mut self) {
        self.stop_playback();
        if self.in_group() {
            self.syncplay.leave();
        }
    }

    /// The app is quitting: stops what these sessions (the signed-in one, and one parked behind
    /// *Add account*) are doing on the server, playback and a SyncPlay group, and waits a
    /// little for the server to hear it, since the process ends right after.
    pub fn shut_down<'a>(all: impl IntoIterator<Item = &'a mut Services>) {
        let mut stopping = Vec::new();
        for services in all {
            stopping.extend(services.playback.stop_for_exit());
            stopping.extend(services.syncplay.leave_for_exit());
        }
        playback::wait_for_exit(stopping);
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
