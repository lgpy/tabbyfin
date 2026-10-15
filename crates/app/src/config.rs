//! Persistent app settings, stored as TOML in the platform config dir. Never holds secrets:
//! access tokens live in [`crate::credentials`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Generated once per install. Jellyfin ties sessions to it.
    pub device_id: String,
    /// The server and user of the last successful sign-in.
    pub server: Option<SavedServer>,
    /// Player volume in percent, remembered between sessions.
    #[serde(default)]
    pub volume: Option<f64>,
    /// Light, dark, or following the system.
    #[serde(default)]
    pub theme: ThemePreference,
    /// The player's clock shows the time left instead of the time played. `None` means no.
    #[serde(default)]
    pub show_remaining_time: Option<bool>,
    /// Subtitle size from the player's settings menu, 1.0 being the subtitles' own. `None`
    /// means 1.0.
    #[serde(default)]
    pub subtitle_scale: Option<f64>,
    /// Bits per second; the server transcodes anything above it. `None` means no limit.
    #[serde(default)]
    pub max_streaming_bitrate: Option<u64>,
    /// Play the next episode when one ends. `None` means yes.
    #[serde(default)]
    pub autoplay_next_episode: Option<bool>,
    /// Skip intros and recaps without asking. `None` means no.
    #[serde(default)]
    pub auto_skip_intro: Option<bool>,
    /// The playback in progress, saved every few seconds. Still set on launch means the app
    /// didn't exit cleanly (e.g. mpv or a driver crashed), so its stop gets reported then.
    #[serde(default)]
    pub unfinished_playback: Option<UnfinishedPlayback>,
    /// Sort and filter per library, by library id.
    #[serde(default)]
    pub library_views: BTreeMap<String, LibraryView>,
    /// The main window's last size and position; restored at launch when it still lands on a
    /// display.
    #[serde(default)]
    pub window_bounds: Option<WindowGeometry>,
    /// Whether the main window was maximized; restored on top of `window_bounds`.
    #[serde(default)]
    pub window_maximized: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follows the system's light or dark appearance, also when it changes.
    System,
    Light,
    #[default]
    Dark,
}

/// The main window's position and size in the global screen space, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnfinishedPlayback {
    pub item_id: String,
    pub media_source_id: String,
    pub play_session_id: Option<String>,
    pub position_ticks: i64,
}

/// How a library grid is sorted and filtered. Keys are the library page's own names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryView {
    pub sort: String,
    pub descending: bool,
    pub filter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedServer {
    pub url: String,
    pub server_id: String,
    pub user_id: String,
    pub username: String,
}

impl SavedServer {
    /// Key for this sign-in's token in the credential store.
    pub fn credential_account(&self) -> String {
        format!("{}/{}", self.server_id, self.user_id)
    }
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "jelly-gpui")
}

/// The config folder. `JELLY_GPUI_CONFIG_DIR` overrides it, e.g. so test runs
/// (`dev/win-run.sh`) don't touch the real sign-in.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("JELLY_GPUI_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    project_dirs()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".jelly-gpui"))
}

/// Log files. Under the config folder when `JELLY_GPUI_CONFIG_DIR` is set, so test runs keep
/// their logs to themselves; otherwise the platform's local data dir.
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("JELLY_GPUI_CONFIG_DIR") {
        return PathBuf::from(dir).join("logs");
    }
    project_dirs()
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".jelly-gpui/logs"))
}

/// Platform cache dir, for data that can be deleted at any time (e.g. images).
pub fn cache_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".jelly-gpui/cache"))
}

fn path() -> PathBuf {
    dir().join("config.toml")
}

impl Config {
    /// Loads the config, creating it (with a fresh device id) if missing or unreadable.
    pub fn load() -> Self {
        let mut config = match std::fs::read_to_string(path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                tracing::warn!("ignoring unreadable config {}: {err}", path().display());
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        if config.device_id.is_empty() {
            config.device_id = uuid::Uuid::new_v4().simple().to_string();
            if let Err(err) = config.save() {
                tracing::warn!("failed to save config: {err:#}");
            }
        }
        config
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = path();
        std::fs::create_dir_all(dir()).context("creating config dir")?;
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }
}
