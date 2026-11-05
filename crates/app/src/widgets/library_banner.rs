//! A library as a banner, the Libraries page's: its picture across the column, its name and how
//! much it holds over a shade at the bottom. A library without a picture is a surface with its
//! kind's icon large at the right.

use egui::accesskit::Role;
use egui::epaint::RectShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2,
    vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_FOLDER, ICON_HOME_STORAGE, ICON_LIBRARY_MUSIC, ICON_MOVIE, ICON_PLAYLIST_PLAY, ICON_TV, ICON_VIDEO_LIBRARY,
};
use jellyfin::models::{BaseItemDto, CollectionType};

use crate::images::ImageCache;
use crate::theme;
use crate::theme::text;
use crate::widgets::image;
use crate::widgets::round_button::glyph;
use crate::widgets::text::landscape_image;

/// Width over height.
const ASPECT: f32 = 2.4;
const RADIUS: u8 = 16;
/// The text's inset from the banner's left and bottom edges.
const INSET: f32 = 16.;
const BOTTOM: f32 = 14.;
/// How much of the banner, from the bottom, the shade under the text covers.
const SHADE: f32 = 0.7;
/// The kind's icon beside the count, and its share of the line.
const SMALL_ICON: f32 = 16.;
const SMALL_ICON_W: f32 = 22.;
/// The icon standing in for the picture.
const BIG_ICON: f32 = 56.;

/// The banner's height at `width`.
pub fn height(width: f32) -> f32 {
    (width / ASPECT).round()
}

/// Draws `view` as a banner `width` wide, with `count` (what it holds, "1,284 movies") under its
/// name once it's known. The response is the banner's, a button named by the library.
pub fn show(ui: &mut Ui, cache: &mut ImageCache, view: &BaseItemDto, count: Option<&str>, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height(width)), Sense::click());
    let name = view.name.clone();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
    let tokens = theme::tokens_of(ui);
    let request = landscape_image(view);
    let (ink, muted) = match &request {
        Some(request) => {
            image::show(ui, cache, rect, Some(request), Some(view), RADIUS);
            let shade = Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * SHADE), rect.max);
            let radius = CornerRadius {
                nw: 0,
                ne: 0,
                sw: RADIUS,
                se: RADIUS,
            };
            let uv = Rect::from_min_max(pos2(0., 0.), pos2(1., 1.));
            ui.painter()
                .add(RectShape::filled(shade, radius, Color32::WHITE).with_texture(shade_texture(ui.ctx()).id(), uv));
            (Color32::WHITE, Color32::from_white_alpha(210))
        }
        None => {
            ui.painter().rect(
                rect,
                RADIUS,
                tokens.surface,
                Stroke::new(1., tokens.surface_border),
                StrokeKind::Inside,
            );
            let center = pos2(rect.right() - INSET - BIG_ICON / 2., rect.center().y);
            glyph(ui, center, icon(view).outlined(), BIG_ICON, tokens.brand);
            (tokens.ink, tokens.ink_muted)
        }
    };
    if response.hovered() || response.is_pointer_button_down_on() {
        image::dim(ui, rect, RADIUS);
    }

    // The name above the count line, both placed by their fonts' heights, so the name stays put
    // when the count arrives.
    let name_font = text::font(ui, "xl");
    let count_font = text::font(ui, "sm");
    let (name_h, count_h) = ui.fonts_mut(|fonts| (fonts.row_height(&name_font), fonts.row_height(&count_font)));
    let left = rect.left() + INSET;
    let inner = rect.width() - 2. * INSET;
    let count_y = rect.bottom() - BOTTOM - count_h;
    let name_y = count_y - 2. - name_h;
    let line = |text: &str, font, color, width| {
        let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = TextWrapping::truncate_at_width(width);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let name = line(&view.name, name_font, ink, inner);
    let count = count.map(|count| line(count, count_font, muted, inner - SMALL_ICON_W));
    let painter = ui.painter();
    painter.galley(pos2(left, name_y), name, ink);
    let icon_font = egui::FontId::new(SMALL_ICON, text::icon(ui, icon(view).font_family()).family);
    painter.text(
        pos2(left + SMALL_ICON / 2., count_y + count_h / 2.),
        Align2::CENTER_CENTER,
        icon(view).codepoint,
        icon_font,
        muted,
    );
    if let Some(count) = count {
        painter.galley(pos2(left + SMALL_ICON_W, count_y), count, muted);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A black ramp, clear at its top: the shade under the text on a picture. Made once per context.
fn shade_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = Id::unique("library_banner_shade");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let pixels = (0..64)
        .map(|y| Color32::from_black_alpha(((y as f32 / 63.).powf(1.4) * 200.) as u8))
        .collect();
    let texture = ctx.load_texture(
        "library_banner_shade",
        egui::ColorImage::new([1, 64], pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// The library's kind as an icon.
fn icon(view: &BaseItemDto) -> MaterialIcon {
    match view.collection_type {
        Some(CollectionType::Movies) => ICON_MOVIE,
        Some(CollectionType::TvShows) => ICON_TV,
        Some(CollectionType::Music) => ICON_LIBRARY_MUSIC,
        Some(CollectionType::HomeVideos) => ICON_HOME_STORAGE,
        Some(CollectionType::BoxSets) => ICON_FOLDER,
        Some(CollectionType::Playlists) => ICON_PLAYLIST_PLAY,
        _ => ICON_VIDEO_LIBRARY,
    }
}
