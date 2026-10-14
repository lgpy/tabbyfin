//! mpv playback control. v1 drives the `mpv` binary over JSON IPC; see `docs/PLAN.md` §5.
//!
//! [`Player::spawn`] starts mpv in its own window and connects to its IPC socket (a Unix
//! socket, or a named pipe on Windows). Commands are async; player state arrives as
//! [`PlayerEvent`]s on a channel. Dropping the [`Player`] kills mpv.
//!
//! Must be used from within a tokio runtime.

mod events;
mod ipc;
mod process;
#[cfg(feature = "software-render")]
pub mod software;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

pub use events::{EndReason, PlayerEvent};
pub use process::locate_mpv;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv was not found at `{0}`")]
    NotFound(PathBuf),
    #[error("failed to start mpv: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("couldn't connect to mpv's IPC socket")]
    Connect,
    #[error("mpv rejected `{command}`: {message}")]
    Command { command: String, message: String },
    #[error("mpv has exited")]
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// mpv binary. `None` uses [`locate_mpv`].
    pub mpv_path: Option<PathBuf>,
    /// Extra command-line arguments, e.g. `--no-config`.
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadRequest {
    pub url: String,
    pub title: Option<String>,
    pub start: Option<Duration>,
    /// Full header lines, e.g. `Authorization: MediaBrowser ...`. Sent over IPC rather than
    /// on the command line, where other local users could read them.
    pub http_headers: Vec<String>,
}

pub struct Player {
    requests: mpsc::UnboundedSender<ipc::Request>,
    /// Dropping this kills the mpv process.
    _kill: oneshot::Sender<()>,
}

impl Player {
    pub async fn spawn(options: SpawnOptions) -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>)> {
        let (stream, kill) = process::spawn(&options).await?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let requests = ipc::start(stream, events_tx);
        let player = Player {
            requests,
            _kill: kill,
        };
        for (id, property) in events::OBSERVED_PROPERTIES.iter().enumerate() {
            player.command(json!(["observe_property", id, property])).await?;
        }
        Ok((player, events_rx))
    }

    /// Runs a raw mpv command, e.g. `json!(["seek", 10, "relative"])`, and returns its data.
    pub async fn command(&self, command: Value) -> Result<Value> {
        let (reply, response) = oneshot::channel();
        self.requests
            .send(ipc::Request { command, reply })
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }

    pub async fn set_property(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        self.command(json!(["set_property", name, value.into()])).await.map(drop)
    }

    /// Loads a file. Options are set as properties first so this works with every mpv
    /// version (the `loadfile` argument list changed in 0.38).
    pub async fn load(&self, req: LoadRequest) -> Result<()> {
        self.set_property("http-header-fields", req.http_headers).await?;
        if let Some(title) = req.title {
            self.set_property("force-media-title", title).await?;
        }
        let start = req.start.map(|d| format!("{:.3}", d.as_secs_f64())).unwrap_or_else(|| "none".into());
        self.set_property("start", start).await?;
        self.command(json!(["loadfile", req.url, "replace"])).await.map(drop)
    }

    /// Adds an external subtitle track, selecting it if `select`. Call after
    /// [`PlayerEvent::FileLoaded`].
    pub async fn add_subtitle(&self, url: &str, title: &str, language: Option<&str>, select: bool) -> Result<()> {
        let flag = if select { "select" } else { "auto" };
        self.command(json!(["sub-add", url, flag, title, language.unwrap_or("")]))
            .await
            .map(drop)
    }

    /// Asks mpv to exit. [`PlayerEvent::Exited`] follows.
    pub async fn quit(&self) -> Result<()> {
        match self.command(json!(["quit"])).await {
            Ok(_) | Err(Error::Closed) => Ok(()),
            Err(err) => Err(err),
        }
    }
}
