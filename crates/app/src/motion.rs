//! The app's animations: a page entering, the shelves' stagger, an image fading in, a toggle's
//! pop, the player's prompt sliding in and its controls fading in, and a band unfolding under
//! what opened it (a series' season episodes).
//!
//! egui can't move what a `Ui` lays out without moving the layout, so a lift is the room above
//! the content closing as it rises, and a fade is the painter's opacity. An animation plays when
//! what it wraps appears: a page opened, a shelf drawn for the first time, an image loaded, the
//! player's controls shown, a prompt appearing. Each helper asks for frames only while it runs,
//! so an idle page costs nothing.
//!
//! The animations setting is egui's own animation time ([`theme::animations`], set by
//! `theme::apply_animations`): with it off every helper draws its content at the end state, at
//! once, and asks for no frames.

use egui::{Id, Rect, Ui, UiBuilder, vec2};

use crate::theme;

/// How far an element rises as it fades in.
pub const LIFT: f32 = 12.;
/// How long an element takes to rise and fade in.
pub const ENTER: f64 = 0.260;
/// How long a loaded image fades in.
pub const FADE: f64 = 0.220;
/// How long a toggle's icon pops.
pub const POP: f64 = 0.420;
/// The delay between one shelf appearing and the next.
pub const SHELF_STAGGER: f64 = 0.060;

/// How long a band takes to unfold to its height (or fold away, or follow its content's).
pub const UNFOLD: f64 = 0.340;
/// How far a band's content slides down from under what opened it as it unfolds.
pub const UNFOLD_SLIDE: f32 = 32.;
/// The delay between one line of an unfolded band's rows and the next.
pub const ROW_STAGGER: f64 = 0.045;
/// The most lines a stagger waits for: a long season's last rows don't trail in.
const STAGGER_LINES: usize = 6;

/// Quick off the mark, settling gently (`ease_out_quint`).
fn ease_out_quint(t: f32) -> f32 {
    1. - (1. - t).powi(5)
}

/// How far into its animation something that appeared at `started` is, `delay` after it appeared:
/// 0 until the delay is over, 1 once `duration` is, eased the same way.
pub fn progress(started: f64, now: f64, delay: f64, duration: f64) -> f32 {
    let t = ((now - started - delay) / duration).clamp(0., 1.) as f32;
    ease_out_quint(t)
}

/// How long line `line` of a band's rows waits after the rows appear, so they come in one after
/// another; lines past the first few come in together.
pub fn row_delay(line: usize) -> f64 {
    ROW_STAGGER * line.min(STAGGER_LINES) as f64
}

/// A smooth start and stop (`ease_in_out_cubic`), for a height whose change moves the page
/// under it: it neither jolts the content below nor overshoots.
fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// The scale a toggle's icon has `elapsed` after it turned on: half its size, a little past it
/// and back, within [`POP`]; 1 before and after, so an older toggle is drawn as it is.
pub fn pop_scale(elapsed: f64) -> f32 {
    if elapsed < 0. || elapsed >= POP {
        return 1.;
    }
    let t = ease_out_quint((elapsed / POP) as f32);
    0.5 + 0.5 * t + 0.35 * (std::f32::consts::PI * t).sin()
}

/// How long a shelf at `index` waits after the page appears, so the shelves come in one after
/// another, down the page.
pub fn shelf_delay(index: usize) -> f64 {
    SHELF_STAGGER * index as f64
}

