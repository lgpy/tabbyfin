//! Seek-bar preview thumbnails from the server's trickplay tiles.
//!
//! The server stores thumbnails in JPEG sheets (by default 10×10 thumbnails of 320 px, 23 MB
//! decoded). A sheet is fetched when the seek bar is hovered over a time it covers, cropped
//! into one image per thumbnail off the UI thread, and kept while it's among the last
//! [`KEEP_SHEETS`] used. Dropping the player page drops it too: a sheet's download is cancelled,
//! and its textures are freed. A thumbnail bigger than the device's largest texture is scaled
//! down as it's cropped.

use std::collections::HashMap;
use std::time::Duration;

use egui::{ColorImage, TextureHandle, TextureOptions};
use jellyfin::Client;
use jellyfin::models::TrickplayInfo;

use crate::runtime;
use crate::task::{self, Receiver, Sender};

/// Decoded sheets kept at a time: the one under the mouse and its neighbour.
const KEEP_SHEETS: usize = 2;

/// Thumbnails are shown small and never magnified much, so linear filtering is enough.
const TEXTURE_OPTIONS: TextureOptions = TextureOptions::LINEAR;

pub struct Trickplay {
    client: Client,
    item_id: String,
    media_source_id: String,
    info: TrickplayInfo,
    sheets: HashMap<u32, Sheet>,
    /// Sheet indexes, least recently used first.
    order: Vec<u32>,
    ctx: egui::Context,
    /// Cropped sheets, for [`Trickplay::poll`] to upload as textures.
    loaded: Receiver<(u32, anyhow::Result<Vec<ColorImage>>)>,
    loading_from: Sender<(u32, anyhow::Result<Vec<ColorImage>>)>,
}

enum Sheet {
    Loading,
    /// One texture per thumbnail, in order. A sheet as a single texture with a UV rect per
    /// thumbnail would be one upload instead of up to a hundred, but a sheet can be wider than
    /// the device's largest texture (10×10 tiles of 320 px is 3200 px), and the tiles as their own
    /// textures always fit.
    Ready(Vec<TextureHandle>),
    Failed,
}

impl Trickplay {
    pub fn new(
        client: Client,
        item_id: String,
        media_source_id: String,
        info: TrickplayInfo,
        ctx: &egui::Context,
    ) -> Self {
        let (loading_from, loaded) = task::channel(ctx);
        Self {
            client,
            item_id,
            media_source_id,
            info,
            sheets: HashMap::new(),
            order: Vec::new(),
            ctx: ctx.clone(),
            loaded,
            loading_from,
        }
    }

    /// Width over height of a thumbnail.
    pub fn aspect_ratio(&self) -> f32 {
        self.info.width as f32 / self.info.height.max(1) as f32
    }

    /// Turns the sheets that finished loading into textures. Call it each frame while the
    /// previews can be shown; [`Trickplay::thumbnail`] calls it too, so a page that only asks for
    /// thumbnails while hovering still gets its sheet, because the load's completion repaints the
    /// app by itself.
    pub fn poll(&mut self) {
        let completed: Vec<_> = self.loaded.drain().collect();
        for (sheet, result) in completed {
            // Evicted meanwhile: the thumbnails aren't wanted any more.
            if !self.sheets.contains_key(&sheet) {
                continue;
            }
            let loaded = match result {
                Ok(images) => Sheet::Ready(
                    images
                        .into_iter()
                        .enumerate()
                        .map(|(slot, image)| {
                            let name = format!("trickplay:{}:{sheet}:{slot}", self.item_id);
                            self.ctx.load_texture(name, image, TEXTURE_OPTIONS)
                        })
                        .collect(),
                ),
                Err(err) => {
                    tracing::warn!("couldn't load trickplay sheet {sheet}: {err:#}");
                    Sheet::Failed
                }
            };
            self.sheets.insert(sheet, loaded);
        }
    }

    /// The thumbnail for `position`, or `None` while its sheet loads (see [`Trickplay::poll`]).
    pub fn thumbnail(&mut self, position: Duration) -> Option<TextureHandle> {
        self.poll();
        let (sheet, slot) = locate(&self.info, position)?;
        self.order.retain(|&i| i != sheet);
        self.order.push(sheet);
        match self.sheets.get(&sheet) {
            Some(Sheet::Ready(images)) => return images.get(slot).cloned(),
            Some(Sheet::Loading | Sheet::Failed) => return None,
            None => {}
        }
        self.sheets.insert(sheet, Sheet::Loading);
        self.evict();
        self.start_load(sheet);
        None
    }

    fn start_load(&mut self, sheet: u32) {
        let (client, item_id, source_id, info) = (
            self.client.clone(),
            self.item_id.clone(),
            self.media_source_id.clone(),
            self.info,
        );
        // The device's largest texture, read where it's known: the crops have to fit into it.
        let max_side = self.ctx.input(|input| input.max_texture_side);
        let sender = self.loading_from.clone();
        let closed = sender.clone();
        runtime::detach(async move {
            let work = async {
                let bytes = client.trickplay_sheet(&item_id, &source_id, info.width, sheet).await?;
                tokio::task::spawn_blocking(move || crop(&bytes, &info, sheet, max_side)).await?
            };
            let result = tokio::select! {
                result = work => Some(result),
                // The player page is gone: the thumbnails aren't wanted any more.
                _ = closed.closed() => None,
            };
            if let Some(result) = result {
                let _ = sender.send((sheet, result));
            }
        });
    }

