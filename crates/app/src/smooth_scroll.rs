//! The scroll feel: `main`'s `smooth_scroll.rs`, as much of it as egui doesn't already have.
//!
//! egui eases a mouse wheel's whole step over a few frames itself (`WheelState`: an exponential
//! approach, about 63 % of what's left every 43 ms) and leaves a trackpad's pixel deltas alone.
//! That is what `main`'s `smooth_wheel` did — it took GPUI's jump back and glided to the same
//! place, 63 % every 70 ms — so the pages' scroll areas keep egui's smoothing. The animations
//! setting is where the two differ: egui smooths a wheel whatever the setting says, where
//! `main`'s reduced motion scrolled the step at once. [`instant_wheel`] gives the step to the
//! frame it arrived in while the setting is off.
//!
//! What egui has no counterpart for is the shelves' glide (`main`'s `glide_by`): their arrows
//! jumped a page of cards. [`Glide`] is that glide's maths, used by `widgets/shelf.rs`: an
//! exponential approach to a target that a second click while it moves aims further.

use egui::{Event, InputOptions, MouseWheelSource, MouseWheelUnit, RawInput, TouchPhase, Vec2};

/// How quickly a glide closes the distance: about 63 % of what's left every `TAU`, as on `main`.
pub const TAU: f32 = 0.07;
/// Close enough to the target to stop, as on `main`.
pub const SETTLE: f32 = 0.5;
/// The most points one of [`instant_wheel`]'s events carries: egui counts a point delta shorter
/// than 8 as already smooth, and adds it to the frame's scroll in one go.
const SMOOTH_STEP: f32 = 7.;

/// A glide in progress: an exponential approach to `target`, as `main`'s `Glide` did.
///
/// egui's offsets grow as the content scrolls down or right, where GPUI's went negative; the
/// maths is the same otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    /// Where the glide is heading.
    pub target: f32,
    /// The offset the glide set last, to notice when something else scrolled the container
    /// (the keyboard cursor, a drag), which ends the glide.
    pub last_set: f32,
    /// When [`last_set`](Self::last_set) was set, in `egui::InputState::time` seconds.
    pub last_tick: f64,
}

impl Glide {
    /// A glide from `offset` towards `offset + distance`, kept within `[0, max]`.
    pub fn new(offset: f32, distance: f32, max: f32, now: f64) -> Self {
        Self {
            target: (offset + distance).clamp(0., max.max(0.)),
            last_set: offset,
            last_tick: now,
        }
    }

    /// Aims `distance` further from where the glide is heading, as `main`'s `start` did for a
    /// second click while one was under way.
    pub fn aim(&mut self, distance: f32, max: f32) {
        self.target = (self.target + distance).clamp(0., max.max(0.));
    }

    /// The offset this frame, and whether it reached the target (which it snaps to, as `main`
    /// snapped within [`SETTLE`]). Meant to be called once a frame until it says it's done.
    pub fn step(&mut self, now: f64) -> (f32, bool) {
        let dt = (now - self.last_tick).max(0.) as f32;
        let t = 1. - (-dt / TAU).exp();
        let mut next = self.last_set + (self.target - self.last_set) * t;
        let done = (self.target - next).abs() < SETTLE;
        if done {
            next = self.target;
        }
        self.last_set = next;
        self.last_tick = now;
        (next, done)
    }
}

/// Hands a wheel step to the frame it arrives in, as `main` scrolled with animations off (its
/// reduced motion). egui smooths a wheel over frames whatever the setting says, but an event it
/// counts as already smooth — a trackpad's, or a point delta shorter than 8 — joins the frame's
/// scroll as it is, so a step is split into those. A step's total distance is unchanged.
pub fn instant_wheel(input: &mut RawInput, options: &InputOptions, viewport_height: f32) {
    let mut events = Vec::with_capacity(input.events.len());
    for event in input.events.drain(..) {
        let Event::MouseWheel {
            unit,
            delta,
            phase,
            source,
            modifiers,
        } = &event
        else {
            events.push(event);
            continue;
        };
        // egui already passes a trackpad's deltas through; a ctrl/⌘ wheel is a zoom.
        let smooth = matches!(source, MouseWheelSource::Trackpad | MouseWheelSource::Momentum)
            || (*unit == MouseWheelUnit::Point && delta.length() < 8.);
        let ends = matches!(phase, TouchPhase::End | TouchPhase::Cancel);
        if smooth || ends || *delta == Vec2::ZERO || modifiers.matches_any(options.zoom_modifier) {
            events.push(event);
            continue;
        }
        let points = match unit {
            MouseWheelUnit::Point => *delta,
            MouseWheelUnit::Line => *delta * options.line_scroll_speed,
            MouseWheelUnit::Page => *delta * viewport_height,
        };
        let chunks = (points.length() / SMOOTH_STEP).ceil().max(1.) as usize;
        let step = points / chunks as f32;
        for _ in 0..chunks {
            events.push(Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: step,
                phase: TouchPhase::Move,
                source: MouseWheelSource::Unknown,
                modifiers: *modifiers,
            });
        }
    }
    input.events = events;
}

#[cfg(test)]
mod tests {
    use egui::{Event, InputOptions, Modifiers, MouseWheelSource, MouseWheelUnit, RawInput, TouchPhase, vec2};

    use super::{Glide, SETTLE, TAU, instant_wheel};

