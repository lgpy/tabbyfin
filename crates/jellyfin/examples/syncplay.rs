//! A scripted SyncPlay member, for testing the app against a second participant.
//!
//! ```sh
//! SYNCPLAY_SCRIPT="queue:<item id> wait:5 pause wait:3 unpause wait:5 seek:30 wait:5 stop leave" \
//!     cargo run -p jellyfin --example syncplay
//! ```
//!
//! It joins the group named `SYNCPLAY_GROUP` (default `dev group`), creating it if there's
//! none, and prints every group update and command. Like a player, it answers queue changes
//! and seeks with `Buffering` then `Ready`. `SYNCPLAY_SCRIPT` lists actions run in order:
//! `queue:<item id>`, `pause`, `unpause`, `seek:<seconds>`, `stop`, `leave`, `wait:<seconds>`.
//! Without a script it stays in the group until killed. Same server env as the smoke example,
//! plus `SYNCPLAY_DEVICE` to run several members at once.

use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, bail};
use jellyfin::{
    BufferReport, Client, ClientInfo, CommandKind, GroupUpdate, ServerClock, ServerMessage, SocketEvent, Ticks,
};
use tokio::sync::mpsc::UnboundedReceiver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.into());
    let device = env("SYNCPLAY_DEVICE", "tabbyfin-syncplay-test");
    let anon = Client::new(
        &env("JELLYFIN_URL", "localhost:8096"),
        ClientInfo {
            client: "tabbyfin-syncplay".into(),
            device: device.clone(),
            device_id: device,
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let auth = anon
        .authenticate_by_name(&env("JELLYFIN_USER", "dev"), &env("JELLYFIN_PASS", "dev"))
        .await?;
    let client = anon.with_token(auth.access_token.as_str());
    let start = Instant::now();
    let log = move |text: String| println!("[{:6.2}] {text}", start.elapsed().as_secs_f64());

    let mut events = client.socket();
    match tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
        Ok(Some(SocketEvent::Connected)) => {}
        other => bail!("websocket didn't connect: {other:?}"),
    }
    client.report_capabilities(&Default::default()).await?;
    let mut clock = ServerClock::default();
    for _ in 0..4 {
        clock.add(client.clock_sample().await?);
    }
    let ping = clock.best().context("no clock sample")?.round_trip.as_millis() as u64;
    client.syncplay_ping(ping).await?;

    let group_name = env("SYNCPLAY_GROUP", "dev group");
    let groups = client.syncplay_groups().await?;
    match groups.iter().find(|g| g.group_name == group_name) {
        Some(group) => {
            log(format!("joining {:?} ({:?})", group.group_name, group.participants));
            client.syncplay_join(&group.group_id).await?;
        }
        None => {
            log(format!("creating {group_name:?}"));
            client.syncplay_new(&group_name).await?;
        }
    }

    let script: Vec<String> = env("SYNCPLAY_SCRIPT", "")
        .split_whitespace()
        .map(String::from)
        .collect();
    let script_is_empty = script.is_empty();
    let actions = {
        let client = client.clone();
        async move {
            for action in script {
                let (name, arg) = action.split_once(':').unwrap_or((&action, ""));
                log(format!("> {action}"));
                let result = match name {
                    "wait" => {
                        tokio::time::sleep(Duration::from_secs_f64(arg.parse()?)).await;
                        Ok(())
                    }
                    "queue" => client.syncplay_set_queue(&[arg.to_owned()], 0, Ticks::default()).await,
                    "pause" => client.syncplay_pause().await,
                    "unpause" => client.syncplay_unpause().await,
                    "seek" => {
                        client
                            .syncplay_seek(Ticks::from(Duration::from_secs_f64(arg.parse()?)))
                            .await
                    }
                    "stop" => client.syncplay_stop().await,
                    "leave" => client.syncplay_leave().await,
                    other => bail!("unknown action {other:?}"),
                };
                if let Err(err) = result {
                    log(format!("! {action} failed: {err}"));
                }
            }
            anyhow::Ok(())
        }
    };
    let member = Member {
        client: client.clone(),
        clock,
        log: Box::new(log),
        playlist_item_id: String::new(),
        position: Ticks::default(),
        playing_since: None,
    };
    let scripted = !script_is_empty;
    tokio::select! {
        result = actions, if scripted => {
            result?;
            // Let the last updates arrive.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        result = member.run(&mut events) => result?,
    }
    Ok(())
}

/// Plays along: a pretend player whose position advances while "playing".
struct Member {
    client: Client,
    clock: ServerClock,
    log: Box<dyn Fn(String) + Send>,
    playlist_item_id: String,
    position: Ticks,
    playing_since: Option<(Instant, Ticks)>,
}

impl Member {
    async fn run(mut self, events: &mut UnboundedReceiver<SocketEvent>) -> anyhow::Result<()> {
        while let Some(event) = events.recv().await {
            let SocketEvent::Message(message) = event else { continue };
            match message {
                ServerMessage::SyncPlayGroupUpdate(update) => {
                    (self.log)(format!("update {update:?}"));
                    match update {
                        GroupUpdate::PlayQueue(queue) => {
                            if let Some(item) = queue.current()
                                && item.playlist_item_id != self.playlist_item_id
                            {
                                self.playlist_item_id = item.playlist_item_id.clone();
                                self.position = queue.start_position_ticks;
                                self.playing_since = None;
                                self.report(false, queue.is_playing).await?;
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                self.report(true, queue.is_playing).await?;
                            }
                        }
                        GroupUpdate::GroupLeft => return Ok(()),
                        _ => {}
                    }
                }
                ServerMessage::SyncPlayCommand(command) => {
                    let local = self.clock.to_local(command.when);
                    let lead = match local.duration_since(SystemTime::now()) {
                        Ok(ahead) => format!("in {} ms", ahead.as_millis()),
                        Err(late) => format!("{} ms late", late.duration().as_millis()),
                    };
                    (self.log)(format!(
                        "command {:?} at {:.2} s, {lead}",
                        command.kind,
                        command.position.as_secs_f64()
                    ));
                    self.position = command.position;
                    match command.kind {
                        CommandKind::Unpause => self.playing_since = Some((Instant::now(), command.position)),
                        CommandKind::Pause | CommandKind::Stop => self.playing_since = None,
                        CommandKind::Seek => {
                            self.playing_since = None;
                            self.report(false, false).await?;
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            self.report(true, false).await?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `Buffering` (`ready: false`) or `Ready` at the current pretend position.
    async fn report(&mut self, ready: bool, is_playing: bool) -> anyhow::Result<()> {
        if let Some((since, from)) = self.playing_since {
            self.position = Ticks::from(Duration::from(from) + since.elapsed());
        }
        let report = BufferReport {
            when: self.clock.server_now(),
            position: self.position,
            is_playing,
            playlist_item_id: self.playlist_item_id.clone(),
        };
        if ready {
            self.client.syncplay_ready(&report).await?;
        } else {
            self.client.syncplay_buffering(&report).await?;
        }
        (self.log)(format!(
            "sent {} at {:.2} s",
            if ready { "Ready" } else { "Buffering" },
            report.position.as_secs_f64()
        ));
        Ok(())
    }
}
