//! Live check of the session websocket and the server clock.
//!
//! ```sh
//! cargo run -p jellyfin --example socket
//! ```
//!
//! Connects `/socket` with header auth, reports capabilities, samples `/GetUtcTime`, then
//! toggles an item's watched state and waits for the server's `UserDataChanged` push. Same env
//! as the smoke example: `JELLYFIN_URL` (default `localhost:8096`), `JELLYFIN_USER` /
//! `JELLYFIN_PASS` (default `dev` / `dev`), plus `JELLYFIN_SOCKET_HOLD` (seconds to stay
//! connected afterwards, checking the keep-alive).

use std::time::Duration;

use anyhow::{Context, bail};
use jellyfin::models::BaseItemKind;
use jellyfin::{Client, ClientInfo, ItemsQuery, ServerClock, ServerMessage, SocketEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.into());
    let anon = Client::new(
        &env("JELLYFIN_URL", "localhost:8096"),
        ClientInfo {
            client: "jelly-gpui-socket".into(),
            device: "socket-test".into(),
            device_id: "jelly-gpui-socket-test".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let auth = anon
        .authenticate_by_name(&env("JELLYFIN_USER", "dev"), &env("JELLYFIN_PASS", "dev"))
        .await?;
    let client = anon.with_token(auth.access_token.as_str());

    let mut events = client.socket();
    match wait(&mut events, 10).await {
        Some(SocketEvent::Connected) => println!("ok: websocket connected with header auth"),
        other => bail!("expected Connected, got {other:?}"),
    }
    client.report_capabilities(&Default::default()).await?;
    println!("ok: capabilities reported");

    let mut clock = ServerClock::default();
    for _ in 0..4 {
        clock.add(client.clock_sample().await?);
    }
    let best = clock.best().context("no clock sample")?;
    println!(
        "ok: server clock offset {:.1} ms (round trip {:.1} ms)",
        best.offset_us as f64 / 1000.,
        best.round_trip.as_secs_f64() * 1000.
    );

    let movie = client
        .items(&ItemsQuery {
            include_item_types: vec![BaseItemKind::Movie],
            recursive: true,
            limit: Some(1),
            ..Default::default()
        })
        .await?
        .items
        .into_iter()
        .next()
        .context("no movie on the server")?;
    let played = movie.user_data.as_ref().is_some_and(|d| d.played);
    let toggle = |played: bool| {
        let (client, id) = (client.clone(), movie.id.clone());
        async move {
            if played {
                client.mark_played(&id).await
            } else {
                client.mark_unplayed(&id).await
            }
        }
    };
    toggle(!played).await?;
    let pushed = loop {
        match wait(&mut events, 10).await {
            Some(SocketEvent::Message(ServerMessage::UserDataChanged { item_ids })) => break item_ids,
            Some(_) => continue,
            None => {
                toggle(played).await?;
                bail!("no UserDataChanged within 10 s");
            }
        }
    };
    toggle(played).await?;
    if !pushed.contains(&movie.id) {
        bail!("UserDataChanged for {pushed:?}, expected {}", movie.id);
    }
    println!("ok: UserDataChanged pushed for {:?}", movie.name);

    // `JELLYFIN_SOCKET_HOLD=150`: stay connected that long, to check the keep-alive.
    if let Ok(hold) = env("JELLYFIN_SOCKET_HOLD", "").parse::<u64>() {
        let until = tokio::time::Instant::now() + Duration::from_secs(hold);
        while let Ok(event) = tokio::time::timeout_at(until, events.recv()).await {
            if let Some(SocketEvent::Disconnected) | None = event {
                bail!("disconnected while holding");
            }
        }
        println!("ok: still connected after {hold} s");
    }
    Ok(())
}

async fn wait(events: &mut tokio::sync::mpsc::UnboundedReceiver<SocketEvent>, secs: u64) -> Option<SocketEvent> {
    tokio::time::timeout(Duration::from_secs(secs), events.recv())
        .await
        .ok()
        .flatten()
}
