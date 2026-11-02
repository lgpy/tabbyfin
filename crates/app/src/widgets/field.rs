//! The app's text fields: a muted label above a roomy single-line field, sunk into the
//! surface (the fill and outline come from the theme's visuals, `theme::visuals`).

use egui::{Id, Margin, Pos2, Response, RichText, TextBuffer, TextEdit, Ui, pos2};
use egui_material_icons::MaterialIcon;

use crate::theme::{self, text};
use crate::widgets::round_button::{self, RoundButton};

/// A field's name above it, in the muted ink.
pub fn label(ui: &mut Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .font(text::font(ui, "sm"))
            .color(theme::tokens_of(ui).ink_muted),
    );
    ui.add_space(theme::SPACE.xxs);
}

/// A single-line field over the width it's given, in the body's larger size with room around
/// the text; the caller adds the rest (password, interactivity) before showing it.
///
/// `hint` shows only while the field is empty and doesn't have the keyboard: egui keeps it
/// next to the caret until something is typed, which reads as text already in the field.
pub fn single_line<'t>(ui: &Ui, id: Id, text: &'t mut dyn TextBuffer, hint: &str) -> TextEdit<'t> {
    let edit = TextEdit::singleline(text)
        .id(id)
        .font(text::font(ui, "base"))
        .text_color(theme::tokens_of(ui).ink)
        .margin(Margin::symmetric(12, 9))
        .desired_width(f32::INFINITY);
    if hint.is_empty() || ui.memory(|memory| memory.focused()) == Some(id) {
        edit
    } else {
        edit.hint_text(hint.to_owned())
    }
}

/// The room a [`trailing`] button takes at a field's right end: what [`single_line`]'s
/// margin has to leave, so the text doesn't run under it.
pub const TRAILING: f32 = 36.;

/// [`single_line`] with room at its right end for a [`trailing`] button.
pub fn with_trailing<'t>(ui: &Ui, id: Id, text: &'t mut dyn TextBuffer, hint: &str) -> TextEdit<'t> {
    single_line(ui, id, text, hint).margin(Margin {
        left: 12,
        right: TRAILING as i8,
        top: 9,
        bottom: 9,
    })
}

/// An icon inside `field`'s right end that acts as a button, such as a password's show/hide
/// toggle, named and tooltipped by `label`. A [`RoundButton`] rather than an `egui::Button`,
/// whose hover frame drew a box inside the field. Not allocated, so it sits over the field
/// rather than after it.
pub fn trailing(ui: &mut Ui, field: &Response, icon: MaterialIcon, label: &str, enabled: bool) -> Response {
    trailing_button(ui, icon, label)
        .enabled(enabled)
        .show_at(ui, trailing_center(field), field.id.with("trailing"))
}

/// A [`trailing`] button sized for `ui`'s text, for a caller that sets more of it (its focus)
/// before showing it at [`trailing_center`].
pub fn trailing_button<'a>(ui: &Ui, icon: MaterialIcon, label: &'a str) -> RoundButton<'a> {
    RoundButton::icon(icon, label).size(TRAILING - 8., text::font(ui, "lg").size)
}

/// Where a [`trailing`] button's centre is in `field`.
pub fn trailing_center(field: &Response) -> Pos2 {
    pos2(field.rect.right() - TRAILING / 2., field.rect.center().y)
}

/// The room a [`leading`] icon takes at a field's left end: what [`single_line`]'s margin has
/// to leave, so the text starts after it.
pub fn leading_room(ui: &Ui) -> f32 {
    12. + text::font(ui, "lg").size + 8.
}

/// [`single_line`] with room at its left end for a [`leading`] icon.
pub fn with_leading<'t>(ui: &Ui, id: Id, text: &'t mut dyn TextBuffer, hint: &str) -> TextEdit<'t> {
    single_line(ui, id, text, hint).margin(Margin {
        left: leading_room(ui).round() as i8,
        right: 12,
        top: 9,
        bottom: 9,
    })
}

/// An icon inside `field`'s left end, such as a search box's magnifier. Painted, so it takes no
/// room in `ui`'s layout and sits over the field: draw it after the field is added, or the
/// field's own background covers it.
pub fn leading(ui: &Ui, field: &Response, icon: MaterialIcon) {
    let size = text::font(ui, "lg").size;
    round_button::glyph(
        ui,
        pos2(field.rect.left() + 12. + size / 2., field.rect.center().y),
        icon,
        size,
        theme::tokens_of(ui).ink_muted,
    );
}
