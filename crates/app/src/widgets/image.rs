//! A server image filling its space, from the image cache: cover fit (cropped to the space,
//! never stretched), rounded corners, and its blurhash underneath while it loads. An image fades
//! in over its blurhash when it shows ([`crate::motion`]).
//!
//! The caller owns the space (a card allocates its own rectangle) and the request, so the widget
//! only draws. Every request carries the size it's drawn at ([`ImageRequest::fill_width`] in
//! [`ImageCache::get`]'s key), which keeps the download small and the cache entry's own; a
//! full-width banner asks a fixed width instead ([`banner`]), so resizing doesn't fetch
//! it again.

use egui::epaint::RectShape;
use egui::{Color32, CornerRadius, Rect, Shape, Ui, Vec2};
use jellyfin::ImageRequest;
use jellyfin::models::BaseItemDto;

use crate::images::ImageCache;
use crate::{motion, theme};

/// The cards' rounded corners.
pub const RADIUS: u8 = 12;

/// Draws `request` filling `rect`: cover-cropped, with `radius` rounded corners (0 for square),
/// over its blurhash while it loads. `blurhash_from` is the item whose blurhash stands in for
/// the image (a chapter's or a person's own image has none).
///
/// Asking every frame is what keeps a queued image near the front of the cache's queue, so
/// call this for whatever is on screen even when nothing is loaded yet.
pub fn show(
    ui: &Ui,
    cache: &mut ImageCache,
    rect: Rect,
    request: Option<&ImageRequest>,
    blurhash_from: Option<&BaseItemDto>,
    radius: u8,
) {
    ui.painter()
        .extend(shapes(ui, cache, rect, request, blurhash_from, radius, None, 0.5));
}

/// How far down a banner taller than its band is cut, as a share of what doesn't fit: 0 keeps
/// its top edge, 0.5 cuts evenly. Backdrops mostly have their subject in the top third, and on a
/// wide window a 16:9 one is about twice the band's height, so an even cut loses the faces;
/// keeping the very top shows sky above them.
const BANNER_ANCHOR: f32 = 0.15;

/// A banner's shapes, for the caller to paint where it wants them (the item page's backdrop,
/// under the top bar: `views::UnderBar`). Square, and asking the server for a fixed `width`
/// rather than twice the drawn width: a banner that fills the window would otherwise be fetched
/// (and kept in the cache) again at every window size, maximised state or zoom level. It is
/// still scaled to `rect`, and a picture taller than the band is cut a little below its top
/// ([`BANNER_ANCHOR`]).
pub fn banner(
    ui: &Ui,
    cache: &mut ImageCache,
    rect: Rect,
    request: Option<&ImageRequest>,
    blurhash_from: Option<&BaseItemDto>,
    width: u32,
) -> Vec<Shape> {
    shapes(ui, cache, rect, request, blurhash_from, 0, Some(width), BANNER_ANCHOR)
}

#[allow(clippy::too_many_arguments)]
fn shapes(
    ui: &Ui,
    cache: &mut ImageCache,
    rect: Rect,
    request: Option<&ImageRequest>,
    blurhash_from: Option<&BaseItemDto>,
    radius: u8,
    width: Option<u32>,
    anchor: f32,
) -> Vec<Shape> {
    let radius = CornerRadius::same(radius);
    // What shows before the blurhash arrives, or when there's no image at all: the grey
    // under a card.
    let mut shapes = vec![Shape::from(RectShape::filled(
        rect,
        radius,
        theme::tokens_of(ui).inset_hover,
    ))];
    let Some(request) = request else { return shapes };
    // Twice the drawn width, so it stays sharp on a high-DPI screen (the server doesn't
    // upscale), or the width the caller fixed.
    let width = width.unwrap_or((rect.width() * 2.).ceil() as u32);
    let request = request.clone().fill_width(width);
    let image = cache.get(&request);
    let preview = match &image {
        // The blurhash is only a guess at what's coming, so it's for while the image loads.
        None => request
            .tag
            .as_deref()
            .and_then(|tag| blurhash_from?.blur_hash(request.image_type, tag))
            .and_then(|hash| cache.placeholder(hash)),
        Some(_) => None,
    };
    let picture = |texture: &egui::TextureHandle, tint: Color32| {
        let uv = cover_uv(texture.size_vec2(), rect.size(), anchor);
        Shape::from(RectShape::filled(rect, radius, tint).with_texture(texture.id(), uv))
    };
    if let Some(texture) = &image {
        // Fading in over the blurhash as it loads, and as its page opens.
        let alpha = motion::image_alpha(ui, &request);
        shapes.push(picture(
            texture,
            Color32::from_white_alpha((alpha * 255.).round() as u8),
        ));
    } else if let Some(texture) = &preview {
        shapes.push(picture(texture, Color32::WHITE));
    }
    shapes
}

