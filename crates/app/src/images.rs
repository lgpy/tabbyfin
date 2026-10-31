//! Server images as egui textures: a disk cache, a memory budget by decoded bytes, a queue that
//! loads what's on screen first, and blurhash placeholders.
//!
//! Views call [`ImageCache::get`] while drawing. A miss starts a background load (disk first,
//! then network) and returns `None`; the load's completion repaints the app, and the next frame
//! gets the texture. While an image loads, its blurhash (from the item's `ImageBlurHashes`) is
//! drawn instead: a tiny blurred preview decoded in memory ([`ImageCache::placeholder`]).
//!
//! Images are decoded here rather than through egui's texture loaders, which have no memory
//! budget, no queue that drops what scrolled past, and no disk cache or blurhash. Memory is
//! bounded by [`MEMORY_BUDGET`]: past it, the images used longest ago are dropped, which frees
//! their textures, and they reload from disk when shown again.
//!
//! Loads wait in a queue, [`MAX_LOADS`] at a time, the most recently asked for first. Scrolling
//! fast through a big library asks for every card that passes; the ones no longer asked for by
//! the time a slot frees up have scrolled away and are dropped instead of loaded. The queue is
//! looked at in [`ImageCache::tick`], before the next frame, once the frame that asked for
//! everything on screen is done.
//!
//! Loads stop when the cache is dropped, images larger than the device's largest texture are
//! scaled down as they're decoded, and a trim that can't get under the budget yet asks for the
//! frame that runs it again, so an idle app doesn't keep the memory.

use std::collections::HashMap;
use std::hash::Hash;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use egui::{ColorImage, TextureFilter, TextureHandle, TextureOptions};
use futures_util::FutureExt as _;
use jellyfin::{Client, ImageRequest};

use crate::runtime;
use crate::task::{self, Deadline, Receiver, Sender};

pub struct ImageCache {
    client: Client,
    dir: PathBuf,
    ctx: egui::Context,
    entries: Lru<ImageRequest, Entry>,
    /// Decoded blurhashes, by hash.
    placeholders: Lru<String, Option<TextureHandle>>,
    /// A trim waiting for recently used images to age, while over the budget.
    trim: Deadline,
    /// Images waiting to load, with when each was last asked for.
    queue: HashMap<ImageRequest, Instant>,
    /// When any image was last asked for: queued ones asked for well before it are stale.
    newest_want: Instant,
    /// Loads in progress.
    running: usize,
    /// A look at the queue is due once the current frame has rendered.
    pump_scheduled: bool,
    /// Finished loads, for [`ImageCache::tick`] to upload.
    loaded: Receiver<Loaded>,
    loading_from: Sender<Loaded>,
}

/// Downloads and decodes at once.
const MAX_LOADS: usize = 6;
/// A queued image not asked for within this of the newest request has scrolled out of view:
/// views ask for what they show on every frame, and the queue is looked at after a frame
/// (see `schedule_pump`), so this only has to cover one frame's own rendering.
const STALE: Duration = Duration::from_millis(50);

/// Decoded images kept in memory, in bytes (a 300×450 poster is about 0.5 MB).
const MEMORY_BUDGET: usize = 256 << 20;
/// Blurhash previews: they're tiny, but one per item seen adds up in a big library.
const PLACEHOLDER_BUDGET: usize = 16 << 20;
/// Every preview is its own GPU texture, and drivers can pad an allocation far beyond its 4 KB
/// of pixels, so the count is capped as well. The cap is well above what a page shows at once:
/// past it, previews still on screen would be evicted and decoded again every frame.
const PLACEHOLDER_LIMIT: usize = 2048;
/// Images used this recently are never dropped, even over the budget: they're likely on
/// screen, and dropping them would only reload them.
const KEEP_RECENT: Duration = Duration::from_secs(10);
/// Blurhash previews are decoded at this size and scaled up (they're blurry by design).
const PLACEHOLDER_SIZE: u32 = 32;

/// Posters are drawn smaller than their pixels, so they're filtered down rather than sampled.
/// egui_glow builds the mip chain; egui-wgpu 0.37 gives each texture one level, so it only
/// filters linearly.
const TEXTURE_OPTIONS: TextureOptions = TextureOptions::LINEAR.with_mipmap_mode(Some(TextureFilter::Linear));

