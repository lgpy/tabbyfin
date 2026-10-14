//! Server image loading with an in-memory and on-disk cache.
//!
//! Views call [`ImageCache::get`] while rendering. A miss starts a background load (disk first,
//! then network) and returns `None`. When the load finishes the cache notifies its observers,
//! so views that render images observe the global cache entity.
//!
//! While an image loads, its blurhash (from the item's `ImageBlurHashes`) is drawn instead:
//! a tiny blurred preview decoded in memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::*;
use jellyfin::{Client, ImageRequest};

use crate::runtime;

pub struct ImageCache {
    client: Client,
    dir: PathBuf,
    entries: HashMap<ImageRequest, Entry>,
    /// Decoded blurhashes, by hash.
    placeholders: HashMap<String, Option<Arc<RenderImage>>>,
}

/// Blurhash previews are decoded at this size and scaled up (they're blurry by design).
const PLACEHOLDER_SIZE: u32 = 32;

enum Entry {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

struct GlobalImageCache(Entity<ImageCache>);

impl Global for GlobalImageCache {}

impl ImageCache {
    /// Installs the cache for a signed-in session, replacing any previous one.
    pub fn init(client: Client, cx: &mut App) {
        let cache = cx.new(|_| ImageCache {
            client,
            dir: crate::config::cache_dir().join("images"),
            entries: HashMap::new(),
            placeholders: HashMap::new(),
        });
        cx.set_global(GlobalImageCache(cache));
    }

    pub fn global(cx: &App) -> Entity<ImageCache> {
        cx.global::<GlobalImageCache>().0.clone()
    }

    /// Drops loaded images from memory. They reload from disk or the server when next shown.
    pub fn clear_memory(&mut self) {
        self.entries.retain(|_, entry| matches!(entry, Entry::Loading));
    }

    /// The blurred preview for a blurhash, or `None` if it doesn't decode.
    pub fn placeholder(&mut self, hash: &str) -> Option<Arc<RenderImage>> {
        self.placeholders
            .entry(hash.to_owned())
            .or_insert_with(|| decode_blurhash(hash))
            .clone()
    }

    /// Returns the image if it's loaded, otherwise starts loading it and returns `None`.
    pub fn get(&mut self, req: &ImageRequest, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        match self.entries.get(req) {
            Some(Entry::Ready(image)) => return Some(image.clone()),
            Some(Entry::Loading | Entry::Failed) => return None,
            None => {}
        }
        self.entries.insert(req.clone(), Entry::Loading);

        let client = self.client.clone();
        let path = cache_path(&self.dir, req);
        let req = req.clone();
        cx.spawn(async move |this, cx| {
            let fetch = req.clone();
            let result = runtime::spawn(async move { load(&client, &fetch, &path).await }).await;
            this.update(cx, |this, cx| {
                let entry = match result {
                    Ok(image) => Entry::Ready(Arc::new(image)),
                    Err(err) => {
                        tracing::warn!(item = %req.item_id, "image load failed: {err:#}");
                        Entry::Failed
                    }
                };
                this.entries.insert(req, entry);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }
}

fn decode_blurhash(hash: &str) -> Option<Arc<RenderImage>> {
    let size = PLACEHOLDER_SIZE;
    let mut pixels = blurhash::decode(hash, size, size, 1.).ok()?;
    // GPUI wants BGRA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(size, size, pixels)?;
    Some(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
        buffer
    )])))
}

async fn load(client: &Client, req: &ImageRequest, path: &Path) -> anyhow::Result<Image> {
    if let Ok(bytes) = tokio::fs::read(path).await
        && let Some(format) = sniff_format(&bytes)
    {
        return Ok(Image::from_bytes(format, bytes));
    }
    let bytes = client.image(req).await?;
    let format = sniff_format(&bytes).context("unrecognized image format")?;
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    if let Err(err) = tokio::fs::write(path, &bytes).await {
        tracing::debug!("couldn't cache image at {}: {err}", path.display());
    }
    Ok(Image::from_bytes(format, bytes))
}

/// The image tag changes whenever the image does, so it's part of the file name and stale
/// entries are simply never read again.
fn cache_path(dir: &Path, req: &ImageRequest) -> PathBuf {
    let shard = req.item_id.get(..2).unwrap_or("xx");
    let name = format!(
        "{}-{}-{}-{}-{}x{}",
        req.item_id,
        req.image_type.as_str().unwrap_or("Other"),
        req.index.unwrap_or(0),
        req.tag.as_deref().unwrap_or("none"),
        req.fill_width.unwrap_or(0),
        req.fill_height.unwrap_or(0),
    );
    dir.join(shard).join(name)
}

fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        _ => None,
    }
}