    #[test]
    fn a_glide_closes_the_distance_exponentially() {
        let mut glide = Glide::new(0., 100., 1000., 0.);
        let (at_tau, done) = glide.step(f64::from(TAU));
        assert!(!done);
        assert!((at_tau - 100. * (1. - (-1f32).exp())).abs() < 0.01, "{at_tau}");
        // At the same distance again after another `TAU`: another 63 % of what's left.
        let (at_two, _) = glide.step(f64::from(TAU) * 2.);
        assert!((at_two - 100. * (1. - (-2f32).exp())).abs() < 0.05, "{at_two}");
    }

    #[test]
    fn a_glide_is_frame_rate_independent() {
        // One second split in three, and the same second in one step.
        let mut in_parts = Glide::new(0., 100., 1000., 0.);
        let mut at = 0.;
        for part in 1..=3 {
            at = in_parts.step(f64::from(TAU) * f64::from(part) / 3.).0;
        }
        let (in_one, _) = Glide::new(0., 100., 1000., 0.).step(f64::from(TAU));
        assert!((at - in_one).abs() < 0.001, "{at} vs {in_one}");
    }

    #[test]
    fn a_glide_settles_on_its_target_and_stops() {
        let mut glide = Glide::new(0., 100., 1000., 0.);
        let mut steps = 0;
        let mut last = 0.;
        loop {
            steps += 1;
            assert!(steps < 100, "the glide never settled");
            let (offset, done) = glide.step(1. / 60. * f64::from(steps));
            assert!(offset >= last, "{last} -> {offset}");
            last = offset;
            if done {
                break;
            }
        }
        assert_eq!(last, 100.);
        assert!(100. - SETTLE < last);
    }

    #[test]
    fn a_second_aim_goes_further_from_the_target() {
        let mut glide = Glide::new(0., 300., 1000., 0.);
        glide.aim(300., 1000.);
        assert_eq!(glide.target, 600.);
        // Kept within the container's ends.
        glide.aim(1_000., 1000.);
        assert_eq!(glide.target, 1000.);
        glide.aim(-2_000., 1000.);
        assert_eq!(glide.target, 0.);
    }

    #[test]
    fn a_glide_stops_at_the_ends() {
        // A row that can only move 80 points, asked for a page of 500.
        let glide = Glide::new(20., 500., 80., 0.);
        assert_eq!(glide.target, 80.);
        let glide = Glide::new(20., -500., 80., 0.);
        assert_eq!(glide.target, 0.);
        // No content to move at all.
        assert_eq!(Glide::new(0., 500., -10., 0.).target, 0.);
    }

    fn wheel(unit: MouseWheelUnit, delta: egui::Vec2, source: MouseWheelSource) -> Event {
        Event::MouseWheel {
            unit,
            delta,
            phase: TouchPhase::Move,
            source,
            modifiers: Modifiers::NONE,
        }
    }

    fn events(events: Vec<Event>) -> RawInput {
        RawInput {
            events,
            ..Default::default()
        }
    }

    /// The deltas of the wheel events in `input`, in points.
    fn points(input: &RawInput) -> Vec<egui::Vec2> {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                Event::MouseWheel { unit, delta, .. } => {
                    assert_eq!(*unit, MouseWheelUnit::Point, "a split step is in points");
                    Some(*delta)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_wheel_step_lands_in_one_frame() {
        let options = InputOptions::default();
        let mut input = events(vec![wheel(
            MouseWheelUnit::Line,
            vec2(0., -1.),
            MouseWheelSource::Unknown,
        )]);
        instant_wheel(&mut input, &options, 800.);
        let points = points(&input);
        assert!(points.len() > 1, "a line step is split: {points:?}");
        // Each one is short enough for egui to add it to the frame's scroll as it is.
        assert!(points.iter().all(|delta| delta.length() < 8.));
        let total = points.into_iter().fold(egui::Vec2::ZERO, |sum, delta| sum + delta);
        assert!(
            (total - vec2(0., -options.line_scroll_speed)).length() < 0.001,
            "{total}"
        );
    }

    #[test]
    fn a_page_step_uses_the_window_height() {
        let options = InputOptions::default();
        let mut input = events(vec![wheel(
            MouseWheelUnit::Page,
            vec2(0., 1.),
            MouseWheelSource::Unknown,
        )]);
        instant_wheel(&mut input, &options, 800.);
        let total = points(&input)
            .into_iter()
            .fold(egui::Vec2::ZERO, |sum, delta| sum + delta);
        assert!((total - vec2(0., 800.)).length() < 0.001, "{total}");
    }

    #[test]
    fn a_trackpads_delta_is_left_alone() {
        let options = InputOptions::default();
        let event = wheel(MouseWheelUnit::Point, vec2(0., 40.), MouseWheelSource::Trackpad);
        let mut input = events(vec![event.clone()]);
        instant_wheel(&mut input, &options, 800.);
        assert_eq!(input.events, [event]);
        // A small point delta is already smooth to egui.
        let event = wheel(MouseWheelUnit::Point, vec2(0., 5.), MouseWheelSource::Unknown);
        let mut input = events(vec![event.clone()]);
        instant_wheel(&mut input, &options, 800.);
        assert_eq!(input.events, [event]);
    }

    #[test]
    fn a_zoom_and_the_end_of_a_gesture_are_left_alone() {
        let options = InputOptions::default();
        let zoom = Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0., 1.),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Unknown,
            modifiers: options.zoom_modifier,
        };
        let end = Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0., 1.),
            phase: TouchPhase::End,
            source: MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        };
        let mut input = events(vec![zoom.clone(), end.clone()]);
        instant_wheel(&mut input, &options, 800.);
        assert_eq!(input.events, [zoom, end]);
    }
}
