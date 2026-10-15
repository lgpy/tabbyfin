//! UI zoom, like a web browser's: Ctrl+= and Ctrl+- step through [`LEVELS`], Ctrl+0 resets.
//! The zoom sets the rem size (the theme's font size), so text and gpui-kit's controls scale;
//! the app's own fixed sizes use [`dp`] so they scale with it.

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::config::Config;

actions!(zoom, [ZoomIn, ZoomOut, ResetZoom]);

/// The rem size at 100%: the default theme's font size.
const BASE_REM: f32 = 16.;
const LEVELS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.];

struct UiZoom(f32);

impl Global for UiZoom {}

/// A length given in pixels at 100% zoom, scaling with the zoom.
pub fn dp(pixels: f32) -> Rems {
    rems(pixels / BASE_REM)
}

/// [`dp`] in pixels, for layout arithmetic.
pub fn dp_px(pixels: f32, window: &Window) -> Pixels {
    window.rem_size() * (pixels / BASE_REM)
}

pub fn init(config: &Config, cx: &mut App) {
    let zoom = config.ui_zoom.map_or(1., snap);
    cx.set_global(UiZoom(zoom));
    let modifier = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-="), ZoomIn, None),
        KeyBinding::new(&format!("{modifier}-+"), ZoomIn, None),
        KeyBinding::new(&format!("{modifier}-shift-="), ZoomIn, None),
        KeyBinding::new(&format!("{modifier}--"), ZoomOut, None),
        KeyBinding::new(&format!("{modifier}-0"), ResetZoom, None),
    ]);
    cx.on_action(|_: &ZoomIn, cx| step(1, cx));
    cx.on_action(|_: &ZoomOut, cx| step(-1, cx));
    cx.on_action(|_: &ResetZoom, cx| set(1., cx));
    apply(cx);
}

/// Sets the rem size for the current zoom. Called again after every theme change, which
/// resets the theme's font size.
pub fn apply(cx: &mut App) {
    let zoom = cx.try_global::<UiZoom>().map_or(1., |z| z.0);
    Theme::global_mut(cx).font_size = px(BASE_REM * zoom);
}

fn step(direction: i32, cx: &mut App) {
    let current = cx.global::<UiZoom>().0;
    let index = LEVELS.iter().position(|level| *level == current).unwrap_or(4) as i32;
    let next = (index + direction).clamp(0, LEVELS.len() as i32 - 1) as usize;
    set(LEVELS[next], cx);
}

fn set(zoom: f32, cx: &mut App) {
    if cx.global::<UiZoom>().0 == zoom {
        return;
    }
    cx.set_global(UiZoom(zoom));
    apply(cx);
    cx.refresh_windows();
    let mut config = Config::load();
    config.ui_zoom = (zoom != 1.).then_some(zoom);
    if let Err(err) = config.save() {
        tracing::warn!("couldn't save the zoom: {err}");
    }
}

/// The level nearest a saved zoom, so stepping from it lands on the levels.
fn snap(zoom: f32) -> f32 {
    LEVELS
        .into_iter()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.)
}
