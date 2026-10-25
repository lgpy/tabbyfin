//! The player page: video, controls, seek bar, menus, the skip button and up-next card. U10
//! fills it; the steps its keys, wheel and remote commands use are here.

/// Seconds per seek key press (and remote rewind or fast-forward).
pub const SEEK_STEP: f64 = 10.;
/// Volume percent per key press, wheel notch or remote volume step.
pub const VOLUME_STEP: f64 = 5.;
/// Wheel lines per volume step. X11 and Windows normalise a wheel notch to three lines, so
/// one notch is one step; a trackpad's pixel deltas are turned into lines first.
pub const SCROLL_LINES_PER_STEP: f32 = 3.;
/// Trackpad pixels per wheel line, for platforms that send pixel deltas (macOS).
pub const PIXELS_PER_LINE: f32 = 30.;
/// The most volume steps one wheel event makes. Windows scales a notch by the user's lines per
/// notch, and its "one screen at a time" setting turns a notch into billions of lines.
pub const MAX_STEPS_PER_SCROLL: f32 = 4.;

/// Turns a wheel delta (in lines, positive is scrolling up) into whole volume steps, keeping
/// the leftover lines for the next event. One event makes at most `MAX_STEPS_PER_SCROLL`.
pub fn scroll_steps(pending: f32, lines: f32) -> (f32, i32) {
    let limit = MAX_STEPS_PER_SCROLL * SCROLL_LINES_PER_STEP;
    let total = (pending + lines).clamp(-limit, limit);
    let steps = (total / SCROLL_LINES_PER_STEP).trunc() as i32;
    (total - steps as f32 * SCROLL_LINES_PER_STEP, steps)
}

#[cfg(test)]
mod tests {
    use super::{MAX_STEPS_PER_SCROLL, SCROLL_LINES_PER_STEP, scroll_steps};

    #[test]
    fn a_wheel_notch_is_one_volume_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP);
        assert_eq!(steps, 1);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_fast_flick_keeps_every_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP * 4.);
        assert_eq!(steps, 4);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn small_deltas_add_up() {
        let (pending, steps) = scroll_steps(0., 1.);
        assert_eq!((pending, steps), (1., 0));
        let (pending, steps) = scroll_steps(pending, 2.);
        assert_eq!((pending, steps), (0., 1));
    }

    #[test]
    fn scrolling_down_is_negative() {
        let (pending, steps) = scroll_steps(0., -2. * SCROLL_LINES_PER_STEP);
        assert_eq!(steps, -2);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_huge_delta_is_capped() {
        let (pending, steps) = scroll_steps(0., 4_294_967_295.);
        assert_eq!(steps, MAX_STEPS_PER_SCROLL as i32);
        assert_eq!(pending, 0.);
        let (_, steps) = scroll_steps(0., -4_294_967_295.);
        assert_eq!(steps, -(MAX_STEPS_PER_SCROLL as i32));
    }

    #[test]
    fn scrolling_back_cancels() {
        let (pending, steps) = scroll_steps(0., -1.5);
        assert_eq!(steps, 0);
        let (_, steps) = scroll_steps(pending, 1.5);
        assert_eq!(steps, 0);
    }
}
