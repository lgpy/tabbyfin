//! The app's look: dark, light or the system's theme, text size, zoom (Ctrl+= / Ctrl+- /
//! Ctrl+0), animations, and the colours over video (`over_video`). A1 fills it.

use crate::config::Config;

/// Applies the config's appearance, at launch and when Settings changes it.
pub fn apply(_ctx: &egui::Context, _config: &Config) {}
