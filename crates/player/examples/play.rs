//! Plays a URL in mpv for a few seconds, exercising load, seek, pause and quit, and prints
//! every event.
//!
//! ```sh
//! cargo run -p player --example play -- <url> ["Header: value"]
//! ```

use std::time::Duration;

use player::{LoadRequest, Player, PlayerEvent, SpawnOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("usage: play <url> [header]");
    let http_headers = args.collect();

    let (player, mut events) = Player::spawn_external(SpawnOptions::default()).await?;
    player
        .load(LoadRequest {
            url,
            title: Some("player example".into()),
            start: Some(Duration::from_secs(5)),
            http_headers,
        })
        .await?;

    let script = async {
        tokio::time::sleep(Duration::from_secs(4)).await;
        println!("-> seek +20s");
        player.command(json!(["seek", 20, "relative"])).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("-> pause");
        player.set_property("pause", true).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("-> quit");
        player.quit().await?;
        anyhow::Ok(())
    };
    let printer = async {
        while let Some(event) = events.recv().await {
            println!("{event:?}");
            if event == PlayerEvent::Exited {
                break;
            }
        }
    };
    let (script, ()) = tokio::join!(script, printer);
    script
}
