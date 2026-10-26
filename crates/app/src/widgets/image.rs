//! A server image filling its space, from I1's cache: cover fit (cropped to the space, never
//! stretched), rounded corners, and its blurhash underneath while it loads.
//!
//! The caller owns the space (a card allocates its own rectangle) and the request, so the widget
//! only draws. Every request carries the size it's drawn at ([`ImageRequest::fill_width`] in
//! [`ImageCache::get`]'s key), which keeps the download small and the cache entry's own.

use egui::{Color32, CornerRadius, Rect, Ui, Vec2};
use jellyfin::ImageRequest;
use jellyfin::models::BaseItemDto;

use crate::images::ImageCache;

/// The cards' rounded corners, `main`'s `rounded_xl`.
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
    let painter = ui.painter();
    // What shows before the blurhash arrives, or when there's no image at all.
    painter.rect_filled(rect, radius, ui.visuals().faint_bg_color);
    let Some(request) = request else { return };
    // Twice the drawn width, so it stays sharp on a high-DPI screen; the server doesn't upscale.
    let request = request.clone().fill_width((rect.width() * 2.).ceil() as u32);
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
    if let Some(texture) = image.as_ref().or(preview.as_ref()) {
        egui::Image::from_texture(texture)
            .uv(cover_uv(texture.size_vec2(), rect.size()))
            .corner_radius(CornerRadius::same(radius))
            .paint_at(ui, rect);
    }
}

/// Darkens what's drawn in `rect` while the pointer is on it, as `main`'s cards did on hover.
pub fn dim(ui: &Ui, rect: Rect, radius: u8) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius), Color32::from_black_alpha(51));
}

/// The part of a texture to show so it fills `target` without stretching it: the widest or
/// tallest centred band of the texture that has `target`'s aspect ratio.
fn cover_uv(image: Vec2, target: Vec2) -> Rect {
    let scale = (target.x / image.x).max(target.y / image.y);
    let shown = target / scale;
    Rect::from_center_size(
        egui::pos2(0.5, 0.5),
        Vec2::new((shown.x / image.x).clamp(0., 1.), (shown.y / image.y).clamp(0., 1.)),
    )
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2, pos2};

    use super::cover_uv;

    fn uv(size: [f32; 2], target: [f32; 2]) -> Rect {
        cover_uv(Vec2::from(size), Vec2::from(target))
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
    fn cover_fit_leaves_a_matching_box_whole() {
        let same = uv([300., 450.], [150., 225.]);
        assert_eq!(same, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)));
        // A wider box than the picture: the picture fills its height and loses its sides.
        let wide = uv([300., 450.], [150., 300.]);
        assert!((wide.height() - 1.).abs() < 1e-6);
        assert!((wide.width() - 0.75).abs() < 1e-6);
    }
}
