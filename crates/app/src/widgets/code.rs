//! A short code for the user to read off and type somewhere else, such as Quick Connect's: large
//! monospace characters in an inset across the width it's given, split into two halves the way
//! a number is read out.

use egui::accesskit::Role;
use egui::{Align2, FontId, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, vec2};

use crate::theme;

/// The inset's height.
const HEIGHT: f32 = 76.;
/// The characters' size.
const SIZE: f32 = 36.;
/// The space between the two halves, either side of the middle.
const SPLIT: f32 = 14.;

/// Draws `code`, named to AccessKit as the code itself.
pub fn show(ui: &mut Ui, code: &str) -> Response {
    let tokens = theme::tokens_of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, code));
    let painter = ui.painter();
    painter.rect(
        rect,
        tokens.radius_control,
        tokens.inset,
        Stroke::new(1., tokens.surface_border),
        StrokeKind::Inside,
    );
    let font = FontId::monospace(SIZE);
    let half = code
        .char_indices()
        .nth(code.chars().count() / 2)
        .map_or(code.len(), |(at, _)| at);
    let (left, right) = code.split_at(half);
    let center = rect.center();
    painter.text(
        center - vec2(SPLIT, 0.),
        Align2::RIGHT_CENTER,
        left,
        font.clone(),
        tokens.ink,
    );
    painter.text(center + vec2(SPLIT, 0.), Align2::LEFT_CENTER, right, font, tokens.ink);
    response
}
