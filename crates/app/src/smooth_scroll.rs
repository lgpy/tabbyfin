//! Smooth scrolling. GPUI moves a scroll container by a whole wheel step at once;
//! [`smooth_wheel`] takes that jump back and glides to the same place over a few frames, and
//! [`glide_by`] glides a container by a given distance (a shelf's arrow buttons). Trackpads
//! send pixel deltas that are already smooth, and are left alone; with reduced motion, glides
//! jump.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// How quickly a glide closes the distance: about 63% of what's left every `TAU`.
const TAU: Duration = Duration::from_millis(70);
/// Close enough to the target to stop.
const SETTLE: Pixels = px(0.5);

/// A glide in progress.
struct Glide {
    axis: Axis,
    target: Pixels,
    /// The offset the glide set last, to notice when something else scrolls the container
    /// (the keyboard cursor, a drag), which ends the glide.
    last_set: Pixels,
    last_tick: Instant,
}

thread_local! {
    /// Glides in progress, by container. The app has one window, and a container's key is
    /// unique within it.
    static GLIDES: RefCell<HashMap<ElementId, Glide>> = RefCell::new(HashMap::new());
}

fn along(point: Point<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => point.y,
        Axis::Horizontal => point.x,
    }
}

fn set_along(point: &mut Point<Pixels>, axis: Axis, value: Pixels) {
    match axis {
        Axis::Vertical => point.y = value,
        Axis::Horizontal => point.x = value,
    }
}

/// A scroll-wheel listener for a vertically scrolling container that tracks `handle`. `key`
/// names the container; it must be unique among the app's scroll containers.
///
/// It relies on GPUI running the container's own wheel handler before this one (bubble
/// listeners run last-registered first, and an element's scroll handler is registered after
/// its `on_scroll_wheel` listeners), so the jump is already applied and can be taken back.
pub fn smooth_wheel(
    key: impl Into<ElementId>,
    handle: &ScrollHandle,
) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static {
    let key = key.into();
    let handle = handle.clone();
    move |event, window, cx| {
        if event.delta.precise() || cx.reduce_motion() {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height());
        // What GPUI applied to a container that only scrolls vertically.
        let applied = if delta.y.is_zero() { delta.x } else { delta.y };
        if applied.is_zero() {
            return;
        }
        let mut offset = handle.offset();
        offset.y -= applied;
        handle.set_offset(offset);
        // Scrolling down is a negative offset; the wheel's delta is already signed that way.
        start(key.clone(), &handle, Axis::Vertical, applied, window);
    }
}

/// Glides the container that tracks `handle` by `distance` along `axis` (negative is towards
/// the end, as offsets go), from where a glide already under way is heading. Jumps with reduced
/// motion.
pub fn glide_by(
    key: impl Into<ElementId>,
    handle: &ScrollHandle,
    axis: Axis,
    distance: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    if cx.reduce_motion() {
        let max = along(handle.max_offset(), axis);
        let mut offset = handle.offset();
        let target = (along(offset, axis) + distance).clamp(-max, px(0.));
        set_along(&mut offset, axis, target);
        handle.set_offset(offset);
        window.refresh();
        return;
    }
    start(key.into(), handle, axis, distance, window);
}

/// Aims the container's glide `distance` further, starting one if none is under way.
fn start(key: ElementId, handle: &ScrollHandle, axis: Axis, distance: Pixels, window: &mut Window) {
    let offset = along(handle.offset(), axis);
    let max = along(handle.max_offset(), axis);
    let new = GLIDES.with(|glides| {
        let mut glides = glides.borrow_mut();
        match glides.get_mut(&key) {
            // Another step while gliding: aim further.
            Some(glide) if glide.axis == axis && glide.last_set == offset => {
                glide.target = (glide.target + distance).clamp(-max, px(0.));
                false
            }
            _ => {
                glides.insert(
                    key.clone(),
                    Glide {
                        axis,
                        target: (offset + distance).clamp(-max, px(0.)),
                        last_set: offset,
                        last_tick: Instant::now(),
                    },
                );
                true
            }
        }
    });
    if new {
        let handle = handle.clone();
        window.on_next_frame(move |window, cx| tick(key, handle, window, cx));
    }
}

/// Moves the glide one frame closer to its target, and asks for another frame until it's there.
fn tick(key: ElementId, handle: ScrollHandle, window: &mut Window, cx: &mut App) {
    let mut offset = handle.offset();
    let more = GLIDES.with(|glides| {
        let mut glides = glides.borrow_mut();
        let Some(glide) = glides.get_mut(&key) else {
            return false;
        };
        if along(offset, glide.axis) != glide.last_set || cx.reduce_motion() {
            glides.remove(&key);
            return false;
        }
        let now = Instant::now();
        let t = 1. - (-(now - glide.last_tick).as_secs_f32() / TAU.as_secs_f32()).exp();
        let mut next = glide.last_set + (glide.target - glide.last_set) * t;
        let done = (glide.target - next).abs() < SETTLE;
        if done {
            next = glide.target;
        }
        set_along(&mut offset, glide.axis, next);
        glide.last_set = next;
        glide.last_tick = now;
        if done {
            glides.remove(&key);
        }
        !done
    });
    if offset != handle.offset() {
        handle.set_offset(offset);
        window.refresh();
    }
    if more {
        window.on_next_frame(move |window, cx| tick(key, handle, window, cx));
    }
}
