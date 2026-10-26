//! The app's look: dark, light or the system's theme, text size, zoom, animations, and the
//! colours over video (`over_video`).
//!
//! On `main` this was `theme.rs`, `zoom.rs` and `motion.rs`. The zoom keys live here now, with
//! egui in charge of the theme it draws in: on Windows and macOS `ctx.set_theme` with
//! [`egui::ThemePreference::System`] follows the system's light or dark appearance as it changes.
//! On Linux egui gets no system theme from winit, so [`SystemTheme`] reads the desktop's from the
//! XDG portal and follows the portal's changes (as `main` did).

use std::collections::BTreeMap;
use std::sync::Mutex;

use egui::FontId;

use crate::config::{Config, ThemePreference};
use crate::task::Task;

/// Text sizes the settings offer, in percent.
pub const TEXT_SIZES: [u16; 5] = [90, 100, 115, 130, 150];

/// What "the system's theme" is: egui's own support where winit reports the system theme, and
/// on Linux light, which is what `main` showed when the desktop said nothing, until the
/// [`SystemTheme`] the app keeps alongside sets the desktop's.
#[cfg(not(target_os = "linux"))]
const SYSTEM_PREFERENCE: egui::ThemePreference = egui::ThemePreference::System;
#[cfg(target_os = "linux")]
const SYSTEM_PREFERENCE: egui::ThemePreference = egui::ThemePreference::Light;

/// The zoom levels the zoom keys step through, as on `main`, 1.0 being 100 %.
const ZOOM_LEVELS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.];

/// Applies the config's appearance: theme, zoom, text size and animations. Called through
/// `App::apply_appearance`, at launch and when Settings changes one of them, which also puts the
/// desktop's colour scheme back on Linux.
pub fn apply(ctx: &egui::Context, config: &Config) {
    ctx.set_theme(match config.theme {
        ThemePreference::System => SYSTEM_PREFERENCE,
        ThemePreference::Light => egui::ThemePreference::Light,
        ThemePreference::Dark => egui::ThemePreference::Dark,
    });
    // The app's zoom keys (`keys.rs`) step through [`ZOOM_LEVELS`] and save the choice; egui's
    // own would step by 10 % and forget it between launches.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    ctx.set_zoom_factor(config.ui_zoom.map_or(1., snap));
    apply_text_size(ctx, config.text_size);
    apply_animations(ctx, config.animations.unwrap_or(true));
}

/// The zoom a config asks for, snapped to a level.
pub fn zoom(config: &Config) -> f32 {
    config.ui_zoom.map_or(1., snap)
}

/// One step up (`1`) or down (`-1`) the zoom levels from `current`, saved as `ui_zoom` for the
/// next launch. Returns the zoom in effect, which is `current` at either end of the levels.
///
/// `current` is the caller's, because `ctx.zoom_factor` only changes at the start of the next
/// pass: two keys in one frame would otherwise both step from the same level.
pub fn step_zoom(ctx: &egui::Context, current: f32, direction: i32) -> f32 {
    let index = ZOOM_LEVELS.iter().position(|level| *level == current).unwrap_or(4) as i32;
    let next = (index + direction).clamp(0, ZOOM_LEVELS.len() as i32 - 1) as usize;
    choose_zoom(ctx, current, ZOOM_LEVELS[next])
}

/// Back to 100 %, from `current` (see [`step_zoom`]).
pub fn reset_zoom(ctx: &egui::Context, current: f32) -> f32 {
    choose_zoom(ctx, current, 1.)
}

/// Sets the UI zoom, which scales everything egui draws, the layout with it, and remembers it.
fn choose_zoom(ctx: &egui::Context, current: f32, zoom: f32) -> f32 {
    if current == zoom {
        return zoom;
    }
    ctx.set_zoom_factor(zoom);
    let mut config = Config::load();
    config.ui_zoom = (zoom != 1.).then_some(zoom);
    if let Err(err) = config.save() {
        tracing::warn!("couldn't save the zoom: {err}");
    }
    zoom
}