    fn evict(&mut self) {
        while self.order.len() > KEEP_SHEETS {
            let oldest = self.order.remove(0);
            self.sheets.remove(&oldest);
        }
    }
}

/// Which sheet holds the thumbnail for `position`, and its place in the sheet.
fn locate(info: &TrickplayInfo, position: Duration) -> Option<(u32, usize)> {
    if info.thumbnail_count == 0 || info.interval == 0 {
        return None;
    }
    let per_sheet = info.tile_width * info.tile_height;
    if per_sheet == 0 {
        return None;
    }
    let index = (position.as_millis() / u128::from(info.interval)).min(u128::from(info.thumbnail_count - 1)) as u32;
    Some((index / per_sheet, (index % per_sheet) as usize))
}

/// Decodes sheet `sheet` and cuts it into its thumbnails (the last sheet may be partly empty),
/// each fitted into a texture `max_side` pixels wide and tall at most.
fn crop(bytes: &[u8], info: &TrickplayInfo, sheet: u32, max_side: usize) -> anyhow::Result<Vec<ColorImage>> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)?.into_rgba8();
    let per_sheet = info.tile_width * info.tile_height;
    let count = info.thumbnail_count.saturating_sub(sheet * per_sheet).min(per_sheet);
    let mut thumbnails = Vec::with_capacity(count as usize);
    for slot in 0..count {
        let (x, y) = (
            (slot % info.tile_width) * info.width,
            (slot / info.tile_width) * info.height,
        );
        if x + info.width > image.width() || y + info.height > image.height() {
            break;
        }
        let thumbnail = image::imageops::crop_imm(&image, x, y, info.width, info.height).to_image();
        let thumbnail = crate::images::fit(thumbnail, max_side);
        let size = [thumbnail.width() as usize, thumbnail.height() as usize];
        thumbnails.push(ColorImage::from_rgba_unmultiplied(size, thumbnail.as_raw()));
    }
    Ok(thumbnails)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::ImageEncoder as _;
    use jellyfin::models::TrickplayInfo;

    use super::{crop, locate};

    fn info(thumbnail_count: u32) -> TrickplayInfo {
        TrickplayInfo {
            width: 320,
            height: 180,
            tile_width: 10,
            tile_height: 10,
            thumbnail_count,
            interval: 10_000,
        }
    }

    #[test]
    fn positions_map_to_sheets_and_slots() {
        let info = info(250);
        assert_eq!(locate(&info, Duration::ZERO), Some((0, 0)));
        assert_eq!(locate(&info, Duration::from_secs(19)), Some((0, 1)));
        assert_eq!(locate(&info, Duration::from_secs(1000)), Some((1, 0)));
        assert_eq!(locate(&info, Duration::from_secs(2_345)), Some((2, 34)));
        // Past the last thumbnail: the last one.
        assert_eq!(locate(&info, Duration::from_secs(99_999)), Some((2, 49)));
    }

    #[test]
    fn no_thumbnails_no_location() {
        assert_eq!(locate(&info(0), Duration::from_secs(5)), None);
    }

    #[test]
    fn a_sheet_is_cut_into_its_thumbnails() {
        // A 2×2 sheet of 8×8 tiles: red, green, blue, and white, which the third thumbnail
        // doesn't use (the count is 3).
        let tiles = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
        let mut pixels = Vec::with_capacity(16 * 16 * 3);
        for y in 0..16 {
            for x in 0..16 {
                pixels.extend_from_slice(&tiles[(y / 8) * 2 + x / 8]);
            }
        }
        let sheet = image::RgbImage::from_raw(16, 16, pixels).expect("16x16");
        let mut jpeg = Vec::new();
        // JPEG has no alpha, so the sheet is RGB (the crops come out RGBA anyway).
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .write_image(sheet.as_raw(), 16, 16, image::ExtendedColorType::Rgb8)
            .expect("encode");
        let info = TrickplayInfo {
            width: 8,
            height: 8,
            tile_width: 2,
            tile_height: 2,
            thumbnail_count: 3,
            interval: 10_000,
        };
        let thumbnails = crop(&jpeg, &info, 0, 4096).expect("crop");
        assert_eq!(thumbnails.len(), 3);
        let middle = |thumbnail: &egui::ColorImage| thumbnail.pixels[8 * 4 + 4];
        let near = |pixel: egui::Color32, [red, green, blue]: [u8; 3]| {
            pixel.r().abs_diff(red) < 12 && pixel.g().abs_diff(green) < 12 && pixel.b().abs_diff(blue) < 12
        };
        for (thumbnail, tile) in thumbnails.iter().zip(tiles) {
            assert_eq!(thumbnail.size, [8, 8]);
            assert!(near(middle(thumbnail), tile), "{:?} isn't {tile:?}", middle(thumbnail));
        }
    }
}
