//! mpv playback for the app.
//!
//! libmpv renders into a [`libmpv::VideoOutput`] the UI draws, in a form the UI said it can take
//! ([`libmpv::VideoImport`]); see `docs/in-app-video.md`. By
//! default it runs in a separate player host process (this executable with `--player-host`,
//! see [`host`] and [`remote`]), so an mpv or GPU driver crash doesn't take the app down;
//! `TABBYFIN_PLAYER=embedded` runs it inside the app instead. Commands are async; player
//! state arrives as [`PlayerEvent`]s on a channel. Dropping the [`Player`] stops playback. Must
//! be used from within a tokio runtime.

mod events;
pub mod host;
pub mod libmpv;
mod remote;
mod wire;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

pub use events::{EndReason, PlayerEvent};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to start a player thread: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("libmpv isn't available: {0}")]
    LibmpvUnavailable(String),
    #[error("mpv rejected `{command}`: {message}")]
    Command { command: String, message: String },
    /// GPU video output couldn't be set up; playback uses software rendering instead.
    #[error("{0}")]
    GpuUnavailable(String),
    #[error("mpv has exited")]
    Closed,
    /// The player host process failed (see [`remote`]).
    #[error("player process: {0}")]
    Host(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Default)]
pub struct LoadRequest {
    pub url: String,
    pub title: Option<String>,
    pub start: Option<Duration>,
    /// Full header lines, e.g. `Authorization: MediaBrowser ...`.
    pub http_headers: Vec<String>,
}

/// An mpv instance.
pub struct Player(Backend);

enum Backend {
    /// libmpv in this process. Its calls block, so they run on tokio's blocking pool.
    Embedded(Arc<libmpv::EmbeddedPlayer>),
    /// libmpv in a player host process. Boxed: it's much larger than the embedded handle.
    Remote(Box<remote::RemotePlayer>),
}

type Started = (Player, mpsc::UnboundedReceiver<PlayerEvent>, libmpv::VideoOutput);

impl Player {
    /// Starts mpv, in a player host process unless `TABBYFIN_PLAYER=embedded`, with a video
    /// output whose frames the UI can draw (`import`). Fails with [`Error::LibmpvUnavailable`]
    /// where libmpv can't be loaded.
    pub async fn spawn(import: libmpv::VideoImport) -> Result<Started> {
        if std::env::var("TABBYFIN_PLAYER").as_deref() == Ok("embedded") {
            return Self::spawn_embedded(import).await;
        }
        let (player, events, video) = remote::RemotePlayer::spawn(import).await?;
        Ok((Player(Backend::Remote(Box::new(player))), events, video))
    }

    /// Starts mpv in this process.
    pub(crate) async fn spawn_embedded(import: libmpv::VideoImport) -> Result<Started> {
        let (player, events) = tokio::task::spawn_blocking(move || libmpv::EmbeddedPlayer::spawn(import))
            .await
            .map_err(|_| Error::Closed)??;
        let video = player.video();
        Ok((Player(Backend::Embedded(Arc::new(player))), events, video))
    }

    /// Runs a raw mpv command, e.g. `json!(["seek", 10, "relative"])`.
    pub async fn command(&self, command: Value) -> Result<()> {
        let args = match command {
            Value::Array(args) => args,
            other => vec![other],
        };
        match &self.0 {
            Backend::Embedded(player) => {
                let player = player.clone();
                blocking(move || player.command(&args)).await
            }
            Backend::Remote(player) => player.command(args).await,
        }
    }

    pub async fn set_property(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        let value = value.into();
        match &self.0 {
            Backend::Embedded(player) => {
                let (player, name) = (player.clone(), name.to_owned());
                blocking(move || player.set_property(&name, &value)).await
            }
            Backend::Remote(player) => player.set_property(name, value).await,
        }
    }

    /// Loads a file. Options are set as properties first so this works with every mpv
    /// version (the `loadfile` argument list changed in 0.38).
    pub async fn load(&self, req: LoadRequest) -> Result<()> {
        self.set_property("http-header-fields", req.http_headers).await?;
        if let Some(title) = req.title {
            self.set_property("force-media-title", title).await?;
        }
        let start = req
            .start
            .map(|d| format!("{:.3}", d.as_secs_f64()))
            .unwrap_or_else(|| "none".into());
        self.set_property("start", start).await?;
        self.command(json!(["loadfile", req.url, "replace"])).await
    }

    /// Adds an external subtitle track, selecting it if `select`. Call after
    /// [`PlayerEvent::FileLoaded`].
    pub async fn add_subtitle(&self, url: &str, title: &str, language: Option<&str>, select: bool) -> Result<()> {
        let flag = if select { "select" } else { "auto" };
        self.command(json!(["sub-add", url, flag, title, language.unwrap_or("")]))
            .await
    }

    /// Asks mpv to stop. [`PlayerEvent::Exited`] follows.
    pub async fn quit(&self) -> Result<()> {
        match self.command(json!(["quit"])).await {
            Ok(()) | Err(Error::Closed) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

async fn blocking(f: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    tokio::task::spawn_blocking(f).await.map_err(|_| Error::Closed)?
}

/// The [`EndReason::Error`] message when the player host process died (an mpv or GPU driver
/// crash), as opposed to a stream or file error.
pub const HOST_CRASHED: &str = "the player process stopped unexpectedly";

/// Whether this process was started as the player host (`--player-host`). The app checks this
/// first thing in `main` and then calls [`host::run`] instead of starting its UI.
pub fn is_host_process() -> bool {
    std::env::args().nth(1).as_deref() == Some(host::HOST_ARG)
}
