//! Server image loading with an in-memory and on-disk cache.
//!
//! Views call [`ImageCache::get`] while rendering. A miss starts a background load (disk first,
//! then network) and returns `None`. When the load finishes the cache notifies its observers,
//! so views that render images observe the global cache entity.
//!
//! While an image loads, its blurhash (from the item's `ImageBlurHashes`) is drawn instead:
//! a tiny blurred preview decoded in memory.
//!
//! Images are decoded here rather than by GPUI, whose asset cache and texture atlas keep every
//! image ever shown. Memory is bounded by [`MEMORY_BUDGET`]: past it, the images used longest
//! ago are dropped from memory and from the atlas, and reload from disk when shown again.
//!
//! Loads wait in a queue, [`MAX_LOADS`] at a time, the most recently asked for first. Scrolling
//! fast through a big library asks for every card that passes; the ones no longer asked for by
//! the time a slot frees up have scrolled away and are dropped instead of loaded.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui_kit::*;
use jellyfin::{Client, ImageRequest};

use crate::runtime;

pub struct ImageCache {
    client: Client,
    dir: PathBuf,
    entries: Lru<ImageRequest, Entry>,
    /// Decoded blurhashes, by hash.
    placeholders: Lru<String, Option<Arc<RenderImage>>>,
    /// A trim waiting for recently used images to age, while over the budget.
    trim: Option<Task<()>>,
    /// Images waiting to load, with when each was last asked for.
    queue: HashMap<ImageRequest, Instant>,
    /// When any image was last asked for: queued ones asked for well before it are stale.
    newest_want: Instant,
    /// Loads in progress.
    running: usize,
    /// A look at the queue is due once the current frame has rendered.
    pump_scheduled: bool,
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
/// Images used this recently are never dropped, even over the budget: they're likely on
/// screen, and dropping them would only reload them.
const KEEP_RECENT: Duration = Duration::from_secs(10);
/// Blurhash previews are decoded at this size and scaled up (they're blurry by design).
const PLACEHOLDER_SIZE: u32 = 32;

enum Entry {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

impl Entry {
    fn image(&self) -> Option<&Arc<RenderImage>> {
        match self {
            Entry::Ready(image) => Some(image),
            Entry::Loading | Entry::Failed => None,
        }
    }
}

struct GlobalImageCache(Entity<ImageCache>);

impl Global for GlobalImageCache {}

impl ImageCache {
    /// Installs the cache for a signed-in session, replacing any previous one.
    pub fn init(client: Client, cx: &mut App) {
        let cache = cx.new(|_| ImageCache {
            client,
            dir: crate::config::cache_dir().join("images"),
            entries: Lru::new(MEMORY_BUDGET),
            placeholders: Lru::new(PLACEHOLDER_BUDGET),
            trim: None,
            queue: HashMap::new(),
            newest_want: Instant::now(),
            running: 0,
            pump_scheduled: false,
        });
        cx.set_global(GlobalImageCache(cache));
    }

    pub fn global(cx: &App) -> Entity<ImageCache> {
        cx.global::<GlobalImageCache>().0.clone()
    }

    /// Drops loaded images from memory. They reload from disk or the server when next shown.
    pub fn clear_memory(&mut self, cx: &mut Context<Self>) {
        let dropped = self.entries.take_where(|entry| !matches!(entry, Entry::Loading));
        release(dropped.into_iter().filter_map(|entry| entry.image().cloned()), cx);
    }

    /// The blurred preview for a blurhash, or `None` if it doesn't decode.
    pub fn placeholder(&mut self, hash: &str, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        if let Some(preview) = self.placeholders.get(hash) {
            return preview.clone();
        }
        let preview = decode_blurhash(hash);
        let size = preview.as_deref().map_or(0, image_size);
        let evicted = self.placeholders.insert(hash.to_owned(), preview.clone(), size);
        release(evicted.into_iter().flatten(), cx);
        preview
    }

    /// Returns the image if it's loaded, otherwise queues it (or keeps it near the front of the
    /// queue) and returns `None`.
    pub fn get(&mut self, req: &ImageRequest, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let now = Instant::now();
        self.newest_want = now;
        match self.entries.get(req) {
            Some(Entry::Ready(image)) => return Some(image.clone()),
            Some(Entry::Loading) => {
                if let Some(wanted) = self.queue.get_mut(req) {
                    *wanted = now;
                }
                return None;
            }
            Some(Entry::Failed) => return None,
            None => {}
        }
        let evicted = self.entries.insert(req.clone(), Entry::Loading, 0);
        release(evicted.iter().filter_map(|entry| entry.image().cloned()), cx);
        self.queue.insert(req.clone(), now);
        self.schedule_pump(cx);
        None
    }

