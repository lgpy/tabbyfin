//! The small marks that say something about what they sit on: the tick on a watched card, a
//! status dot, and a label in the player's bars.

use egui::accesskit::Role;
use egui::{Color32, Pos2, Rect, Sense, Ui, WidgetInfo, vec2};
use egui_material_icons::icons::ICON_CHECK;

use crate::theme::{self, text};
use crate::widgets::round_button;

/// The tick's circle, its distance from the corner it sits in, and the tick's size.
const CHECK_RADIUS: f32 = 8.;
const CHECK_INSET: f32 = 4.;
const CHECK_ICON: f32 = 12.;
/// A dot beside a heading's note.
const DOT: f32 = 8.;
/// A dot inside a row's subtitle.
pub const DOT_SMALL: f32 = 6.;
/// A label's room left and right of its text, and above and below it.
const LABEL_PAD_X: f32 = 8.;
const LABEL_PAD_Y: f32 = 2.;

/// A `brand` circle with a tick, in `rect`'s top right corner. The tick is centred by its glyph,
/// so it reads the same on a dark image and a light one.
pub fn check(ui: &Ui, rect: Rect) {
    let tokens = theme::tokens_of(ui);
    let offset = CHECK_INSET + CHECK_RADIUS;
    let center = rect.right_top() + vec2(-offset, offset);
    ui.painter().circle_filled(center, CHECK_RADIUS, tokens.brand);
    round_button::glyph(ui, center, ICON_CHECK, CHECK_ICON, tokens.brand_foreground);
}

/// A status dot added to `ui`'s layout.
pub fn dot(ui: &mut Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
    dot_at(ui, rect.center(), DOT, colour);
}

/// A status dot `diameter` across on `center`, for a row that places its own parts.
pub fn dot_at(ui: &Ui, center: Pos2, diameter: f32, colour: Color32) {
    ui.painter().circle_filled(center, diameter / 2., colour);
}

/// How wide [`label`] draws `name`, for a bar that leaves room for its labels.
pub fn label_width(ui: &Ui, name: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_owned(), text::font(ui, "xs"), Color32::WHITE);
    galley.size().x + 2. * LABEL_PAD_X
}

/// `name` in `xs` on a pill of `fill`, fully round at any text size. The colours are the caller's,
/// since what a label sits on (the player's video) can't take the theme's.
pub fn label(ui: &mut Ui, name: &str, fill: Color32, colour: Color32) {
    let font = text::font(ui, "xs");
    let height = ui.fonts_mut(|fonts| fonts.row_height(&font)) + 2. * LABEL_PAD_Y;
    let (rect, response) = ui.allocate_exact_size(vec2(label_width(ui, name), height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, name));
    ui.painter().rect_filled(rect, (height / 2.).round() as u8, fill);
    let galley = ui.painter().layout_no_wrap(name.to_owned(), font, colour);
    ui.painter().galley(rect.center() - galley.size() / 2., galley, colour);
}