/// The time `id` last appeared: now, unless it was drawn in the previous pass. A page that comes
/// back on top, a card scrolled out and back, an image that finished loading after its card went
/// away: each starts its animation again.
///
/// Passes, not frames: egui can run several passes in one frame (a discard pass for a `Grid` or a
/// popup's sizing, say), and an element drawn in both is one appearance. Counting frames, the
/// second pass would read as the same frame and start the animation over. An element drawn only
/// in the second pass of a frame is still drawn in consecutive passes, so it keeps its start too.
pub fn appear(ctx: &egui::Context, id: impl std::hash::Hash + std::fmt::Debug) -> f64 {
    #[derive(Clone, Copy)]
    struct Seen {
        started: f64,
        pass: u64,
    }

    let id = Id::unique(("motion-appear", id));
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|input| input.time);
    let seen = ctx.data(|data| data.get_temp::<Seen>(id));
    // Drawn again in the same pass counts too: two cards for one person (an actor who also
    // produced) share an image, and the second would otherwise restart the fade the first had
    // just continued, leaving both stuck near transparent.
    let mut seen = match seen.filter(|seen| seen.pass == pass || seen.pass + 1 == pass) {
        Some(seen) => seen,
        None => Seen { started: now, pass },
    };
    seen.pass = pass;
    ctx.data_mut(|data| data.insert_temp(id, seen));
    seen.started
}

/// Draws `content` faded in while it rises into place, starting `delay` after it appears.
/// Returns what `content` returned.
pub fn enter<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    delay: f64,
    content: impl FnOnce(&mut Ui) -> R,
) -> R {
    let ctx = ui.ctx().clone();
    if !theme::animations(&ctx) {
        return content(ui);
    }
    let started = appear(&ctx, id);
    let now = ctx.input(|input| input.time);
    let t = progress(started, now, delay, ENTER);
    if t >= 1. {
        return content(ui);
    }
    // egui can only move what it lays out, so the room the element takes above itself closes as
    // it rises instead.
    ui.add_space(LIFT * (1. - t));
    let drawn = faded(ui, t, content);
    ctx.request_repaint();
    drawn
}

/// The opacity a loaded image has: it fades in when it first shows, over its blurhash.
pub fn image_alpha(ui: &Ui, id: impl std::hash::Hash + std::fmt::Debug) -> f32 {
    let ctx = ui.ctx();
    if !theme::animations(ctx) {
        return 1.;
    }
    let started = appear(ctx, id);
    let now = ui.input(|input| input.time);
    let t = progress(started, now, 0., FADE);
    if t < 1. {
        ctx.request_repaint();
    }
    t
}

/// What [`unfold`] drew: `content`'s result, and whether the band has stopped moving (at its
/// content's height, or folded away).
pub struct Unfolded<R> {
    pub inner: R,
    pub settled: bool,
}

/// Draws `content` in a band that unfolds under what's above it while `open`, and folds away
/// once it isn't: its height eases to its content's, so what's below moves down (or up) with it
/// instead of jumping, while the content slides down from under the band's top edge and fades
/// in. Its height follows its content's afterwards too (a loader giving way to a season's rows,
/// another season opening in its place). `id` is the band's, not its content's, so a season
/// opening in place of another resizes the band rather than unfolding a new one.
///
/// A folded band takes no room and its content, clipped away, takes no clicks.
pub fn unfold<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    open: bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Unfolded<R> {
    #[derive(Clone, Copy)]
    struct Fold {
        /// The height the band eases `from`, `to` its content's, since `started`.
        from: f32,
        to: f32,
        started: f64,
        /// When it began unfolding, or folding away.
        turned: f64,
        open: bool,
        pass: u64,
    }

    let ctx = ui.ctx().clone();
    if !theme::animations(&ctx) {
        let inner = match open {
            true => content(ui),
            false => content(&mut ui.new_child(UiBuilder::new().invisible())),
        };
        return Unfolded { inner, settled: true };
    }

    let id = Id::unique(("motion-unfold", id));
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|input| input.time);
    // A band that wasn't drawn in the previous pass unfolds from nothing again.
    let fold = ctx.data(|data| data.get_temp::<Fold>(id));
    let mut fold = fold
        .filter(|fold| fold.pass == pass || fold.pass + 1 == pass)
        .unwrap_or(Fold {
            from: 0.,
            to: 0.,
            started: now,
            turned: now,
            open,
            pass,
        });
    let height = |fold: &Fold| {
        let t = ((now - fold.started) / UNFOLD).clamp(0., 1.) as f32;
        fold.from + (fold.to - fold.from) * ease_in_out_cubic(t)
    };
    if fold.open != open {
        fold = Fold {
            from: height(&fold),
            to: if open { fold.to } else { 0. },
            started: now,
            turned: now,
            open,
            pass,
        };
    }
    fold.pass = pass;
    let shown = height(&fold);
    // How open it looks: the slide and the fade, quick to come in and gentle to go.
    let turned = progress(fold.turned, now, 0., UNFOLD);
    let openness = if open { turned } else { 1. - turned };

    let top = ui.cursor().min;
    let width = ui.available_width();
    let slide = UNFOLD_SLIDE * (1. - openness);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(top - vec2(0., slide), vec2(width, f32::INFINITY)))
            .layout(*ui.layout()),
    );
    child.shrink_clip_rect(Rect::from_min_size(top, vec2(width, shown)));
    child.set_opacity(child.painter().opacity() * openness);
    let inner = content(&mut child);
    let measured = child.min_rect().bottom() - (top.y - slide);

    // The content's height moved (it loaded, or another took its place): ease on to it from
    // where the band is now, so it doesn't jump.
    let target = if open { measured } else { 0. };
    if (target - fold.to).abs() > 0.5 {
        fold = Fold {
            from: shown,
            to: target,
            started: now,
            ..fold
        };
    }
    ctx.data_mut(|data| data.insert_temp(id, fold));
    if shown > 0. {
        ui.advance_cursor_after_rect(Rect::from_min_size(top, vec2(width, shown)));
    }
    let settled = now - fold.started >= UNFOLD && now - fold.turned >= UNFOLD;
    if !settled {
        ctx.request_repaint();
    }
    Unfolded { inner, settled }
}

