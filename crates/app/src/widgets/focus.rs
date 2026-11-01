//! The keyboard's ring: one look for every place the keyboard (focus or a page's cursor) sits
//! on something, so the ring reads the same on a button, a row and a card.

use egui::{CornerRadius, Rect, Response, Stroke, StrokeKind, Ui};

use crate::theme;

/// The ring's width.
pub const WIDTH: f32 = 2.;
/// The space between a control and a ring drawn outside it.
pub const GAP: f32 = 2.;

/// Which side of the shape's edge the ring goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Clear of the shape by [`GAP`], for free-standing controls (bar buttons, selects, pills).
    Outside,
    /// On the shape's own edge, for rows and cards, which sit in lists and scroll areas that
    /// would clip a ring outside them.
    Inside,
}

/// The ring round `rect`, a shape with corners of `radius`. An outside ring follows the shape
/// (its radius grows by the gap), so a circle gets a circle and a pill a pill.
pub fn ring(ui: &Ui, rect: Rect, radius: f32, place: Place) {
    let (rect, radius, kind) = match place {
        Place::Outside => (rect.expand(GAP), radius + GAP, StrokeKind::Outside),
        Place::Inside => (rect, radius, StrokeKind::Inside),
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius.round().clamp(0., 255.) as u8),
        Stroke::new(WIDTH, theme::tokens_of(ui).brand),
        kind,
    );
}

/// An outside ring round an egui widget (a `Button`, a `TextEdit`), on the theme's widget radius.
pub fn ring_widget(ui: &Ui, response: &Response) {
    let radius = ui.visuals().widgets.inactive.corner_radius.nw as f32;
    ring(ui, response.rect, radius, Place::Outside);
}
