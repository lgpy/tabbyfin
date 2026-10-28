//! The main window as it opens: title, icon, the app id desktops know it by, its size, and the
//! placement saved from the last run.
//!
//! The placement is kept the way eframe's own window storage keeps it. The frame's top-left is
//! in physical pixels together with the scale factor it was measured at, because on Windows
//! winit converts a position given in points with the scale factor of the monitor the window is
//! created on, which needn't be the one it was saved on; the content size stays in OS points,
//! which the UI zoom doesn't touch. A maximized window keeps its last windowed size and changes
//! only the flag, and fullscreen playback leaves the placement alone, since it's transient.
//!
//! Whether a saved spot is still on a display is settled before the window opens on Windows,
//! where the app places its own window and the OS can be asked ([`placed_bounds`]). A spot on no
//! monitor there means the window opens centred. Elsewhere the window manager or AppKit keeps a
//! window on screen, and Wayland doesn't let an app place its window at all, so the spot is used
//! as saved.

use std::time::Duration;

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::config::{Config, WindowGeometry};
use crate::task::Deadline;

/// The window size used when nothing usable is saved.
const DEFAULT_SIZE: Vec2 = vec2(1280.0, 800.0);
/// A saved side shorter than this is treated as a corrupt config, not a window.
const MIN_SIDE: f32 = 100.0;
/// The smallest window the layout works in: below it the player's control bar and an item
/// page's title line no longer fit. Restored bounds are grown to it too, since the platform
/// only applies it to resizing.
pub const MIN_SIZE: Vec2 = vec2(800.0, 500.0);
/// How long after the last change the placement is saved: dragging produces a stream of
/// changes and only the end of it matters.
const SAVE_AFTER: Duration = Duration::from_millis(500);
/// Positions at or beyond this aren't places: X11 reports about -32768 for a window that's still
/// being mapped, and Windows parks a minimized window at -32000.
const UNPLACED: f32 = -32_000.0;

/// Makes the process DPI-aware the way winit does (per monitor, version 2), before anything
/// asks Windows about monitors. winit only does it when the event loop starts, and until then
/// Windows hands out monitor rectangles scaled to 96 DPI, which the saved physical position
/// can't be compared with. Doing it again later (winit, or a manifest first) changes nothing.
#[cfg(windows)]
pub fn become_dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        SetProcessDpiAwarenessContext,
    };
    unsafe {
        // Version 2 needs Windows 10 1703; 1607 has only the first.
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
        }
    }
}

/// The window to open: at the saved placement, or at the default size when there's none (the
/// platform centres it then: `NativeOptions::centered`).
pub fn viewport(launch: Option<Launch>) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Tabbyfin")
        .with_app_id(app_id())
        .with_min_inner_size(MIN_SIZE);
    let viewport = match launch {
        // The window is created at zoom 1, so points here are OS points. egui-winit sets the
        // position before it maximizes, which is the order Windows needs (moving a maximized
        // window restores it).
        Some(Launch { placed, .. }) => viewport
            .with_position(placed.position / placed.scale)
            .with_inner_size(placed.size)
            .with_maximized(placed.maximized),
        None => viewport.with_inner_size(DEFAULT_SIZE),
    };
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

/// The placement remembered between launches.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    /// The frame's top-left, in physical pixels.
    position: Pos2,
    /// The content area's size, in OS points.
    size: Vec2,
    /// The scale factor `position` was measured at.
    scale: f32,
    maximized: bool,
}

/// What the window looks like at one moment, as far as saving cares.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WindowState {
    /// Where the window is, at the scale factor of the monitor it's on.
    placed: Placed,
    fullscreen: bool,
}

/// The saved placement the window opens with.
#[derive(Debug, Clone, Copy)]
pub struct Launch {
    placed: Placed,
    /// Whether `placed.position` really is in physical pixels and can be set again exactly
    /// once the window exists. A config from before the scale factor was saved has the
    /// position in points, kept as "physical at scale 1", which is the same spot.
    #[cfg_attr(not(windows), allow(dead_code))]
    physical: bool,
}

