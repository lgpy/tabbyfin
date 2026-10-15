//! The light or dark theme, from the user's choice or the system's appearance.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::config::{Config, ThemePreference};

/// Applies the theme for `preference`. With a window, the system appearance is read from it,
/// which is more reliable on Linux than the app-wide one.
pub fn apply(preference: ThemePreference, window: Option<&mut Window>, cx: &mut App) {
    match preference {
        ThemePreference::System => Theme::sync_system_appearance(window, cx),
        ThemePreference::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePreference::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
    // A theme change resets the font size the zoom sets.
    crate::zoom::apply(cx);
}

/// Follows the system appearance in `window` while the preference is to.
pub fn follow_system(window: &mut Window) -> Subscription {
    window.observe_window_appearance(|window, cx| {
        if Config::load().theme == ThemePreference::System {
            apply(ThemePreference::System, Some(window), cx);
        }
    })
}
