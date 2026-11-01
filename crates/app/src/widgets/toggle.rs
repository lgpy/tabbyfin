//! The cozy look's on/off switch: a pill track sunk like an inset while off and filled with
//! `brand` while on, with a round knob that slides between the ends. Settings' flags are
//! switches.
//!
//! It's one focusable widget (Space or Enter flips it, as egui's checkbox did), named to
//! AccessKit as a switch with its state, and takes `focus::ring` for the keyboard.

use egui::accesskit::Role;
use egui::{Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, pos2, vec2};

use crate::theme;
use crate::widgets::focus::{self, Place};

/// The track's size; the knob is the track's height less [`KNOB_GAP`] on each side.
pub const SIZE: Vec2 = vec2(40., 22.);
const KNOB_GAP: f32 = 3.;

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

    let tokens = theme::tokens_of(ui);
    // Zero with the animations setting off (`theme::apply_animations`), so it jumps.
    let t = ui.ctx().animate_bool_responsive(response.id, value);
    let radius = rect.height() / 2.;
    let (fill, outline) = if value {
        (tokens.brand, tokens.brand)
    } else if response.hovered() {
        (tokens.inset_hover, tokens.ink_muted)
    } else {
        (tokens.inset, tokens.surface_border)
    };
    let fill = if value && response.hovered() {
        tokens.brand_hover
    } else {
        fill
    };
    ui.painter()
        .rect(rect, radius, fill, Stroke::new(1., outline), StrokeKind::Inside);
    let knob = radius - KNOB_GAP;
    let left = rect.left() + radius;
    let x = left + (rect.width() - 2. * radius) * t;
    // On `brand` the knob is the text colour that sits on it; off, it's the muted ink, so the
    // off state reads without colour.
    let knob_color = if value {
        tokens.brand_foreground
    } else {
        tokens.ink_muted
    };
    ui.painter().circle_filled(pos2(x, rect.center().y), knob, knob_color);
    if ui.memory(|memory| memory.has_focus(response.id)) {
        focus::ring(ui, rect, radius, Place::Outside);
    }
    response
}
