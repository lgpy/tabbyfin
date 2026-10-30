//! The cozy look's raised panel: a [`theme::Tokens::surface`] lifted off the backdrop by a soft
//! shadow, with the larger radius. The sign-in form sits in one; dialogs and settings groups can
//! take it as they move to the look.

use egui::{CornerRadius, Frame, Margin, Shadow, Stroke, Ui};

use crate::theme;

/// The room inside a surface.
pub const PADDING: i8 = 28;

/// A surface's frame under `ui`'s theme.
pub fn frame(ui: &Ui) -> Frame {
    let tokens = theme::tokens_of(ui);
    Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1., tokens.surface_border))
        .corner_radius(CornerRadius::same(tokens.radius_surface))
        .inner_margin(Margin::same(PADDING))
        .shadow(Shadow {
            offset: [0, 12],
            blur: 40,
            spread: 0,
            color: tokens.surface_shadow,
        })
}

/// A centred title in the surface's ink, with a quieter line under it: the top of a surface.
pub fn heading(ui: &mut Ui, title: &str, subtitle: &str) {
    let tokens = theme::tokens_of(ui);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(theme::text::font(ui, "2xl"))
                .color(tokens.ink),
        );
        ui.add_space(4.);
        ui.label(
            egui::RichText::new(subtitle)
                .font(theme::text::font(ui, "base"))
                .color(tokens.ink_muted),
        );
    });
}