/// The placement from the config, when it's sane (and, on Windows, still on a monitor), or
/// nothing: the window opens at its default size, centred.
pub fn launch(config: &Config) -> Option<Launch> {
    let saved = config.window_bounds.filter(sane)?;
    let scale = saved.scale.unwrap_or(1.);
    if !(scale.is_finite() && scale > 0.) {
        return None;
    }
    #[cfg(windows)]
    if !on_a_monitor(saved) {
        return None;
    }
    Some(Launch {
        placed: Placed {
            position: pos2(saved.x, saved.y),
            size: vec2(saved.width.max(MIN_SIZE.x), saved.height.max(MIN_SIZE.y)),
            scale,
            maximized: config.window_maximized,
        },
        physical: saved.scale.is_some(),
    })
}

/// The saved placement as a physical rectangle, when it's sane and its centre lands on one of
/// `monitors` (physical rectangles too), together with that monitor's index. The centre decides
/// because that is what Windows matches a window against; a window straddling two monitors
/// comes back centred instead. Only Windows has the monitors before the window opens.
#[cfg_attr(not(windows), allow(dead_code))]
fn placed_bounds(saved: WindowGeometry, monitors: &[Rect]) -> Option<(Rect, usize)> {
    if !sane(&saved) {
        return None;
    }
    // A config from before the scale factor is in points; at 100 % they are the same numbers.
    let scale = saved.scale.unwrap_or(1.);
    let size = vec2(saved.width.max(MIN_SIZE.x), saved.height.max(MIN_SIZE.y)) * scale;
    let rect = Rect::from_min_size(pos2(saved.x, saved.y), size);
    let centre = rect.center();
    let monitor = monitors.iter().position(|monitor| monitor.contains(centre))?;
    Some((rect, monitor))
}

/// Whether the saved placement's centre still lands on a monitor, asked of the OS before the
/// window opens. Needs [`become_dpi_aware`] first, for physical monitor rectangles.
#[cfg(windows)]
fn on_a_monitor(saved: WindowGeometry) -> bool {
    match placed_bounds(saved, &monitors()) {
        Some((_, monitor)) => {
            tracing::info!(monitor, "the saved window placement is on a monitor");
            true
        }
        None => {
            tracing::info!("the saved window placement is on no monitor; centring the window");
            false
        }
    }
}

/// The monitors' rectangles, in physical pixels.
#[cfg(windows)]
fn monitors() -> Vec<Rect> {
    use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    use windows::core::BOOL;

    unsafe extern "system" fn collect(monitor: HMONITOR, _dc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
        // The callback only borrows the list for the length of `EnumDisplayMonitors`.
        let monitors = unsafe { &mut *(data.0 as *mut Vec<Rect>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let rect = info.rcMonitor;
            monitors.push(Rect::from_min_max(
                pos2(rect.left as f32, rect.top as f32),
                pos2(rect.right as f32, rect.bottom as f32),
            ));
        }
        TRUE
    }

    let mut monitors = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&raw mut monitors as isize));
    }
    monitors
}

fn sane(saved: &WindowGeometry) -> bool {
    [saved.x, saved.y, saved.width, saved.height]
        .iter()
        .all(|value| value.is_finite())
        && saved.width >= MIN_SIDE
        && saved.height >= MIN_SIDE
}

/// Folds one window state into the remembered placement. Fullscreen keeps the previous
/// placement, because playback fullscreen is transient. A maximized window keeps its last
/// windowed bounds and changes only the flag: Windows and Wayland report the restore bounds
/// while maximized, but X11 reports the maximized rect.
fn remember(previous: Placed, state: WindowState) -> Placed {
    if state.fullscreen {
        return previous;
    }
    if state.placed.maximized {
        return Placed {
            maximized: true,
            ..previous
        };
    }
    state.placed
}

/// Whether the window didn't open where it was saved (Windows): winit converts the launch
/// position with the scale factor of the monitor the window is created on, so a spot saved on a
/// monitor of another scale lands elsewhere. A maximized window's position is its monitor's,
/// not the saved one, so for it a scale factor other than the saved one is the sign.
#[cfg_attr(not(windows), allow(dead_code))]
fn misplaced(saved: Placed, now: Placed) -> bool {
    if now.maximized {
        (now.scale - saved.scale).abs() > 0.01
    } else {
        let off = now.position - saved.position;
        off.x.abs() > 1. || off.y.abs() > 1.
    }
}

