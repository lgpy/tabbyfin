//! The cozy look's text fields: a muted label above a roomy single-line field, sunk into the
//! surface (the fill and outline come from [`theme::cozy_style`]).

use egui::{CursorIcon, FontId, Id, Margin, Rect, Response, RichText, Sense, TextBuffer, TextEdit, Ui, Vec2, pos2};
use egui_material_icons::MaterialIcon;

use crate::theme::{self, text};

/// A field's name above it, in the muted ink.
pub fn label(ui: &mut Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .font(text::font(ui, "sm"))
            .color(theme::tokens_of(ui).ink_muted),
    );
    ui.add_space(2.);
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
/// toggle. Painted rather than an `egui::Button`, whose hover frame drew a box inside the field;
/// it rests in the muted ink and darkens over a soft circle while hovered. Not allocated, so
/// it sits over the field rather than after it.
pub fn trailing(ui: &mut Ui, field: &Response, icon: MaterialIcon, enabled: bool) -> Response {
    let size = TRAILING - 8.;
    let rect = Rect::from_center_size(
        pos2(field.rect.right() - TRAILING / 2., field.rect.center().y),
        Vec2::splat(size),
    );
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, field.id.with("trailing"), sense);
    let tokens = theme::tokens_of(ui);
    let hovered = enabled && response.hovered();
    if hovered {
        ui.painter().circle_filled(rect.center(), size / 2., tokens.inset_hover);
    }
    let color = if hovered { tokens.ink } else { tokens.ink_muted };
    let color = if enabled { color } else { color.gamma_multiply(0.5) };
    // The icon font's glyphs sit low in their line, so they're centred by their own bounds
    // rather than the line's.
    let font = FontId::new(text::font(ui, "lg").size, icon.font_family());
    let galley = ui.painter().layout_no_wrap(icon.codepoint.to_owned(), font, color);
    let glyph = galley.rows.first().map_or(galley.rect, |row| row.rect());
    ui.painter()
        .galley(rect.center() - glyph.center().to_vec2(), galley, color);
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}
