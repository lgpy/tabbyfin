//! Animations, and the setting that turns them off. Off sets GPUI's reduce-motion flag, which
//! GPUI's and gpui-kit's own animations follow, as do these and smooth scrolling.
//!
//! An animation plays when its element first renders, so each of these plays when what it
//! wraps appears: a page opened, an image loaded, the player's controls shown.

use std::time::{Duration, Instant};

use gpui_kit::component::Icon;
use gpui_kit::*;

use crate::config::Config;

/// How far an element rises as it fades in.
const LIFT: f32 = 12.;
const ENTER: Duration = Duration::from_millis(260);
const FADE: Duration = Duration::from_millis(220);
const POP: Duration = Duration::from_millis(420);

pub fn init(config: &Config, cx: &mut App) {
    cx.set_reduce_motion(!config.animations.unwrap_or(true));
}

pub fn set_enabled(on: bool, cx: &mut App) {
    cx.set_reduce_motion(!on);
}

/// Fades `element` in while it rises into place, starting `delay` after it appears.
pub fn enter<E: Styled + IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    delay: Duration,
) -> AnimationElement<E> {
    animate(element, id, ENTER, delay, |el, t| {
        el.opacity(t).relative().top(px(LIFT * (1. - t)))
    })
}

/// Fades `element` in.
pub fn fade_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>) -> AnimationElement<E> {
    animate(element, id, FADE, Duration::ZERO, |el, t| el.opacity(t))
}

/// Fades `element` in while it slides in from the right.
pub fn slide_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>) -> AnimationElement<E> {
    animate(element, id, ENTER, Duration::ZERO, |el, t| {
        el.opacity(t).relative().left(px(LIFT * 2. * (1. - t)))
    })
}

/// How long [`morph`] takes, or nothing with animations off.
pub fn morph_duration(cx: &App) -> Duration {
    if cx.reduce_motion() { Duration::ZERO } else { ENTER }
}

/// Moves `element` from `from` to `to` (its offset right of its place, its width, kept within
/// its parent's, and its opacity): a control opening out of another one at a different spot,
/// or closing back into it. The other one [`fade`]s alongside, over the same time.
pub fn morph<E: Styled + IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    from: (Pixels, Pixels, f32),
    to: (Pixels, Pixels, f32),
) -> AnimationElement<E> {
    animate(element, id, ENTER, Duration::ZERO, move |el, t| {
        el.relative()
            .left(from.0 + (to.0 - from.0) * t)
            .w(from.1 + (to.1 - from.1) * t)
            .max_w_full()
            .opacity(from.2 + (to.2 - from.2) * t)
    })
}

/// Fades `element` from opacity `from` to `to`, as [`morph`] moves.
pub fn fade<E: Styled + IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    from: f32,
    to: f32,
) -> AnimationElement<E> {
    animate(element, id, ENTER, Duration::ZERO, move |el, t| {
        el.opacity(from + (to - from) * t)
    })
}

/// Grows `icon` from half its size to a little past it and back: a toggle's icon filling in as
/// it turns on. Only within [`POP`] of `started`, so it doesn't play again when the page shows
/// again later; `id` is new for each start.
pub fn pop(icon: Icon, id: impl Into<ElementId>, started: Option<Instant>) -> AnyElement {
    if started.is_none_or(|at| at.elapsed() >= POP) {
        return icon.into_any_element();
    }
    let ease = ease_out_quint();
    icon.with_animation(id, Animation::new(POP), move |icon, t| {
        let t = ease(t);
        let scale = 0.5 + 0.5 * t + 0.35 * (std::f32::consts::PI * t).sin();
        icon.transform(Transformation::scale(size(scale, scale)))
    })
    .into_any_element()
}

/// Runs `animator` from 0 to 1 over `duration`, eased out, after `delay`.
fn animate<E: IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    duration: Duration,
    delay: Duration,
    animator: impl Fn(E, f32) -> E + 'static,
) -> AnimationElement<E> {
    let total = (delay + duration).as_secs_f32();
    let start = delay.as_secs_f32() / total;
    let ease = ease_out_quint();
    element.with_animation(
        id,
        Animation::new(delay + duration).with_easing(move |t| ease(((t - start) / (1. - start)).clamp(0., 1.))),
        animator,
    )
}
