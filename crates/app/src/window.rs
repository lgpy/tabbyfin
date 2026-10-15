//! Placement of the main window: restored from the config at launch, saved after it changes.

use std::rc::Rc;

use gpui_kit::*;

use crate::config::{Config, WindowGeometry};

/// The window size used when nothing usable is saved.
const DEFAULT_WIDTH: f32 = 1280.0;
const DEFAULT_HEIGHT: f32 = 800.0;
/// A saved side shorter than this is treated as a corrupt config, not a window.
const MIN_SIDE: f32 = 100.0;
/// The smallest window the layout works in: below it the player's control bar no longer fits.
/// Restored bounds are grown to it too, since the platform only applies it to resizing.
pub const MIN_WIDTH: f32 = 900.0;
pub const MIN_HEIGHT: f32 = 500.0;

pub fn min_size() -> Size<Pixels> {
    size(px(MIN_WIDTH), px(MIN_HEIGHT))
}

/// Where to open the main window.
pub struct Launch {
    /// The display the window was last on. `None` lets the platform pick, which is the main
    /// display.
    pub display_id: Option<DisplayId>,
    /// The placement the window opens with, which is also the starting point for remembering
    /// later changes.
    pub placement: Placement,
}

impl Launch {
    /// The bounds for `WindowOptions`; GPUI maximizes a window opened with `Maximized`.
    pub fn window_bounds(&self) -> WindowBounds {
        if self.placement.maximized {
            WindowBounds::Maximized(self.placement.bounds)
        } else {
            WindowBounds::Windowed(self.placement.bounds)
        }
    }
}

/// Where to open the main window: the saved spot and its display when the window's centre still
/// lands on a connected display, otherwise the default centred window. The display is passed
/// along because Windows only places a window on a display it is told about and assumes the
/// primary monitor otherwise, which would pull a window saved on a second monitor back to it.
pub fn launch_placement(config: &Config, cx: &App) -> Launch {
    let displays = cx.displays();
    match config
        .window_bounds
        .and_then(|saved| placed_bounds(saved, &display_bounds(&displays)))
    {
        Some((bounds, index)) => Launch {
            display_id: Some(displays[index].id()),
            placement: Placement {
                bounds,
                maximized: config.window_maximized,
            },
        },
        None => Launch {
            display_id: None,
            placement: Placement {
                bounds: Bounds::centered(None, size(px(DEFAULT_WIDTH), px(DEFAULT_HEIGHT)), cx),
                maximized: false,
            },
        },
    }
}

/// What the window looks like at one moment, as far as saving cares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowState {
    pub bounds: Bounds<Pixels>,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// The placement remembered between launches: the last windowed bounds, and whether the window
/// was maximized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub bounds: Bounds<Pixels>,
    pub maximized: bool,
}

/// Folds one window state into the remembered placement. Fullscreen keeps the previous placement,
/// because playback fullscreen is transient. A maximized window keeps its last windowed bounds
/// and changes only the flag: Windows and Wayland report the restore bounds while maximized, but
/// X11 reports the maximized rect.
pub fn remember(previous: Placement, state: WindowState) -> Placement {
    if state.fullscreen {
        return previous;
    }
    if state.maximized {
        return Placement {
            bounds: previous.bounds,
            maximized: true,
        };
    }
    Placement {
        bounds: state.bounds,
        maximized: false,
    }
}

/// Saves the placement, so the next launch reopens the window there. Bounds that land on no
/// display are ignored, so a window dragged off every screen doesn't come back off-screen.
pub fn save_placement(placement: Placement, cx: &App) {
    if !lands_on_display(&placement.bounds, &display_bounds(&cx.displays())) {
        return;
    }
    let mut config = Config::load();
    config.window_bounds = Some(geometry_of(placement.bounds));
    config.window_maximized = placement.maximized;
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
}

fn geometry_of(bounds: Bounds<Pixels>) -> WindowGeometry {
    WindowGeometry {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
    }
}

/// The saved bounds, when they are sane and the window's centre lands on a connected display,
/// together with that display's index. The centre decides because that is what Windows matches
/// against a monitor; a window straddling two displays comes back centred instead.
fn placed_bounds(saved: WindowGeometry, displays: &[Bounds<Pixels>]) -> Option<(Bounds<Pixels>, usize)> {
    if !usable(&saved) {
        return None;
    }
    let bounds = Bounds::new(
        point(px(saved.x), px(saved.y)),
        size(px(saved.width.max(MIN_WIDTH)), px(saved.height.max(MIN_HEIGHT))),
    );
    let center = bounds.center();
    let display = displays.iter().position(|display| display.contains(&center))?;
    Some((bounds, display))
}