/// Keeps the saved placement in step with the window: saving the size, position and maximized
/// state shortly after they stop changing, and on close.
pub struct Placement {
    /// The placement last remembered: the launch's, then what the window reports.
    remembered: Option<Placed>,
    /// The launch, until the first frame with the window placed has finished it.
    launch: Option<Launch>,
    /// The zoom this frame's window rects were measured with (see [`Placement::measure`]).
    measured_zoom: f32,
    /// The UI zoom the last tick saw, and the frame it was in.
    zoom: f32,
    frame: u64,
    /// The remembered placement changed since it was last saved.
    unsaved: bool,
    save: Deadline,
}

impl Placement {
    pub fn new(launch: Option<Launch>) -> Self {
        Self {
            remembered: launch.map(|launch| launch.placed),
            launch,
            // egui starts unzoomed, and the window's first rects are measured before the
            // config's zoom applies.
            measured_zoom: 1.,
            zoom: 1.,
            frame: 0,
            unsaved: false,
            save: Deadline::default(),
        }
    }

    /// Each frame, before the UI. Every move or resize of the window gives a frame, whose
    /// input already has the window's new rects.
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.observe(ctx);
        if self.save.fire(ctx) {
            self.save_now();
        }
    }

    fn observe(&mut self, ctx: &egui::Context) {
        let Some(state) = self.measure(ctx) else {
            return;
        };
        if let Some(launch) = self.launch.take()
            && finish_launch(ctx, launch, state)
        {
            // The window is about to change, and its next report gives another frame.
            return;
        }
        let previous = self.remembered.unwrap_or(state.placed);
        let placement = remember(previous, state);
        if self.remembered != Some(placement) {
            self.remembered = Some(placement);
            self.unsaved = true;
            self.save.set(SAVE_AFTER);
        }
    }

    /// The window as this frame's input reports it, once the platform has placed it (Wayland
    /// reports no position at all, and a minimized window has none).
    fn measure(&mut self, ctx: &egui::Context) -> Option<WindowState> {
        // The window's rects are in egui points, physical pixels over the UI zoom and the
        // monitor's scale. egui-winit measures them before the frame starts, with the zoom the
        // previous frame left, and a newly chosen zoom only applies once this frame's first
        // pass begins; so the zoom to undo is the one the last tick of the previous frame saw.
        let frame = ctx.cumulative_frame_nr();
        if frame != self.frame {
            self.frame = frame;
            self.measured_zoom = self.zoom;
        }
        self.zoom = ctx.zoom_factor();
        let zoom = self.measured_zoom;
        let state = ctx.input(|input| {
            let viewport = input.viewport();
            let outer = viewport.outer_rect?;
            let inner = viewport.inner_rect?;
            let scale = viewport.native_pixels_per_point?;
            Some(WindowState {
                placed: Placed {
                    position: outer.min * (zoom * scale),
                    size: inner.size() * zoom,
                    scale,
                    maximized: viewport.maximized.unwrap_or(false),
                },
                fullscreen: viewport.fullscreen.unwrap_or(false),
            })
        })?;
        let position = state.placed.position;
        (position.x > UNPLACED && position.y > UNPLACED).then_some(state)
    }

    /// On close: saves a change that's still waiting.
    pub fn save_now(&mut self) {
        self.save.cancel();
        if !std::mem::take(&mut self.unsaved) {
            return;
        }
        let Some(placed) = self.remembered else {
            return;
        };
        let mut config = Config::load();
        config.window_bounds = Some(WindowGeometry {
            x: placed.position.x,
            y: placed.position.y,
            width: placed.size.x,
            height: placed.size.y,
            scale: Some(placed.scale),
        });
        config.window_maximized = placed.maximized;
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
    }
}