/// Runs `content` with the painter's opacity multiplied by `alpha`, and puts the opacity back
/// afterwards so what's drawn next isn't faded with it.
pub fn faded<R>(ui: &mut Ui, alpha: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    if alpha >= 1. {
        return content(ui);
    }
    let previous = ui.painter().opacity();
    ui.set_opacity(previous * alpha);
    let drawn = content(ui);
    ui.set_opacity(previous);
    drawn
}

#[cfg(test)]
mod tests {
    use egui::{Context, RawInput};

    use super::{ENTER, FADE, LIFT, POP, SHELF_STAGGER, appear, pop_scale, progress, shelf_delay};

    #[test]
    fn an_entry_holds_still_through_its_delay_then_ends() {
        // A shelf waits its stagger out, invisible, then rises and fades.
        let delay = shelf_delay(3);
        assert_eq!(delay, 3. * SHELF_STAGGER);
        assert_eq!(progress(100., 100., delay, ENTER), 0.);
        assert_eq!(progress(100., 100. + delay, delay, ENTER), 0.);
        assert_eq!(progress(100., 100. + delay + ENTER, delay, ENTER), 1.);
        // Once it's over it stays over, however long the page is up.
        assert_eq!(progress(100., 3600., delay, ENTER), 1.);
        // Eased out: more than halfway there after half the time, but never past the end.
        let half = progress(100., 100. + delay + ENTER / 2., delay, ENTER);
        assert!(half > 0.5 && half < 1., "{half}");
    }

    #[test]
    fn shelves_come_in_one_after_another() {
        assert_eq!(shelf_delay(0), 0.);
        assert_eq!(shelf_delay(1), SHELF_STAGGER);
        // The second shelf has started while the third is still waiting, and the first has
        // finished before the second has.
        let now = SHELF_STAGGER * 1.5;
        assert!(progress(0., now, shelf_delay(0), ENTER) > 0.);
        assert!(progress(0., now, shelf_delay(1), ENTER) > 0.);
        assert_eq!(progress(0., now, shelf_delay(2), ENTER), 0.);
        assert_eq!(progress(0., ENTER, shelf_delay(0), ENTER), 1.);
        assert!(progress(0., ENTER, shelf_delay(1), ENTER) < 1.);
    }

    #[test]
    fn a_fade_is_done_when_its_time_is() {
        assert_eq!(progress(10., 10., 0., FADE), 0.);
        assert_eq!(progress(10., 10. + FADE, 0., FADE), 1.);
        assert!(progress(10., 10. + FADE / 2., 0., FADE) > 0.5);
    }

