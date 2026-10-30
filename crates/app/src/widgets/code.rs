//! A short code for the user to read off and type somewhere else, such as Quick Connect's: large
//! monospace characters in an inset across the width it's given, split into two halves the way
//! a number is read out. A copy button at the inset's right end puts it on the clipboard, for
//! pasting into a browser on the same machine.

use std::time::Duration;

use egui::accesskit::Role;
use egui::{Align2, Button, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, vec2};
use egui_material_icons::icons::{ICON_CHECK, ICON_CONTENT_COPY};

use crate::theme;
use crate::widgets::button;

/// The inset's height.
const HEIGHT: f32 = 76.;
/// The characters' size.
const SIZE: f32 = 36.;
/// The space between the two halves, either side of the middle.
const SPLIT: f32 = 14.;
/// The copy button's side, and its room from the inset's right edge.
const BUTTON: f32 = 32.;
const BUTTON_INSET: f32 = 14.;
/// The copy button's icon size, beside the code's large characters.
const ICON: f32 = 22.;
/// How long the copy button shows a check mark after a click.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// Draws `code`, named to AccessKit as the code itself, with its copy button.
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
    copy_button(ui, rect, code);
    response
}

/// The button that copies `code`, at `inset`'s right end. A check mark stands in for its icon
/// for a moment after a click, since nothing else shows the copy worked.
fn copy_button(ui: &mut Ui, inset: Rect, code: &str) {
    let ctx = ui.ctx().clone();
    let id = Id::unique("code-copied");
    let now = ctx.input(|input| input.time);
    // How long the check mark has left, while it shows.
    let left = ctx
        .data(|data| data.get_temp::<f64>(id))
        .map(|at| COPIED_FOR.as_secs_f64() - (now - at))
        .filter(|left| *left > 0.);
    let copied = left.is_some();
    let (icon, label) = if copied {
        (ICON_CHECK, "Copied")
    } else {
        (ICON_CONTENT_COPY, "Copy code")
    };
    let icon = icon.outlined();
    let font = FontId::new(ICON, icon.font_family());
    let label_text = button::icon_text(icon, ICON, "", font, theme::tokens_of(ui).ink_muted);
    let button = Button::new(label_text).frame(false).min_size(vec2(BUTTON, BUTTON));
    let center = egui::pos2(inset.right() - BUTTON_INSET - BUTTON / 2., inset.center().y);
    let response = ui.place(Rect::from_center_size(center, vec2(BUTTON, BUTTON)), button);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if response.clicked() {
        ctx.copy_text(code.to_owned());
        ctx.data_mut(|data| data.insert_temp(id, now));
        ctx.request_repaint_after(COPIED_FOR);
    } else if let Some(left) = left {
        ctx.request_repaint_after(Duration::from_secs_f64(left));
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label);
}