/// A texture the caller already has (a downloaded poster, `ImageCache::get_file`) filling
/// `rect`, cover-cropped like [`show`]'s pictures, with `radius` rounded corners.
pub fn texture(ui: &Ui, rect: Rect, texture: &egui::TextureHandle, radius: u8) {
    let uv = cover_uv(texture.size_vec2(), rect.size(), 0.5);
    ui.painter()
        .add(RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE).with_texture(texture.id(), uv));
}

/// Dims what's drawn in `rect` while the pointer is on it: the page's background at 20 % over
/// the picture, so a card dims on a light theme as well as a dark one instead of only darkening.
pub fn dim(ui: &Ui, rect: Rect, radius: u8) {
    let background = theme::tokens_of(ui).backdrop_top;
    let fill = egui::Color32::from_rgba_unmultiplied(background.r(), background.g(), background.b(), 51);
    ui.painter().rect_filled(rect, CornerRadius::same(radius), fill);
}

/// The part of a texture to show so it fills `target` without stretching it: the widest or
/// tallest band of the texture that has `target`'s aspect ratio, centred across, and `anchor`
/// of the way down what's cut off its height (0 keeps the top edge, 0.5 cuts evenly).
fn cover_uv(image: Vec2, target: Vec2, anchor: f32) -> Rect {
    let scale = (target.x / image.x).max(target.y / image.y);
    let shown = target / scale;
    let size = Vec2::new((shown.x / image.x).clamp(0., 1.), (shown.y / image.y).clamp(0., 1.));
    let top = (1. - size.y) * anchor;
    Rect::from_min_size(egui::pos2((1. - size.x) / 2., top), size)
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2, pos2};

    use super::cover_uv;

    fn uv(size: [f32; 2], target: [f32; 2]) -> Rect {
        cover_uv(Vec2::from(size), Vec2::from(target), 0.5)
    }

    #[test]
    fn cover_fit_crops_a_tall_poster_into_a_wide_card() {
        // A 2:3 poster in a 260×146 card keeps its full width and shows the middle band of its
        // height: 146 / 600 of it, once scaled to 260 wide.
        let poster = uv([400., 600.], [260., 146.]);
        assert!((poster.width() - 1.).abs() < 1e-6);
        assert!((poster.height() - 146. / 600. / (260. / 400.)).abs() < 1e-6);
        assert!((poster.center().y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_banner_is_cut_a_little_below_its_top() {
        // A 16:9 backdrop in a wide, short band: the full width, and `BANNER_ANCHOR` of what
        // doesn't fit cut off the top, the rest off the bottom.
        let banner = cover_uv(Vec2::new(1280., 720.), Vec2::new(1280., 400.), super::BANNER_ANCHOR);
        assert!((banner.width() - 1.).abs() < 1e-6);
        assert!((banner.height() - 400. / 720.).abs() < 1e-6);
        let cut = 1. - banner.height();
        assert!((banner.top() - cut * super::BANNER_ANCHOR).abs() < 1e-6);
        assert!((1. - banner.bottom() - cut * (1. - super::BANNER_ANCHOR)).abs() < 1e-6);
    }

    #[test]
    fn a_card_is_cut_evenly() {
        let card = uv([400., 600.], [260., 146.]);
        assert!((card.top() - (1. - card.bottom())).abs() < 1e-6);
    }

    #[test]
    fn cover_fit_leaves_a_matching_box_whole() {
        let same = uv([300., 450.], [150., 225.]);
        assert_eq!(same, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)));
        // A wider box than the picture: the picture fills its height and loses its sides.
        let wide = uv([300., 450.], [150., 300.]);
        assert!((wide.height() - 1.).abs() < 1e-6);
        assert!((wide.width() - 0.75).abs() < 1e-6);
    }
}