/// The level nearest a saved zoom, so stepping from it lands on the levels.
fn snap(zoom: f32) -> f32 {
    ZOOM_LEVELS
        .into_iter()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.)
}

/// The text size factor for a setting: the chosen percentage, or the system's.
pub fn text_scale(setting: Option<u16>) -> f32 {
    let scale = match setting {
        Some(percent) => f32::from(percent) / 100.,
        None => system_text_scale(),
    };
    scale.clamp(0.75, 2.)
}

/// The text styles at 100 %, which the text size setting multiplies. egui's own until the
/// restyle (R1) registers its sizes and named styles with [`set_text_styles`].
static BASE_TEXT_STYLES: Mutex<Option<BTreeMap<egui::TextStyle, FontId>>> = Mutex::new(None);

/// Sets the text styles the setting scales, at 100 %: for the restyle (R1), whose own sizes and
/// named styles (`TextStyle::Name`) scale with the setting from then on. Apply the setting after
/// (or let the next [`apply`] do it).
pub fn set_text_styles(styles: BTreeMap<egui::TextStyle, FontId>) {
    *BASE_TEXT_STYLES.lock().unwrap() = Some(styles);
}

/// Scales the app's text styles, which sizes text and the controls drawn around it. Fixed layout
/// sizes don't follow, as on `main`: the setting makes text bigger, not the window's chrome.
fn apply_text_size(ctx: &egui::Context, setting: Option<u16>) {
    let scale = text_scale(setting);
    let mut base = BASE_TEXT_STYLES.lock().unwrap();
    let base = base.get_or_insert_with(egui::style::default_text_styles);
    ctx.all_styles_mut(|style| {
        // From the 100 % sizes every time, so applying this again doesn't compound.
        for (text_style, font) in base.iter() {
            style
                .text_styles
                .insert(text_style.clone(), FontId::new(font.size * scale, font.family.clone()));
        }
    });
}

/// Off makes every widget change instant, scrolling included, as GPUI's reduce-motion flag did
/// for `main`.
fn apply_animations(ctx: &egui::Context, on: bool) {
    let animation_time = if on { egui::Style::default().animation_time } else { 0. };
    let scroll = if on {
        egui::style::ScrollAnimation::default()
    } else {
        egui::style::ScrollAnimation::none()
    };
    ctx.all_styles_mut(|style| {
        style.animation_time = animation_time;
        style.scroll_animation = scroll;
    });
}

/// The desktop's colour scheme, from the XDG desktop portal's settings: what "follow the
/// system" means on Linux, where winit reports no system theme (its X11 backend has none, its
/// Wayland one only knows what the app set). The same source `main` used.
#[cfg(target_os = "linux")]
pub struct SystemTheme {
    /// The watch. Dropping it stops the listening, which is what happens when the preference
    /// stops following the system or the app exits.
    _watch: Task<()>,
    updates: crate::task::Receiver<egui::Theme>,
    /// What the portal said last, to set again after [`apply`] (which knows only light).
    latest: Option<egui::Theme>,
}

#[cfg(target_os = "linux")]
impl SystemTheme {
    /// Starts watching the desktop's colour scheme, off the UI thread.
    pub fn start(ctx: &egui::Context) -> Self {
        let (sender, updates) = crate::task::channel(ctx);
        let watch = Task::spawn(
            ctx,
            watch_colour_scheme(sender, PORTAL_BUS, PORTAL_PATH, PORTAL_SETTINGS),
        );
        Self {
            _watch: watch,
            updates,
            latest: None,
        }
    }

    /// Applies the scheme the portal reported since the last frame. The app keeps this only
    /// while the preference is the system's, so it never overrides a chosen theme.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some(theme) = self.updates.drain().last() {
            tracing::info!(?theme, "the desktop's colour scheme");
            self.latest = Some(theme);
            ctx.set_theme(theme);
        }
    }

    /// Sets the desktop's scheme again, after [`apply`] set its light stand-in.
    pub fn reapply(&self, ctx: &egui::Context) {
        if let Some(theme) = self.latest {
            ctx.set_theme(theme);
        }
    }
}