fn usable(saved: &WindowGeometry) -> bool {
    [saved.x, saved.y, saved.width, saved.height]
        .iter()
        .all(|value| value.is_finite())
        && saved.width >= MIN_SIDE
        && saved.height >= MIN_SIDE
}

/// Whether the window would be visible on one of these displays.
fn lands_on_display(bounds: &Bounds<Pixels>, displays: &[Bounds<Pixels>]) -> bool {
    displays.iter().any(|display| display.intersects(bounds))
}

fn display_bounds(displays: &[Rc<dyn PlatformDisplay>]) -> Vec<Bounds<Pixels>> {
    displays.iter().map(|display| display.bounds()).collect()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, Pixels, point, px, size};

    use super::{Placement, WindowState, lands_on_display, placed_bounds, remember};
    use crate::config::WindowGeometry;

    /// A rectangle to stand in for a display.
    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    fn saved(x: f32, y: f32, width: f32, height: f32) -> WindowGeometry {
        WindowGeometry { x, y, width, height }
    }

    fn windowed(x: f32, y: f32, width: f32, height: f32) -> WindowState {
        WindowState {
            bounds: rect(x, y, width, height),
            maximized: false,
            fullscreen: false,
        }
    }

    fn placement(x: f32, y: f32, width: f32, height: f32, maximized: bool) -> Placement {
        Placement {
            bounds: rect(x, y, width, height),
            maximized,
        }
    }

    #[test]
    fn a_windowed_state_is_remembered() {
        let previous = placement(0., 0., 1280., 800., true);
        let remembered = remember(previous, windowed(10., 20., 800., 600.));
        assert!(!remembered.maximized);
        assert_eq!(remembered.bounds.origin.x.as_f32(), 10.);
        assert_eq!(remembered.bounds.size.width.as_f32(), 800.);
    }

    #[test]
    fn maximizing_keeps_the_last_windowed_bounds() {
        let previous = placement(10., 20., 800., 600., false);
        let state = WindowState {
            bounds: rect(0., 0., 1920., 1080.),
            maximized: true,
            fullscreen: false,
        };
        let remembered = remember(previous, state);
        assert!(remembered.maximized);
        assert_eq!(remembered.bounds.size.width.as_f32(), 800.);
    }

    #[test]
    fn unmaximizing_updates_the_bounds() {
        let previous = placement(10., 20., 800., 600., true);
        let remembered = remember(previous, windowed(30., 40., 900., 700.));
        assert!(!remembered.maximized);
        assert_eq!(remembered.bounds.origin.x.as_f32(), 30.);
    }

    #[test]
    fn fullscreen_leaves_the_placement_alone() {
        let previous = placement(10., 20., 800., 600., false);
        let state = WindowState {
            bounds: rect(0., 0., 1920., 1080.),
            maximized: false,
            fullscreen: true,
        };
        assert_eq!(remember(previous, state), previous);
    }

    #[test]
    fn a_window_still_on_a_display_keeps_its_place() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let (bounds, display) = placed_bounds(saved(120., 60., 1280., 800.), &displays).expect("kept");
        assert_eq!(display, 0);
        assert_eq!(bounds.origin.x.as_f32(), 120.);
        assert_eq!(bounds.origin.y.as_f32(), 60.);
        assert_eq!(bounds.size.width.as_f32(), 1280.);
        assert_eq!(bounds.size.height.as_f32(), 800.);
    }

    #[test]
    fn a_window_on_the_second_display_keeps_its_place() {
        let displays = [rect(0., 0., 1920., 1080.), rect(1920., 0., 1920., 1080.)];
        let (bounds, display) = placed_bounds(saved(2400., 100., 1280., 800.), &displays).expect("kept");
        assert_eq!(display, 1);
        assert_eq!(bounds.origin.x.as_f32(), 2400.);
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
        assert_eq!(bounds.size.width.as_f32(), 900.);
        assert_eq!(bounds.size.height.as_f32(), 500.);
        assert_eq!(bounds.origin.x.as_f32(), 100.);
    }

    #[test]
    fn a_window_with_a_nonsense_size_falls_back() {
        let displays = [rect(0., 0., 1920., 1080.)];
        assert!(placed_bounds(saved(0., 0., f32::NAN, 800.), &displays).is_none());
        assert!(placed_bounds(saved(0., 0., 20., 800.), &displays).is_none());
    }

    #[test]
    fn landing_on_a_display_needs_overlap() {
        let displays = [rect(0., 0., 1920., 1080.)];
        assert!(lands_on_display(&rect(1900., 1000., 1280., 800.), &displays));
        assert!(!lands_on_display(&rect(1920., 0., 1280., 800.), &displays));
    }
}
