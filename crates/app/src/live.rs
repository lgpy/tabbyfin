//! The session websocket while signed in: keeps this session's capabilities reported, refreshes
//! pages when watch data changes elsewhere (another device, the web dashboard), hands SyncPlay
//! its updates and commands, and remote control its commands.
//!
//! `main`'s `changes.rs` is gone: a watch-data refresh is a bump of [`Services::user_data`],
//! which pages watch.

use std::time::Duration;

use jellyfin::{Capabilities, Client, GeneralCommand, ServerMessage, SocketEvent};

use crate::services::Services;
use crate::task::{self, Deadline, Task};

/// Batches the bursts of `UserDataChanged` (one per item, e.g. marking a season watched).
const COALESCE: Duration = Duration::from_millis(500);

/// The burst rule for `UserDataChanged`: one refresh per burst, `COALESCE` after its first
/// message, and none while playback is active (a session changes the resume point itself; pages
/// refresh when it ends, which bumps `user_data` too).
#[derive(Debug)]
struct Refresh {
    /// How long after the first message of a burst the refresh is due.
    after: Duration,
    deadline: Deadline,
}

impl Default for Refresh {
    fn default() -> Self {
        Self::after(COALESCE)
    }
}

impl Refresh {
    fn after(after: Duration) -> Self {
        Self {
            after,
            deadline: Deadline::default(),
        }
    }

    /// A change from the server: the first of a burst starts the window, the others join it.
    fn note(&mut self) {
        if !self.deadline.is_set() {
            self.deadline.set(self.after);
        }
    }

    /// Whether a refresh is due now, which also ends the burst. One that comes due during
    /// playback is dropped rather than postponed.
    fn due(&mut self, ctx: &egui::Context, playing: bool) -> bool {
        self.deadline.fire(ctx) && !playing
    }
}

/// The open session websocket. Dropping it (with the services, on sign-out) disconnects.
pub struct Live {
    ctx: egui::Context,
    client: Client,
    events: task::Receiver<SocketEvent>,
    /// The last connect's capabilities report, while it runs: a reconnect replaces it, and
    /// signing out cancels it.
    capabilities: Option<Task<jellyfin::Result<()>>>,
    /// A watch-data refresh, once its burst is due.
    refresh: Refresh,
    _socket: Task<()>,
}

impl Live {
    /// Connects, and keeps reconnecting with backoff (`Client::socket`). What the server sends
    /// waits in a channel that wakes the UI, for [`Services::tick_live`].
    pub fn connect(client: Client, ctx: &egui::Context) -> Self {
        let (sender, events) = task::channel(ctx);
        let socket_client = client.clone();
        let socket = Task::spawn(ctx, async move {
            let mut socket = socket_client.socket();
            while let Some(event) = socket.recv().await {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        Self {
            ctx: ctx.clone(),
            client,
            events,
            capabilities: None,
            refresh: Refresh::default(),
            _socket: socket,
        }
    }

    /// Tells the server what this session can do, on every connect: it decides which clients
    /// get a remote control for this one.
    fn report_capabilities(&mut self) {
        let client = self.client.clone();
        self.capabilities = Some(Task::spawn(&self.ctx, async move {
            let capabilities = Capabilities {
                supports_media_control: true,
                supported_commands: GeneralCommand::SUPPORTED.to_vec(),
            };
            client.report_capabilities(&capabilities).await
        }));
    }

    /// What the last capabilities report answered, once it's done, so a failure can be logged
    /// and an expired token can end the session.
    fn take_capabilities(&mut self) -> Option<jellyfin::Result<()>> {
        let result = self.capabilities.as_mut().and_then(Task::poll);
        if result.is_some() {
            self.capabilities = None;
        }
        result
    }

    /// A `UserDataChanged` from the server: one refresh per burst, `COALESCE` after its first
    /// message.
    fn note_user_data(&mut self) {
        self.refresh.note();
    }

    /// Whether the burst's refresh is due, which also ends it.
    fn refresh_due(&mut self, playing: bool) -> bool {
        self.refresh.due(&self.ctx, playing)
    }
}

impl Services {
    /// Hands what the server pushed since the last frame to whoever handles it. From
    /// [`Services::tick`], so it goes on while the window is hidden.
    pub(crate) fn tick_live(&mut self) {
        let events: Vec<SocketEvent> = self.live.events.drain().collect();
        for event in events {
            match event {
                SocketEvent::Connected => {
                    self.live.report_capabilities();
                    #[cfg(debug_assertions)]
                    self.syncplay.debug_join();
                }
                SocketEvent::Message(ServerMessage::SyncPlayGroupUpdate(update)) => self.on_syncplay_update(update),
                SocketEvent::Message(ServerMessage::SyncPlayCommand(command)) => self.on_syncplay_command(command),
                SocketEvent::Message(ServerMessage::Play(request)) => self.on_remote_play(request),
                SocketEvent::Message(ServerMessage::Playstate(request)) => self.on_remote_playstate(request),
                SocketEvent::Message(ServerMessage::GeneralCommand(command)) => self.on_remote_command(command),
                // One refresh per burst. Other messages keep flowing meanwhile (SyncPlay
                // commands often arrive right after a playback report's UserDataChanged).
                SocketEvent::Message(ServerMessage::UserDataChanged { .. }) => self.live.note_user_data(),
                SocketEvent::Disconnected | SocketEvent::Message(_) => {}
            }
        }
        // Reporting is how other clients learn they can control this session; a failure is
        // only logged, but an expired token ends the session like any other call.
        if let Some(Err(err)) = self.live.take_capabilities() {
            let message = self.handle_error(&err);
            tracing::warn!("couldn't report session capabilities: {message}");
        }
        // Playback itself changes the resume point every few seconds; pages refresh when it
        // ends anyway (its end bumps `user_data` too).
        if self.live.refresh_due(self.playback.is_active()) {
            tracing::debug!("watch data changed elsewhere: pages refresh");
            self.user_data += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Refresh;

    /// One refresh per burst, `COALESCE` after its first message, and none while playback is
    /// active: the rule `tick_live` follows.
    #[test]
    fn a_burst_refreshes_once_and_not_during_playback() {
        let ctx = egui::Context::default();
        let mut refresh = Refresh::after(Duration::from_millis(10));
        assert!(!refresh.due(&ctx, false), "nothing to refresh before a change");
        refresh.note();
        assert!(refresh.deadline.is_set(), "the first message starts the burst");
        refresh.note();
        refresh.note();
        assert!(refresh.deadline.is_set(), "later messages join the pending refresh");
        std::thread::sleep(Duration::from_millis(20));
        assert!(refresh.due(&ctx, false), "the burst refreshes once it's due");
        assert!(!refresh.deadline.is_set());
        assert!(!refresh.due(&ctx, false), "and only once");
        refresh.note();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!refresh.due(&ctx, true), "playback changes the resume point itself");
        assert!(!refresh.deadline.is_set(), "a due refresh during playback is dropped");
    }
}