/// Nothing to watch: egui follows the system theme itself where winit reports it.
#[cfg(not(target_os = "linux"))]
pub struct SystemTheme;

#[cfg(not(target_os = "linux"))]
impl SystemTheme {
    pub fn start(_ctx: &egui::Context) -> Self {
        Self
    }

    pub fn tick(&mut self, _ctx: &egui::Context) {}

    pub fn reapply(&self, _ctx: &egui::Context) {}
}

/// Whether the app has to follow the desktop's colour scheme itself, which is Linux (and there
/// only while the theme preference is the system's).
#[cfg(target_os = "linux")]
pub fn follows_the_desktop(config: &Config) -> bool {
    config.theme == ThemePreference::System
}

#[cfg(not(target_os = "linux"))]
pub fn follows_the_desktop(_config: &Config) -> bool {
    false
}

#[cfg(target_os = "linux")]
const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
#[cfg(target_os = "linux")]
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
#[cfg(target_os = "linux")]
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
/// The desktop's appearance, whose `color-scheme` is the light or dark preference.
const APPEARANCE: &str = "org.freedesktop.appearance";
/// The key in [`APPEARANCE`] that says which theme the desktop prefers.
const COLOR_SCHEME: &str = "color-scheme";

/// The theme a portal colour scheme asks for: 1 is dark, 2 is light. 0 (no preference), a
/// missing portal and anything else are light, as `main` showed without a signal.
fn scheme_theme(scheme: u32) -> egui::Theme {
    match scheme {
        1 => egui::Theme::Dark,
        _ => egui::Theme::Light,
    }
}

/// Reads the desktop's colour scheme and then follows the portal's changes, until the receiver
/// is gone (the app dropped the watch) or the connection does. A portal that can't be read
/// (light stays) is still followed, in case it says something later.
#[cfg(target_os = "linux")]
async fn watch_colour_scheme(sender: crate::task::Sender<egui::Theme>, bus: &str, path: &str, interface: &str) {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!("no session bus, so no desktop colour scheme to follow: {err}");
            return;
        }
    };
    let proxy = match zbus::Proxy::new(&connection, bus, path, interface).await {
        Ok(proxy) => proxy,
        Err(err) => {
            tracing::info!("no desktop portal settings to follow: {err}");
            return;
        }
    };
    // Listening before reading, so a change between the two isn't missed.
    let changes = proxy
        .receive_signal("SettingChanged")
        .await
        .map_err(|err| tracing::info!("couldn't follow the desktop colour scheme's changes: {err}"))
        .ok();
    if let Some(theme) = read_colour_scheme(&proxy).await
        && sender.send(theme).is_err()
    {
        return;
    }
    let Some(mut changes) = changes else {
        return;
    };
    use futures_util::StreamExt as _;
    while let Some(message) = changes.next().await {
        let Some(theme) = colour_scheme_change(&message) else {
            continue;
        };
        if sender.send(theme).is_err() {
            break;
        }
    }
}

/// The desktop's colour scheme as the portal has it now. `ReadOne` arrived in
/// xdg-desktop-portal 1.15; older ones (Ubuntu 22.04 has 1.14) only have `Read`, whose value
/// comes wrapped in one more variant.
#[cfg(target_os = "linux")]
async fn read_colour_scheme(proxy: &zbus::Proxy<'_>) -> Option<egui::Theme> {
    let args = &(APPEARANCE, COLOR_SCHEME);
    let value: zbus::zvariant::OwnedValue = match proxy.call("ReadOne", args).await {
        Ok(value) => value,
        Err(_) => proxy
            .call("Read", args)
            .await
            .map_err(|err| tracing::info!("couldn't read the desktop's colour scheme: {err}"))
            .ok()?,
    };
    Some(scheme_theme(scheme_value(&value)))
}