enum Entry {
    Loading,
    Ready(TextureHandle),
    Failed,
}

/// A finished load: what was asked for, the texture's name for debugging, and the pixels or the
/// error. The pixels travel rather than a texture so that `tick`, on the UI thread, creates every
/// texture the cache holds.
type Loaded = (ImageRequest, String, anyhow::Result<ColorImage>);

impl ImageCache {
    /// The cache for a signed-in session. Dropping it stops the loads: a download in flight is
    /// cancelled, and a decode already running on a blocking thread finishes and is thrown away.
    pub fn new(client: Client, ctx: &egui::Context) -> Self {
        let (loading_from, loaded) = task::channel(ctx);
        Self {
            client,
            dir: crate::config::cache_dir().join("images"),
            ctx: ctx.clone(),
            entries: Lru::new(MEMORY_BUDGET),
            placeholders: Lru::with_limit(PLACEHOLDER_BUDGET, PLACEHOLDER_LIMIT),
            trim: Deadline::default(),
            queue: HashMap::new(),
            newest_want: Instant::now(),
            running: 0,
            pump_scheduled: false,
            loaded,
            loading_from,
        }
    }

    /// Drops loaded images from memory. They reload from disk or the server when next shown.
    pub fn clear_memory(&mut self) {
        // The handles the entries held drop here, which frees their textures.
        self.entries.take_where(|entry| !matches!(entry, Entry::Loading));
    }

    /// The blurred preview for a blurhash, or `None` if it doesn't decode.
    pub fn placeholder(&mut self, hash: &str) -> Option<TextureHandle> {
        if let Some(preview) = self.placeholders.get(hash) {
            return preview.clone();
        }
        let preview = decode_blurhash(hash).map(|image| {
            self.ctx
                .load_texture(format!("blurhash:{hash}"), image, TEXTURE_OPTIONS)
        });
        let size = preview.as_ref().map_or(0, texture_size);
        // What this evicts drops here, and its textures go with it.
        self.placeholders.insert(hash.to_owned(), preview.clone(), size);
        preview
    }

    /// Returns the image if it's loaded, otherwise queues it (or keeps it near the front of the
    /// queue) and returns `None`.
    pub fn get(&mut self, req: &ImageRequest) -> Option<TextureHandle> {
        let now = Instant::now();
        self.newest_want = now;
        match self.entries.get(req) {
            Some(Entry::Ready(texture)) => return Some(texture.clone()),
            Some(Entry::Loading) => {
                if let Some(wanted) = self.queue.get_mut(req) {
                    *wanted = now;
                }
                return None;
            }
            Some(Entry::Failed) => return None,
            None => {}
        }
        // What this evicts drops here, and its textures go with it.
        self.entries.insert(req.clone(), Entry::Loading, 0);
        self.queue.insert(req.clone(), now);
        self.schedule_pump();
        None
    }

    /// Whether an image is still queued or loading. A mockup waits for this before it takes its
    /// picture, so it shows the images rather than their blurred previews.
    #[cfg(all(test, feature = "mockup"))]
    pub fn busy(&self) -> bool {
        self.running > 0 || self.pump_scheduled || !self.queue.is_empty()
    }

    /// Uploads what finished loading, looks at the queue once the frame that asked is done, and
    /// drops aged images while over the budget. Called from `Services::tick`, before each frame.
    pub fn tick(&mut self) {
        let completed: Vec<Loaded> = self.loaded.drain().collect();
        let finished = !completed.is_empty();
        for (req, name, result) in completed {
            self.running -= 1;
            let (entry, size) = match result {
                Ok(image) => {
                    let size = image.pixels.len() * 4;
                    (Entry::Ready(self.ctx.load_texture(name, image, TEXTURE_OPTIONS)), size)
                }
                Err(err) => {
                    tracing::warn!(item = %req.item_id, "image load failed: {err:#}");
                    (Entry::Failed, 0)
                }
            };
            // What this evicts drops here, and its textures go with it.
            self.entries.insert(req, entry, size);
            self.trim_later();
        }
        // A slot freed up, so the next queued images can start. `main` pumped in the same two
        // places: after a frame asked (the scheduled look) and as each load finished.
        let scheduled = std::mem::take(&mut self.pump_scheduled);
        if scheduled || finished {
            self.pump();
        }
        if self.trim.fire(&self.ctx) {
            let evicted = self.entries.trim(Instant::now());
            if !evicted.is_empty() {
                tracing::debug!(
                    dropped = evicted.len(),
                    left = self.entries.total,
                    "image memory over budget; dropped the oldest images"
                );
            }
            // Dropping the handles frees their textures at once.
            drop(evicted);
            self.trim_later();
        }
    }