/// Puts right what the platform didn't do with the launch placement, once the window is up,
/// and says whether it asked the window for anything. On Windows a spot saved on a monitor of
/// another scale is set again in physical pixels (as eframe does with its own saved
/// placement), which also restores a maximized window, so it's maximized again after. X11
/// window managers ignore a maximize asked for before the window is mapped, so a window that
/// should be maximized and isn't is asked once more. This frame isn't remembered, since the
/// window is about to change; from the next one on the window's own report counts, so a
/// maximize that lands within the save delay keeps the flag, and on a platform that never
/// maximizes (a bare X server) the window is saved as it really is.
fn finish_launch(ctx: &egui::Context, launch: Launch, state: WindowState) -> bool {
    #[cfg(windows)]
    let moved = launch.physical && misplaced(launch.placed, state.placed);
    #[cfg(not(windows))]
    let moved = false;
    if moved {
        tracing::info!("moving the window to its saved spot, in physical pixels");
        let pixels_per_point = ctx.pixels_per_point();
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
            launch.placed.position / pixels_per_point,
        ));
    }
    let maximize = launch.placed.maximized && (moved || !state.placed.maximized);
    if maximize {
        tracing::debug!("asking the window to maximize");
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    }
    moved || maximize
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, Vec2, pos2, vec2};

    use super::{Launch, MIN_SIZE, Placed, Placement, WindowState, launch, misplaced, placed_bounds, remember};
    use crate::config::{Config, WindowGeometry};

    /// A rectangle to stand in for a display or a window.
    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(width, height))
    }

    fn saved(x: f32, y: f32, width: f32, height: f32) -> WindowGeometry {
        WindowGeometry {
            x,
            y,
            width,
            height,
            scale: Some(1.),
        }
    }

    fn placement(x: f32, y: f32, width: f32, height: f32, maximized: bool) -> Placed {
        Placed {
            position: pos2(x, y),
            size: vec2(width, height),
            scale: 1.,
            maximized,
        }
    }

    fn windowed(x: f32, y: f32, width: f32, height: f32) -> WindowState {
        WindowState {
            placed: placement(x, y, width, height, false),
            fullscreen: false,
        }
    }

    #[test]
    fn a_windowed_state_is_remembered() {
        let previous = placement(0., 0., 1280., 800., true);
        let remembered = remember(previous, windowed(10., 20., 800., 600.));
        assert!(!remembered.maximized);
        assert_eq!(remembered.position, pos2(10., 20.));
        assert_eq!(remembered.size, vec2(800., 600.));
    }

    #[test]
    fn maximizing_keeps_the_last_windowed_bounds() {
        let previous = placement(10., 20., 800., 600., false);
        let state = WindowState {
            placed: placement(0., 0., 1920., 1080., true),
            fullscreen: false,
        };
        let remembered = remember(previous, state);
        assert!(remembered.maximized);
        assert_eq!(remembered.size, vec2(800., 600.));
    }

    #[test]
    fn unmaximizing_updates_the_bounds() {
        let previous = placement(10., 20., 800., 600., true);
        let remembered = remember(previous, windowed(30., 40., 900., 700.));
        assert!(!remembered.maximized);
        assert_eq!(remembered.position, pos2(30., 40.));
    }

    #[test]
    fn fullscreen_leaves_the_placement_alone() {
        let previous = placement(10., 20., 800., 600., false);
        let state = WindowState {
            fullscreen: true,
            ..windowed(0., 0., 1920., 1080.)
        };
        assert_eq!(remember(previous, state), previous);
    }

    #[test]
    fn a_window_still_on_a_display_keeps_its_place() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let (bounds, display) = placed_bounds(saved(120., 60., 1280., 800.), &displays).expect("kept");
        assert_eq!(display, 0);
        assert_eq!(bounds.min, pos2(120., 60.));
        assert_eq!(bounds.size(), vec2(1280., 800.));
    }

    #[test]
    fn a_window_on_the_second_display_keeps_its_place() {
        let displays = [rect(0., 0., 1920., 1080.), rect(1920., 0., 1920., 1080.)];
        let (bounds, display) = placed_bounds(saved(2400., 100., 1280., 800.), &displays).expect("kept");
        assert_eq!(display, 1);
        assert_eq!(bounds.min, pos2(2400., 100.));
    }

    #[test]
    fn a_window_on_a_removed_display_falls_back() {
        let displays = [rect(0., 0., 1920., 1080.)];
        assert!(placed_bounds(saved(2400., 100., 1280., 800.), &displays).is_none());
    }

    #[test]
    fn a_window_whose_centre_lies_between_displays_falls_back() {
        // Monitors of different heights leave a gap at the bottom right.
        let displays = [rect(0., 0., 1920., 1080.), rect(1920., -400., 1920., 1080.)];
        assert!(placed_bounds(saved(1500., 900., 1280., 800.), &displays).is_none());
        let (_, display) = placed_bounds(saved(1600., 100., 1280., 800.), &displays).expect("kept");
        assert_eq!(display, 1);
    }

    #[test]
    fn a_window_smaller_than_the_minimum_grows_to_it() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let (bounds, _) = placed_bounds(saved(100., 100., 700., 400.), &displays).expect("kept");
        assert_eq!(bounds.size(), MIN_SIZE);
        assert_eq!(bounds.min, pos2(100., 100.));
        let config = Config {
            window_bounds: Some(saved(100., 100., 700., 400.)),
            ..Default::default()
        };
        let launch = launch(&config).expect("kept");
        assert_eq!(launch.placed.size, MIN_SIZE);
        assert_eq!(launch.placed.position, pos2(100., 100.));
    }

    #[test]
    fn a_window_with_a_nonsense_size_falls_back() {
        let displays = [rect(0., 0., 1920., 1080.)];
        assert!(placed_bounds(saved(0., 0., f32::NAN, 800.), &displays).is_none());
        assert!(placed_bounds(saved(0., 0., 20., 800.), &displays).is_none());
        // And it doesn't open there either, nor at a nonsense scale.
        let mut zero_scale = saved(0., 0., 1280., 800.);
        zero_scale.scale = Some(0.);
        for nonsense in [saved(0., 0., f32::NAN, 800.), saved(0., 0., 20., 800.), zero_scale] {
            let config = Config {
                window_bounds: Some(nonsense),
                ..Default::default()
            };
            assert!(launch(&config).is_none());
        }
    }

    #[test]
    fn the_monitor_check_works_in_physical_pixels() {
        // A spot on a 150 % monitor: 2400 physical pixels in, not 2400 points.
        let displays = [rect(0., 0., 3840., 2160.)];
        let mut at_150 = saved(2400., 100., 1280., 800.);
        at_150.scale = Some(1.5);
        assert!(placed_bounds(at_150, &displays).is_some());
        // The same numbers on a 100 % monitor are off its right edge.
        let displays = [rect(0., 0., 1920., 1080.)];
        assert!(placed_bounds(at_150, &displays).is_none());
    }

    #[test]
    fn a_saved_placement_is_where_the_window_opens() {
        let config = Config {
            window_bounds: Some(saved(120., 60., 1280., 800.)),
            window_maximized: true,
            ..Default::default()
        };
        let launch = launch(&config).expect("kept");
        assert_eq!(launch.placed.position, pos2(120., 60.));
        assert_eq!(launch.placed.size, vec2(1280., 800.));
        assert!(launch.placed.maximized);
        assert!(launch.physical);
    }

    #[test]
    fn a_config_from_before_the_scale_factor_opens_in_points() {
        let config = Config {
            window_bounds: Some(WindowGeometry {
                scale: None,
                ..saved(2400., 100., 1280., 800.)
            }),
            ..Default::default()
        };
        let launch = launch(&config).expect("kept");
        // Points, kept as physical pixels at scale 1: the window opens at 2400 points, and
        // nothing sets it again in physical pixels.
        assert_eq!(launch.placed.position / launch.placed.scale, pos2(2400., 100.));
        assert!(!launch.physical);
    }

    #[test]
    fn nothing_saved_means_a_centred_window() {
        assert!(launch(&Config::default()).is_none());
    }

    #[test]
    fn a_window_that_opened_elsewhere_is_misplaced() {
        let saved = Placed {
            position: pos2(3000., 100.),
            scale: 1.,
            ..placement(0., 0., 1280., 800., false)
        };
        // Within a pixel (rounding on the way through points) is where it was saved.
        assert!(!misplaced(
            saved,
            Placed {
                position: pos2(3000.6, 99.5),
                ..saved
            }
        ));
        // Converted with a 150 % monitor's scale.
        assert!(misplaced(
            saved,
            Placed {
                position: pos2(4500., 150.),
                ..saved
            }
        ));
        // Maximized: on a monitor of the saved scale, or not.
        let maximized = Placed {
            position: pos2(2992., -8.),
            maximized: true,
            ..saved
        };
        assert!(!misplaced(saved, maximized));
        assert!(misplaced(
            saved,
            Placed {
                scale: 1.5,
                ..maximized
            }
        ));
    }

    /// A stand-in for egui-winit: reports a window at `position` (physical pixels) with a
    /// content area of `size` OS points, measured with the zoom the context has before the
    /// frame starts, as `update_viewport_info` does. Returns the placement commands sent.
    fn frame(
        ctx: &egui::Context,
        placement: &mut Placement,
        position: Pos2,
        size: Vec2,
        maximized: bool,
    ) -> Vec<egui::ViewportCommand> {
        let scale = 1.5;
        let pixels_per_point = ctx.zoom_factor() * scale;
        let mut input = egui::RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.native_pixels_per_point = Some(scale);
        info.outer_rect = Some(Rect::from_min_size(
            position / pixels_per_point,
            (size * scale + vec2(0., 30.)) / pixels_per_point,
        ));
        info.inner_rect = Some(Rect::from_min_size(
            (position + vec2(0., 30.)) / pixels_per_point,
            size * scale / pixels_per_point,
        ));
        info.maximized = Some(maximized);
        info.fullscreen = Some(false);
        let mut output = ctx.run_pass(input, |ctx| placement.observe(ctx));
        // No renderer here to take the font atlas.
        output.textures_delta.clear();
        output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.commands.clone())
            .unwrap_or_default()
            .into_iter()
            // egui sends its own (the theme) on the first pass.
            .filter(|command| {
                matches!(
                    command,
                    egui::ViewportCommand::OuterPosition(_) | egui::ViewportCommand::Maximized(_)
                )
            })
            .collect()
    }

    #[test]
    fn the_ui_zoom_changes_nothing_about_the_placement() {
        let ctx = egui::Context::default();
        let mut placement = Placement::new(None);
        // The config's zoom is chosen before the first frame (`App::new`).
        ctx.set_zoom_factor(1.25);
        let (position, size) = (pos2(300., 120.), vec2(1100., 650.));
        frame(&ctx, &mut placement, position, size, false);
        let expected = Placed {
            position,
            size,
            scale: 1.5,
            maximized: false,
        };
        assert_eq!(placement.remembered, Some(expected));
        // A zoom key: the frame after is measured with the old zoom, the one after that with
        // the new one, and neither is a change.
        ctx.set_zoom_factor(1.5);
        placement.unsaved = false;
        for _ in 0..3 {
            frame(&ctx, &mut placement, position, size, false);
            assert_eq!(placement.remembered, Some(expected));
        }
        assert!(!placement.unsaved);
    }

    #[test]
    fn the_last_report_of_a_move_is_remembered() {
        let ctx = egui::Context::default();
        let mut placement = Placement::new(None);
        let size = vec2(1100., 650.);
        frame(&ctx, &mut placement, pos2(300., 120.), size, false);
        frame(&ctx, &mut placement, pos2(310., 125.), size, false);
        assert_eq!(
            placement.remembered.map(|placed| placed.position),
            Some(pos2(310., 125.))
        );
        assert!(placement.unsaved);
    }

    #[test]
    fn an_unplaced_window_is_not_a_move() {
        let ctx = egui::Context::default();
        let mut placement = Placement::new(None);
        // X11 reports this while a window is being mapped.
        frame(&ctx, &mut placement, pos2(-32730., -32709.), vec2(1280., 800.), false);
        assert_eq!(placement.remembered, None);
    }

    #[test]
    fn a_maximized_launch_the_platform_ignored_is_asked_again_once() {
        let ctx = egui::Context::default();
        let config = Config {
            window_bounds: Some(saved(120., 60., 1280., 800.)),
            window_maximized: true,
            ..Default::default()
        };
        let launch: Launch = launch(&config).expect("kept");
        let mut placement = Placement::new(Some(launch));
        let commands = frame(&ctx, &mut placement, pos2(120., 60.), vec2(1280., 800.), false);
        assert_eq!(commands, [egui::ViewportCommand::Maximized(true)]);
        // That frame isn't remembered: the window is about to change.
        assert_eq!(placement.remembered, Some(launch.placed));
        assert!(!placement.unsaved);
        // Asked once only; after that the window's word counts, here a platform that never
        // maximizes.
        let commands = frame(&ctx, &mut placement, pos2(120., 60.), vec2(1280., 800.), false);
        assert!(commands.is_empty());
        assert_eq!(placement.remembered.map(|placed| placed.maximized), Some(false));
    }

    #[test]
    fn a_maximized_launch_that_worked_is_left_alone() {
        let ctx = egui::Context::default();
        let config = Config {
            window_bounds: Some(WindowGeometry {
                scale: Some(1.5),
                ..saved(120., 60., 1280., 800.)
            }),
            window_maximized: true,
            ..Default::default()
        };
        let launch = launch(&config).expect("kept");
        let mut placement = Placement::new(Some(launch));
        let commands = frame(&ctx, &mut placement, pos2(-12., -12.), vec2(2560., 1500.), true);
        assert!(commands.is_empty());
        // Maximized keeps the windowed bounds it opened with.
        assert_eq!(placement.remembered, Some(launch.placed));
    }
}
