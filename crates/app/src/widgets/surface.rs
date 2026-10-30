//! The cozy look's raised panel: a [`theme::Tokens::surface`] lifted off the backdrop by a soft
//! shadow, with the larger radius. The sign-in form sits in one; dialogs and settings groups can
//! take it as they move to the look.

use egui::accesskit::Role;
use egui::{CornerRadius, Frame, Margin, Rect, Sense, Shadow, Spinner, Stroke, Ui, Vec2, WidgetInfo, pos2, vec2};

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

/// A small spinner and a muted line beside it, centred together on the surface's width: what the
/// surface is waiting for ("Waiting for approval…"). `ui.horizontal` would start the pair at the
/// left edge, so it's laid out by hand.
pub fn waiting(ui: &mut Ui, line: &str) {
    let tokens = theme::tokens_of(ui);
    let galley = ui
        .painter()
        .layout_no_wrap(line.to_owned(), theme::text::font(ui, "sm"), tokens.ink_muted);
    let (spinner, gap) = (14., 8.);
    let width = spinner + gap + galley.size().x;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y.max(spinner)), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, line));
    let left = rect.center().x - width / 2.;
    Spinner::new().size(spinner).color(tokens.ink_muted).paint_at(
        ui,
        Rect::from_center_size(pos2(left + spinner / 2., rect.center().y), Vec2::splat(spinner)),
    );
    ui.painter().galley(
        pos2(left + spinner + gap, rect.center().y - galley.size().y / 2.),
        galley,
        tokens.ink_muted,
    );
}
