//! The cozy look's on/off switch: a pill track sunk like an inset while off and filled with
//! `brand` while on, with a round knob that slides between the ends. Settings' flags are
//! switches, and the playlist dialog's sharing is a [`row`].
//!
//! Each is one widget named to AccessKit as a switch with its state. [`switch`] is focusable
//! (Space or Enter flips it, as egui's checkbox did) and takes `focus::ring` for the keyboard;
//! a [`row`] can stay out of egui's focus for a view whose keys are its own (a dialog).

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, pos2, vec2};

use crate::theme::{self, text};
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};

/// The track's size; the knob is the track's height less [`KNOB_GAP`] on each side.
pub const SIZE: Vec2 = vec2(40., 22.);
const KNOB_GAP: f32 = 3.;
/// A [`row`]'s room above and below its label, and left of it and right of the track.
const ROW_PADDING: Vec2 = vec2(12., 9.);

/// A switch for `on`, called `name` for screen readers. Returns its response, `changed()` when a
/// click or key flipped it.
pub fn switch(ui: &mut Ui, on: &mut bool, name: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let value = *on;
    response.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, value, name));
    track(ui, rect, response.id, value, response.hovered());
    if ui.memory(|memory| memory.has_focus(response.id)) {
        focus::ring(ui, rect, rect.height() / 2., Place::Outside);
    }
    response
}

/// A switch with its `label` across the `ui`'s width, the label at the left and the track at the
/// right end, the whole row the target. It doesn't flip itself: the caller flips `on` when it's
/// `clicked()` (and may refuse, while busy). With `Focus::Skip` it takes clicks only, and
/// `ringed` puts the keyboard's ring on it for a view that moves its own highlight.
pub fn row(ui: &mut Ui, on: bool, label: &str, focus: Focus, ringed: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let font = text::font(ui, "base");
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let height = line.max(SIZE.y) + 2. * ROW_PADDING.y;
    let sense = match focus {
        Focus::Tab => Sense::click(),
        Focus::Skip => Sense::CLICK,
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, on, label));

    let radius = tokens.radius_control;
    let fill = if response.hovered() || ringed {
        tokens.inset_hover
    } else {
        tokens.inset
    };
    ui.painter().rect_filled(rect, radius, fill);
    if ringed || ui.memory(|memory| memory.has_focus(response.id)) {
        focus::ring(ui, rect, radius as f32, Place::Inside);
    }
    let track_rect = Rect::from_min_size(
        pos2(rect.right() - ROW_PADDING.x - SIZE.x, rect.center().y - SIZE.y / 2.),
        SIZE,
    );
    track(ui, track_rect, response.id, on, response.hovered());

    let left = rect.left() + ROW_PADDING.x;
    let mut job = LayoutJob::simple_singleline(label.to_owned(), font, tokens.ink);
    job.wrap = TextWrapping::truncate_at_width((track_rect.left() - ROW_PADDING.x - left).max(0.));
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(pos2(left, rect.center().y - galley.size().y / 2.), galley, tokens.ink);
    response
}

/// The track and its knob in `rect`, the knob's slide animated under `id`.
fn track(ui: &Ui, rect: Rect, id: Id, on: bool, hovered: bool) {
    let tokens = theme::tokens_of(ui);
    // Zero with the animations setting off (`theme::apply_animations`), so it jumps.
    let t = ui.ctx().animate_bool_responsive(id, on);
    let radius = rect.height() / 2.;
    let (fill, outline) = match (on, hovered) {
        (true, false) => (tokens.brand, tokens.brand),
        (true, true) => (tokens.brand_hover, tokens.brand_hover),
        (false, false) => (tokens.inset, tokens.surface_border),
        (false, true) => (tokens.inset_hover, tokens.ink_muted),
    };
    ui.painter()
        .rect(rect, radius, fill, Stroke::new(1., outline), StrokeKind::Inside);
    let x = rect.left() + radius + (rect.width() - 2. * radius) * t;
    // On `brand` the knob is the text colour that sits on it; off, it's the muted ink, so the
    // off state reads without colour.
    let knob = if on { tokens.brand_foreground } else { tokens.ink_muted };
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), radius - KNOB_GAP, knob);
}