/// The colour scheme in a portal `SettingChanged` signal, if it's about the appearance.
#[cfg(target_os = "linux")]
fn colour_scheme_change(message: &zbus::Message) -> Option<egui::Theme> {
    let (namespace, key, value): (String, String, zbus::zvariant::OwnedValue) = message.body().deserialize().ok()?;
    (namespace == APPEARANCE && key == COLOR_SCHEME).then(|| scheme_theme(scheme_value(&value)))
}

/// The `u32` in a portal settings value, unwrapping the variant `Read` adds; 0 (no preference)
/// when it's another type.
#[cfg(target_os = "linux")]
fn scheme_value(value: &zbus::zvariant::Value) -> u32 {
    match value {
        zbus::zvariant::Value::U32(scheme) => *scheme,
        zbus::zvariant::Value::Value(inner) => scheme_value(inner),
        _ => 0,
    }
}

/// The system's text size as last read.
static SYSTEM_SCALE: Mutex<Option<f32>> = Mutex::new(None);

/// The system's text size: Windows' *Accessibility → Text size*, GNOME's text scaling factor
/// (also set by its *Large Text* switch), KDE's general font size against its default 10 pt;
/// 1.0 elsewhere, before the first read or if it can't be read. Reading it starts processes, so
/// the read itself happens in [`read_system_text_scale`], off the UI thread.
pub fn system_text_scale() -> f32 {
    SYSTEM_SCALE.lock().unwrap().unwrap_or(1.)
}

/// Reads the system text size again without blocking the UI thread (`reg`, `gsettings` and
/// `kdeglobals` are slow). Systems don't tell apps about a change, so this runs at launch and
/// when the window gets the focus back; pass the result to [`note_system_text_scale`].
pub fn refresh_system_text_scale(ctx: &egui::Context) -> Task<f32> {
    Task::spawn(ctx, async {
        tokio::task::spawn_blocking(read_valid_system_text_scale)
            .await
            .unwrap_or(1.)
    })
}

/// Whether the system text size has been read yet.
pub fn system_text_scale_read() -> bool {
    SYSTEM_SCALE.lock().unwrap().is_some()
}

/// Remembers a freshly read system text size, and says whether it changed (from 100 %, which is
/// what was in use before the first read).
pub fn note_system_text_scale(scale: f32) -> bool {
    SYSTEM_SCALE.lock().unwrap().replace(scale).unwrap_or(1.) != scale
}

