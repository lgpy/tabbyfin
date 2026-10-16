//! The light or dark theme, from the user's choice or the system's appearance, and the text
//! size.

use std::sync::Mutex;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::config::{Config, ThemePreference};

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
    let theme = Theme::global_mut(cx);
    // Rounder than gpui-kit's 6 and 8.
    theme.radius = px(10.);
    theme.radius_lg = px(12.);
    // The light theme's grey for secondary text is 4.35:1 on its light-grey fills (selected
    // rows, chips, menus), under the 4.5:1 WCAG AA asks of small text; one step darker
    // (neutral-600) is 7.2:1. The dark theme's passes as it is (6:1 or more).
    if !theme.mode.is_dark() {
        theme.muted_foreground = rgb(0x525252).into();
    }
    crate::zoom::apply(cx);
}

/// Applies a changed text size setting.
pub fn apply_text_size(cx: &mut App) {
    crate::zoom::apply(cx);
}

/// The text size factor for a setting: the chosen percentage, or the system's.
pub fn text_scale(setting: Option<u16>) -> f32 {
    let scale = match setting {
        Some(percent) => f32::from(percent) / 100.,
        None => system_text_scale(),
    };
    scale.clamp(0.75, 2.)
}

/// The system's text size as last read (`recheck_system_text_size`).
static SYSTEM_SCALE: Mutex<Option<f32>> = Mutex::new(None);

/// The system's text size: Windows' *Accessibility → Text size*, GNOME's text scaling factor
/// (also set by its *Large Text* switch), KDE's general font size against its default 10 pt;
/// 1.0 elsewhere or if it can't be read. Read at launch, then again whenever the window
/// becomes active.
pub fn system_text_scale() -> f32 {
    *SYSTEM_SCALE
        .lock()
        .unwrap()
        .get_or_insert_with(read_valid_system_text_scale)
}

fn read_valid_system_text_scale() -> f32 {
    read_system_text_scale()
        .filter(|s| s.is_finite() && *s > 0.)
        .unwrap_or(1.)
}

/// Follows changes to the system text size while the setting follows the system. Systems
/// don't tell apps reliably, so it's read again whenever `window` becomes active: after
/// changing it in the system settings, switching back to the app applies it.
pub fn follow_system_text_size<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    cx.observe_window_activation(window, |_, window, cx| {
        if !window.is_window_active() || Config::load().text_size.is_some() {
            return;
        }
        // Reading it starts a process (`reg`, `gsettings`); keep that off the UI thread.
        let read = cx.background_spawn(async { read_valid_system_text_scale() });
        cx.spawn(async move |_, cx| {
            let scale = read.await;
            let changed = SYSTEM_SCALE.lock().unwrap().replace(scale) != Some(scale);
            if changed {
                tracing::info!(scale, "system text size changed");
                cx.update(|cx| {
                    apply_text_size(cx);
                    cx.refresh_windows();
                });
            }
        })
        .detach();
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
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.split(':').any(|d| d == "KDE"));
    if kde {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
        let kdeglobals = std::fs::read_to_string(config.join("kdeglobals")).ok();
        // No `font` entry: KDE's default size, so 100%.
        return Some(
            kdeglobals
                .as_deref()
                .and_then(parse_kde_font_size)
                .unwrap_or(KDE_DEFAULT_POINTS)
                / KDE_DEFAULT_POINTS,
        );
    }
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

/// KDE's default general font size (Noto Sans 10), which counts as 100%.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const KDE_DEFAULT_POINTS: f32 = 10.;

/// The point size of the general font in `kdeglobals`: `font=Noto Sans,11,-1,5,…` in the
/// `[General]` group.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_kde_font_size(kdeglobals: &str) -> Option<f32> {
    let mut general = false;
    for line in kdeglobals.lines().map(str::trim) {
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general && let Some(value) = line.strip_prefix("font=") {
            return value
                .split(',')
                .nth(1)?
                .trim()
                .parse()
                .ok()
                .filter(|points: &f32| *points > 0.);
        }
    }
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
    use super::{parse_kde_font_size, parse_reg_dword, text_scale};

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

    #[test]
    fn reads_the_kde_font_size() {
        let kdeglobals = "[General]\nColorScheme=BreezeDark\nfont=Noto Sans,12,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\n[WM]\nfont=Noto Sans,9,-1,5,700,0,0,0,0,0\n";
        assert_eq!(parse_kde_font_size(kdeglobals), Some(12.));
        // The general font only, not the window title's.
        assert_eq!(parse_kde_font_size("[WM]\nfont=Noto Sans,9,-1,5\n"), None);
        assert_eq!(parse_kde_font_size("[General]\nfont=Inter,10.5,-1,5\n"), Some(10.5));
    }
}

/// Colours over the video. The player stays dark whatever the theme, since its controls sit on
/// the picture; these are its tokens, so views don't scatter black and white literals.
pub mod over_video {
    use gpui_kit::{Hsla, black, white};

    /// Behind the picture, and its letterbox bars.
    pub fn backdrop() -> Hsla {
        black()
    }

    /// Where the top bar's gradient starts (it fades to [`clear`]).
    pub fn scrim() -> Hsla {
        black().opacity(0.7)
    }

    /// Where the control bar's gradient starts, a little darker for the seek bar's sake.
    pub fn scrim_strong() -> Hsla {
        black().opacity(0.8)
    }

    pub fn clear() -> Hsla {
        black().opacity(0.)
    }

    /// Cards and labels floating on the picture: the up-next card, the seek preview's time.
    pub fn surface() -> Hsla {
        black().opacity(0.8)
    }

    /// Small labels in the bars (Transcoding, the speed).
    pub fn surface_subtle() -> Hsla {
        black().opacity(0.5)
    }

    pub fn foreground() -> Hsla {
        white()
    }

    pub fn muted_foreground() -> Hsla {
        white().opacity(0.7)
    }

    /// Chapter marks on the seek bar and the preview's outline.
    pub fn mark() -> Hsla {
        white().opacity(0.6)
    }

    /// Behind a card's progress bar, over its image. Dark against the fill, with a light outline
    /// (`track_outline`) so the bar's full length still shows on dark images.
    pub fn track() -> Hsla {
        black().opacity(0.6)
    }

    pub fn track_outline() -> Hsla {
        white().opacity(0.5)
    }
}