    #[test]
    fn a_toggle_pops_from_half_size_and_settles() {
        assert_eq!(pop_scale(-0.1), 1., "before it turned on, nothing pops");
        assert!((pop_scale(0.) - 0.5).abs() < 1e-6, "a pop starts at half size");
        assert!((pop_scale(POP) - 1.).abs() < 1e-6);
        assert_eq!(pop_scale(POP + 1.), 1., "an older toggle is drawn as it is");
        // A little past its size partway through.
        let overshoot = (1..100)
            .map(|i| pop_scale(POP * f64::from(i) / 100.))
            .fold(0.5_f32, f32::max);
        assert!(overshoot > 1.05 && overshoot < 1.2, "{overshoot}");
    }

    /// The delay egui asks for the next frame after drawing `contents` at `time`.
    fn repaint_delay(ctx: &Context, time: f64, contents: impl FnMut(&mut egui::Ui)) -> std::time::Duration {
        let mut output = ctx.run_ui(
            RawInput {
                time: Some(time),
                ..Default::default()
            },
            contents,
        );
        // The deltas are the renderer's to apply; a test has none.
        output.textures_delta.clear();
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    #[test]
    fn an_entry_asks_for_frames_only_while_it_runs() {
        let ctx = Context::default();
        ctx.all_styles_mut(|style| style.animation_time = egui::Style::default().animation_time);
        let entry = |ui: &mut egui::Ui| {
            super::enter(ui, "page", 0., |_| {});
        };
        // A pass's delay is what the pass before it asked for: the entry asks for each frame
        // while it runs, and the first frames ask once anyway (fonts, the first pass).
        assert_eq!(repaint_delay(&ctx, 0., entry), std::time::Duration::ZERO);
        assert_eq!(repaint_delay(&ctx, 0.1, entry), std::time::Duration::ZERO);
        assert_eq!(repaint_delay(&ctx, 1., entry), std::time::Duration::ZERO);
        // Done: an idle page that drew nothing moving asks for no frame.
        assert_eq!(repaint_delay(&ctx, 2., entry), std::time::Duration::MAX);
        assert_eq!(repaint_delay(&ctx, 10., entry), std::time::Duration::MAX);
    }

    #[test]
    fn with_animations_off_an_entry_is_drawn_at_once_and_asks_for_nothing() {
        let ctx = Context::default();
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.;
            // No scroll animation either, as `theme::apply_animations` leaves it.
            style.scroll_animation = egui::style::ScrollAnimation::none();
        });
        let drawn = std::cell::Cell::new(0);
        let entry = |ui: &mut egui::Ui| {
            super::enter(ui, "page", 0., |_| drawn.set(drawn.get() + 1));
        };
        repaint_delay(&ctx, 0., entry);
        assert_eq!(drawn.get(), 1, "the content is drawn on its first frame");
        repaint_delay(&ctx, 0.05, entry);
        assert_eq!(drawn.get(), 2);
        // It's at its end state, so nothing is left to ask for.
        assert_eq!(repaint_delay(&ctx, 0.1, entry), std::time::Duration::MAX);
        assert_eq!(drawn.get(), 3);
    }

    #[test]
    fn the_lift_is_main_s_twelve_points() {
        assert_eq!(LIFT, 12.);
    }

    /// Draws `contents` for one pass at `time`.
    fn pass_at(ctx: &Context, time: f64, contents: impl FnMut(&mut egui::Ui)) {
        let input = RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, contents);
        output.textures_delta.clear();
    }

    #[test]
    fn what_keeps_drawing_keeps_its_start() {
        let ctx = Context::default();
        let mut seen = Vec::new();
        for time in [0., 0.1, 0.2] {
            pass_at(&ctx, time, |ui| seen.push(appear(ui.ctx(), "page")));
        }
        assert_eq!(seen, [0., 0., 0.]);
    }

    #[test]
    fn a_discard_pass_is_the_same_appearance() {
        let ctx = Context::default();
        let mut seen = Vec::new();
        let mut first = ctx.run_ui(
            RawInput {
                time: Some(0.),
                ..Default::default()
            },
            |ui| seen.push(appear(ui.ctx(), "page")),
        );
        first.textures_delta.clear();
        // A frame that asks for a second pass, as a Grid's or a popup's sizing does.
        let mut second = ctx.run_ui(
            RawInput {
                time: Some(0.1),
                ..Default::default()
            },
            |ui| {
                seen.push(appear(ui.ctx(), "page"));
                if ui.ctx().current_pass_index() == 0 {
                    ui.ctx().request_discard("test");
                }
            },
        );
        second.textures_delta.clear();
        assert_eq!(seen.len(), 3, "the second frame ran two passes");
        assert_eq!(seen, [0., 0., 0.], "one appearance across the frame's passes");
    }

    /// The room a band takes below where it starts, drawn at `time`, `open` or not, around a
    /// block of content `content` tall.
    fn band_height(ctx: &Context, time: f64, open: bool, content: f32) -> (f32, bool) {
        let mut out = (0., false);
        pass_at(ctx, time, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let top = ui.cursor().min.y;
            let unfolded = super::unfold(ui, "band", open, |ui| {
                ui.allocate_space(egui::vec2(100., content));
            });
            out = (ui.cursor().min.y - top, unfolded.settled);
        });
        out
    }

    fn animated() -> Context {
        let ctx = Context::default();
        ctx.all_styles_mut(|style| style.animation_time = egui::Style::default().animation_time);
        ctx
    }

    #[test]
    fn a_band_unfolds_to_its_content_and_settles() {
        let ctx = animated();
        assert_eq!(band_height(&ctx, 0., true, 200.), (0., false), "it starts folded");
        let (half, settled) = band_height(&ctx, super::UNFOLD / 2., true, 200.);
        assert!(half > 50. && half < 150. && !settled, "{half}");
        assert_eq!(band_height(&ctx, super::UNFOLD + 0.01, true, 200.), (200., true));
    }

    #[test]
    fn a_band_follows_its_content_from_where_it_is() {
        let ctx = animated();
        band_height(&ctx, 0., true, 100.);
        band_height(&ctx, 1., true, 100.);
        // Its rows arrived: it grows from the loader's height, not from nothing.
        let (start, _) = band_height(&ctx, 2., true, 300.);
        assert_eq!(start, 100.);
        let (grown, _) = band_height(&ctx, 2. + super::UNFOLD / 2., true, 300.);
        assert!(grown > 150. && grown < 250., "{grown}");
        assert_eq!(band_height(&ctx, 3., true, 300.), (300., true));
    }

    #[test]
    fn a_closed_band_folds_away_to_nothing() {
        let ctx = animated();
        band_height(&ctx, 0., true, 200.);
        // Closed: it starts folding from its full height.
        assert_eq!(band_height(&ctx, 1., false, 200.), (200., false));
        let (folding, settled) = band_height(&ctx, 1. + super::UNFOLD / 2., false, 200.);
        assert!(folding > 0. && folding < 200. && !settled, "{folding}");
        assert_eq!(band_height(&ctx, 2., false, 200.), (0., true));
    }

    #[test]
    fn with_animations_off_a_band_is_at_once_open_or_gone() {
        let ctx = Context::default();
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.;
            style.scroll_animation = egui::style::ScrollAnimation::none();
        });
        assert_eq!(band_height(&ctx, 0., true, 200.), (200., true));
        assert_eq!(band_height(&ctx, 0.05, false, 200.), (0., true));
    }

    #[test]
    fn what_goes_away_and_comes_back_starts_over() {
        let ctx = Context::default();
        pass_at(&ctx, 0., |ui| {
            assert_eq!(appear(ui.ctx(), "page"), 0.);
        });
        pass_at(&ctx, 1., |_| {});
        pass_at(&ctx, 2., |ui| {
            assert_eq!(appear(ui.ctx(), "page"), 2.);
        });
    }
}
