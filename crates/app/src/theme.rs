//! The light or dark theme, from the user's choice or the system's appearance, and the text
//! size.

use std::sync::OnceLock;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::config::{Config, ThemePreference};

/// The theme's text size at 100% (its rem: most text and controls are sized in rems).
const BASE_FONT_SIZE: f32 = 16.;
/// Text sizes the settings offer, in percent.
pub const TEXT_SIZES: [u16; 5] = [90, 100, 115, 130, 150];

/// Applies the theme for `preference`. With a window, the system appearance is read from it,
/// which is more reliable on Linux than the app-wide one.
pub fn apply(preference: ThemePreference, window: Option<&mut Window>, cx: &mut App) {
    match preference {
        ThemePreference::System => Theme::sync_system_appearance(window, cx),
        ThemePreference::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePreference::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
    // A theme change loads the theme afresh, so these go on top each time.
    Theme::update(cx, |theme| {
        // The light theme's grey for secondary text is 4.35:1 on its light-grey fills (selected
        // rows, chips, menus), under the 4.5:1 WCAG AA asks of small text; one step darker
        // (neutral-600) is 7.2:1. The dark theme's passes as it is (6:1 or more).
        if !theme.mode.is_dark() {
            theme.muted_foreground = rgb(0x525252).into();
        }
        theme.font_size = px(BASE_FONT_SIZE * text_scale(Config::load().text_size));
    });
}

/// Applies a changed text size setting.
pub fn apply_text_size(cx: &mut App) {
    let size = px(BASE_FONT_SIZE * text_scale(Config::load().text_size));
    Theme::update(cx, |theme| theme.font_size = size);
}

/// The text size factor for a setting: the chosen percentage, or the system's.
pub fn text_scale(setting: Option<u16>) -> f32 {
    let scale = match setting {
        Some(percent) => f32::from(percent) / 100.,
        None => system_text_scale(),
    };
    scale.clamp(0.75, 2.)
}

/// The system's text size, read once: Windows' *Accessibility → Text size*, GNOME's text
/// scaling factor (also set by its *Large Text* switch); 1.0 elsewhere or if it can't be read.
pub fn system_text_scale() -> f32 {
    static SCALE: OnceLock<f32> = OnceLock::new();
    *SCALE.get_or_init(|| {
        read_system_text_scale()
            .filter(|s| s.is_finite() && *s > 0.)
            .unwrap_or(1.)
    })
}

#[cfg(windows)]
fn read_system_text_scale() -> Option<f32> {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW: no console flashing up.
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Accessibility",
            "/v",
            "TextScaleFactor",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    parse_reg_dword(&String::from_utf8_lossy(&output.stdout)).map(|percent| percent as f32 / 100.)
}

#[cfg(target_os = "linux")]
fn read_system_text_scale() -> Option<f32> {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "text-scaling-factor"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read_system_text_scale() -> Option<f32> {
    None
}

/// The value of a `REG_DWORD` line from `reg query`, e.g. `TextScaleFactor REG_DWORD 0x7d`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_dword(output: &str) -> Option<u32> {
    let line = output.lines().find(|line| line.contains("REG_DWORD"))?;
    let hex = line.split_whitespace().last()?.strip_prefix("0x")?;
    u32::from_str_radix(hex, 16).ok()
}

/// Follows the system appearance in `window` while the preference is to.
pub fn follow_system(window: &mut Window) -> Subscription {
    window.observe_window_appearance(|window, cx| {
        if Config::load().theme == ThemePreference::System {
            apply(ThemePreference::System, Some(window), cx);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_reg_dword, text_scale};

    #[test]
    fn reads_the_windows_text_size() {
        let output =
            "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Accessibility\r\n    TextScaleFactor    REG_DWORD    0x7d\r\n";
        assert_eq!(parse_reg_dword(output), Some(125));
        assert_eq!(
            parse_reg_dword("ERROR: The system was unable to find the specified registry key"),
            None
        );
    }

    #[test]
    fn chosen_sizes_win_and_stay_in_range() {
        assert_eq!(text_scale(Some(130)), 1.3);
        assert_eq!(text_scale(Some(400)), 2.);
    }
}
