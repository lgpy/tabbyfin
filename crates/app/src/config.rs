//! Persistent app settings, stored as TOML in the platform config dir. Never holds secrets:
//! access tokens live in [`crate::credentials`].

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
    /// Bits per second; the server transcodes anything above it. `None` means no limit.
    #[serde(default)]
    pub max_streaming_bitrate: Option<u64>,
    /// Play the next episode when one ends. `None` means yes.
    #[serde(default)]
    pub autoplay_next_episode: Option<bool>,
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
