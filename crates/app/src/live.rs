//! The session websocket while signed in: keeps this session's capabilities reported,
//! refreshes pages when watch data changes elsewhere (another device, the web dashboard), and
//! hands SyncPlay its updates and commands.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;
use jellyfin::{Client, ServerMessage, SocketEvent};

use crate::changes::Changes;
use crate::playback::PlaybackManager;
use crate::runtime;
use crate::syncplay::SyncPlay;

/// Batches the bursts of `UserDataChanged` (one per item, e.g. marking a season watched).
const COALESCE: Duration = Duration::from_millis(500);

/// Connects and handles server pushes until the task is dropped (on sign-out).
pub fn connect(client: Client, cx: &mut App) -> Task<()> {
    cx.spawn(async move |cx| {
        let mut events = runtime::spawn({
            let client = client.clone();
            async move { client.socket() }
        })
        .await;
        let report_capabilities = |client: &Client| {
            let client = client.clone();
            runtime::detach(async move {
                if let Err(err) = client.report_capabilities().await {
                    tracing::warn!("couldn't report session capabilities: {err}");
                }
            });
        };
        let refresh_pending = Rc::new(Cell::new(false));
        while let Some(event) = events.recv().await {
            match event {
                SocketEvent::Connected => report_capabilities(&client),
                SocketEvent::Message(ServerMessage::SyncPlayGroupUpdate(update)) => {
                    cx.update(|cx| SyncPlay::global(cx).update(cx, |s, cx| s.handle_update(update, cx)));
                }
                SocketEvent::Message(ServerMessage::SyncPlayCommand(command)) => {
                    cx.update(|cx| SyncPlay::global(cx).update(cx, |s, cx| s.handle_command(command, cx)));
                }
                // One refresh per burst. Other messages keep flowing meanwhile (SyncPlay
                // commands often arrive right after a playback report's UserDataChanged).
                SocketEvent::Message(ServerMessage::UserDataChanged { .. }) if !refresh_pending.replace(true) => {
                    let refresh_pending = refresh_pending.clone();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(COALESCE).await;
                        refresh_pending.set(false);
                        // Playback itself changes the resume point every few seconds; pages
                        // refresh when it ends anyway.
                        cx.update(|cx| {
                            if !PlaybackManager::global(cx).read(cx).is_active() {
                                Changes::notify_user_data(cx);
                            }
                        });
                    })
                    .detach();
                }
                _ => {}
            }
        }
    })
}
