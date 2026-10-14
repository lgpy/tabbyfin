//! The session websocket while signed in: keeps this session's capabilities reported and
//! refreshes pages when watch data changes elsewhere (another device, the web dashboard).

use std::time::Duration;

use gpui_kit::*;
use jellyfin::{Client, ServerMessage, SocketEvent};

use crate::changes::Changes;
use crate::playback::PlaybackManager;
use crate::runtime;

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
        while let Some(event) = events.recv().await {
            match event {
                SocketEvent::Connected => report_capabilities(&client),
                SocketEvent::Message(ServerMessage::UserDataChanged { .. }) => {
                    cx.background_executor().timer(COALESCE).await;
                    while let Ok(event) = events.try_recv() {
                        if let SocketEvent::Connected = event {
                            report_capabilities(&client);
                        }
                    }
                    // Playback itself changes the resume point every few seconds; pages
                    // refresh when it ends anyway.
                    cx.update(|cx| {
                        if !PlaybackManager::global(cx).read(cx).is_active() {
                            Changes::notify_user_data(cx);
                        }
                    });
                }
                _ => {}
            }
        }
    })
}
