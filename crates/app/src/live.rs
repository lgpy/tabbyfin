//! The session websocket while signed in. P2 brought the part SyncPlay needs: the connection,
//! and the group updates and commands handed to SyncPlay. P3 fills in the rest of `main`'s
//! `live.rs`: the capabilities reported on each connect, watch-data refreshes and remote
//! control.

use jellyfin::{Client, ServerMessage, SocketEvent};

use crate::services::Services;
use crate::task::{self, Task};

/// The open session websocket. Dropping it (with the services, on sign-out) disconnects.
pub struct Live {
    events: task::Receiver<SocketEvent>,
    _socket: Task<()>,
}

impl Live {
    /// Connects, and keeps reconnecting with backoff (`Client::socket`). What the server sends
    /// waits in a channel that wakes the UI, for [`Services::tick_live`].
    pub fn connect(client: Client, ctx: &egui::Context) -> Self {
        let (sender, events) = task::channel(ctx);
        let socket = Task::spawn(ctx, async move {
            let mut socket = client.socket();
            while let Some(event) = socket.recv().await {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        Self {
            events,
            _socket: socket,
        }
    }
}

impl Services {
    /// Hands what the server pushed since the last frame to whoever handles it. From
    /// [`Services::tick`], so it goes on while the window is hidden.
    pub(crate) fn tick_live(&mut self) {
        let events: Vec<SocketEvent> = self.live.events.drain().collect();
        for event in events {
            match event {
                // P3: report the capabilities here, on every connect.
                SocketEvent::Connected => {
                    #[cfg(debug_assertions)]
                    self.syncplay.debug_join();
                }
                SocketEvent::Message(ServerMessage::SyncPlayGroupUpdate(update)) => self.on_syncplay_update(update),
                SocketEvent::Message(ServerMessage::SyncPlayCommand(command)) => self.on_syncplay_command(command),
                // P3: remote control (`Play`, `Playstate`, `GeneralCommand`) and
                // `UserDataChanged`.
                SocketEvent::Disconnected | SocketEvent::Message(_) => {}
            }
        }
    }
}
