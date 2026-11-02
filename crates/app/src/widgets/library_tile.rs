//! A library as a small surface: its picture as a thumbnail (or its kind's icon when it has
//! none) and its name. Home's *My Media* row is these rather than image cards, so the libraries
//! read as places to go rather than things to watch (C9, picked by a mockup).

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, CursorIcon, Rect, Response, Sense, Stroke, StrokeKind, TextFormat, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::icons::{ICON_MOVIE, ICON_PLAYLIST_PLAY, ICON_TV, ICON_VIDEO_LIBRARY};
use jellyfin::models::{BaseItemDto, CollectionType};

use crate::images::ImageCache;
use crate::theme;
use crate::theme::text;
use crate::widgets::focus::{self, Place};
use crate::widgets::image;
use crate::widgets::round_button::glyph;
use crate::widgets::text::landscape_image;

/// The tile's size.
pub const SIZE: egui::Vec2 = egui::vec2(236., 76.);
/// The tile's corners: a popup's, between a panel's and a control's.
const RADIUS: u8 = 16;
/// The thumbnail's inset from the tile's edge, and its size and corners.
const INSET: f32 = 10.;
const THUMB: egui::Vec2 = egui::vec2(96., 56.);
const THUMB_RADIUS: u8 = 10;
/// Between the thumbnail and the name.
const GAP: f32 = 14.;
/// Room left under the tile for its shadow, which the shelf's scroll area would otherwise clip.
const SHADOW_ROOM: f32 = 16.;

/// Draws `view` as a tile, with the keyboard cursor's ring when `selected`. The response is the
/// tile's, a button named by the library to AccessKit.
pub fn show(ui: &mut Ui, cache: &mut ImageCache, view: &BaseItemDto, selected: bool) -> Response {
    let (outer, response) = ui.allocate_exact_size(vec2(SIZE.x, SIZE.y + SHADOW_ROOM), Sense::click());
    let rect = Rect::from_min_size(outer.min, SIZE);
    let name = view.name.clone();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
    let tokens = theme::tokens_of(ui);
    let hovered = response.hovered() && ui.rect_contains_pointer(rect);
    let painter = ui.painter();
    painter.add(theme::popup_shadow(tokens).as_shape(rect, RADIUS));
    // Under the pointer the outline darkens, as a select's does.
    let outline = if hovered {
        tokens.ink_muted
    } else {
        tokens.surface_border
    };
    painter.rect(
        rect,
        RADIUS,
        tokens.surface,
        Stroke::new(1., outline),
        StrokeKind::Inside,
    );

    let thumb = Rect::from_min_size(rect.min + vec2(INSET, INSET), THUMB);
    match landscape_image(view) {
        Some(request) => image::show(ui, cache, thumb, Some(&request), Some(view), THUMB_RADIUS),
        None => {
            painter.rect_filled(thumb, THUMB_RADIUS, tokens.inset_hover);
            glyph(ui, thumb.center(), icon(view).outlined(), 26., tokens.brand);
        }
    }

    // The name on one line, cut short with an ellipsis when it doesn't fit.
    let left = thumb.right() + GAP;
    let mut job = LayoutJob::single_section(
        view.name.clone(),
        TextFormat {
            font_id: text::font(ui, "base"),
            color: tokens.ink,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width(rect.right() - INSET - left);
    let galley = ui.painter().layout_job(job);
    let at = Align2::LEFT_CENTER.anchor_size(pos2(left, rect.center().y), galley.size());
    ui.painter().galley(at.min, galley, tokens.ink);

    if selected {
        focus::ring(ui, rect, RADIUS as f32, Place::Inside);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The icon standing in for a library without a picture.
fn icon(view: &BaseItemDto) -> egui_material_icons::MaterialIcon {
    match view.collection_type {
        Some(CollectionType::Movies) => ICON_MOVIE,
        Some(CollectionType::TvShows) => ICON_TV,
        Some(CollectionType::Playlists) => ICON_PLAYLIST_PLAY,
        _ => ICON_VIDEO_LIBRARY,
    }
}
