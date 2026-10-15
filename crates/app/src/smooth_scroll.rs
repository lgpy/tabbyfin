//! Smooth mouse-wheel scrolling. GPUI moves a scroll container by a whole wheel step at once;
//! [`smooth_wheel`] takes that jump back and glides to the same place over a few frames.
//! Trackpads send pixel deltas that are already smooth, and are left alone, as is everything
//! when the system asks for reduced motion.

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
    target: Pixels,
    /// The offset the glide set last, to notice when something else scrolls the container
    /// (the keyboard cursor, a drag), which ends the glide.
    last_set: Pixels,
    last_tick: Instant,
}

thread_local! {
    /// Glides in progress, by container. The app has one window, and a container's key is
    /// unique within it.
    static GLIDES: RefCell<HashMap<&'static str, Glide>> = RefCell::new(HashMap::new());
}

/// A scroll-wheel listener for a vertically scrolling container that tracks `handle`. `key`
/// names the container; it must be unique among the app's scroll containers.
///
/// It relies on GPUI running the container's own wheel handler before this one (bubble
/// listeners run last-registered first, and an element's scroll handler is registered after
/// its `on_scroll_wheel` listeners), so the jump is already applied and can be taken back.
pub fn smooth_wheel(
    key: &'static str,
    handle: &ScrollHandle,
) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static {
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

        let max = handle.max_offset().y;
        let start = GLIDES.with(|glides| {
            let mut glides = glides.borrow_mut();
            match glides.get_mut(key) {
                // Another step while gliding: aim further.
                Some(glide) if glide.last_set == offset.y => {
                    glide.target = (glide.target + applied).clamp(-max, px(0.));
                    false
                }
                _ => {
                    glides.insert(
                        key,
                        Glide {
                            target: (offset.y + applied).clamp(-max, px(0.)),
                            last_set: offset.y,
                            last_tick: Instant::now(),
                        },
                    );
                    true
                }
            }
        });
        if start {
            let handle = handle.clone();
            window.on_next_frame(move |window, cx| tick(key, handle, window, cx));
        }
    }
}

/// Moves the glide one frame closer to its target, and asks for another frame until it's there.
fn tick(key: &'static str, handle: ScrollHandle, window: &mut Window, cx: &mut App) {
    let mut offset = handle.offset();
    let more = GLIDES.with(|glides| {
        let mut glides = glides.borrow_mut();
        let Some(glide) = glides.get_mut(key) else {
            return false;
        };
        if offset.y != glide.last_set || cx.reduce_motion() {
            glides.remove(key);
            return false;
        }
        let now = Instant::now();
        let t = 1. - (-(now - glide.last_tick).as_secs_f32() / TAU.as_secs_f32()).exp();
        let mut next = glide.last_set + (glide.target - glide.last_set) * t;
        let done = (glide.target - next).abs() < SETTLE;
        if done {
            next = glide.target;
        }
        offset.y = next;
        glide.last_set = next;
        glide.last_tick = now;
        if done {
            glides.remove(key);
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