    /// Schedules the queue's next look for the next frame: the frame that just asked for images
    /// has to finish first, so that only what is still asked for by then is loaded. The repaint
    /// is what makes that frame happen, since this might have been the last one otherwise.
    fn schedule_pump(&mut self) {
        if std::mem::replace(&mut self.pump_scheduled, true) {
            return;
        }
        self.ctx.request_repaint();
    }

    /// Starts queued loads while there are free slots, the most recently wanted first, and
    /// drops the stale ones (their entry goes too, so they queue again if shown again).
    fn pump(&mut self) {
        while self.running < MAX_LOADS {
            let (next, stale) = next_load(&mut self.queue, self.newest_want);
            for req in stale {
                self.entries.remove(&req);
            }
            let Some(req) = next else { break };
            self.start_load(req);
        }
    }

    fn start_load(&mut self, req: ImageRequest) {
        self.running += 1;
        let client = self.client.clone();
        let path = cache_path(&self.dir, &req);
        let name = format!("image:{}", path.file_name().unwrap_or_default().to_string_lossy());
        // The device's largest texture, read on the UI thread, which is the one that knows it:
        // the loader fits the image into it.
        let max_side = self.ctx.input(|input| input.max_texture_side);
        let sender = self.loading_from.clone();
        let closed = sender.clone();
        let requested = req.clone();
        runtime::detach(async move {
            let work = async {
                tokio::select! {
                    result = load(&client, &requested, &path, max_side) => Some(result),
                    // The cache is gone (signed out): stop waiting for the download.
                    _ = closed.closed() => None,
                }
            };
            let result = match AssertUnwindSafe(work).catch_unwind().await {
                Ok(Some(result)) => result,
                Ok(None) => return,
                // A panic still has to free the load slot, so it counts as a failed load.
                Err(_) => Err(anyhow::anyhow!("the image loader panicked")),
            };
            let _ = sender.send((requested, name, result));
        });
    }

    /// Still over the budget after an insert means the images in the way were used recently,
    /// typically the page just left. Trims again once they've aged, so they don't wait for the
    /// next insert. It asks for that frame itself: nothing else is drawing while the app sits
    /// idle, so otherwise the trim would never run.
    fn trim_later(&mut self) {
        if self.trim.is_set() || !self.entries.over_budget() {
            return;
        }
        self.trim.set(KEEP_RECENT);
        self.ctx.request_repaint_after(KEEP_RECENT);
    }
}

/// Takes the most recently wanted entry off the queue, unless it's stale: then it and the rest
/// (older still) come off as stale instead.
fn next_load<K: Hash + Eq + Clone>(queue: &mut HashMap<K, Instant>, newest: Instant) -> (Option<K>, Vec<K>) {
    let Some((key, wanted)) = queue
        .iter()
        .max_by_key(|(_, wanted)| **wanted)
        .map(|(k, w)| (k.clone(), *w))
    else {
        return (None, Vec::new());
    };
    if newest.checked_sub(STALE).is_some_and(|before| wanted < before) {
        return (None, std::mem::take(queue).into_keys().collect());
    }
    queue.remove(&key);
    (Some(key), Vec::new())
}

/// Bytes of decoded pixels, for the placeholder budget.
fn texture_size(texture: &TextureHandle) -> usize {
    let [width, height] = texture.size();
    width * height * 4
}

fn decode_blurhash(hash: &str) -> Option<ColorImage> {
    let size = PLACEHOLDER_SIZE;
    let pixels = blurhash::decode(hash, size, size, 1.).ok()?;
    Some(ColorImage::from_rgba_unmultiplied(
        [size as usize, size as usize],
        &pixels,
    ))
}

