//! The main window as it opens: title, icon, the app id desktops know it by, and its size.
//!
//! A1 adds its placement: restoring the saved size, position and maximized state at launch, and
//! saving them when they change.

use egui::{Vec2, vec2};

use crate::config::Config;

/// The window size used when nothing usable is saved.
const DEFAULT_SIZE: Vec2 = vec2(1280.0, 800.0);
/// The smallest window the layout works in: below it the player's control bar and an item
/// page's title line no longer fit.
pub const MIN_SIZE: Vec2 = vec2(800.0, 500.0);

pub fn viewport(_config: &Config) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Tabbyfin")
        .with_app_id(app_id())
        .with_inner_size(DEFAULT_SIZE)
        .with_min_inner_size(MIN_SIZE);
    match eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/tabbyfin.png")) {
        Ok(icon) => viewport.with_icon(icon),
        Err(err) => {
            tracing::warn!("couldn't read the window icon: {err}");
            viewport
        }
    }
}

/// The Wayland app id (the class on X11), which desktops match to the installed .desktop file
/// for the window's icon and name: `tabbyfin.desktop` from the .deb and the AppImage. A Flatpak
/// installs it under the app's id, which it sets as `FLATPAK_ID`.
fn app_id() -> String {
    std::env::var("FLATPAK_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "tabbyfin".to_owned())
}

/// Keeps the saved placement in step with the window. A1 fills it: saving the size, position and
/// maximized state shortly after they stop changing and on close, and moving a window saved on
/// a display that's gone back on screen.
#[derive(Default)]
pub struct Placement;

impl Placement {
    pub fn new(_config: &Config) -> Self {
        Self
    }

    /// Each frame, before the UI.
    pub fn tick(&mut self, _ctx: &egui::Context) {}

    /// On close: saves a change that's still waiting.
    pub fn save_now(&mut self) {}
}