fn read_valid_system_text_scale() -> f32 {
    read_system_text_scale()
        .filter(|s| s.is_finite() && *s > 0.)
        .unwrap_or(1.)
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

#[cfg(test)]
mod tests {
    use super::{
        apply_animations, apply_text_size, parse_kde_font_size, parse_reg_dword, scheme_theme, set_text_styles, snap,
        text_scale,
    };

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

    #[test]
    fn a_saved_zoom_lands_on_a_level() {
        assert_eq!(snap(1.), 1.);
        assert_eq!(snap(0.72), 0.75);
        assert_eq!(snap(1.4), 1.5);
        // Well above the last level.
        assert_eq!(snap(4.), 2.);
    }

    #[test]
    fn animations_off_makes_every_change_instant() {
        let ctx = egui::Context::default();
        apply_animations(&ctx, false);
        let style = ctx.global_style();
        assert_eq!(style.animation_time, 0.);
        assert_eq!(style.scroll_animation, egui::style::ScrollAnimation::none());
        // Turning them back on restores what egui's own style had.
        apply_animations(&ctx, true);
        let style = ctx.global_style();
        assert_eq!(style.animation_time, egui::Style::default().animation_time);
        assert_eq!(style.scroll_animation, egui::style::ScrollAnimation::default());
    }

    /// One test for every text style, since the 100 % table is the app's and a test that
    /// replaced it would be seen by others.
    #[test]
    fn the_text_size_scales_every_text_style_once() {
        let ctx = egui::Context::default();
        let size = |ctx: &egui::Context, text_style: &egui::TextStyle| ctx.global_style().text_styles[text_style].size;
        let body = egui::TextStyle::Body;
        let normal = size(&ctx, &body);
        apply_text_size(&ctx, Some(150));
        assert_eq!(size(&ctx, &body), normal * 1.5);
        // The setting is applied again whenever another one changes.
        apply_text_size(&ctx, Some(150));
        assert_eq!(size(&ctx, &body), normal * 1.5);
        apply_text_size(&ctx, Some(100));
        assert_eq!(size(&ctx, &body), normal);

        // The restyle's own sizes and named styles scale too.
        let title = egui::TextStyle::Name("title".into());
        let mut styles = egui::style::default_text_styles();
        styles.insert(title.clone(), egui::FontId::proportional(20.));
        styles.insert(body.clone(), egui::FontId::proportional(15.));
        set_text_styles(styles);
        apply_text_size(&ctx, Some(130));
        assert_eq!(size(&ctx, &body), 19.5);
        assert_eq!(size(&ctx, &title), 26.);
    }

    /// Stand-ins for xdg-desktop-portal's settings interface, served on the session bus under
    /// names of their own so a real portal isn't touched.
    #[cfg(target_os = "linux")]
    mod portal {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{Duration, Instant};

        use zbus::zvariant::{OwnedValue, Value};

        use super::super::{APPEARANCE, COLOR_SCHEME, watch_colour_scheme};

        const PATH: &str = "/org/freedesktop/portal/desktop";

        /// Which reads the fake portal answers.
        #[derive(Clone, Copy)]
        enum Version {
            /// 1.15 and later: `ReadOne`.
            Current,
            /// 1.14 and earlier: only `Read`, its value in a variant of its own.
            Old,
            /// Answers no read, as a portal whose backend is missing.
            Mute,
        }

        struct FakePortal {
            scheme: Arc<AtomicU32>,
            version: Version,
        }

        impl FakePortal {
            fn value(&self, namespace: &str, key: &str) -> zbus::fdo::Result<u32> {
                if namespace != APPEARANCE || key != COLOR_SCHEME {
                    return Err(zbus::fdo::Error::InvalidArgs(format!("{namespace} {key}")));
                }
                Ok(self.scheme.load(Ordering::SeqCst))
            }
        }

        #[zbus::interface(name = "org.freedesktop.portal.Settings")]
        impl FakePortal {
            async fn read_one(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
                match self.version {
                    Version::Current => Ok(OwnedValue::from(self.value(&namespace, &key)?)),
                    Version::Old | Version::Mute => Err(zbus::fdo::Error::UnknownMethod("ReadOne".into())),
                }
            }

            async fn read(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
                match self.version {
                    Version::Current | Version::Old => {
                        let wrapped = Value::new(Value::from(self.value(&namespace, &key)?));
                        Ok(OwnedValue::try_from(wrapped).expect("a plain value"))
                    }
                    Version::Mute => Err(zbus::fdo::Error::Failed("no backend".into())),
                }
            }

            #[zbus(signal)]
            async fn setting_changed(
                emitter: &zbus::object_server::SignalEmitter<'_>,
                namespace: &str,
                key: &str,
                value: OwnedValue,
            ) -> zbus::Result<()>;
        }

        /// Waits for the next colour scheme the watch sends; while waiting, `poke` runs every
        /// 100 ms (to send a change the watch may not be listening for yet).
        fn next(updates: &mut crate::task::Receiver<egui::Theme>, mut poke: impl FnMut()) -> egui::Theme {
            let start = Instant::now();
            loop {
                poke();
                for _ in 0..5 {
                    if let Some(theme) = updates.drain().last() {
                        return theme;
                    }
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "the portal's colour scheme never arrived"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }

        /// Serves a fake portal of `version` starting at dark, and checks what the watch reads
        /// at first (if anything) and that it follows a change to light.
        fn follows(name: &'static str, version: Version, reads: bool) {
            let ctx = egui::Context::default();
            let (sender, mut updates) = crate::task::channel(&ctx);
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let scheme = Arc::new(AtomicU32::new(1));
            let offering = FakePortal {
                scheme: scheme.clone(),
                version,
            };
            let server = crate::runtime::start(async move {
                let connection = zbus::connection::Builder::session()
                    .expect("a session bus")
                    .name(name)
                    .expect("a bus name")
                    .serve_at(PATH, offering)
                    .expect("the fake portal")
                    .build()
                    .await
                    .expect("the fake portal's connection");
                ready_tx.send(connection).expect("the test is still there");
                std::future::pending::<()>().await;
            });
            let connection = ready_rx.recv_timeout(Duration::from_secs(5)).expect("the fake portal");
            let watch = crate::runtime::start(watch_colour_scheme(
                sender,
                name,
                PATH,
                "org.freedesktop.portal.Settings",
            ));

            if reads {
                assert_eq!(next(&mut updates, || {}), egui::Theme::Dark);
            }
            // The desktop switches to light: the portal changes its value and says so.
            scheme.store(2, Ordering::SeqCst);
            let say_light = || {
                let connection = connection.clone();
                crate::runtime::start(async move {
                    let emitter = zbus::object_server::SignalEmitter::new(&connection, PATH).expect("an emitter");
                    FakePortal::setting_changed(&emitter, APPEARANCE, COLOR_SCHEME, OwnedValue::from(2u32))
                        .await
                        .expect("the signal");
                });
            };
            assert_eq!(next(&mut updates, say_light), egui::Theme::Light);

            watch.abort();
            server.abort();
        }

        fn have_a_session_bus() -> bool {
            let there = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
            if !there {
                eprintln!("no session bus here: the portal watch isn't tested");
            }
            there
        }

        #[test]
        fn the_desktops_colour_scheme_is_read_and_followed() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakePortal", Version::Current, true);
            }
        }

        #[test]
        fn an_older_portal_is_read_with_read() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakeOldPortal", Version::Old, true);
            }
        }

        #[test]
        fn a_portal_that_cant_be_read_is_still_followed() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakeMutePortal", Version::Mute, false);
            }
        }
    }

    #[test]
    fn a_portal_colour_scheme_maps_to_a_theme() {
        assert_eq!(scheme_theme(1), egui::Theme::Dark);
        assert_eq!(scheme_theme(2), egui::Theme::Light);
        // No preference, and anything the portal grows later.
        assert_eq!(scheme_theme(0), egui::Theme::Light);
        assert_eq!(scheme_theme(7), egui::Theme::Light);
    }
}

