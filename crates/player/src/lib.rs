//! mpv playback control.
//!
//! Two backends behind one [`Player`]:
//! - **Embedded** (`libmpv` feature): libmpv runs inside this process and renders into a
//!   [`libmpv::VideoOutput`] the UI draws. See `docs/in-app-video.md`.
//! - **External**: the `mpv` binary in its own window, driven over JSON IPC (a Unix socket, or
//!   a named pipe on Windows). Used when libmpv isn't available.
//!
//! Commands are async; player state arrives as [`PlayerEvent`]s on a channel. Dropping the
//! [`Player`] stops playback. Must be used from within a tokio runtime.

mod events;
mod ipc;
#[cfg(feature = "libmpv")]
pub mod libmpv;
mod process;

use std::path::PathBuf;
#[cfg(feature = "libmpv")]
use std::sync::Arc;
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
    #[error("libmpv isn't available: {0}")]
    LibmpvUnavailable(String),
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
    /// Full header lines, e.g. `Authorization: MediaBrowser ...`. Set as a property rather
    /// than on a command line, where other local users could read them.
    pub http_headers: Vec<String>,
}

pub enum Player {
    External(ExternalPlayer),
    #[cfg(feature = "libmpv")]
    Embedded(Arc<libmpv::EmbeddedPlayer>),
}

impl Player {
    /// Starts mpv in its own window.
    pub async fn spawn_external(options: SpawnOptions) -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>)> {
        let (player, events) = ExternalPlayer::spawn(options).await?;
        Ok((Player::External(player), events))
    }

    /// Starts mpv inside this process. Fails with [`Error::LibmpvUnavailable`] where libmpv
    /// can't be loaded.
    #[cfg(feature = "libmpv")]
    pub async fn spawn_embedded(
        options: libmpv::EmbeddedOptions,
    ) -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>, libmpv::VideoOutput)> {
        let (player, events) = tokio::task::spawn_blocking(move || libmpv::EmbeddedPlayer::spawn(&options))
            .await
            .map_err(|_| Error::Closed)??;
        let video = player.video();
        Ok((Player::Embedded(Arc::new(player)), events, video))
    }

    /// Runs a raw mpv command, e.g. `json!(["seek", 10, "relative"])`.
    pub async fn command(&self, command: Value) -> Result<()> {
        match self {
            Player::External(player) => player.command(command).await.map(drop),
            #[cfg(feature = "libmpv")]
            Player::Embedded(player) => {
                let player = player.clone();
                let args = match command {
                    Value::Array(args) => args,
                    other => vec![other],
                };
                blocking(move || player.command(&args)).await
            }
        }
    }

    pub async fn set_property(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        let value = value.into();
        match self {
            Player::External(player) => player.command(json!(["set_property", name, value])).await.map(drop),
            #[cfg(feature = "libmpv")]
            Player::Embedded(player) => {
                let (player, name) = (player.clone(), name.to_owned());
                blocking(move || player.set_property(&name, &value)).await
            }
        }
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
        self.command(json!(["loadfile", req.url, "replace"])).await
    }

    /// Adds an external subtitle track, selecting it if `select`. Call after
    /// [`PlayerEvent::FileLoaded`].
    pub async fn add_subtitle(&self, url: &str, title: &str, language: Option<&str>, select: bool) -> Result<()> {
        let flag = if select { "select" } else { "auto" };
        self.command(json!(["sub-add", url, flag, title, language.unwrap_or("")])).await
    }

    /// Asks mpv to stop. [`PlayerEvent::Exited`] follows.
    pub async fn quit(&self) -> Result<()> {
        match self.command(json!(["quit"])).await {
            Ok(()) | Err(Error::Closed) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

#[cfg(feature = "libmpv")]
async fn blocking(f: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    tokio::task::spawn_blocking(f).await.map_err(|_| Error::Closed)?
}

/// The `mpv` binary in its own window, over JSON IPC.
pub struct ExternalPlayer {
    requests: mpsc::UnboundedSender<ipc::Request>,
    /// Dropping this kills the mpv process.
    _kill: oneshot::Sender<()>,
}

impl ExternalPlayer {
    async fn spawn(options: SpawnOptions) -> Result<(Self, mpsc::UnboundedReceiver<PlayerEvent>)> {
        let (stream, kill) = process::spawn(&options).await?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let requests = ipc::start(stream, events_tx);
        let player = Self { requests, _kill: kill };
        for (id, property) in events::OBSERVED_PROPERTIES.iter().enumerate() {
            player.command(json!(["observe_property", id, property])).await?;
        }
        Ok((player, events_rx))
    }

    async fn command(&self, command: Value) -> Result<Value> {
        let (reply, response) = oneshot::channel();
        self.requests
            .send(ipc::Request { command, reply })
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }
}
