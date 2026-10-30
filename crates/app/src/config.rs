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
    /// The account restored at launch: the one used last, until it signs out.
    pub server: Option<SavedServer>,
    /// Saved sign-ins, most recently used first. Their tokens are in the credential store.
    #[serde(default)]
    pub accounts: Vec<SavedServer>,
    /// Player volume in percent, remembered between sessions.
    #[serde(default)]
    pub volume: Option<f64>,
    /// Light, dark, or following the system.
    #[serde(default)]
    pub theme: ThemePreference,
    /// Text size in percent of the normal size. `None` follows the system's setting.
    #[serde(default)]
    pub text_size: Option<u16>,
    /// The player's clock shows the time left instead of the time played. `None` means no.
    #[serde(default)]
    pub show_remaining_time: Option<bool>,
    /// Look for a newer release on launch. `None` means yes.
    #[serde(default)]
    pub check_for_updates: Option<bool>,
    /// When the update check last ran, in seconds since the Unix epoch.
    #[serde(default)]
    pub update_checked_at: Option<u64>,
    /// Text subtitle colour. `None` means white.
    #[serde(default)]
    pub subtitle_color: Option<SubtitleColor>,
    /// Text subtitle outline or box. `None` means an outline.
    #[serde(default)]
    pub subtitle_style: Option<SubtitleStyle>,
    /// Subtitle size from the player's settings menu, 1.0 being the subtitles' own. `None`
    /// means 1.0.
    #[serde(default)]
    pub subtitle_scale: Option<f64>,
    /// Bits per second; the server transcodes anything above it. `None` means no limit.
    #[serde(default)]
    pub max_streaming_bitrate: Option<u64>,
    /// Measure the connection before playing and cap the bitrate to it, instead of
    /// `max_streaming_bitrate`. `None` means no.
    #[serde(default)]
    pub automatic_bitrate: Option<bool>,
    /// Ask for HEVC rather than H.264 when the server transcodes (if it allows HEVC encoding).
    /// `None` means no.
    #[serde(default)]
    pub prefer_hevc: Option<bool>,
    /// Play the next episode when one ends. `None` means yes.
    #[serde(default)]
    pub autoplay_next_episode: Option<bool>,
    /// Skip intros and recaps without asking. `None` means no.
    #[serde(default)]
    pub auto_skip_intro: Option<bool>,
    /// Animations, smooth scrolling included. `None` means on.
    #[serde(default)]
    pub animations: Option<bool>,
    /// UI zoom from Ctrl+= and Ctrl+-, 1.0 being 100%. `None` means 1.0.
    #[serde(default)]
    pub ui_zoom: Option<f32>,
    /// The playback in progress, saved every few seconds. Still set on launch means the app
    /// didn't exit cleanly (e.g. mpv or a driver crashed), so its stop gets reported then.
    #[serde(default)]
    pub unfinished_playback: Option<UnfinishedPlayback>,
    /// Sort and filter per library, by library id.
    #[serde(default)]
    pub library_views: BTreeMap<String, LibraryView>,
    /// The main window's last placement (frame position, content size and scale factor);
    /// restored at launch when it still lands on a display.
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleColor {
    #[default]
    White,
    Yellow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleStyle {
    /// A dark outline and shadow, mpv's default.
    #[default]
    Outline,
    /// A translucent dark box behind the text.
    Box,
}

/// The main window's last placement: the frame's top-left in physical pixels (or in points in
/// configs saved before `scale`), the content area's size in OS points, and the scale factor the
/// position was measured at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The monitor scale factor `x` and `y` were saved with. `None` in configs from before it
    /// was kept, whose position is in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnfinishedPlayback {
    /// The account that was playing ([`SavedServer::credential_account`]); only that one
    /// reports the stop. Missing in configs from before multiple accounts.
    #[serde(default)]
    pub account: Option<String>,
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
    /// Genre names: items with all of them show (and with all of the tags, when those are set
    /// too). Empty shows everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Genres and tags whose items are left out, whatever else they have.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_tags: Vec<String>,
    /// An official rating (PG-13…); `None` shows every rating.
    #[serde(default)]
    pub rating: Option<String>,
    /// The ratings earlier versions ticked any number of; [`LibraryView::upgrade`] keeps the
    /// first as `rating`.
    #[serde(default, skip_serializing)]
    pub ratings: Vec<String>,
    /// The one genre earlier versions saved; [`LibraryView::upgrade`] moves it to `genres`.
    #[serde(default, skip_serializing)]
    pub genre: Option<String>,
    /// A year ("2024") or a decade ("2020s"); `None` shows every year.
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
}

impl LibraryView {
    /// Moves what older versions saved into the current fields.
    pub fn upgrade(&mut self) {
        if let Some(genre) = self.genre.take()
            && !self.genres.contains(&genre)
        {
            self.genres.push(genre);
        }
        if self.rating.is_none() {
            self.rating = self.ratings.first().cloned();
        }
        self.ratings.clear();
        // Favorites was a progress filter once; it has its own shelf and page now.
        if self.filter == "favorites" {
            self.filter = "all".into();
        }
    }
}

/// A sign-in: a user on a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedServer {
    pub url: String,
    pub server_id: String,
    pub user_id: String,
    pub username: String,
    /// The server's name, to tell accounts apart. Missing in configs from before multiple
    /// accounts.
    #[serde(default)]
    pub server_name: Option<String>,
}

impl SavedServer {
    /// Key for this sign-in's token in the credential store, and its identity.
    pub fn credential_account(&self) -> String {
        account_key(&self.server_id, &self.user_id)
    }

    /// "dev on tabbyfin dev", or the address without a server name.
    pub fn label(&self) -> String {
        let server = self.server_name.clone().unwrap_or_else(|| self.url.clone());
        format!("{} on {server}", self.username)
    }
}