/// Colours over the video. The player stays dark whatever the theme, since its controls sit on
/// the picture; these are its tokens, so views don't scatter black and white literals.
pub mod over_video {
    use egui::Color32;

    /// Behind the picture, and its letterbox bars.
    pub const BACKDROP: Color32 = Color32::BLACK;

    /// Where the top bar's gradient starts (it fades to [`CLEAR`]).
    pub const SCRIM: Color32 = Color32::from_black_alpha(179);

    /// Where the control bar's gradient starts, a little darker for the seek bar's sake.
    pub const SCRIM_STRONG: Color32 = Color32::from_black_alpha(204);

    pub const CLEAR: Color32 = Color32::from_black_alpha(0);

    /// Cards and labels floating on the picture: the up-next card, the seek preview's time.
    pub const SURFACE: Color32 = Color32::from_black_alpha(204);

    /// Small labels in the bars (Transcoding, the speed).
    pub const SURFACE_SUBTLE: Color32 = Color32::from_black_alpha(128);

    pub const FOREGROUND: Color32 = Color32::WHITE;

    /// White at 70 %.
    pub const MUTED_FOREGROUND: Color32 = Color32::from_rgba_premultiplied(179, 179, 179, 179);

    /// Chapter marks on the seek bar and the preview's outline: white at 60 %.
    pub const MARK: Color32 = Color32::from_rgba_premultiplied(153, 153, 153, 153);

    /// Behind a card's progress bar, over its image. Dark against the fill, with a light outline
    /// ([`TRACK_OUTLINE`]) so the bar's full length still shows on dark images.
    pub const TRACK: Color32 = Color32::from_black_alpha(153);

    /// White at 50 %.
    pub const TRACK_OUTLINE: Color32 = Color32::from_rgba_premultiplied(128, 128, 128, 128);
}
