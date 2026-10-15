//! Seek-bar preview thumbnails from the server's trickplay tiles.
//!
//! The server stores thumbnails in JPEG sheets (by default 10×10 thumbnails of 320 px, 23 MB
//! decoded). A sheet is fetched when the seek bar is hovered over a time it covers, cropped
//! into one image per thumbnail off the UI thread, and kept while it's among the last
//! [`KEEP_SHEETS`] used. Everything is freed, texture atlas included, with the player.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use jellyfin::Client;
use jellyfin::models::TrickplayInfo;

use crate::runtime;

/// Decoded sheets kept at a time: the one under the mouse and its neighbour.
const KEEP_SHEETS: usize = 2;

pub struct Trickplay {
    client: Client,
    item_id: String,
    media_source_id: String,
    info: TrickplayInfo,
    sheets: HashMap<u32, Sheet>,
    /// Sheet indexes, least recently used first.
    order: Vec<u32>,
}

enum Sheet {
    Loading,
    /// One image per thumbnail, in order.
    Ready(Vec<Arc<RenderImage>>),
    Failed,
}

impl Trickplay {
    pub fn new(
        client: Client,
        item_id: String,
        media_source_id: String,
        info: TrickplayInfo,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|this: &mut Self, cx| {
            let images = this.sheets.drain().flat_map(|(_, sheet)| match sheet {
                Sheet::Ready(images) => images,
                Sheet::Loading | Sheet::Failed => Vec::new(),
            });
            release(images.collect(), cx);
        })
        .detach();
        Self {
            client,
            item_id,
            media_source_id,
            info,
            sheets: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Width over height of a thumbnail.
    pub fn aspect_ratio(&self) -> f32 {
        self.info.width as f32 / self.info.height.max(1) as f32
    }

    /// The thumbnail for `position`, or `None` while its sheet loads (it notifies when done).
    pub fn thumbnail(&mut self, position: Duration, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let (sheet, slot) = locate(&self.info, position)?;
        self.order.retain(|&i| i != sheet);
        self.order.push(sheet);
        match self.sheets.get(&sheet) {
            Some(Sheet::Ready(images)) => return images.get(slot).cloned(),
            Some(Sheet::Loading | Sheet::Failed) => return None,
            None => {}
        }
        self.sheets.insert(sheet, Sheet::Loading);
        self.evict(cx);

        let (client, item_id, source_id, info) = (
            self.client.clone(),
            self.item_id.clone(),
            self.media_source_id.clone(),
            self.info,
        );
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move {
                let bytes = client.trickplay_sheet(&item_id, &source_id, info.width, sheet).await?;
                let images = tokio::task::spawn_blocking(move || crop(&bytes, &info, sheet)).await??;
                anyhow::Ok(images)
            })
            .await;
            this.update(cx, |this, cx| {
                let loaded = match result {
                    Ok(images) => Sheet::Ready(images),
                    Err(err) => {
                        tracing::warn!("couldn't load trickplay sheet {sheet}: {err:#}");
                        Sheet::Failed
                    }
                };
                // Evicted meanwhile: the images aren't wanted any more.
                if !this.sheets.contains_key(&sheet) {
                    if let Sheet::Ready(images) = loaded {
                        release(images, cx);
                    }
                    return;
                }
                this.sheets.insert(sheet, loaded);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    fn evict(&mut self, cx: &mut App) {
        while self.order.len() > KEEP_SHEETS {
            let oldest = self.order.remove(0);
            if let Some(Sheet::Ready(images)) = self.sheets.remove(&oldest) {
                release(images, cx);
            }
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

/// Decodes sheet `sheet` and cuts it into its thumbnails (the last sheet may be partly empty).
fn crop(bytes: &[u8], info: &TrickplayInfo, sheet: u32) -> anyhow::Result<Vec<Arc<RenderImage>>> {
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
        let mut thumbnail = image::imageops::crop_imm(&image, x, y, info.width, info.height).to_image();
        // GPUI wants BGRA.
        for pixel in thumbnail.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        thumbnails.push(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            thumbnail
        )])));
    }
    Ok(thumbnails)
}

/// Frees thumbnails from the windows' texture atlases, after the current update.
fn release(images: Vec<Arc<RenderImage>>, cx: &mut App) {
    if images.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for image in images {
            cx.drop_image(image, None);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jellyfin::models::TrickplayInfo;

    use super::locate;

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
}
