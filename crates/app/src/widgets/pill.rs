//! A fully round pill, sunk like an inset: a tag on the item page ([`tag`], small and static) and
//! a value in the library's filters ([`filter`], regular and clickable).

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::icons::{ICON_CHECK, ICON_REMOVE};

use crate::theme::{self, text};
use crate::widgets::focus::{self, Place};

/// A tag's room either side of its text, and above and below it, 1 point of each its outline.
const TAG_PAD_X: f32 = 11.;
const TAG_PAD_Y: f32 = 4.;
/// A filter's room left and right of its contents, and between its icon and text.
const FILTER_INSET: f32 = 12.;
const ICON_GAP: f32 = 4.;
/// A filter's room above and below its text at the larger text sizes, and its least height.
const FILTER_PAD_Y: f32 = 7.;
const FILTER_MIN_H: f32 = 28.;

/// Where a value stands in a filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Off,
    Included,
    Excluded,
}

/// The shape both sizes share.
fn shape(ui: &Ui, rect: Rect, fill: Color32, stroke: Stroke) {
    let radius = CornerRadius::same((rect.height() / 2.).round() as u8);
    ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
}

/// How wide [`tag`] draws `name`, so a wrapping row can start a new line before a pill that
/// wouldn't fit.
pub fn tag_width(ui: &Ui, name: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_owned(), text::font(ui, "xs"), Color32::WHITE);
    galley.size().x + 2. * TAG_PAD_X
}

/// A tag: `inset`, a 1 pt `surface_border` outline, its text `xs` in `ink_muted`.
pub fn tag(ui: &mut Ui, name: &str) -> Response {
    let tokens = theme::tokens_of(ui);
    let font = text::font(ui, "xs");
    let height = ui.fonts_mut(|fonts| fonts.row_height(&font)) + 2. * TAG_PAD_Y;
    let (rect, response) = ui.allocate_exact_size(vec2(tag_width(ui, name), height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, name));
    shape(ui, rect, tokens.inset, Stroke::new(1., tokens.surface_border));
    let at = pos2(rect.left() + TAG_PAD_X, rect.center().y);
    ui.painter().text(at, Align2::LEFT_CENTER, name, font, tokens.ink_muted);
    response
}

/// A value to filter by: sunk in `inset` with the field's outline while off, filled `brand` with
/// a tick while included, and outlined in `danger` with a minus while excluded. `highlighted`
/// rings it for the keyboard. A name too long for `ui` ends in an ellipsis.
pub fn filter(ui: &mut Ui, label: &str, mark: Mark, highlighted: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let font = text::font(ui, "sm");
    let icon = match mark {
        Mark::Off => None,
        Mark::Included => Some(ICON_CHECK),
        Mark::Excluded => Some(ICON_REMOVE),
    };
    let icon_font = text::icon(ui, ICON_CHECK.font_family());
    let icon_w = icon.map_or(0., |_| icon_font.size + ICON_GAP);
    let painter = ui.painter().clone();
    let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), tokens.ink);
    let height = (font.size + 2. * FILTER_PAD_Y).max(FILTER_MIN_H);
    let max_w = ui.available_width().max(height);
    let width = (galley.size().x + icon_w + 2. * FILTER_INSET).min(max_w);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::CLICK);
    response.widget_info(|| {
        let state = match mark {
            Mark::Off => "",
            Mark::Included => ", included",
            Mark::Excluded => ", excluded",
        };
        WidgetInfo::labeled(Role::Button, true, format!("{label}{state}"))
    });

    // Under the pointer, the inset ones take `inset_hover` and the included one `brand_hover`.
    let hovered = response.hovered();
    let inset = if hovered { tokens.inset_hover } else { tokens.inset };
    let (fill, stroke, color) = match mark {
        Mark::Off => (inset, Stroke::new(1., tokens.surface_border), tokens.ink),
        Mark::Included => (
            if hovered { tokens.brand_hover } else { tokens.brand },
            Stroke::NONE,
            tokens.brand_foreground,
        ),
        Mark::Excluded => (inset, Stroke::new(1., tokens.danger), tokens.danger),
    };
    shape(ui, rect, fill, stroke);
    if highlighted {
        focus::ring(ui, rect, height / 2., Place::Outside);
    }

    let mut x = rect.left() + FILTER_INSET;
    if let Some(icon) = icon {
        painter.text(
            pos2(x + icon_font.size / 2., rect.center().y),
            Align2::CENTER_CENTER,
            icon.codepoint,
            icon_font.clone(),
            color,
        );
        x += icon_w;
    }
    let mut job = LayoutJob::single_section(
        label.to_owned(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width((rect.right() - FILTER_INSET - x).max(0.));
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.), galley, color);
    response
}
