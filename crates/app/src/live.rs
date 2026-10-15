//! The session websocket while signed in: keeps this session's capabilities reported,
//! refreshes pages when watch data changes elsewhere (another device, the web dashboard),
//! hands SyncPlay its updates and commands, and remote control its commands.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;
use jellyfin::{Capabilities, Client, GeneralCommand, ServerMessage, SocketEvent};

use crate::changes::Changes;
use crate::playback::PlaybackManager;
use crate::remote::Remote;
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
                let capabilities = Capabilities {
                    supports_media_control: true,
                    supported_commands: GeneralCommand::SUPPORTED.to_vec(),
                };
                if let Err(err) = client.report_capabilities(&capabilities).await {
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
                SocketEvent::Message(ServerMessage::Play(request)) => {
                    cx.update(|cx| Remote::global(cx).update(cx, |r, cx| r.handle_play(request, cx)));
                }
                SocketEvent::Message(ServerMessage::Playstate(request)) => {
                    cx.update(|cx| Remote::global(cx).update(cx, |r, cx| r.handle_playstate(request, cx)));
                }
                SocketEvent::Message(ServerMessage::GeneralCommand(command)) => {
                    cx.update(|cx| Remote::global(cx).update(cx, |r, cx| r.handle_command(command, cx)));
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