/// Reads the image from the disk cache, or downloads and caches it, then decodes it, fitted
/// into a texture `max_side` pixels wide and tall at most.
async fn load(client: &Client, req: &ImageRequest, path: &Path, max_side: usize) -> anyhow::Result<ColorImage> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) if sniff_format(&bytes).is_some() => bytes,
        _ => {
            let bytes = client.image(req).await?;
            sniff_format(&bytes).context("unrecognized image format")?;
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            if let Err(err) = tokio::fs::write(path, &bytes).await {
                tracing::debug!("couldn't cache image at {}: {err}", path.display());
            }
            bytes
        }
    };
    tokio::task::spawn_blocking(move || decode(&bytes, max_side)).await?
}

fn decode(bytes: &[u8], max_side: usize) -> anyhow::Result<ColorImage> {
    let format = sniff_format(bytes).context("unrecognized image format")?;
    let pixels = image::load_from_memory_with_format(bytes, format)?.into_rgba8();
    let pixels = fit(pixels, max_side);
    let size = [pixels.width() as usize, pixels.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, pixels.as_raw()))
}

/// Fits an image into `max_side` × `max_side`, keeping its aspect ratio. wgpu refuses a texture
/// bigger than the device's limit (egui only has a debug assertion for it), and the server doesn't
/// upscale, so a big enough source image would otherwise crash release builds.
pub(crate) fn fit(pixels: image::RgbaImage, max_side: usize) -> image::RgbaImage {
    let (width, height) = (pixels.width(), pixels.height());
    if width as usize <= max_side && height as usize <= max_side {
        return pixels;
    }
    let scale = max_side as f32 / width.max(height) as f32;
    let width = ((width as f32 * scale).round() as u32).max(1);
    let height = ((height as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&pixels, width, height, image::imageops::FilterType::Triangle)
}

/// A map that keeps its total size under a budget by dropping the entries used longest ago,
/// except those used within [`KEEP_RECENT`]. A count cap keeps values that cost far more than
/// their size (the blurhash textures) from piling up while they're all new.
struct Lru<K, V> {
    entries: HashMap<K, (V, usize, Instant)>,
    total: usize,
    budget: usize,
    max_entries: usize,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    fn new(budget: usize) -> Self {
        Self::with_limit(budget, usize::MAX)
    }

    fn with_limit(budget: usize, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            total: 0,
            budget,
            max_entries,
        }
    }

    fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (value, _, used) = self.entries.get_mut(key)?;
        *used = Instant::now();
        Some(value)
    }

    /// Inserts or replaces an entry of `size` bytes and returns what was dropped to make room.
    fn insert(&mut self, key: K, value: V, size: usize) -> Vec<V> {
        self.insert_at(key, value, size, Instant::now())
    }

    fn insert_at(&mut self, key: K, value: V, size: usize, now: Instant) -> Vec<V> {
        if let Some((_, old, _)) = self.entries.insert(key, (value, size, now)) {
            self.total -= old;
        }
        self.total += size;
        self.trim(now)
    }

    fn over_budget(&self) -> bool {
        self.total > self.budget
    }

    /// Drops the entries used longest ago, until under the budget or only recently used ones
    /// are left, and returns them. Over the count cap, the oldest entries go even if they're
    /// recent: the cap is what keeps many small values from piling up.
    fn trim(&mut self, now: Instant) -> Vec<V> {
        let mut evicted = Vec::new();
        while self.total > self.budget || self.entries.len() > self.max_entries {
            let over_count = self.entries.len() > self.max_entries;
            let oldest = self
                .entries
                .iter()
                .filter(|(_, (_, size, used))| over_count || (*size > 0 && now.duration_since(*used) >= KEEP_RECENT))
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(key, _)| key.clone());
            let Some(key) = oldest else { break };
            if let Some((value, size, _)) = self.entries.remove(&key) {
                self.total -= size;
                evicted.push(value);
            }
        }
        evicted
    }

    fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (value, size, _) = self.entries.remove(key)?;
        self.total -= size;
        Some(value)
    }

    /// Removes the entries matching `drop` and returns them.
    fn take_where(&mut self, drop: impl Fn(&V) -> bool) -> Vec<V> {
        let keys: Vec<K> = self
            .entries
            .iter()
            .filter(|(_, (value, _, _))| drop(value))
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| self.entries.remove(&key))
            .map(|(value, size, _)| {
                self.total -= size;
                value
            })
            .collect()
    }
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