    /// Looks at the queue after the current frame, once every image on screen was asked for
    /// in it, so anything older is known to be off screen.
    fn schedule_pump(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.pump_scheduled, true) {
            return;
        }
        // Deferred work runs once the current update, and so the frame being drawn, is done.
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| {
                this.pump_scheduled = false;
                this.pump(cx);
            })
            .ok();
        });
    }

    /// Starts queued loads while there are free slots, the most recently wanted first, and
    /// drops the stale ones (their entry goes too, so they queue again if shown again).
    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.running < MAX_LOADS {
            let (next, stale) = next_load(&mut self.queue, self.newest_want);
            for req in stale {
                self.entries.remove(&req);
            }
            let Some(req) = next else { break };
            self.start_load(req, cx);
        }
    }

    fn start_load(&mut self, req: ImageRequest, cx: &mut Context<Self>) {
        self.running += 1;
        let client = self.client.clone();
        let path = cache_path(&self.dir, &req);
        cx.spawn(async move |this, cx| {
            let fetch = req.clone();
            let result = runtime::spawn(async move { load(&client, &fetch, &path).await }).await;
            this.update(cx, |this, cx| {
                this.running -= 1;
                let (entry, size) = match result {
                    Ok(image) => {
                        let size = image_size(&image);
                        (Entry::Ready(image), size)
                    }
                    Err(err) => {
                        tracing::warn!(item = %req.item_id, "image load failed: {err:#}");
                        (Entry::Failed, 0)
                    }
                };
                let evicted = this.entries.insert(req, entry, size);
                release(evicted.iter().filter_map(|entry| entry.image().cloned()), cx);
                this.trim_later(cx);
                this.pump(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
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

/// Frees dropped images from the windows' texture atlases. Deferred, because the cache is
/// usually called while a window renders, and that window can't be reached until it's done.
impl ImageCache {
    /// Still over the budget after an insert means the images in the way were used recently,
    /// typically the page just left. Trims again once they've aged, so they don't wait for the
    /// next insert.
    fn trim_later(&mut self, cx: &mut Context<Self>) {
        if self.trim.is_some() || !self.entries.over_budget() {
            return;
        }
        self.trim = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEEP_RECENT).await;
            this.update(cx, |this, cx| {
                this.trim = None;
                let evicted = this.entries.trim(Instant::now());
                release(evicted.iter().filter_map(|entry| entry.image().cloned()), cx);
                this.trim_later(cx);
            })
            .ok();
        }));
    }
}

fn release(images: impl IntoIterator<Item = Arc<RenderImage>>, cx: &mut App) {
    let images: Vec<_> = images.into_iter().collect();
    if !images.is_empty() {
        cx.defer(move |cx| {
            for image in images {
                cx.drop_image(image, None);
            }
        });
    }
}

/// Bytes of decoded pixels.
fn image_size(image: &RenderImage) -> usize {
    let size = image.size(0);
    size.width.0 as usize * size.height.0 as usize * 4
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

/// Reads the image from the disk cache, or downloads and caches it, then decodes it.
async fn load(client: &Client, req: &ImageRequest, path: &Path) -> anyhow::Result<Arc<RenderImage>> {
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
    tokio::task::spawn_blocking(move || decode(&bytes)).await?
}

fn decode(bytes: &[u8]) -> anyhow::Result<Arc<RenderImage>> {
    let format = match sniff_format(bytes).context("unrecognized image format")? {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Webp => image::ImageFormat::WebP,
        _ => image::ImageFormat::Gif,
    };
    let mut pixels = image::load_from_memory_with_format(bytes, format)?.into_rgba8();
    // GPUI wants BGRA.
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
        pixels
    )])))
}

/// A map that keeps its total size under a budget by dropping the entries used longest ago,
/// except those used within [`KEEP_RECENT`].
struct Lru<K, V> {
    entries: HashMap<K, (V, usize, Instant)>,
    total: usize,
    budget: usize,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            total: 0,
            budget,
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
    /// are left, and returns them.
    fn trim(&mut self, now: Instant) -> Vec<V> {
        let mut evicted = Vec::new();
        while self.total > self.budget {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, (_, size, used))| *size > 0 && now.duration_since(*used) >= KEEP_RECENT)
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

fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use std::collections::HashMap;

    use super::{KEEP_RECENT, Lru, STALE, next_load};

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
