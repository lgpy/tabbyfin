//! The cozy look's raised panel: a [`theme::Tokens::surface`] lifted off the backdrop by a soft
//! shadow, with the larger radius. The sign-in form and the dialogs sit in one; settings groups
//! can take it as they move to the look. Also what goes on one: its heading, a waiting line, a
//! notice, and the gaps between them.

use egui::accesskit::Role;
use egui::{
    CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Shadow, Spinner, Stroke, Ui, Vec2, WidgetInfo, pos2,
    vec2,
};
use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_ERROR, ICON_INFO};

use crate::theme::{self, SPACE, text};

/// The room inside a surface.
pub const PADDING: i8 = 28;

/// The vertical gaps on a surface, named by what they separate so a screen picks one rather
/// than a number. A field's label sits [`SPACE`]`.xxs` above its field (`field::label`).
///
/// Under the [`heading`], before the step's content.
pub const GAP_HEADING: f32 = SPACE.xl;
/// Between groups: the saved accounts and the address under them, the last field and the main
/// button, a code and its waiting line, and the panel and the aside under it.
pub const GAP_GROUP: f32 = SPACE.lg;
/// Between fields, and under a [`notice`] or a [`waiting`] line.
pub const GAP_FIELD: f32 = SPACE.md;
/// Between the items of a list: rows, and buttons stacked under each other.
pub const GAP_ITEM: f32 = SPACE.sm;

/// A surface's frame under `ui`'s theme.
pub fn frame(ui: &Ui) -> Frame {
    frame_of(theme::tokens_of(ui))
}

/// A surface's frame in `tokens`, for what has no `Ui` yet (a dialog's modal).
pub fn frame_of(tokens: &theme::Tokens) -> Frame {
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
    ui.vertical_centered(|ui| heading_lines(ui, title, Some(subtitle)));
}

/// [`heading`] against the surface's left edge, for a surface that isn't a centred form (a
/// dialog), with the quieter line only when there's one to say.
pub fn heading_left(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.vertical(|ui| heading_lines(ui, title, subtitle));
}

fn heading_lines(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    let tokens = theme::tokens_of(ui);
    ui.label(RichText::new(title).font(text::font(ui, "2xl")).color(tokens.ink));
    if let Some(subtitle) = subtitle {
        ui.add_space(SPACE.xs);
        ui.label(
            RichText::new(subtitle)
                .font(text::font(ui, "base"))
                .color(tokens.ink_muted),
        );
    }
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

/// What a [`notice`] tells the user, which picks its colour and icon.
// Sign-in only shows errors so far; Settings and the dialogs take the other kinds as they move.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Something failed ("Wrong username or password.").
    Danger,
    /// Something to know that isn't a failure (a server starting up).
    Info,
    /// Something worked.
    Success,
}

impl Kind {
    /// What a screen reader says for the notice's icon.
    pub fn label(self) -> &'static str {
        match self {
            Self::Danger => "Error",
            Self::Info => "Information",
            Self::Success => "Success",
        }
    }
}

/// A message boxed on the surface, under its heading: tinted and outlined in `kind`'s colour,
/// with that colour's icon before the message. The message itself is in the ink, since the
/// status colours are too pale for text on a light surface.
pub fn notice(ui: &mut Ui, kind: Kind, message: &str) {
    let tokens = theme::tokens_of(ui);
    let (color, icon) = match kind {
        Kind::Danger => (tokens.danger, ICON_ERROR),
        Kind::Info => (tokens.info, ICON_INFO),
        Kind::Success => (tokens.success, ICON_CHECK_CIRCLE),
    };
    let font = text::font(ui, "base");
    Frame::new()
        .fill(color.gamma_multiply(0.1))
        .stroke(Stroke::new(1., color))
        .corner_radius(tokens.radius_control)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE.sm;
                // Painted in a square as tall as the message's first line, so the icon sits on
                // that line whatever the text size, and wrapped lines start under the message.
                let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(line), Sense::hover());
                response.widget_info(|| WidgetInfo::labeled(Role::Image, true, kind.label()));
                let glyph_font = FontId::new(text::font(ui, "lg").size, icon.font_family());
                let galley = ui
                    .painter()
                    .layout_no_wrap(icon.codepoint.to_owned(), glyph_font, color);
                // The icon font's glyphs sit low in their line, so they're centred by their bounds.
                let glyph = galley.rows.first().map_or(galley.rect, |row| row.rect());
                ui.painter()
                    .galley(rect.center() - glyph.center().to_vec2(), galley, color);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(message).font(font).color(tokens.ink));
                });
            });
        });
}