/// The format from the bytes themselves: a server error page served with `Content-Type: image`
/// must not reach the decoder.
fn sniff_format(bytes: &[u8]) -> Option<image::ImageFormat> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some(image::ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', ..] => Some(image::ImageFormat::Png),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(image::ImageFormat::WebP),
        [b'G', b'I', b'F', b'8', ..] => Some(image::ImageFormat::Gif),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use egui::{Color32, ColorImage};
    use image::ImageEncoder as _;
    use jellyfin::Client;
    use jellyfin::models::ImageType;

    use super::{Entry, ImageCache, KEEP_RECENT, Lru, MEMORY_BUDGET, STALE, TEXTURE_OPTIONS, decode, next_load};
    use crate::session::client_info;

    fn request(item_id: &str) -> jellyfin::ImageRequest {
        jellyfin::ImageRequest::new(item_id, ImageType::Primary, Some("tag".into()))
    }

    /// A cache whose entry budget is small enough for the tests' images to cross it.
    fn cache(ctx: &egui::Context, budget: usize) -> ImageCache {
        let client = Client::new("http://localhost:8096/", client_info("test")).expect("a client");
        let mut cache = ImageCache::new(client, ctx);
        cache.entries = Lru::new(budget);
        cache
    }

    fn texture(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
        ctx.load_texture(name, ColorImage::filled([1, 1], Color32::WHITE), TEXTURE_OPTIONS)
    }

    #[test]
    fn images_decode_as_rgba() {
        // A red and a blue pixel, which must come out in RGBA order, not swapped to BGRA.
        let pixels = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).expect("2x1");
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(pixels.as_raw(), 2, 1, image::ExtendedColorType::Rgba8)
            .expect("encode");
        let decoded = decode(&png, 4096).expect("decode");
        assert_eq!(decoded.size, [2, 1]);
        assert_eq!(decoded.pixels[0], egui::Color32::from_rgb(255, 0, 0));
        assert_eq!(decoded.pixels[1], egui::Color32::from_rgb(0, 0, 255));
    }

    #[test]
    fn images_bigger_than_the_texture_limit_are_scaled_down() {
        let pixels = image::RgbaImage::from_raw(4, 2, vec![255; 4 * 2 * 4]).expect("4x2");
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(pixels.as_raw(), 4, 2, image::ExtendedColorType::Rgba8)
            .expect("encode");
        let decoded = decode(&png, 2).expect("decode");
        assert_eq!(decoded.size, [2, 1], "fitted into 2x2, keeping the aspect ratio");
    }

    #[test]
    fn a_finished_load_is_uploaded() {
        let ctx = egui::Context::default();
        let mut cache = cache(&ctx, MEMORY_BUDGET);
        let req = request("a");
        cache.entries.insert(req.clone(), Entry::Loading, 0);
        cache.running = 1;
        let pixels = ColorImage::filled([2, 2], Color32::WHITE);
        cache
            .loading_from
            .send((req.clone(), "test".into(), Ok(pixels)))
            .expect("the cache is listening");
        cache.tick();
        assert_eq!(cache.running, 0, "the slot is free again");
        assert!(matches!(cache.entries.get(&req), Some(Entry::Ready(_))));
    }

    #[test]
    fn the_delayed_trim_frees_aged_images_and_asks_for_its_frame() {
        let ctx = egui::Context::default();
        let mut cache = cache(&ctx, 60);
        let long_ago = Instant::now() - KEEP_RECENT * 2;
        // Two images loaded a while ago, together over the budget.
        for (index, id) in ["a", "b"].into_iter().enumerate() {
            let req = request(id);
            cache.entries.insert(req.clone(), Entry::Loading, 0);
            cache
                .entries
                .insert_at(req, Entry::Ready(texture(&ctx, &format!("t{index}"))), 40, long_ago);
        }
        assert_eq!(cache.entries.total, 80);
        cache.trim_later();
        assert!(cache.trim.is_set(), "the trim is armed");
        assert!(
            ctx.has_requested_repaint(),
            "and it asks for the frame that runs it, even if the app is idle"
        );
        // That frame: the oldest goes at its deadline, not on the next insert.
        cache.trim.set(Duration::ZERO);
        cache.tick();
        assert_eq!(cache.entries.total, 40);
        assert_eq!(cache.entries.entries.len(), 1);
    }

    #[test]
    fn a_trim_that_cannot_drop_everything_rearms_itself() {
        let ctx = egui::Context::default();
        let mut cache = cache(&ctx, usize::MAX);
        let long_ago = Instant::now() - KEEP_RECENT * 2;
        for (id, size, used) in [("a", 40, long_ago), ("b", 80, Instant::now())] {
            let req = request(id);
            cache.entries.insert(req.clone(), Entry::Loading, 0);
            cache
                .entries
                .insert_at(req, Entry::Ready(texture(&ctx, id)), size, used);
        }
        // Over the budget: the aged one can go, the recent one can't yet.
        cache.entries.budget = 50;
        cache.trim.set(Duration::ZERO);
        cache.tick();
        assert_eq!(cache.entries.total, 80);
        assert!(cache.trim.is_set(), "the trim comes back once the recent image ages");
        assert!(ctx.has_requested_repaint(), "and asks for that frame");
    }

    #[test]
    fn the_most_recently_wanted_image_loads_first() {
        let now = Instant::now();
        let mut queue = HashMap::from([("old", now - Duration::from_millis(10)), ("new", now)]);
        assert_eq!(next_load(&mut queue, now), (Some("new"), vec![]));
        assert_eq!(next_load(&mut queue, now), (Some("old"), vec![]));
        assert_eq!(next_load(&mut queue, now), (None, vec![]));
    }

    #[test]
    fn images_scrolled_past_are_dropped_unloaded() {
        let now = Instant::now();
        let long_ago = now - STALE * 4;
        let mut queue = HashMap::from([("a", long_ago), ("b", long_ago - STALE)]);
        let (next, mut stale) = next_load(&mut queue, now);
        stale.sort();
        assert_eq!((next, stale), (None, vec!["a", "b"]));
        assert!(queue.is_empty());
    }

    #[test]
    fn the_least_recently_used_go_first() {
        let start = Instant::now();
        let later = |secs| start + Duration::from_secs(secs);
        let mut lru = Lru::new(100);
        lru.insert_at("a", 'a', 40, later(0));
        lru.insert_at("b", 'b', 40, later(1));
        // Touching "a" makes "b" the oldest.
        lru.entries.get_mut("a").expect("a").2 = later(2);
        let evicted = lru.insert_at("c", 'c', 40, later(60));
        assert_eq!(evicted, ['b']);
        assert_eq!(lru.total, 80);
        assert!(lru.entries.contains_key("a") && lru.entries.contains_key("c"));
    }

    #[test]
    fn recently_used_entries_stay_over_the_budget() {
        let now = Instant::now();
        let mut lru = Lru::new(100);
        lru.insert_at("a", 'a', 80, now);
        let evicted = lru.insert_at("b", 'b', 80, now + KEEP_RECENT / 2);
        assert!(evicted.is_empty());
        assert_eq!(lru.total, 160);
        // Once "a" is old enough, the next insert drops it.
        let evicted = lru.insert_at("c", 'c', 10, now + KEEP_RECENT * 2);
        assert_eq!(evicted, ['a']);
    }

    #[test]
    fn the_count_cap_drops_entries_that_are_still_recent() {
        let now = Instant::now();
        let mut lru = Lru::with_limit(usize::MAX, 2);
        lru.insert_at("a", 'a', 1, now);
        lru.insert_at("b", 'b', 1, now + Duration::from_secs(1));
        let evicted = lru.insert_at("c", 'c', 1, now + Duration::from_secs(2));
        assert_eq!(evicted, ['a'], "the oldest goes even though it's recent");
        assert_eq!(lru.entries.len(), 2);
    }

    #[test]
    fn replacing_an_entry_updates_the_total() {
        let now = Instant::now();
        let mut lru = Lru::new(100);
        lru.insert_at("a", 'a', 0, now);
        lru.insert_at("a", 'A', 30, now);
        assert_eq!(lru.total, 30);
        assert_eq!(lru.take_where(|v| *v == 'A'), ['A']);
        assert_eq!(lru.total, 0);
    }
}