/// The identity of a user on a server, as [`SavedServer::credential_account`].
pub fn account_key(server_id: &str, user_id: &str) -> String {
    format!("{server_id}/{user_id}")
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "tabbyfin")
}

/// The config folder. `TABBYFIN_CONFIG_DIR` overrides it, e.g. so test runs
/// (`dev/win-run.sh`) don't touch the real sign-in.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TABBYFIN_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    project_dirs()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".tabbyfin"))
}

/// Log files. Under the config folder when `TABBYFIN_CONFIG_DIR` is set, so test runs keep
/// their logs to themselves; otherwise the platform's local data dir.
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TABBYFIN_CONFIG_DIR") {
        return PathBuf::from(dir).join("logs");
    }
    project_dirs()
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".tabbyfin/logs"))
}

/// Platform cache dir, for data that can be deleted at any time (e.g. images).
/// The image cache. Under the config folder when `TABBYFIN_CONFIG_DIR` is set, so test runs
/// don't fill the real cache.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TABBYFIN_CONFIG_DIR") {
        return PathBuf::from(dir).join("cache");
    }
    project_dirs()
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".tabbyfin/cache"))
}

fn path() -> PathBuf {
    dir().join("config.toml")
}

impl Config {
    /// Loads the config, creating it (with a fresh device id) if missing or unreadable.
    pub fn load() -> Self {
        let mut config = match std::fs::read_to_string(path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                // The defaults get saved over it soon (a new device id at once), so keep a copy:
                // the saved accounts are in there.
                let backup = path().with_extension("toml.unreadable");
                let kept = std::fs::write(&backup, &text).is_ok();
                tracing::warn!(
                    "ignoring unreadable config {}{}: {err}",
                    path().display(),
                    if kept {
                        format!(" (kept as {})", backup.display())
                    } else {
                        String::new()
                    }
                );
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        config.list_current_account();
        // The views older versions saved keep their one genre in `genre`, which is never written
        // back out: upgrade them here, before anything saves the config again (the restore, the
        // window's placement and the library pages all save it).
        for view in config.library_views.values_mut() {
            view.upgrade();
        }
        if config.device_id.is_empty() {
            config.device_id = uuid::Uuid::new_v4().simple().to_string();
            if let Err(err) = config.save() {
                tracing::warn!("failed to save config: {err:#}");
            }
        }
        config
    }

    /// Configs from before multiple accounts have only the current one; it joins the list.
    fn list_current_account(&mut self) {
        if let Some(current) = self.server.clone()
            && !self
                .accounts
                .iter()
                .any(|a| a.credential_account() == current.credential_account())
        {
            self.accounts.insert(0, current);
        }
    }

    /// Makes `account` the current one and the first in the list.
    pub fn use_account(&mut self, account: SavedServer) {
        self.accounts
            .retain(|a| a.credential_account() != account.credential_account());
        self.accounts.insert(0, account.clone());
        self.server = Some(account);
    }

    /// Drops an account from the list, and as the current one.
    pub fn forget_account(&mut self, key: &str) {
        self.accounts.retain(|a| a.credential_account() != key);
        if self.server.as_ref().is_some_and(|s| s.credential_account() == key) {
            self.server = None;
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = path();
        std::fs::create_dir_all(dir()).context("creating config dir")?;
        let text = toml::to_string_pretty(self)?;
        // Written aside and renamed over the config, so a crash mid-write can't leave half a
        // file (which would read as unreadable and lose the saved accounts).
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
        std::fs::rename(&temp, &path).with_context(|| format!("replacing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, LibraryView, SavedServer};

    #[test]
    fn a_library_view_with_one_genre_upgrades_to_the_list() {
        let mut view: LibraryView =
            toml::from_str("sort = \"name\"\ndescending = false\nfilter = \"all\"\ngenre = \"Comedy\"\n")
                .expect("parses");
        view.upgrade();
        assert_eq!(view.genres, ["Comedy"]);
        assert_eq!(view.genre, None);
        let saved = toml::to_string(&view).expect("serializes");
        assert!(saved.contains("genres") && !saved.contains("genre ="), "{saved}");
    }

    fn account(server: &str, user: &str) -> SavedServer {
        SavedServer {
            url: format!("http://{server}"),
            server_id: server.into(),
            user_id: user.into(),
            username: user.into(),
            server_name: None,
        }
    }

    #[test]
    fn an_old_config_lists_its_account() {
        let mut config = Config {
            server: Some(account("s1", "u1")),
            ..Default::default()
        };
        config.list_current_account();
        config.list_current_account();
        assert_eq!(config.accounts, [account("s1", "u1")]);
    }

    #[test]
    fn the_account_used_last_comes_first() {
        let mut config = Config::default();
        config.use_account(account("s1", "u1"));
        config.use_account(account("s2", "u2"));
        config.use_account(account("s1", "u1"));
        assert_eq!(config.accounts, [account("s1", "u1"), account("s2", "u2")]);
        assert_eq!(config.server, Some(account("s1", "u1")));
    }

    #[test]
    fn forgetting_the_current_account_leaves_none_current() {
        let mut config = Config::default();
        config.use_account(account("s2", "u2"));
        config.use_account(account("s1", "u1"));
        config.forget_account(&account("s1", "u1").credential_account());
        assert_eq!(config.accounts, [account("s2", "u2")]);
        assert_eq!(config.server, None);
        // Forgetting another account keeps the current one.
        config.use_account(account("s1", "u1"));
        config.forget_account(&account("s2", "u2").credential_account());
        assert_eq!(config.server, Some(account("s1", "u1")));
    }
}
