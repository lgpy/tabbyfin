//! Animations, and the setting that turns them off. Off sets GPUI's reduce-motion flag, which
//! GPUI's and gpui-kit's own animations follow, as do these and smooth scrolling.
//!
//! An animation plays when its element first renders, so each of these plays when what it
//! wraps appears: a page opened, an image loaded, the player's controls shown.

use std::time::Duration;

use gpui_kit::*;

use crate::config::Config;

/// How far an element rises as it fades in.
const LIFT: f32 = 12.;
const ENTER: Duration = Duration::from_millis(260);
const FADE: Duration = Duration::from_millis(220);

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
