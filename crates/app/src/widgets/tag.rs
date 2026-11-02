//! A tag as a small pill: sunk like an inset (`inset`, a 1 pt `surface_border` outline), its
//! text `xs` in `ink_muted`. The item page lists an item's tags this way.

use egui::{CornerRadius, Frame, Label, Margin, Response, RichText, Stroke, Ui};

use crate::theme::{self, text};

/// The room either side of the text, and above and below it.
const PAD_X: i8 = 10;
const PAD_Y: i8 = 3;

/// How wide [`show`] draws `tag`, so a wrapping row can start a new line before a pill that
/// wouldn't fit (a frame doesn't wrap as a unit, and its label would break per letter).
pub fn width(ui: &Ui, tag: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(tag.to_owned(), text::font(ui, "xs"), egui::Color32::WHITE);
    galley.size().x + 2. * PAD_X as f32 + 2.
}

pub fn show(ui: &mut Ui, tag: &str) -> Response {
    let tokens = theme::tokens_of(ui);
    let font = text::font(ui, "xs");
    // Fully round at the pill's height.
    let radius = ((ui.fonts_mut(|fonts| fonts.row_height(&font)) + 2. * PAD_Y as f32) / 2.).round() as u8;
    Frame::NONE
        .fill(tokens.inset)
        .stroke(Stroke::new(1., tokens.surface_border))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(Margin::symmetric(PAD_X, PAD_Y))
        .show(ui, |ui| {
            ui.add(Label::new(RichText::new(tag).font(font).color(tokens.ink_muted)).extend());
        })
        .response
}
