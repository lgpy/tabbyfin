//! mpv playback, embedded in the app.
//!
//! libmpv runs inside this process and renders into a [`libmpv::VideoOutput`] the UI draws;
//! see `docs/in-app-video.md`. Commands are async; player state arrives as [`PlayerEvent`]s on
//! a channel. Dropping the [`Player`] stops playback. Must be used from within a tokio runtime.

mod events;
pub mod libmpv;

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

/// An mpv instance. libmpv calls block, so they run on tokio's blocking pool.
pub struct Player(Arc<libmpv::EmbeddedPlayer>);

impl Player {
    /// Starts mpv. Fails with [`Error::LibmpvUnavailable`] where libmpv can't be loaded.
    pub async fn spawn() -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>, libmpv::VideoOutput)> {
        let (player, events) = tokio::task::spawn_blocking(libmpv::EmbeddedPlayer::spawn)
            .await
            .map_err(|_| Error::Closed)??;
        let video = player.video();
        Ok((Player(Arc::new(player)), events, video))
    }

    /// Runs a raw mpv command, e.g. `json!(["seek", 10, "relative"])`.
    pub async fn command(&self, command: Value) -> Result<()> {
        let player = self.0.clone();
        let args = match command {
            Value::Array(args) => args,
            other => vec![other],
        };
        blocking(move || player.command(&args)).await
    }

    pub async fn set_property(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        let (player, name, value) = (self.0.clone(), name.to_owned(), value.into());
        blocking(move || player.set_property(&name, &value)).await
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

async fn blocking(f: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    tokio::task::spawn_blocking(f).await.map_err(|_| Error::Closed)?
}
