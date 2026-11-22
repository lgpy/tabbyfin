//! Downloads of an item's original file, and the local copies playback finds.
//!
//! Everything lives under [`config::downloads_dir`], one folder per account and one per item:
//!
//! ```text
//! <account>/index.json             the queue, in the order it was asked for
//! <account>/session.json           who was signed in, for a launch the server can't answer
//! <account>/pending.json           stop reports the server hasn't heard yet
//! <account>/<item>/item.json       the item as it was downloaded, and what plays it
//! <account>/<item>/media.<ext>     the finished file (a `.part` while it downloads)
//! <account>/<item>/<index>.<ext>   its external subtitles
//! <account>/<item>/primary.<ext>   its poster and backdrop, for offline pages
//! <account>/<item>/backdrop.<ext>
//! ```
//!
//! A finished media file on disk is the source of truth: an item whose folder holds it is done
//! whatever the queue says, so a lost or stale `index.json` never loses a download. `item.json`
//! is written before the media starts, so an interrupted download still knows what it is.
//!
//! A downloaded file plays without the server, so what its playback changes is kept here too:
//! the item's watch state goes back into its `item.json` ([`record_progress`]), and a stop
//! report that couldn't reach the server waits in `pending.json` ([`flush_pending`]).
//!
//! Transfers run on tokio, in at most [`MAX_TRANSFERS`] workers. Each worker takes the next
//! waiting item from the [`Queue`] it shares with [`Downloads`] itself, so the queue moves on
//! with no frame drawn: an Android app in the background draws none, and its downloads carry on
//! under a foreground service, which the platform's notice ([`Activity`]) starts and stops.
//! Dropping [`Downloads`] aborts the workers, leaving each `.part` for the next sign-in. They
//! report through a [`task::channel`] that [`Downloads::tick`] drains; the UI reads
//! [`Downloads::state`] and [`Downloads::entries`], never the disk.
//!
//! With *Download over Wi-Fi only* (Android), a worker waits while there's no unmetered
//! network, and one that's downloading when the phone leaves Wi-Fi stops and puts its item back
//! at the front of the queue, to resume from its `.part`.

use std::collections::VecDeque;
use std::io::{SeekFrom, Write};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::FutureExt;

use jellyfin::models::{
    BaseItemDto, BaseItemKind, ChapterInfo, ItemField, MediaSegmentDto, MediaSourceInfo, MediaStream, MediaStreamType,
    PublicSystemInfo, UserDto,
};
use jellyfin::{Client, ImageRequest, PlaybackInfoRequest, PlaybackReport, Ticks};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;
use url::Url;

use crate::config;
use crate::i18n::{self, tr};
use crate::playback;
use crate::services::Services;
use crate::session::describe_error;
use crate::task;
use crate::toasts::Toast;

/// How many files download at once. More would crowd out playback's stream and the websocket's
/// keep-alive on a slow line without making the queue finish sooner.
const MAX_TRANSFERS: usize = 2;
/// How often a transfer may report its progress. Four a second keeps the bar moving without
/// repainting the window for every chunk.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// How often the platform's notice of the downloads (Android's notification) is told of their
/// progress; a notification updated faster than this is throttled by the system anyway.
const NOTICE_INTERVAL: Duration = Duration::from_secs(1);
/// How long a transfer that failed with Wi-Fi only on waits for the phone to say the network
/// changed: leaving Wi-Fi breaks the connection a moment before it's reported (about 250 ms on
/// the emulator).
const NETWORK_SETTLE: Duration = Duration::from_secs(3);
/// How many more times a transfer that stalled is tried before its item fails. A server that
/// is slow to start answering (a library spun down, or on cloud storage) comes round in this
/// time, and each attempt resumes from the `.part`. Only a stall is tried again: any other
/// error says something a retry wouldn't change.
const STALL_RETRIES: u32 = 2;
/// How long a stalled transfer waits before its next attempt.
const STALL_PAUSE: Duration = Duration::from_secs(5);
/// Image widths kept for offline pages: a poster for a card, a backdrop for a header (twice
/// what's drawn, as `images.rs` asks for).
const PRIMARY_WIDTH: u32 = 600;
const BACKDROP_WIDTH: u32 = 1280;

const INDEX_FILE: &str = "index.json";
const ITEM_FILE: &str = "item.json";
const SESSION_FILE: &str = "session.json";
const PENDING_FILE: &str = "pending.json";
/// How much of an item has to have played for it to count as watched: the server's default
/// `MaxResumePct`, past which it marks the item played and clears the resume point.
const PLAYED_SHARE: f64 = 0.9;

/// What a page shows for one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadState {
    Queued,
    /// Saved as paused; the queue leaves it alone until a page queues it again.
    Paused,
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    Done {
        size: u64,
    },
    /// It failed; the message says why, and [`Downloads::retry`] tries again.
    Failed(String),
}

/// What the item menu and the item page offer for an item's download, by where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Nothing yet: a video, or every episode of a series or season that isn't downloaded.
    Download,
    /// Queued or downloading: stop, and throw away what came so far.
    Cancel,
    /// On disk: delete it.
    Remove,
    Retry,
}

/// A download that finished or failed, for [`Services`] to turn into a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Downloaded { title: String },
    Failed { title: String, message: String },
}

/// How a series' or season's episodes among the downloads add up, for the card that offers the
/// folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChildrenProgress {
    pub done: usize,
    /// Queued, paused or downloading.
    pub pending: usize,
    pub failed: usize,
}

/// `item.json`: everything offline playback needs. Written before the media starts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalItem {
    /// The item, fetched with its chapters and media sources.
    pub item: BaseItemDto,
    /// The version downloaded, from `PlaybackInfo` at the time: its stream list and defaults are
    /// what the player's track mapping follows.
    pub source: MediaSourceInfo,
    #[serde(default)]
    pub subtitles: Vec<LocalSubtitle>,
    #[serde(default)]
    pub segments: Vec<MediaSegmentDto>,
    /// File names in the item's folder, when the images were fetched.
    #[serde(default)]
    pub primary_image: Option<String>,
    #[serde(default)]
    pub backdrop_image: Option<String>,
    /// The media file's name, `media.<container>`.
    pub file: String,
    /// Its size in bytes, as `PlaybackInfo` gave it while the transfer ran, and the length that
    /// arrived once it finished; `0` when the server didn't say. Only a hint until the file is
    /// finished: the server's figure can differ from the bytes it serves.
    pub size: u64,
    /// The length of the finished media file as this transfer counted it, written before the file
    /// takes its name. A file of any other length is damaged. Absent in an `item.json` from before:
    /// those downloads were accepted when the bytes matched `Content-Length`, whatever `size`
    /// says, so the only check they can pass is that the file is there and isn't empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_size: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalSubtitle {
    /// The Jellyfin stream index, as [`MediaStream::index`].
    pub index: i32,
    pub file: String,
}

impl LocalItem {
    /// The poster's path in the item's folder, when one was saved.
    fn poster(&self, dir: &Path) -> Option<PathBuf> {
        self.primary_image.as_ref().map(|file| dir.join(file))
    }
}

/// A finished download, as playback finds it.
#[derive(Debug, Clone)]
pub struct LocalMedia {
    pub path: PathBuf,
    pub source: MediaSourceInfo,
    /// Jellyfin stream index and the file holding it, for the external subtitles that are there.
    pub subtitles: Vec<(i32, PathBuf)>,
    /// For the player's chapter marks and skip buttons when the server can't be asked
    /// (`playback::details`).
    pub chapters: Vec<ChapterInfo>,
    pub segments: Vec<MediaSegmentDto>,
}

impl LocalMedia {
    /// The external subtitles as mpv loads them, in `source.media_streams` order, which is how
    /// `playback::mpv_subtitle_id` numbers them.
    pub fn subtitle_files(&self) -> Vec<(MediaStream, Url)> {
        let files = &self.subtitles;
        self.source
            .media_streams
            .iter()
            .filter(|stream| stream.kind == MediaStreamType::Subtitle && stream.is_external)
            .filter_map(|stream| {
                let (_, path) = files.iter().find(|(index, _)| *index == stream.index)?;
                let url = Url::from_file_path(path).ok()?;
                Some((stream.clone(), url))
            })
            .collect()
    }
}

/// The downloads of the signed-in account: the queue, the transfers running now and what is
/// already on disk. Owned by [`Services`].
pub struct Downloads {
    client: Client,
    /// The account's folder name, from [`crate::config::account_key`], sanitised.
    account: String,
    ctx: egui::Context,
    /// The queue in the order it was asked for, finished items included, so a page can draw it
    /// without a server call.
    entries: Vec<Entry>,
    /// What the workers share with this: the items waiting for one, and the workers running.
    queue: Arc<Mutex<Queue>>,
    /// Series and season expansions being fetched.
    expanding: Vec<task::Task<()>>,
    /// [`Downloads::pause`] ran: the queue stays where it is until this account signs in again
    /// (a new `Downloads`), even if a frame still ticks this one.
    stopped: bool,
    /// *Download over Wi-Fi only*, which the workers watch.
    wifi_only: watch::Sender<bool>,
    /// Whether an unmetered network is there: [`UNMETERED`]'s, or a test's own.
    unmetered: watch::Receiver<bool>,
    updates: task::Receiver<Update>,
    updates_from: task::Sender<Update>,
    /// What `index.json`'s writes share, and the number of the newest snapshot asked for: see
    /// [`Downloads::save`].
    index_writes: Arc<IndexWrites>,
    index_asked: u64,
}

struct Entry {
    item: BaseItemDto,
    state: DownloadState,
    /// The poster saved with the download, for a page drawing it without the server.
    poster: Option<PathBuf>,
}

/// Whether an unmetered network (Wi-Fi, Ethernet) is there, as the platform last said:
/// Android's `MainActivity` reports it ([`set_unmetered`]); elsewhere nothing does, and it stays
/// true. Process-wide, since it's the device's.
static UNMETERED: LazyLock<watch::Sender<bool>> = LazyLock::new(|| watch::Sender::new(true));

/// Whether an unmetered network is there now, from the platform's network callback. Workers
/// held for Wi-Fi wake on it, with or without a frame. Returns whether it changed: the callback
/// repeats the same answer every few seconds as the signal strength moves.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn set_unmetered(unmetered: bool) -> bool {
    UNMETERED.send_if_modified(|now| std::mem::replace(now, unmetered) != unmetered)
}

/// What the workers share with [`Downloads`]. Each worker takes its next item from `waiting`
/// itself, so the queue moves on while the UI draws nothing; the UI adds and removes items, and
/// spawns workers while there's work and room for one ([`Downloads::start_transfers`]).
#[derive(Default)]
struct Queue {
    /// Queued items no worker has taken, in the order they were asked for.
    waiting: VecDeque<BaseItemDto>,
    /// At most [`MAX_TRANSFERS`].
    workers: Vec<Worker>,
    next_worker: usize,
    /// The workers are waiting for an unmetered network.
    held: bool,
    /// When the platform's notice last heard of the downloads, to tell it at most every
    /// [`NOTICE_INTERVAL`] of mere progress.
    told: Option<Instant>,
    /// What the transfers wait for, which the tests shorten.
    patience: Patience,
}

/// How long a transfer waits: for a byte before it counts as stalled, and between the attempts
/// that follow a stall.
#[derive(Clone, Copy)]
struct Patience {
    /// `None` is the client's own limit ([`Client::download_resuming`]).
    stall: Option<Duration>,
    pause: Duration,
}

impl Default for Patience {
    fn default() -> Self {
        Self {
            stall: None,
            pause: STALL_PAUSE,
        }
    }
}

struct Worker {
    id: usize,
    /// Aborts the worker and its transfer, leaving the `.part` for a retry. `None` for the
    /// moment between spawning it and keeping its handle.
    abort: Option<tokio::task::AbortHandle>,
    /// What it's downloading now.
    current: Option<Current>,
}

struct Current {
    item_id: String,
    title: String,
    done: u64,
    total: Option<u64>,
}

/// What the platform's notice of the downloads says while any worker runs: Android's
/// notification, which comes with the foreground service that keeps the process alive.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub title: String,
    pub text: String,
    /// How far along the first transfer is, from 0 to 1; `None` while that isn't known.
    pub progress: Option<f32>,
}

fn lock(queue: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Queue {
    /// What the notice says, or `None` once no worker runs.
    fn activity(&self) -> Option<Activity> {
        if self.workers.is_empty() {
            return None;
        }
        let current: Vec<&Current> = self
            .workers
            .iter()
            .filter_map(|worker| worker.current.as_ref())
            .collect();
        let to_come = current.len() + self.waiting.len();
        let first = current.first().filter(|_| !self.held);
        let Some(first) = first else {
            let title = if self.held {
                tr!("downloads-activity-waiting-wifi")
            } else {
                tr!("downloads-activity-downloading")
            };
            return Some(Activity {
                title,
                text: tr!("downloads-activity-to-come", count = to_come),
                progress: None,
            });
        };
        let total = first.total.filter(|&total| total > 0);
        let text = match total {
            Some(total) => tr!(
                "downloads-activity-progress",
                done = bytes(first.done),
                total = bytes(total)
            ),
            None => tr!("downloads-activity-so-far", done = bytes(first.done)),
        };
        // Two whole labels, as the summary's are: the dot between them isn't a word.
        let text = if to_come > 1 {
            let more = to_come - 1;
            format!("{text} · {}", tr!("downloads-activity-more", count = more))
        } else {
            text
        };
        Some(Activity {
            title: first.title.clone(),
            text,
            progress: total.map(|total| (first.done as f64 / total as f64).clamp(0., 1.) as f32),
        })
    }

    /// Tells the platform's notice where the downloads stand: always when `force`, otherwise
    /// at most every [`NOTICE_INTERVAL`]. Under the queue's lock, so a worker's stale progress
    /// can't land after the last one's "nothing left" and bring the notice back.
    fn tell(&mut self, force: bool) {
        if !force && self.told.is_some_and(|told| told.elapsed() < NOTICE_INTERVAL) {
            return;
        }
        self.told = Some(Instant::now());
        notice(self.activity().as_ref());
    }

    fn worker(&mut self, id: usize) -> Option<&mut Worker> {
        self.workers.iter_mut().find(|worker| worker.id == id)
    }

    /// The next item for worker `id`, or `None` when nothing waits, in which case the worker
    /// is gone from the list at once: [`Downloads::start_transfers`] counting it while it
    /// winds down would leave an item queued after it with no worker.
    fn take(&mut self, id: usize) -> Option<BaseItemDto> {
        let Some(item) = self.waiting.pop_front() else {
            self.workers.retain(|worker| worker.id != id);
            self.tell(true);
            return None;
        };
        if let Some(worker) = self.worker(id) {
            worker.current = Some(Current {
                item_id: item.id.clone(),
                title: playback::title(&item),
                done: 0,
                total: source_size(&item),
            });
        }
        self.tell(true);
        Some(item)
    }
}

/// The platform's notice: Android's notification and foreground service. Nothing elsewhere,
/// where a process isn't stopped for being in the background.
fn notice(activity: Option<&Activity>) {
    #[cfg(target_os = "android")]
    crate::android::downloads_notice(activity);
    #[cfg(not(target_os = "android"))]
    let _ = activity;
}

/// The size the item's own sources give, until `PlaybackInfo` says more precisely.
fn source_size(item: &BaseItemDto) -> Option<u64> {
    item.media_sources
        .first()
        .and_then(|source| source.size)
        .map(|size| size as u64)
}

/// What a transfer reports, to the UI and to the queue it's from (for the notice).
struct Reporter {
    updates: task::Sender<Update>,
    queue: Arc<Mutex<Queue>>,
    worker: usize,
}

impl Reporter {
    fn send(&self, mut update: Update) {
        if let Update::Progress { done, total, .. } = &mut update {
            // A total the bytes have passed was wrong (`PlaybackInfo`'s size, with the server
            // saying none of its own): the transfer has no total to show, rather than a bar
            // beyond full.
            if total.is_some_and(|total| *done > total) {
                *total = None;
            }
        }
        if let Update::Progress { done, total, .. } = &update {
            let mut queue = lock(&self.queue);
            if let Some(current) = queue.worker(self.worker).and_then(|worker| worker.current.as_mut()) {
                current.done = *done;
                current.total = *total;
            }
            queue.tell(false);
        }
        let _ = self.updates.send(update);
    }
}

/// Takes worker `id` off the queue's list however it ends (out of work, aborted, a panic), and
/// tells the notice: the last one gone takes the notice and the service with it.
struct WorkerGuard {
    id: usize,
    queue: Arc<Mutex<Queue>>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let mut queue = lock(&self.queue);
        queue.workers.retain(|worker| worker.id != self.id);
        if queue.workers.is_empty() {
            queue.held = false;
        }
        queue.tell(true);
    }
}

/// What a transfer reports to the UI.
enum Update {
    /// A series' or season's episodes, fetched for the queue.
    Expanded(Vec<BaseItemDto>),
    /// A worker took the item.
    Started {
        item_id: String,
    },
    /// The network went metered under it, with Wi-Fi only on: it's back at the front of the
    /// queue, its `.part` kept.
    Waiting {
        item_id: String,
    },
    Progress {
        item_id: String,
        done: u64,
        total: Option<u64>,
    },
    Done {
        item_id: String,
        size: u64,
        poster: Option<PathBuf>,
    },
    Failed {
        item_id: String,
        message: String,
    },
}

/// `index.json`: the queue and each item's persisted state. Done downloads are found on disk
/// instead, so they aren't in it.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Index {
    #[serde(default)]
    entries: Vec<IndexEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IndexEntry {
    item: BaseItemDto,
    #[serde(default)]
    state: SavedState,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
enum SavedState {
    #[default]
    Queued,
    Paused,
    Failed {
        message: String,
    },
}

impl Downloads {
    /// `wifi_only` is the *Download over Wi-Fi only* setting as it is now
    /// ([`Downloads::set_wifi_only`] follows its changes).
    pub fn new(client: Client, account_key: &str, ctx: &egui::Context, wifi_only: bool) -> Self {
        let (updates_from, updates) = task::channel(ctx);
        let mut downloads = Self {
            client,
            account: account_key.to_owned(),
            ctx: ctx.clone(),
            entries: Vec::new(),
            queue: Arc::default(),
            expanding: Vec::new(),
            stopped: false,
            wifi_only: watch::Sender::new(wifi_only),
            unmetered: UNMETERED.subscribe(),
            updates,
            updates_from,
            index_writes: Arc::default(),
            index_asked: 0,
        };
        downloads.load();
        downloads
    }

    /// A network of the test's own in place of the device's, which is process-wide.
    #[cfg(test)]
    fn with_network(mut self, unmetered: watch::Receiver<bool>) -> Self {
        self.unmetered = unmetered;
        self
    }

    /// *Download over Wi-Fi only* changed: the workers hold, or carry on, at once.
    pub fn set_wifi_only(&mut self, wifi_only: bool) {
        self.wifi_only.send_replace(wifi_only);
    }

    /// Whether the queue is waiting for an unmetered network, for a page to say so. Not while
    /// it's stopped (offline, or signed out), when Wi-Fi wouldn't start it.
    pub fn waiting_for_wifi(&self) -> bool {
        !self.stopped && *self.wifi_only.borrow() && !*self.unmetered.borrow()
    }

    /// Keeps the queue where it is until this account signs in again (`stopped`): for a session
    /// signed in without the server (offline), where nothing can be downloaded and a queued
    /// transfer must neither start nor fail. What's already on disk plays as usual.
    pub fn hold(&mut self) {
        self.stopped = true;
        // Reports already on their way would show up as a transfer with nothing behind it.
        self.updates.drain().for_each(drop);
    }

    /// The queue as it was saved, with each item's state read from its folder: a finished file
    /// wins over what `index.json` remembers.
    fn load(&mut self) {
        let dir = account_dir(&self.account);
        let index: Index = read_json(&dir.join(INDEX_FILE)).unwrap_or_default();
        for entry in index.entries {
            let item_dir = item_dir(&self.account, &entry.item.id);
            // A finished download's own `item.json` over the index's copy, which is the item as
            // it was queued: one that finished with no frame to save the index after it (an
            // Android app in the background) is still in it, and only `item.json` has what its
            // playback since wrote there (`record_progress`).
            let (item, state, poster) = match finished(&item_dir) {
                Some(finished) => (
                    finished.item,
                    DownloadState::Done { size: finished.size },
                    finished.poster,
                ),
                None => (
                    entry.item,
                    match entry.state {
                        SavedState::Queued => DownloadState::Queued,
                        SavedState::Paused => DownloadState::Paused,
                        SavedState::Failed { message } => DownloadState::Failed(message),
                    },
                    None,
                ),
            };
            self.entries.push(Entry { item, state, poster });
        }
        // Finished downloads the index doesn't know (one it forgot, or one copied in).
        for (item, finished) in scan_finished(&dir) {
            if !self.entries.iter().any(|entry| entry.item.id == item.id) {
                self.entries.push(Entry {
                    item,
                    state: DownloadState::Done { size: finished.size },
                    poster: finished.poster,
                });
            }
        }
        // Downloads whose file is cut short or empty (a power loss between the rename and the
        // disk) can't play, so they come again.
        for item in scan_damaged(&dir) {
            if !self.entries.iter().any(|entry| entry.item.id == item.id) {
                tracing::warn!(item = %item.name, "the downloaded file isn't whole; queueing it again");
                self.entries.push(Entry {
                    item,
                    state: DownloadState::Queued,
                    poster: None,
                });
            }
        }
        lock(&self.queue).waiting = self
            .entries
            .iter()
            .filter(|entry| entry.state == DownloadState::Queued)
            .map(|entry| entry.item.clone())
            .collect();
    }

    /// Queues `item` for download: a movie, episode or video straight away, a series or season
    /// as the episodes it expands to, fetched on tokio. Items already queued or done are left
    /// alone. Returns the reason when the server doesn't allow the download, for the caller to
    /// show.
    pub fn request(&mut self, item: &BaseItemDto) -> Result<(), String> {
        match item.kind {
            // A folder's own `CanDownload` is `false` even when its episodes can be downloaded,
            // so each episode is checked as it's queued.
            BaseItemKind::Series | BaseItemKind::Season => {
                self.expand(item);
                Ok(())
            }
            BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video => {
                if !item.can_download {
                    return Err(tr!("downloads-error-not-downloadable"));
                }
                self.queue(vec![item.clone()]);
                Ok(())
            }
            _ => Err(tr!("downloads-error-not-media")),
        }
    }

    /// Fetches a series' or season's episodes on tokio and queues what arrived; the list can be
    /// long, so the UI doesn't wait for it.
    fn expand(&mut self, item: &BaseItemDto) {
        let (client, item, updates) = (self.client.clone(), item.clone(), self.updates_from.clone());
        let task = task::Task::spawn(&self.ctx, async move {
            match crate::item_menu::episodes_of(&client, &item).await {
                Ok(episodes) => {
                    let _ = updates.send(Update::Expanded(episodes));
                }
                Err(err) => tracing::warn!(series = %item.name, "couldn't list the episodes to download: {err}"),
            }
        });
        self.expanding.push(task);
    }

    /// Adds `items` to the queue, skipping the ones already queued or done and the ones the
    /// server doesn't let this account download.
    fn queue(&mut self, items: Vec<BaseItemDto>) {
        let mut added = false;
        for item in items {
            if !item.can_download {
                tracing::info!(item = %item.name, "not downloadable for this account; skipped");
                continue;
            }
            if self.entries.iter().any(|entry| entry.item.id == item.id) {
                continue;
            }
            lock(&self.queue).waiting.push_back(item.clone());
            self.entries.push(Entry {
                item,
                state: DownloadState::Queued,
                poster: None,
            });
            added = true;
        }
        if added {
            self.save();
        }
    }

    /// Stops a transfer and deletes what it wrote. For a finished download that's
    /// [`Downloads::remove`].
    pub fn cancel(&mut self, item_id: &str) {
        let abort = {
            let mut queue = lock(&self.queue);
            queue.waiting.retain(|item| item.id != item_id);
            let running = queue.workers.iter().position(|worker| {
                worker
                    .current
                    .as_ref()
                    .is_some_and(|current| current.item_id == item_id)
            });
            running.and_then(|index| queue.workers.remove(index).abort)
        };
        // Outside the lock: the worker's guard takes it when the worker goes.
        if let Some(abort) = abort {
            abort.abort();
        }
        self.entries.retain(|entry| entry.item.id != item_id);
        delete_dir(item_dir(&self.account, item_id));
        self.save();
    }

    /// Deletes a finished download, and its item from the queue.
    pub fn remove(&mut self, item_id: &str) {
        self.cancel(item_id);
    }

    /// Queues a failed download again.
    pub fn retry(&mut self, item_id: &str) {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.item.id == item_id) else {
            return;
        };
        if matches!(entry.state, DownloadState::Failed(_)) {
            entry.state = DownloadState::Queued;
            lock(&self.queue).waiting.push_back(entry.item.clone());
            self.save();
        }
    }

    /// What the item's download is doing, if it's queued or on disk.
    pub fn state(&self, item_id: &str) -> Option<DownloadState> {
        self.entries
            .iter()
            .find(|entry| entry.item.id == item_id)
            .map(|entry| entry.state.clone())
    }

    /// The queue in order, finished downloads included, for a Downloads page.
    pub fn entries(&self) -> impl Iterator<Item = (&BaseItemDto, &DownloadState)> {
        self.entries.iter().map(|entry| (&entry.item, &entry.state))
    }

    /// The poster saved with the item's download (`primary.<ext>`), for a page drawing it
    /// where the server can't be reached. The offline Downloads page draws these.
    pub fn primary_image_path(&self, item_id: &str) -> Option<PathBuf> {
        self.entries
            .iter()
            .find(|entry| entry.item.id == item_id)
            .and_then(|entry| entry.poster.clone())
    }

    /// Reads the item's `item.json` again, for the page's own copy to follow what the playback
    /// of a downloaded file wrote there (`record_progress`): its resume point and watched
    /// state. Called when such a session ends, after that write.
    pub fn refresh_item(&mut self, item_id: &str) {
        if !self.entries.iter().any(|entry| entry.item.id == item_id) {
            return;
        }
        let dir = item_dir(&self.account, item_id);
        let Some(local) = read_json::<LocalItem>(&dir.join(ITEM_FILE)) else {
            return;
        };
        let Some(entry) = self.entry_mut(item_id) else {
            return;
        };
        if local.item.user_data.is_some() {
            entry.item.user_data = local.item.user_data;
        }
        if entry.poster.is_none() {
            entry.poster = local.primary_image.map(|file| dir.join(file));
        }
    }

    /// What can be done about `item`'s download, `None` for what can't be downloaded. A series
    /// or season always offers its episodes: which of them the account may download, and which
    /// are already here, is sorted out as they're queued.
    pub fn offer(&self, item: &BaseItemDto) -> Option<Offer> {
        match item.kind {
            BaseItemKind::Series | BaseItemKind::Season => Some(Offer::Download),
            BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video => Some(match self.state(&item.id) {
                None if item.can_download => Offer::Download,
                None => return None,
                Some(DownloadState::Done { .. }) => Offer::Remove,
                Some(DownloadState::Failed(_)) => Offer::Retry,
                Some(_) => Offer::Cancel,
            }),
            _ => None,
        }
    }

    /// How the episodes of a series or season among the downloads add up, for its card. `None`
    /// for anything that isn't one.
    pub fn children_progress(&self, item: &BaseItemDto) -> Option<ChildrenProgress> {
        if !matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season) {
            return None;
        }
        let mut progress = ChildrenProgress::default();
        for entry in &self.entries {
            let belongs = match item.kind {
                BaseItemKind::Series => entry.item.series_id.as_deref() == Some(item.id.as_str()),
                _ => entry.item.season_id.as_deref() == Some(item.id.as_str()),
            };
            if !belongs {
                continue;
            }
            match entry.state {
                DownloadState::Done { .. } => progress.done += 1,
                DownloadState::Failed(_) => progress.failed += 1,
                _ => progress.pending += 1,
            }
        }
        Some(progress)
    }

    /// Drains what the transfers reported, starts the next queued ones, and returns what
    /// finished or failed for [`Services`] to toast.
    pub fn tick(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        let mut settled = false;
        let updates: Vec<Update> = self.updates.drain().collect();
        for update in updates {
            match update {
                Update::Expanded(items) => self.queue(items),
                Update::Started { item_id } => {
                    if let Some(entry) = self.entry_mut(&item_id) {
                        let total = source_size(&entry.item);
                        entry.state = DownloadState::Downloading { done: 0, total };
                    }
                }
                Update::Waiting { item_id } => {
                    if let Some(entry) = self.entry_mut(&item_id) {
                        entry.state = DownloadState::Queued;
                    }
                }
                Update::Progress { item_id, done, total } => {
                    if let Some(entry) = self.entry_mut(&item_id) {
                        entry.state = DownloadState::Downloading { done, total };
                    }
                }
                Update::Done { item_id, size, poster } => {
                    let Some(entry) = self.entry_mut(&item_id) else {
                        continue;
                    };
                    entry.state = DownloadState::Done { size };
                    entry.poster = poster;
                    notices.push(Notice::Downloaded {
                        title: playback::title(&entry.item),
                    });
                    settled = true;
                }
                Update::Failed { item_id, message } => {
                    let Some(entry) = self.entry_mut(&item_id) else {
                        continue;
                    };
                    tracing::warn!(item = %entry.item.name, %message, "download failed");
                    entry.state = DownloadState::Failed(message.clone());
                    notices.push(Notice::Failed {
                        title: playback::title(&entry.item),
                        message,
                    });
                    settled = true;
                }
            }
        }
        if settled {
            self.save();
        }
        // Expansions that finished (their result came through the channel) are done with.
        self.expanding.retain_mut(|task| task.poll().is_none());
        self.start_transfers();
        notices
    }

    /// Stops every transfer, saving what isn't finished as queued: the account's next sign-in
    /// starts it again, from its `.part` when there is one.
    pub fn pause(&mut self) {
        self.stopped = true;
        let aborts: Vec<_> = lock(&self.queue)
            .workers
            .drain(..)
            .filter_map(|worker| worker.abort)
            .collect();
        // Outside the lock, which each worker's guard takes as it goes (and tells the notice
        // there's nothing left).
        for abort in aborts {
            abort.abort();
        }
        let mut stopped = false;
        for entry in &mut self.entries {
            if matches!(entry.state, DownloadState::Downloading { .. }) {
                entry.state = DownloadState::Queued;
                stopped = true;
            }
        }
        // Reports already on their way would show up as a transfer with nothing behind it.
        self.updates.drain().for_each(drop);
        // The session is ending (sign-out, a switch, quitting): the write is made here, as it
        // may be the last thing this process does, and a change saved on a thread a moment ago
        // that hasn't landed goes with it.
        if stopped || self.unsaved() {
            self.save_now();
        }
    }

    fn entry_mut(&mut self, item_id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.item.id == item_id)
    }

    /// Spawns workers while items wait and fewer than [`MAX_TRANSFERS`] run. Each takes items
    /// until none wait, so the UI only has to do this when it queued something.
    fn start_transfers(&mut self) {
        if self.stopped {
            return;
        }
        loop {
            let id = {
                let mut queue = lock(&self.queue);
                // Workers with nothing yet (just spawned, or held for Wi-Fi) will take what waits.
                let idle = queue.workers.iter().filter(|worker| worker.current.is_none()).count();
                if queue.workers.len() >= MAX_TRANSFERS || idle >= queue.waiting.len() {
                    return;
                }
                let id = queue.next_worker;
                queue.next_worker += 1;
                queue.workers.push(Worker {
                    id,
                    abort: None,
                    current: None,
                });
                id
            };
            let worker = work(
                id,
                self.queue.clone(),
                self.client.clone(),
                self.account.clone(),
                self.updates_from.clone(),
                self.wifi_only.subscribe(),
                self.unmetered.clone(),
            );
            let abort = crate::runtime::start(worker).abort_handle();
            if let Some(worker) = lock(&self.queue).worker(id) {
                worker.abort = Some(abort);
            }
        }
    }

    /// Writes the queue and each item's persisted state, atomically like the config, on a
    /// blocking thread: the write waits for the disk, which another transfer may be keeping busy,
    /// and a frame mustn't. The snapshot is numbered, and [`write_index`] drops one a newer
    /// snapshot has already replaced, so writes landing out of order can't bring back old state.
    fn save(&mut self) {
        let (seq, path, index) = self.index_snapshot();
        let writes = self.index_writes.clone();
        crate::runtime::detach(async move {
            if let Err(err) = tokio::task::spawn_blocking(move || write_index(&writes, seq, &path, &index)).await {
                tracing::warn!("couldn't save the download queue: {err}");
            }
        });
    }

    /// [`Downloads::save`] on this thread, for the last write of a session: nothing can be left
    /// waiting on a thread that may not run again before the process ends. It takes its turn
    /// behind a write already under way, and any older snapshot still queued is dropped after it.
    fn save_now(&mut self) {
        let (seq, path, index) = self.index_snapshot();
        write_index(&self.index_writes, seq, &path, &index);
    }

    /// Whether a snapshot asked for hasn't reached the disk yet.
    fn unsaved(&self) -> bool {
        self.index_writes.written.load(Ordering::Acquire) < self.index_asked
    }

    /// The queue as it is now, numbered after the ones before it.
    fn index_snapshot(&mut self) -> (u64, PathBuf, Index) {
        self.index_asked += 1;
        let index = Index {
            entries: self
                .entries
                .iter()
                .filter(|entry| !matches!(entry.state, DownloadState::Done { .. }))
                .map(|entry| IndexEntry {
                    item: entry.item.clone(),
                    state: match &entry.state {
                        DownloadState::Queued | DownloadState::Downloading { .. } => SavedState::Queued,
                        DownloadState::Paused => SavedState::Paused,
                        DownloadState::Failed(message) => SavedState::Failed {
                            message: message.clone(),
                        },
                        DownloadState::Done { .. } => SavedState::Queued,
                    },
                })
                .collect(),
        };
        (self.index_asked, account_dir(&self.account).join(INDEX_FILE), index)
    }

    /// Waits until every snapshot asked for is on the disk, for the tests that read it.
    #[cfg(test)]
    fn wait_saved(&self) {
        let start = Instant::now();
        while self.unsaved() {
            assert!(start.elapsed() < Duration::from_secs(10), "the queue was never saved");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// What the writes of one account's `index.json` share: the lock that makes them take turns, and
/// the number of the newest snapshot written (readable without the lock, which a write holds
/// through its fsync).
#[derive(Default)]
struct IndexWrites {
    writing: Mutex<()>,
    written: AtomicU64,
}

/// Writes snapshot `seq` of the queue unless a newer one has been written already. A save waits
/// for the lock, so a slow older write can't land its rename after a newer one's; the one that
/// finds a newer snapshot on the disk just goes.
fn write_index(writes: &IndexWrites, seq: u64, path: &Path, index: &Index) {
    let _writing = writes.writing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if writes.written.load(Ordering::Acquire) >= seq {
        return;
    }
    if index.entries.is_empty() && !path.exists() {
        // Nothing queued yet: don't leave an empty queue behind for every account signed in.
        writes.written.store(seq, Ordering::Release);
        return;
    }
    match write_json(path, index) {
        Ok(()) => writes.written.store(seq, Ordering::Release),
        Err(err) => tracing::warn!("couldn't save the download queue: {err}"),
    }
}

/// Dropped with the session (after [`Services::end`] paused it): the transfers stop, and what
/// wasn't finished stays queued for the account's next sign-in.
impl Drop for Downloads {
    fn drop(&mut self) {
        self.pause();
    }
}

/// Toasts for what the downloads did, as `Services::tick` runs each frame.
impl Services {
    pub(crate) fn tick_downloads(&mut self) {
        for notice in self.downloads.tick() {
            let toast = match notice {
                Notice::Downloaded { title } => Toast::success(tr!("downloads-toast-downloaded", title = title)),
                Notice::Failed { title, message } => {
                    Toast::error(tr!("downloads-toast-failed", title = title, message = message))
                }
            };
            self.toast(toast);
        }
    }
}

/// The finished download of `item_id`, for playback. Reads the disk, so call it off the UI
/// thread (`tokio::task::spawn_blocking`).
pub fn find_local(account_key: &str, item_id: &str) -> Option<LocalMedia> {
    let dir = item_dir(account_key, item_id);
    let local: LocalItem = read_json(&dir.join(ITEM_FILE))?;
    let path = dir.join(&local.file);
    // A finished file is the source of truth: a `.part` isn't a download yet, nor is a file that
    // isn't the size it should be.
    media_length(&dir, &local)?;
    Some(LocalMedia {
        path,
        source: local.source,
        subtitles: local
            .subtitles
            .iter()
            .map(|subtitle| (subtitle.index, dir.join(&subtitle.file)))
            .filter(|(_, path)| path.is_file())
            .collect(),
        chapters: local.item.chapters,
        segments: local.segments,
    })
}

/// `session.json`: the user and the server the account was signed in to.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSession {
    user: UserDto,
    server: PublicSystemInfo,
}

/// Keeps who this account was signed in as, so a launch the server can't answer still knows
/// its name and can show its downloads. A small file write: call it off the UI thread
/// ([`Services::new`] does, on a blocking thread).
pub fn save_session(account_key: &str, user: &UserDto, server: &PublicSystemInfo) {
    let session = SavedSession {
        user: user.clone(),
        server: server.clone(),
    };
    if let Err(err) = write_json(&account_dir(account_key).join(SESSION_FILE), &session) {
        tracing::warn!("couldn't save the session for offline use: {err}");
    }
}

/// The session saved for the account, for a launch the server can't answer. The offline screen
/// signs in with it.
pub fn saved_session(account_key: &str) -> Option<(UserDto, PublicSystemInfo)> {
    let saved: SavedSession = read_json(&account_dir(account_key).join(SESSION_FILE))?;
    Some((saved.user, saved.server))
}

/// Whether the account has a finished download on disk: what makes opening it without the
/// server worth it. The launch's offline check asks this.
pub fn has_downloads(account_key: &str) -> bool {
    !scan_finished(&account_dir(account_key)).is_empty()
}

/// Writes the watch state a downloaded file's playback leaves, so the next offline start
/// resumes where this one stopped: its position, and played (with no position) once it went
/// past the last tenth or ended. A small read and write: call it off the UI thread.
pub fn record_progress(account_key: &str, item_id: &str, position: Ticks, finished: bool) {
    let path = item_dir(account_key, item_id).join(ITEM_FILE);
    let Some(mut local) = read_json::<LocalItem>(&path) else {
        return;
    };
    let total = local.item.run_time_ticks.or(local.source.run_time_ticks);
    let played = finished || total.is_some_and(|total| position.0 as f64 >= total.0 as f64 * PLAYED_SHARE);
    let data = local.item.user_data.get_or_insert_with(Default::default);
    data.played = played;
    data.playback_position_ticks = if played { Ticks::ZERO } else { position };
    if played {
        data.last_played_date = Some(jellyfin::time::format_utc(std::time::SystemTime::now()));
    }
    if let Err(err) = write_json(&path, &local) {
        tracing::warn!("couldn't save where the downloaded item stopped: {err}");
    }
}

/// A stop report the server didn't get, kept in the account's `pending.json` until it can.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingReport {
    pub item_id: String,
    pub media_source_id: String,
    pub position_ticks: Ticks,
    /// When the playback ended, in the server's ISO-8601 format, to tell a later watch of the
    /// item (here or on another device) apart.
    pub played_at: String,
}

/// Held across each read-modify-write of `pending.json` ([`add_pending`], and what
/// [`flush_pending`] writes back), and never across the network: each would otherwise write
/// back what it read and lose the other's change. A plain mutex, since nothing waits on anything
/// while it's held. One for every account's file, as each hold is a small file's.
static PENDING: Mutex<()> = Mutex::new(());

fn lock_pending() -> MutexGuard<'static, ()> {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Appends a stop report the server didn't get, for [`flush_pending`] to send later. A small
/// read and write: call it off the UI thread.
pub fn add_pending(account_key: &str, report: PendingReport) {
    let path = account_dir(account_key).join(PENDING_FILE);
    let _writing = lock_pending();
    let mut pending: Vec<PendingReport> = read_json(&path).unwrap_or_default();
    // Only the item's latest stop says where it stands; sent after an earlier one, it would
    // look older than the playback that one recorded and be skipped.
    pending.retain(|earlier| earlier.item_id != report.item_id);
    pending.push(report);
    if let Err(err) = write_json(&path, &pending) {
        tracing::warn!("couldn't save the playback reports waiting for the server: {err}");
    }
}

/// Sends the stop reports kept while the server couldn't be reached, in the order they
/// happened. An item the server has seen played since (its `LastPlayedDate` is newer than the
/// report's `played_at`) is left alone: that watch wins. Entries the server took or that are
/// settled are dropped, and one that fails to go stays for the next sign-in, with the ones
/// behind it (a report mustn't overtake an earlier one).
pub async fn flush_pending(client: &Client, account_key: &str) {
    let path = account_dir(account_key).join(PENDING_FILE);
    let reading = path.clone();
    let pending: Vec<PendingReport> = match tokio::task::spawn_blocking(move || read_json(&reading)).await {
        Ok(Some(pending)) => pending,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!("couldn't read the pending playback reports: {err}");
            return;
        }
    };
    if pending.is_empty() {
        return;
    }
    let snapshot = pending.clone();
    // Each item's last report only, as `add_pending` keeps them: the server would take an
    // earlier one's stop as a playback newer than the later one.
    let pending: Vec<PendingReport> = pending
        .iter()
        .enumerate()
        .filter(|(index, report)| !pending[index + 1..].iter().any(|later| later.item_id == report.item_id))
        .map(|(_, report)| report.clone())
        .collect();
    tracing::info!(count = pending.len(), "sending the playback reports the server missed");
    let mut left: Vec<PendingReport> = Vec::new();
    let mut blocked = false;
    for report in pending {
        if blocked {
            left.push(report);
            continue;
        }
        let item = match client.item(&report.item_id, &[]).await {
            Ok(item) => item,
            Err(jellyfin::Error::NotFound) => {
                tracing::info!(item = %report.item_id, "the server no longer has the item; dropping its playback report");
                continue;
            }
            Err(err) => {
                tracing::warn!(item = %report.item_id, "couldn't check where the item stands: {err}");
                blocked = true;
                left.push(report);
                continue;
            }
        };
        let watched_at = item
            .user_data
            .as_ref()
            .and_then(|data| data.last_played_date.as_deref())
            .and_then(jellyfin::time::parse_utc);
        if let Some(watched_at) = watched_at
            && jellyfin::time::parse_utc(&report.played_at).is_some_and(|at| watched_at > at)
        {
            tracing::info!(item = %report.item_id, "the item was played again since; leaving the server's state");
            continue;
        }
        let stop = PlaybackReport {
            item_id: report.item_id.clone(),
            media_source_id: report.media_source_id.clone(),
            play_session_id: None,
            position: report.position_ticks,
            ..Default::default()
        };
        match client.report_playback_stopped(&stop).await {
            Ok(()) => tracing::info!(
                item = %report.item_id,
                position = report.position_ticks.as_secs_f64(),
                "reported a playback the server missed"
            ),
            Err(err) => {
                tracing::warn!(item = %report.item_id, "couldn't report a playback the server missed: {err}");
                blocked = true;
                left.push(report);
            }
        }
    }
    // What the file holds now, not what it held when this began: a report added while the
    // requests were out is in it. Only the reports that were read and aren't left to retry come
    // out.
    let writing = path.clone();
    let written = tokio::task::spawn_blocking(move || {
        let _writing = lock_pending();
        let mut current: Vec<PendingReport> = read_json(&writing).unwrap_or_default();
        current.retain(|report| !snapshot.contains(report) || left.contains(report));
        if current.is_empty() {
            match std::fs::remove_file(&writing) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        } else {
            write_json(&writing, &current)
        }
    })
    .await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!("couldn't save the playback reports still to send: {err}"),
        Err(err) => tracing::warn!("couldn't save the playback reports still to send: {err}"),
    }
}

/// One worker: takes the queue's next item and downloads it, until nothing waits. With Wi-Fi
/// only on it waits for an unmetered network first, and stops a transfer the network turns
/// metered under, putting its item back at the front of the queue.
async fn work(
    id: usize,
    queue: Arc<Mutex<Queue>>,
    client: Client,
    account: String,
    updates: task::Sender<Update>,
    mut wifi_only: watch::Receiver<bool>,
    mut unmetered: watch::Receiver<bool>,
) {
    let _guard = WorkerGuard {
        id,
        queue: queue.clone(),
    };
    loop {
        if held(&mut wifi_only, &mut unmetered) {
            tracing::info!("downloads are waiting for Wi-Fi");
            set_held(&queue, true);
            if until_held(false, &mut wifi_only, &mut unmetered).await.is_err() {
                return;
            }
            set_held(&queue, false);
        }
        let Some(item) = lock(&queue).take(id) else {
            return;
        };
        let _ = updates.send(Update::Started {
            item_id: item.id.clone(),
        });
        let reporter = Reporter {
            updates: updates.clone(),
            queue: queue.clone(),
            worker: id,
        };
        let dir = item_dir(&account, &item.id);
        let failed = tokio::select! {
            failed = transfer(client.clone(), item.clone(), dir, &reporter) => failed,
            Ok(()) = until_held(true, &mut wifi_only, &mut unmetered) => Some(Failure::OffWifi),
        };
        let off_wifi = match failed {
            None => false,
            Some(Failure::OffWifi) => true,
            // Leaving Wi-Fi breaks the connection a moment before the phone says the network
            // changed: a failure with Wi-Fi only on waits that moment out, and is the queue
            // waiting for Wi-Fi rather than a failure if the network turns out metered.
            Some(Failure::Error(message)) => {
                let left_wifi = *wifi_only.borrow()
                    && matches!(
                        tokio::time::timeout(NETWORK_SETTLE, until_held(true, &mut wifi_only, &mut unmetered)).await,
                        Ok(Ok(()))
                    );
                if !left_wifi {
                    reporter.send(Update::Failed {
                        item_id: item.id.clone(),
                        message,
                    });
                }
                left_wifi
            }
        };
        if off_wifi {
            tracing::info!(item = %item.name, "off Wi-Fi: the download waits");
            let item_id = item.id.clone();
            lock(&queue).waiting.push_front(item);
            let _ = updates.send(Update::Waiting { item_id });
        }
        let mut queue = lock(&queue);
        if let Some(worker) = queue.worker(id) {
            worker.current = None;
        }
        queue.tell(true);
    }
}

/// Whether downloads wait now: Wi-Fi only is on and there's no unmetered network. Marks both
/// values seen, so [`until_held`] waits for the next change.
fn held(wifi_only: &mut watch::Receiver<bool>, unmetered: &mut watch::Receiver<bool>) -> bool {
    let wifi_only = *wifi_only.borrow_and_update();
    let unmetered = *unmetered.borrow_and_update();
    wifi_only && !unmetered
}

/// Waits until [`held`] is `want`. An error once [`Downloads`] is gone (its setting's sender
/// with it); the network's sender never goes.
async fn until_held(
    want: bool,
    wifi_only: &mut watch::Receiver<bool>,
    unmetered: &mut watch::Receiver<bool>,
) -> Result<(), watch::error::RecvError> {
    loop {
        if held(wifi_only, unmetered) == want {
            return Ok(());
        }
        tokio::select! {
            changed = wifi_only.changed() => changed?,
            changed = unmetered.changed() => changed?,
        }
    }
}

fn set_held(queue: &Mutex<Queue>, held: bool) {
    let mut queue = lock(queue);
    queue.held = held;
    queue.tell(true);
}

/// Why a worker's transfer ended without the file.
enum Failure {
    /// The network went metered under it, with Wi-Fi only on.
    OffWifi,
    /// What went wrong, for the user.
    Error(String),
}

/// Downloads one item; `None` once it's done. A failure is one message: the transfer's own
/// errors are user-facing.
async fn transfer(client: Client, item: BaseItemDto, dir: PathBuf, updates: &Reporter) -> Option<Failure> {
    // A panic in the transfer would otherwise leave the item downloading forever; the UI hears
    // about it like any other failure.
    let result = AssertUnwindSafe(download(&client, &item, &dir, updates))
        .catch_unwind()
        .await;
    match result {
        Ok(Ok(())) => None,
        Ok(Err(message)) => Some(Failure::Error(message)),
        Err(_) => Some(Failure::Error(tr!("downloads-error-stopped"))),
    }
}

/// `item.json`, its subtitles and images, then the media file, resuming where a `.part` stopped.
async fn download(client: &Client, item: &BaseItemDto, dir: &Path, updates: &Reporter) -> Result<(), String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| tr!("downloads-error-folder", error = err.to_string()))?;
    // `PlaybackInfo` gives the version as playback would negotiate it, with its defaults; the
    // item's own fetch supplies chapters and the sources a page shows.
    let info = client
        .playback_info(&item.id, &PlaybackInfoRequest::default())
        .await
        .map_err(|err| describe_error(&err))?;
    let source = info
        .media_sources
        .into_iter()
        .next()
        .ok_or_else(|| tr!("downloads-error-no-media"))?;
    let full = client
        .item(
            &item.id,
            &[ItemField::Chapters, ItemField::MediaSources, ItemField::MediaStreams],
        )
        .await
        .map_err(|err| describe_error(&err))?;
    let file = format!("media.{}", extension(source.container.as_deref()));
    let size = source.size.unwrap_or_default().max(0) as u64;

    let (subtitles, (primary_image, backdrop_image), segments) = tokio::join!(
        subtitles(client, item, &source, dir),
        images(client, item, dir),
        async {
            client
                .media_segments(&item.id)
                .await
                .inspect_err(|err| tracing::debug!(item = %item.name, "no media segments: {err}"))
                .unwrap_or_default()
        },
    );
    let local = LocalItem {
        item: full,
        source,
        subtitles,
        segments,
        primary_image,
        backdrop_image,
        file,
        size,
        verified_size: None,
    };
    let local = save_item(dir, local).await?;
    let Patience { stall, pause } = lock(&updates.queue).patience;
    let mut stalls = 0;
    loop {
        match fetch_media(client, &local, dir, stall, updates).await {
            Ok(()) => return Ok(()),
            Err(Fetch::Failed(message)) => return Err(message),
            Err(Fetch::Stalled(message)) if stalls >= STALL_RETRIES => return Err(message),
            Err(Fetch::Stalled(_)) => {
                stalls += 1;
                tracing::info!(item = %item.name, attempt = stalls, "the download stalled; trying again shortly");
                // Cancelling or pausing aborts the worker, and with it this wait.
                tokio::time::sleep(pause).await;
            }
        }
    }
}

/// Why an attempt at the media file ended without it.
enum Fetch {
    /// The server went quiet ([`jellyfin::Error::Stalled`]): worth another attempt.
    Stalled(String),
    Failed(String),
}

impl From<String> for Fetch {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

fn fetch_error(err: &jellyfin::Error) -> Fetch {
    let message = describe_error(err);
    match err {
        jellyfin::Error::Stalled => Fetch::Stalled(message),
        _ => Fetch::Failed(message),
    }
}

/// The media request, with the stall limit the transfer waits for (`None`: the client's own).
async fn request_media(
    client: &Client,
    url: Url,
    offset: u64,
    if_range: Option<&str>,
    stall: Option<Duration>,
) -> Result<jellyfin::Download, jellyfin::Error> {
    match stall {
        Some(stall) => client.download_with(url, offset, if_range, stall).await,
        None => client.download_resuming(url, offset, if_range).await,
    }
}

/// Writes `item.json` on a blocking thread, since `write_json` waits for the disk to have it.
async fn save_item(dir: &Path, local: LocalItem) -> Result<LocalItem, String> {
    let path = dir.join(ITEM_FILE);
    let failed = |error: String| tr!("downloads-error-write-item", file = ITEM_FILE, error = error);
    tokio::task::spawn_blocking(move || write_json(&path, &local).map(|()| local))
        .await
        .map_err(|err| failed(err.to_string()))?
        .map_err(|err| failed(err.to_string()))
}

/// The media file itself: from the end of an unfinished `.part` when there is one, and renamed
/// into place once all of its bytes are here.
///
/// What the `.part` holds decides where to ask from, and the server's answer whether that
/// holds: `bytes=N-` for N bytes, where a 206 carries on and a 200 or 416 starts the file over.
/// The size `PlaybackInfo` gave (`local.size`) is what the server's library believes, which its
/// file can differ from, so it only feeds the free-space check and the progress bar.
///
/// The first response's validator (`ETag`, else `Last-Modified`) is kept beside the `.part`
/// ([`validator_path`]) and sent back as `If-Range` on a resume, so a file the server replaced
/// since comes whole (a 200) and starts the file over, rather than its tail being appended to
/// the old head.
async fn fetch_media(
    client: &Client,
    local: &LocalItem,
    dir: &Path,
    stall: Option<Duration>,
    updates: &Reporter,
) -> Result<(), Fetch> {
    let item_id = &local.item.id;
    let part = dir.join(format!("{}.part", local.file));
    let finished = dir.join(&local.file);
    let mut offset = tokio::fs::metadata(&part).await.map(|meta| meta.len()).unwrap_or(0);
    let mut restart = false;
    if local.size > 0 {
        check_space(dir, local.size.saturating_sub(offset))?;
    }
    let url = client.download_url(item_id);
    let saved = if offset > 0 {
        tokio::fs::read_to_string(validator_path(&part))
            .await
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    } else {
        None
    };
    let mut download = match request_media(client, url.clone(), offset, saved.as_deref(), stall).await {
        // Nothing left to ask for: the `.part` holds the whole file, or more than the file has
        // now. A 416 doesn't prove the bytes are right, so the file starts over.
        Err(err) if offset > 0 && err.is_range_not_satisfiable() => {
            tracing::info!(item = %local.item.name, "the server has nothing past the `.part`; starting over");
            offset = 0;
            restart = true;
            request_media(client, url, 0, None, stall)
                .await
                .map_err(|err| fetch_error(&err))?
        }
        other => other.map_err(|err| fetch_error(&err))?,
    };
    // The server's own word for the file's length ends the transfer when the bytes reach it;
    // `PlaybackInfo`'s only stands in for it on the bar.
    let total = download.total;
    let shown = total.or((local.size > 0).then_some(local.size));
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&part)
        .await
        .map_err(|err| tr!("downloads-error-open", error = err.to_string()))?;
    let mut done = offset;
    if restart || (offset > 0 && !download.partial) {
        // The server ignored the range, or the file isn't the version the `.part` is of (the
        // `If-Range` didn't match), and it sent the whole file: start the file over.
        tracing::info!(item = %local.item.name, "starting the file over");
        file.set_len(0)
            .await
            .map_err(|err| tr!("downloads-error-restart", error = err.to_string()))?;
        file.seek(SeekFrom::Start(0))
            .await
            .map_err(|err| tr!("downloads-error-restart", error = err.to_string()))?;
        done = 0;
    } else {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| tr!("downloads-error-continue", error = err.to_string()))?;
    }
    if done == 0 {
        // A new file: what the next resume must match is this response's version. A failure only
        // costs that resume its check, and a stale validator only the restart it would get anyway.
        let kept = match &download.validator {
            Some(validator) => tokio::fs::write(validator_path(&part), validator).await,
            None => forget(&validator_path(&part)).await,
        };
        if let Err(err) = kept {
            tracing::warn!(item = %local.item.name, "couldn't keep the file's validator: {err}");
        }
    }
    updates.send(Update::Progress {
        item_id: item_id.clone(),
        done,
        total: shown,
    });
    let mut reported = Instant::now();
    while let Some(chunk) = download.chunk().await.map_err(|err| fetch_error(&err))? {
        file.write_all(&chunk)
            .await
            .map_err(|err| tr!("downloads-error-write", error = err.to_string()))?;
        done += chunk.len() as u64;
        if reported.elapsed() >= PROGRESS_INTERVAL {
            reported = Instant::now();
            updates.send(Update::Progress {
                item_id: item_id.clone(),
                done,
                total: shown,
            });
        }
    }
    file.flush()
        .await
        .map_err(|err| tr!("downloads-error-write", error = err.to_string()))?;
    drop(file);
    if let Some(total) = total.filter(|total| done != *total) {
        // The `.part` stays: the retry resumes from here.
        return Err(tr!("downloads-error-partial", done = done, total = total).into());
    }
    // `item.json` names the length that came, and says it was counted, before the file takes
    // its name: from then on a file of another length is a damaged one. (`PlaybackInfo`'s size
    // can differ from what the server sends.)
    let mut counted = local.clone();
    counted.size = done;
    counted.verified_size = Some(done);
    save_item(dir, counted).await?;
    Ok(finish(&part, &finished, done, local.poster(dir), updates, item_id).await?)
}

/// Where a `.part`'s validator is kept: the `ETag` or `Last-Modified` its download began with.
fn validator_path(part: &Path) -> PathBuf {
    let mut name = part.as_os_str().to_owned();
    name.push(".validator");
    PathBuf::from(name)
}

/// Deletes a file; one that's already gone is what was wanted.
async fn forget(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Renames an all-there `.part` into place and tells the UI. The bytes are synced to the disk
/// before the rename, and the folder after it: after a power loss the rename could otherwise
/// outlive the data, and an empty or short file would pass for a finished download.
async fn finish(
    part: &Path,
    finished: &Path,
    size: u64,
    poster: Option<PathBuf>,
    updates: &Reporter,
    item_id: &str,
) -> Result<(), String> {
    let failed = |err: std::io::Error| tr!("downloads-error-finish", error = err.to_string());
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(part)
        .await
        .map_err(failed)?;
    file.sync_all().await.map_err(failed)?;
    drop(file);
    tokio::fs::rename(part, finished).await.map_err(failed)?;
    if let Some(dir) = finished.parent() {
        let dir = dir.to_owned();
        let _ = tokio::task::spawn_blocking(move || sync_dir(&dir)).await;
    }
    if let Err(err) = forget(&validator_path(part)).await {
        tracing::debug!("couldn't delete the `.part`'s validator: {err}");
    }
    updates.send(Update::Done {
        item_id: item_id.to_owned(),
        size,
        poster,
    });
    Ok(())
}

/// Syncs a folder, so a rename in it survives a power loss. Unix only (Windows has no way to
/// open a folder for it); best effort, since the rename has happened either way.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Refuses before any bytes are written when the disk can't take them, with a message saying
/// what it needs and what's there. A filesystem that can't say (or an odd one, such as a
/// network share with no reported quota) only logs.
fn check_space(dir: &Path, needed: u64) -> Result<(), String> {
    match fs4::available_space(dir) {
        Ok(free) if free < needed => Err(tr!(
            "downloads-error-no-space",
            needed = bytes(needed),
            free = bytes(free)
        )),
        Ok(_) => Ok(()),
        Err(err) => {
            tracing::warn!("couldn't check the free space: {err}");
            Ok(())
        }
    }
}

/// The external subtitle files, saved next to the media. One that fails is logged and left out,
/// and playback goes without it: a missing subtitle mustn't cost the download.
async fn subtitles(client: &Client, item: &BaseItemDto, source: &MediaSourceInfo, dir: &Path) -> Vec<LocalSubtitle> {
    let mut saved = Vec::new();
    for stream in source
        .media_streams
        .iter()
        .filter(|stream| stream.kind == MediaStreamType::Subtitle && stream.is_external)
    {
        let url = client.subtitle_url(&item.id, &source.id, stream);
        let file = format!("{}.{}", stream.index, file_extension(&url));
        match read_body(client, url).await {
            Ok(body) => match tokio::fs::write(dir.join(&file), body).await {
                Ok(()) => saved.push(LocalSubtitle {
                    index: stream.index,
                    file,
                }),
                Err(err) => tracing::warn!(item = %item.name, "couldn't save a subtitle: {err}"),
            },
            Err(err) => tracing::warn!(item = %item.name, "couldn't download a subtitle: {err}"),
        }
    }
    saved
}

/// The item's poster and backdrop, in the sizes offline pages draw them. A failure is logged
/// and left out, as with subtitles.
async fn images(client: &Client, item: &BaseItemDto, dir: &Path) -> (Option<String>, Option<String>) {
    let primary = save_image(client, item.primary_image(), PRIMARY_WIDTH, "primary", dir).await;
    let backdrop = save_image(client, item.backdrop_image(), BACKDROP_WIDTH, "backdrop", dir).await;
    (primary, backdrop)
}

async fn save_image(
    client: &Client,
    request: Option<ImageRequest>,
    width: u32,
    name: &str,
    dir: &Path,
) -> Option<String> {
    let request = request?.fill_width(width);
    let url = client.image_url(&request);
    let body = match read_body(client, url).await {
        Ok(body) => body,
        Err(err) => {
            tracing::warn!("couldn't download the {name} image: {err}");
            return None;
        }
    };
    let file = format!("{name}.{}", image_extension(&body));
    match tokio::fs::write(dir.join(&file), body).await {
        Ok(()) => Some(file),
        Err(err) => {
            tracing::warn!("couldn't save the {name} image: {err}");
            None
        }
    }
}

/// A whole small file's body (an image or subtitle), through the same stall-checked stream as
/// the media.
async fn read_body(client: &Client, url: Url) -> Result<Vec<u8>, jellyfin::Error> {
    let mut download = client.download(url, 0).await?;
    let mut body = Vec::new();
    while let Some(chunk) = download.chunk().await? {
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The account's folder: the key is `<server id>/<user id>`, and anything else a file name
/// can't take becomes `_`.
fn account_dir(account_key: &str) -> PathBuf {
    config::downloads_dir().join(safe_name(account_key))
}

fn item_dir(account_key: &str, item_id: &str) -> PathBuf {
    account_dir(account_key).join(safe_name(item_id))
}

fn safe_name(value: &str) -> String {
    // No dots either: `..` would name the parent folder.
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

/// A finished download as its folder holds it: the item as `item.json` has it, the media
/// file's size, and its poster when one was saved with it.
struct Finished {
    item: BaseItemDto,
    size: u64,
    poster: Option<PathBuf>,
}

/// `Some` when the item's folder holds its finished media file; the state a done download is
/// in, whatever `index.json` says.
fn finished(dir: &Path) -> Option<Finished> {
    let local: LocalItem = read_json(&dir.join(ITEM_FILE))?;
    let size = media_length(dir, &local)?;
    Some(Finished {
        size,
        poster: local.poster(dir),
        item: local.item,
    })
}

/// The length of the item's media file in `dir` when it's all there: as long as the transfer
/// that finished it counted (`verified_size`), or any length but none for a download from before
/// that was recorded. A shorter or empty file is what a power loss can leave behind a rename,
/// and it mustn't play or count as done. `size` isn't a check: it can be the library's figure
/// for a file the server serves at another length.
fn media_length(dir: &Path, local: &LocalItem) -> Option<u64> {
    let meta = std::fs::metadata(dir.join(&local.file)).ok()?;
    let whole = meta.is_file() && meta.len() > 0 && local.verified_size.is_none_or(|verified| meta.len() == verified);
    whole.then_some(meta.len())
}

/// The items in the account's folder whose media file is there but not whole, to be downloaded
/// again.
fn scan_damaged(account: &Path) -> Vec<BaseItemDto> {
    let Ok(dir) = std::fs::read_dir(account) else {
        return Vec::new();
    };
    dir.filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let dir = entry.path();
            let local: LocalItem = read_json(&dir.join(ITEM_FILE))?;
            (dir.join(&local.file).is_file() && media_length(&dir, &local).is_none()).then_some(local.item)
        })
        .collect()
}

/// Every finished download in the account's folder, for an index that lost some.
fn scan_finished(account: &Path) -> Vec<(BaseItemDto, Finished)> {
    let Ok(dir) = std::fs::read_dir(account) else {
        return Vec::new();
    };
    let mut found: Vec<(BaseItemDto, Finished)> = dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let finished = finished(&entry.path())?;
            Some((finished.item.clone(), finished))
        })
        .collect();
    found.sort_by(|(a, _), (b, _)| a.name.cmp(&b.name));
    found
}

/// Deletes an item's folder. A transfer that's still being aborted may hold its file open for a
/// moment, and Windows won't delete a folder while one of its files is; the delete retries on
/// tokio instead of failing the first time.
fn delete_dir(dir: PathBuf) {
    crate::runtime::detach(async move {
        let mut delay = Duration::from_millis(20);
        for _ in 0..8 {
            match tokio::fs::remove_dir_all(&dir).await {
                Ok(()) => return,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
                Err(err) => {
                    tracing::debug!("couldn't delete {} yet: {err}", dir.display());
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
            }
        }
        tracing::warn!("couldn't delete {}", dir.display());
    });
}

/// A JSON file, `None` (with a warning) when it's missing or unreadable: a broken download
/// folder shouldn't take the queue down with it.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = match std::fs::read(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!("couldn't read {}: {err}", path.display());
            return None;
        }
    };
    serde_json::from_slice(&text)
        .inspect_err(|err| tracing::warn!("ignoring unreadable {}: {err}", path.display()))
        .ok()
}

/// Writes a JSON file aside and renames it over the old one, so a crash mid-write can't leave
/// half a file the next launch can't read (as `config.save` does). The data is synced before the
/// rename (and the folder after it, on Unix): after a power loss the rename could otherwise
/// survive without the data, leaving an empty file. That waits for the disk, so the callers that
/// matter run off the UI thread and the runtime's workers (`item.json`, `pending.json`).
///
/// The temp file's name is unique to the write, so two writers of one file can't trip over each
/// other's.
fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_vec_pretty(value)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(
        "{name}.{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&text)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written?;
    sync_dir(dir);
    Ok(())
}

/// The file extension for a media source's container (`mkv`); `bin` when the server doesn't say,
/// which mpv still plays (it sniffs the content).
fn extension(container: Option<&str>) -> String {
    let name = container
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    if name.is_empty() {
        "bin".to_owned()
    } else {
        name.to_ascii_lowercase()
    }
}

/// The extension in a subtitle URL, which `Client::subtitle_url` builds with the format in its
/// path (`Stream.ass`).
fn file_extension(url: &Url) -> String {
    url.path()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext)
        .filter(|ext| (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("srt")
        .to_ascii_lowercase()
}

/// The image's format from its magic bytes, for the file name (the server sends no name).
fn image_extension(body: &[u8]) -> &'static str {
    match body {
        [0xFF, 0xD8, 0xFF, ..] => "jpg",
        [0x89, b'P', b'N', b'G', ..] => "png",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        _ => "img",
    }
}

/// A byte count for a message: `4.3 GB`, `512 MB`, `12 kB`.
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    let mut value = count as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1000. {
            break;
        }
        value /= 1000.;
        unit = next;
    }
    if unit == "B" {
        format!("{count} B")
    } else {
        format!("{} {unit}", i18n::decimal(format!("{value:.1}")))
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;

    use jellyfin::ClientInfo;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::ui_tests::ConfigDir;

    const ACCOUNT: &str = "server/user";
    const ITEM: &str = "0123456789abcdef0123456789abcdef";

    fn client(server: &MockServer) -> Client {
        client_at(&server.uri())
    }

    /// A client for a URL, for the tests that need none of the server's answers.
    fn client_at(url: &str) -> Client {
        let info = ClientInfo {
            client: "tabbyfin-tests".into(),
            device: "tests".into(),
            device_id: "test-device".into(),
            version: "0.1.0".into(),
        };
        Client::new(url, info).expect("the server's URL").with_token("token")
    }

    /// Waits for the tokio runtime's answer, as the UI's tasks do: the runtime is the app's, and
    /// the test thread only waits.
    fn block_on<F>(future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (send, receive) = std::sync::mpsc::channel();
        crate::runtime::detach(async move {
            let _ = send.send(future.await);
        });
        receive.recv().expect("the runtime answered")
    }

    fn start() -> Arc<MockServer> {
        block_on(async { Arc::new(MockServer::start().await) })
    }

    /// Mounts a mock on the runtime, as the UI tests' `FixtureServer` does.
    fn mount(server: &Arc<MockServer>, mock: Mock) {
        let server = server.clone();
        block_on(async move { mock.mount(&server).await });
    }

    /// An episode or movie as the server sends one, with the chapters and images a download
    /// keeps.
    fn item_json(id: &str) -> Value {
        json!({
            "Id": id,
            "Name": "Downloaded",
            "Type": "Movie",
            "CanDownload": true,
            "ImageTags": {"Primary": "poster"},
            "MediaSources": [{
                "Id": "source",
                "Container": "mkv",
                "Size": 5,
                "MediaStreams": [{"Index": 0, "Type": "Video", "Codec": "h264"}],
            }],
            "Chapters": [{"StartPositionTicks": 0, "Name": "Intro"}],
        })
    }

    fn item(id: &str) -> BaseItemDto {
        serde_json::from_value(item_json(id)).expect("the item parses")
    }

    /// The routes a download of `id` needs: playback negotiation, the item itself and its
    /// poster. The media download is the test's own.
    fn serve_item(server: &Arc<MockServer>, id: &str) {
        serve_item_sized(server, id, Some(5));
    }

    /// [`serve_item`] with the size `PlaybackInfo` gives the file: `None` when the server doesn't
    /// know it.
    fn serve_item_sized(server: &Arc<MockServer>, id: &str, size: Option<u64>) {
        let mut source = json!({
            "Id": "source",
            "Container": "mkv",
            "MediaStreams": [{"Index": 0, "Type": "Video", "Codec": "h264"}],
        });
        if let Some(size) = size {
            source["Size"] = json!(size);
        }
        let playback = json!({"MediaSources": [source]});
        for (http_method, route, body) in [
            ("POST", format!("/Items/{id}/PlaybackInfo"), playback),
            ("GET", format!("/Items/{id}"), item_json(id)),
        ] {
            mount(
                server,
                Mock::given(method(http_method))
                    .and(path(route))
                    .respond_with(ResponseTemplate::new(200).set_body_json(&body)),
            );
        }
        mount(
            server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{id}/Images/Primary")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nposter")),
        );
    }

    /// Ticks the downloads as frames would until `done` holds, returning the notices along the
    /// way.
    fn tick_until(downloads: &mut Downloads, mut done: impl FnMut(&Downloads) -> bool) -> Vec<Notice> {
        let mut notices = Vec::new();
        let start = Instant::now();
        loop {
            notices.extend(downloads.tick());
            if done(downloads) {
                return notices;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "the download never settled");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn dir(id: &str) -> PathBuf {
        super::item_dir(ACCOUNT, id)
    }

    #[test]
    fn a_download_writes_the_file_and_what_plays_it() {
        let _config = ConfigDir::new("downloads-full");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello"))
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");

        let notices = tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(
            notices,
            [Notice::Downloaded {
                title: "Downloaded".into()
            }]
        );
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        let local = find_local(ACCOUNT, ITEM).expect("the finished download");
        assert_eq!(local.path, dir(ITEM).join("media.mkv"));
        assert_eq!(local.chapters.len(), 1);
        assert_eq!(local.source.id, "source");
        // The poster came too, and `item.json` names it.
        let saved: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        assert_eq!(saved.primary_image.as_deref(), Some("primary.png"));
        assert!(dir(ITEM).join("primary.png").is_file());
        assert_eq!(
            downloads.primary_image_path(ITEM),
            Some(dir(ITEM).join("primary.png")),
            "the page draws the saved poster, not the server's"
        );
        // The length that arrived is on record, as what a finished file is checked against.
        assert_eq!(saved.verified_size, Some(5));
        // Done downloads are found on disk, not in the index.
        downloads.wait_saved();
        let index: Index = read_json(&account_dir(ACCOUNT).join(INDEX_FILE)).expect("index.json");
        assert!(index.entries.is_empty());
    }

    fn find_dir_contents(dir: &Path) -> Vec<u8> {
        std::fs::read(dir.join("media.mkv")).expect("the media file")
    }

    /// A `.part` from an interrupted download gets the rest of the file with a range request.
    #[test]
    fn an_unfinished_download_resumes_from_its_part() {
        let _config = ConfigDir::new("downloads-resume");
        let server = start();
        serve_item(&server, ITEM);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("a partial file");
        mount(
            &server,
            // Only a request that asks for the rest of the file matches; anything else would
            // be a 404 and fail the download.
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .and(wiremock::matchers::header("range", "bytes=3-"))
                .respond_with(
                    ResponseTemplate::new(206)
                        .insert_header("content-range", "bytes 3-4/5")
                        .set_body_bytes(b"lo"),
                )
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        assert!(!dir(ITEM).join("media.mkv.part").exists());
        // No validator was kept with that `.part`, so the resume asks for the range alone.
        let asked = server.clone();
        let requests = block_on(async move { asked.received_requests().await.expect("recorded") });
        let ranged: Vec<_> = requests
            .iter()
            .filter(|request| request.headers.contains_key("range"))
            .collect();
        assert_eq!(ranged.len(), 1);
        assert!(!ranged[0].headers.contains_key("if-range"));
    }

    /// A `.part` that stopped while the file was one version resumes with that version's
    /// validator as `If-Range`; the server still has that file, so it sends the rest, and the
    /// validator goes with the `.part`.
    #[test]
    fn a_resume_sends_the_validator_it_kept() {
        let _config = ConfigDir::new("downloads-if-range");
        let server = start();
        serve_item(&server, ITEM);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("a partial file");
        std::fs::write(dir(ITEM).join("media.mkv.part.validator"), "\"v1\"").expect("a validator");
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .and(wiremock::matchers::header("range", "bytes=3-"))
                .and(wiremock::matchers::header("if-range", "\"v1\""))
                .respond_with(
                    ResponseTemplate::new(206)
                        .insert_header("content-range", "bytes 3-4/5")
                        .set_body_bytes(b"lo"),
                )
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        assert!(!dir(ITEM).join("media.mkv.part.validator").exists());
    }

    /// The file was replaced on the server while its `.part` waited: the `If-Range` no longer
    /// matches, so the server sends the new file whole (a 200), and the finished file is that
    /// one, not the old head with the new tail.
    #[test]
    fn a_file_replaced_on_the_server_is_downloaded_again_from_the_start() {
        let _config = ConfigDir::new("downloads-replaced");
        let server = start();
        serve_item(&server, ITEM);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("the old head");
        std::fs::write(dir(ITEM).join("media.mkv.part.validator"), "\"old\"").expect("a validator");
        mount(
            &server,
            // The server's answer to an `If-Range` that's out of date: the whole new file. A 206
            // would append "ld" to "hel".
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .and(wiremock::matchers::header("if-range", "\"old\""))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("etag", "\"new\"")
                        .set_body_bytes(b"world"),
                )
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"world");
        assert!(!dir(ITEM).join("media.mkv.part").exists());
    }

    /// A server on loopback whose download dies after three of its five bytes, an `ETag` in its
    /// headers: what an interrupted connection leaves. Wiremock only answers whole responses.
    /// The rest of what a transfer asks for is the item's, or a 404 (a poster, segments).
    fn dying_server(etag: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port") });
        let url = format!("http://{}", listener.local_addr().expect("its address"));
        crate::runtime::detach(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 1024];
                    while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                        match stream.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    let target = request.split_whitespace().nth(1).unwrap_or_default();
                    let target = target.split('?').next().unwrap_or_default().to_owned();
                    let reply = if target.contains("/PlaybackInfo") {
                        let source = json!({"Id": "source", "Container": "mkv", "Size": 5, "MediaStreams": []});
                        json_reply(&json!({"MediaSources": [source]}))
                    } else if target.ends_with("/Download") {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: 5\r\netag: {etag}\r\nconnection: close\r\n\r\nhel"
                        );
                        head.into_bytes()
                    } else if target.ends_with(&format!("/Items/{ITEM}")) {
                        json_reply(&item_json(ITEM))
                    } else {
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()
                    };
                    let _ = stream.write_all(&reply).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        url
    }

    fn json_reply(body: &Value) -> Vec<u8> {
        let body = body.to_string();
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// A download keeps its first response's validator beside the `.part`, for the resume.
    #[test]
    fn a_download_keeps_the_validator_of_its_first_response() {
        let _config = ConfigDir::new("downloads-validator");
        let url = dying_server("\"v1\"");
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client_at(&url), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Failed(_)))
        });
        // The connection broke after three bytes: the `.part` stays, with its validator.
        assert_eq!(
            std::fs::read(dir(ITEM).join("media.mkv.part")).expect("the part"),
            b"hel"
        );
        let kept = std::fs::read_to_string(dir(ITEM).join("media.mkv.part.validator"));
        assert_eq!(kept.expect("the validator").trim(), "\"v1\"");
    }

    /// With the file's size unknown, a `.part` that already holds all of it (a crash after the
    /// last write, before the rename) gets a 416 for the rest. That isn't the item failing: the
    /// file starts over.
    #[test]
    fn a_complete_part_of_unknown_size_starts_over_on_a_416() {
        let _config = ConfigDir::new("downloads-416");
        let server = start();
        serve_item_sized(&server, ITEM, None);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hello").expect("a complete file");
        let route = format!("/Items/{ITEM}/Download");
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(route.clone()))
                .and(wiremock::matchers::header("range", "bytes=5-"))
                .respond_with(ResponseTemplate::new(416))
                .expect(1),
        );
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello"))
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        let notices = tick_until(&mut downloads, |downloads| {
            matches!(
                downloads.state(ITEM),
                Some(DownloadState::Done { .. } | DownloadState::Failed(_))
            )
        });
        assert!(matches!(notices[..], [Notice::Downloaded { .. }]), "{notices:?}");
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        assert!(!dir(ITEM).join("media.mkv.part").exists());
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// The file the server sends can be longer than `PlaybackInfo` said (a library scan that's
    /// behind): `item.json` takes the length that came, or the finished file would never match
    /// it and the download would vanish at the next start.
    #[test]
    fn the_length_that_arrived_is_the_size_item_json_keeps() {
        let _config = ConfigDir::new("downloads-size-differs");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello!!")),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { size: 7 }))
        });
        let local = find_local(ACCOUNT, ITEM).expect("it still counts as downloaded");
        assert_eq!(std::fs::read(local.path).expect("the file"), b"hello!!");
    }

    /// A server that ignores the range sends the whole file: the `.part` is started over rather
    /// than having the file appended to it.
    #[test]
    fn a_server_ignoring_the_range_starts_the_file_over() {
        let _config = ConfigDir::new("downloads-no-range");
        let server = start();
        serve_item(&server, ITEM);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("a partial file");
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello"))
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
    }

    /// Signing out pauses a transfer; what it didn't finish is saved as queued, and the
    /// account's next sign-in (a new `Downloads`) starts it again.
    #[test]
    fn a_paused_download_is_queued_for_the_next_sign_in() {
        let _config = ConfigDir::new("downloads-paused");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_bytes(b"hello")
                        .set_delay(Duration::from_millis(500)),
                ),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Downloading { .. }))
        });
        downloads.pause();
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
        let index: Index = read_json(&account_dir(ACCOUNT).join(INDEX_FILE)).expect("index.json");
        assert_eq!(index.entries.len(), 1);
        assert!(matches!(index.entries[0].state, SavedState::Queued));
        drop(downloads);

        // The next sign-in finds the queue and carries on with it.
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
    }

    #[test]
    fn an_item_the_server_doesnt_let_this_account_download_is_refused() {
        let _config = ConfigDir::new("downloads-refused");
        let server = start();
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        let mut movie = item(ITEM);
        movie.can_download = false;
        let refused = downloads.request(&movie);
        assert_eq!(refused, Err("This item can't be downloaded from this server.".into()));
        assert_eq!(downloads.state(ITEM), None);
        assert_eq!(downloads.entries().count(), 0);
        // A folder's own flag doesn't matter; its episodes' do.
        let mut series = item("series");
        series.kind = BaseItemKind::Series;
        series.can_download = false;
        assert!(downloads.request(&series).is_ok());
        assert_eq!(downloads.state("series"), None);
    }

    #[test]
    fn find_local_only_sees_a_finished_file() {
        let _config = ConfigDir::new("downloads-find");
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        write_json(
            &dir(ITEM).join(ITEM_FILE),
            &serde_json::from_value::<LocalItem>(json!({
                "item": item_json(ITEM),
                "source": {"Id": "source", "Container": "mkv", "MediaStreams": []},
                "file": "media.mkv",
                "size": 5,
                "verified_size": 5,
            }))
            .expect("a local item"),
        )
        .expect("item.json");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("a partial file");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        std::fs::rename(dir(ITEM).join("media.mkv.part"), dir(ITEM).join("media.mkv")).expect("rename");
        assert!(
            find_local(ACCOUNT, ITEM).is_none(),
            "a file shorter than the recorded size isn't the download"
        );
        std::fs::write(dir(ITEM).join("media.mkv"), b"").expect("an empty file");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        std::fs::write(dir(ITEM).join("media.mkv"), b"hello").expect("the whole file");
        assert!(find_local(ACCOUNT, ITEM).is_some());
    }

    /// A power loss can leave the rename without the data: a media file that's empty or shorter
    /// than the length `item.json` has on record doesn't count as done, and comes again with the
    /// queue. With no length on record (a download from before it was kept), any file with
    /// something in it passes.
    #[test]
    fn a_cut_short_download_is_queued_again_instead_of_played() {
        let _config = ConfigDir::new("downloads-cut-short");
        finished_download(item_json(ITEM));
        std::fs::write(dir(ITEM).join("media.mkv"), b"he").expect("a short file");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        assert!(!has_downloads(ACCOUNT));

        let ctx = egui::Context::default();
        let downloads = Downloads::new(client_at("http://127.0.0.1:1/"), ACCOUNT, &ctx, false);
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
        assert_eq!(lock(&downloads.queue).waiting.len(), 1);
        drop(downloads);

        let path = dir(ITEM).join(ITEM_FILE);
        let mut local: LocalItem = read_json(&path).expect("item.json");
        local.verified_size = None;
        write_json(&path, &local).expect("item.json");
        assert!(
            find_local(ACCOUNT, ITEM).is_some(),
            "no length on record: the file is taken as it is"
        );
        std::fs::write(dir(ITEM).join("media.mkv"), b"").expect("an empty file");
        assert!(find_local(ACCOUNT, ITEM).is_none(), "but not an empty one");
    }

    /// A download from before the length was recorded: its `item.json` has the size
    /// `PlaybackInfo` gave, which was never checked against the bytes that came (the transfer was
    /// accepted when they matched `Content-Length`). A library that reported another size than
    /// the file it served left a download that plays: it isn't invisible offline, and isn't
    /// downloaded again.
    #[test]
    fn a_download_from_before_the_length_was_recorded_is_taken_as_it_is() {
        let _config = ConfigDir::new("downloads-old-shape");
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        // As an older version wrote it: no `verified_size`, and a size that isn't the file's.
        let old = json!({
            "item": item_json(ITEM),
            "source": {"Id": "source", "Container": "mkv", "MediaStreams": []},
            "subtitles": [],
            "segments": [],
            "file": "media.mkv",
            "size": 9,
        });
        std::fs::write(dir(ITEM).join(ITEM_FILE), old.to_string()).expect("item.json");
        std::fs::write(dir(ITEM).join("media.mkv"), b"hello").expect("the media file");

        let local = find_local(ACCOUNT, ITEM).expect("it plays");
        assert_eq!(std::fs::read(local.path).expect("the file"), b"hello");
        assert!(has_downloads(ACCOUNT));
        let ctx = egui::Context::default();
        let downloads = Downloads::new(client_at("http://127.0.0.1:1/"), ACCOUNT, &ctx, false);
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Done { size: 5 }));
        assert!(lock(&downloads.queue).waiting.is_empty(), "not queued again");

        // The same sizes from a transfer that counted the bytes are another matter: that file is
        // not the length it recorded.
        let mut counted: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        counted.verified_size = Some(9);
        write_json(&dir(ITEM).join(ITEM_FILE), &counted).expect("item.json");
        assert!(find_local(ACCOUNT, ITEM).is_none());
    }

    /// A `.part` longer than the size `PlaybackInfo` gave is resumed all the same: the server
    /// has the file, and says where it goes on.
    #[test]
    fn a_part_longer_than_the_reported_size_resumes_and_finishes_as_the_whole_file() {
        let _config = ConfigDir::new("downloads-long-part");
        let server = start();
        // `PlaybackInfo` says 5 bytes; the file is 10.
        serve_item(&server, ITEM);
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hello!!").expect("seven bytes of it");
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .and(wiremock::matchers::header("range", "bytes=7-"))
                .respond_with(
                    ResponseTemplate::new(206)
                        .insert_header("content-range", "bytes 7-9/10")
                        .set_body_bytes(b"!!!"),
                )
                .expect(1),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        downloads.request(&item(ITEM)).expect("queued");
        let notices = tick_until(&mut downloads, |downloads| {
            matches!(
                downloads.state(ITEM),
                Some(DownloadState::Done { .. } | DownloadState::Failed(_))
            )
        });
        assert!(matches!(notices[..], [Notice::Downloaded { .. }]), "{notices:?}");
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Done { size: 10 }));
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello!!!!!");
        let local: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        assert_eq!((local.size, local.verified_size), (10, Some(10)));
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// Bytes past the total the bar has (the server gave none of its own) leave the transfer
    /// without a total, not a bar past full.
    #[test]
    fn progress_past_the_total_has_no_total() {
        let ctx = egui::Context::default();
        let (updates, mut reports) = task::channel(&ctx);
        let reporter = Reporter {
            updates,
            queue: Arc::default(),
            worker: 0,
        };
        for (done, total) in [(4, Some(5)), (5, Some(5)), (7, Some(5)), (7, None)] {
            reporter.send(Update::Progress {
                item_id: ITEM.into(),
                done,
                total,
            });
        }
        let totals: Vec<_> = reports
            .drain()
            .filter_map(|update| match update {
                Update::Progress { total, .. } => Some(total),
                _ => None,
            })
            .collect();
        assert_eq!(totals, [Some(5), Some(5), None, None]);
    }

    /// A server that answers once the first try has given up: the transfer asks again by itself,
    /// and the item doesn't fail.
    #[test]
    fn a_stalled_download_is_tried_again() {
        let _config = ConfigDir::new("downloads-stall-once");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(Stalls::times(1)),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        lock(&downloads.queue).patience = quick();
        downloads.request(&item(ITEM)).expect("queued");
        let notices = tick_until(&mut downloads, |downloads| {
            matches!(
                downloads.state(ITEM),
                Some(DownloadState::Done { .. } | DownloadState::Failed(_))
            )
        });
        assert!(matches!(notices[..], [Notice::Downloaded { .. }]), "{notices:?}");
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        assert_eq!(media_requests(&server), 2, "the stalled try and the one that worked");
    }

    /// A server that never answers fails the item after the retries, not before and not never.
    #[test]
    fn a_download_that_keeps_stalling_fails_after_its_retries() {
        let _config = ConfigDir::new("downloads-stall-always");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(Stalls::times(usize::MAX)),
        );
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        lock(&downloads.queue).patience = quick();
        downloads.request(&item(ITEM)).expect("queued");
        let notices = tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Failed(_)))
        });
        assert!(matches!(notices[..], [Notice::Failed { .. }]), "{notices:?}");
        assert_eq!(media_requests(&server), 1 + STALL_RETRIES as usize);
    }

    /// Waits that fit a test: a stall is 400 ms of nothing, and the pause after it a few.
    fn quick() -> Patience {
        Patience {
            stall: Some(Duration::from_millis(400)),
            pause: Duration::from_millis(20),
        }
    }

    /// How many times the media file was asked for.
    fn media_requests(server: &Arc<MockServer>) -> usize {
        let asked = server.clone();
        let requests = block_on(async move { asked.received_requests().await.expect("recorded") });
        requests
            .iter()
            .filter(|request| request.url.path() == format!("/Items/{ITEM}/Download"))
            .count()
    }

    /// Answers "hello", but leaves the first `times` requests waiting far past the stall limit.
    struct Stalls {
        times: usize,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl Stalls {
        fn times(times: usize) -> Self {
            Self {
                times,
                asked: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl wiremock::Respond for Stalls {
        fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
            let template = ResponseTemplate::new(200).set_body_bytes(b"hello");
            if self.asked.fetch_add(1, Ordering::SeqCst) < self.times {
                template.set_delay(Duration::from_secs(4))
            } else {
                template
            }
        }
    }

    /// Two snapshots of the queue written out of turn leave the newer one on disk, whichever
    /// reaches the file last.
    #[test]
    fn an_older_queue_snapshot_never_replaces_a_newer_one() {
        let _config = ConfigDir::new("downloads-index-order");
        let path = account_dir(ACCOUNT).join(INDEX_FILE);
        let snapshot = |ids: &[&str]| Index {
            entries: ids
                .iter()
                .map(|id| IndexEntry {
                    item: item(id),
                    state: SavedState::Queued,
                })
                .collect(),
        };
        let writes = IndexWrites::default();
        write_index(&writes, 2, &path, &snapshot(&[ITEM, OTHERS[0]]));
        write_index(&writes, 1, &path, &snapshot(&[ITEM]));
        let index: Index = read_json(&path).expect("index.json");
        assert_eq!(index.entries.len(), 2, "the later snapshot stays");
        write_index(&writes, 3, &path, &snapshot(&[]));
        let index: Index = read_json(&path).expect("index.json");
        assert!(index.entries.is_empty(), "a newer one still goes through");
    }

    /// A change made right before the session ends is on disk when it has: sign-out and quitting
    /// don't wait for the thread that was to write it.
    #[test]
    fn the_last_queue_change_is_written_when_the_session_ends() {
        let _config = ConfigDir::new("downloads-index-last");
        let ctx = egui::Context::default();
        let (_network, unmetered) = watch::channel(false);
        let mut downloads =
            Downloads::new(client_at("http://127.0.0.1:1/"), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.queue(vec![item(ITEM)]);
        downloads.queue(vec![item(OTHERS[0])]);
        downloads.cancel(ITEM);
        downloads.pause();
        let index: Index = read_json(&account_dir(ACCOUNT).join(INDEX_FILE)).expect("index.json");
        let ids: Vec<_> = index.entries.iter().map(|entry| entry.item.id.as_str()).collect();
        assert_eq!(ids, [OTHERS[0]]);
    }

    #[test]
    fn account_keys_become_one_folder_name() {
        let _config = ConfigDir::new("downloads-names");
        assert_eq!(account_dir(ACCOUNT), config::downloads_dir().join("server_user"));
        assert_eq!(safe_name("../../etc"), "______etc");
    }

    /// Writes a downloaded item's `item.json` and its media file, as a finished download leaves
    /// them, for the tests that play one.
    fn finished_download(item: Value) -> LocalItem {
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        let local: LocalItem = serde_json::from_value(json!({
            "item": item,
            "source": {"Id": "source", "Container": "mkv", "MediaStreams": []},
            "file": "media.mkv",
            "size": 5,
            "verified_size": 5,
        }))
        .expect("a local item");
        write_json(&dir(ITEM).join(ITEM_FILE), &local).expect("item.json");
        std::fs::write(dir(ITEM).join("media.mkv"), b"hello").expect("the media file");
        local
    }

    #[test]
    fn the_session_snapshot_round_trips() {
        let _config = ConfigDir::new("downloads-session");
        let auth: jellyfin::models::AuthenticationResult = crate::ui_tests::fixture_json("authenticate");
        let info: PublicSystemInfo = crate::ui_tests::fixture_json("public_info");
        assert_eq!(saved_session(ACCOUNT), None, "nothing saved yet");
        save_session(ACCOUNT, &auth.user, &info);
        let (user, server) = saved_session(ACCOUNT).expect("the saved session");
        assert_eq!(user, auth.user);
        assert_eq!(server, info);
        assert_eq!(user.name, "dev");
        assert!(user.configuration.is_some());
        assert!(account_dir(ACCOUNT).join(SESSION_FILE).is_file());
    }

    #[test]
    fn only_an_account_with_a_finished_download_has_downloads() {
        let _config = ConfigDir::new("downloads-have");
        assert!(!has_downloads(ACCOUNT));
        std::fs::create_dir_all(dir(ITEM)).expect("the item's folder");
        write_json(
            &dir(ITEM).join(ITEM_FILE),
            &serde_json::from_value::<LocalItem>(json!({
                "item": item_json(ITEM),
                "source": {"Id": "source", "Container": "mkv", "MediaStreams": []},
                "file": "media.mkv",
                "size": 5,
            }))
            .expect("a local item"),
        )
        .expect("item.json");
        assert!(!has_downloads(ACCOUNT), "`item.json` alone isn't a download");
        std::fs::write(dir(ITEM).join("media.mkv"), b"hello").expect("the media file");
        assert!(has_downloads(ACCOUNT));
    }

    /// Past the last tenth, a downloaded item counts as watched: offline, its next play starts
    /// at the beginning rather than resuming near the end.
    #[test]
    fn a_downloaded_files_progress_is_kept_for_offline_resume() {
        let _config = ConfigDir::new("downloads-progress");
        let mut value = item_json(ITEM);
        value["RunTimeTicks"] = json!(10_000_000_000_i64); // 1000 s
        finished_download(value);
        let ctx = egui::Context::default();
        // Nothing asks this server anything: the test only plays with the item's files.
        let mut downloads = Downloads::new(client_at("http://127.0.0.1:1/"), ACCOUNT, &ctx, false);
        assert!(downloads.primary_image_path(ITEM).is_none(), "no poster was saved");

        record_progress(ACCOUNT, ITEM, Ticks(2_000_000_000), false); // 200 s of 1000: partway
        let saved: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        let data = saved.item.user_data.expect("the watch state");
        assert_eq!(data.playback_position_ticks, Ticks(2_000_000_000));
        assert!(!data.played);
        assert_eq!(data.last_played_date, None, "only a watched item gets a date");
        downloads.refresh_item(ITEM);
        let entry = downloads.entries().next().expect("the entry").0;
        assert_eq!(
            entry.user_data.as_ref().map(|data| data.playback_position_ticks),
            Some(Ticks(2_000_000_000)),
            "the page's copy has the new position too"
        );

        record_progress(ACCOUNT, ITEM, Ticks(9_500_000_000), false); // 95 %: watched
        let saved: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        let data = saved.item.user_data.expect("the watch state");
        assert!(data.played);
        assert_eq!(data.playback_position_ticks, Ticks::ZERO, "watched items don't resume");
        let played_at = data.last_played_date.as_deref().expect("when it was played");
        assert!(jellyfin::time::parse_utc(played_at).is_some(), "{played_at}");
    }

    /// A download that finished with no frame to save the queue after it is still queued in
    /// `index.json`, as the item was when it was asked for: the next sign-in takes the item from
    /// its `item.json`, where its offline playback left the resume point.
    #[test]
    fn a_finished_download_still_in_the_index_keeps_its_resume_point() {
        let _config = ConfigDir::new("downloads-stale-index");
        let mut value = item_json(ITEM);
        value["RunTimeTicks"] = json!(10_000_000_000_i64);
        finished_download(value);
        let index = Index {
            entries: vec![IndexEntry {
                item: item(ITEM),
                state: SavedState::Queued,
            }],
        };
        write_json(&account_dir(ACCOUNT).join(INDEX_FILE), &index).expect("index.json");
        record_progress(ACCOUNT, ITEM, Ticks(2_000_000_000), false);

        let ctx = egui::Context::default();
        let downloads = Downloads::new(client_at("http://127.0.0.1:1/"), ACCOUNT, &ctx, false);
        let (entry, state) = downloads.entries().next().expect("the entry");
        assert_eq!(state, &DownloadState::Done { size: 5 });
        assert_eq!(
            entry.user_data.as_ref().map(|data| data.playback_position_ticks),
            Some(Ticks(2_000_000_000))
        );
        assert_eq!(downloads.entries().count(), 1, "listed once");
    }

    /// An item played to the end counts as watched even when the server never said how long it
    /// is.
    #[test]
    fn an_item_played_to_the_end_is_watched() {
        let _config = ConfigDir::new("downloads-finished");
        finished_download(item_json(ITEM));
        record_progress(ACCOUNT, ITEM, Ticks(1), true);
        let saved: LocalItem = read_json(&dir(ITEM).join(ITEM_FILE)).expect("item.json");
        let data = saved.item.user_data.expect("the watch state");
        assert!(data.played);
        assert_eq!(data.playback_position_ticks, Ticks::ZERO);
    }

    fn pending(item_id: &str, played_at: &str) -> PendingReport {
        PendingReport {
            item_id: item_id.to_owned(),
            media_source_id: "source".to_owned(),
            position_ticks: Ticks(200_000_000),
            played_at: played_at.to_owned(),
        }
    }

    fn write_pending(reports: &[PendingReport]) {
        write_json(&account_dir(ACCOUNT).join(PENDING_FILE), &reports.to_vec()).expect("pending.json");
    }

    fn read_pending() -> Vec<PendingReport> {
        read_json(&account_dir(ACCOUNT).join(PENDING_FILE)).unwrap_or_default()
    }

    /// Serves the item's own answer to `/Items/{id}` with `last_played` as its `LastPlayedDate`.
    fn serve_user_data(server: &Arc<MockServer>, last_played: Option<&str>) {
        let mut value = item_json(ITEM);
        if let Some(last_played) = last_played {
            value["UserData"] = json!({"LastPlayedDate": last_played});
        }
        mount(
            server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(&value)),
        );
    }

    fn serve_stop(server: &Arc<MockServer>, status: u16, times: u64) {
        mount(
            server,
            Mock::given(method("POST"))
                .and(path("/Sessions/Playing/Stopped"))
                .respond_with(ResponseTemplate::new(status))
                .expect(times),
        );
    }

    #[test]
    fn a_pending_report_goes_out_once_the_server_can_be_reached() {
        let _config = ConfigDir::new("downloads-pending-sent");
        let server = start();
        serve_user_data(&server, None);
        serve_stop(&server, 204, 1);
        write_pending(&[pending(ITEM, "2026-10-14T19:00:00.000000Z")]);
        let api = client(&server);
        block_on(async move { flush_pending(&api, ACCOUNT).await });
        assert_eq!(read_pending(), [], "a sent report is dropped");
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// An item played twice offline sends only its last stop: the first one, once sent, would
    /// make the server's last played date newer than the second and have it skipped.
    #[test]
    fn only_an_items_last_pending_report_goes_out() {
        let _config = ConfigDir::new("downloads-pending-last");
        add_pending(ACCOUNT, pending(ITEM, "2026-10-14T19:00:00.000000Z"));
        let mut later = pending(ITEM, "2026-10-14T19:05:00.000000Z");
        later.position_ticks = Ticks(300_000_000);
        add_pending(ACCOUNT, later.clone());
        assert_eq!(read_pending(), [later.clone()], "the later stop replaces the earlier");

        // A file written before that, holding both: still only the last goes out.
        write_pending(&[pending(ITEM, "2026-10-14T19:00:00.000000Z"), later]);
        let server = start();
        serve_user_data(&server, None);
        mount(
            &server,
            Mock::given(method("POST"))
                .and(path("/Sessions/Playing/Stopped"))
                .and(wiremock::matchers::body_partial_json(
                    json!({"PositionTicks": 300_000_000}),
                ))
                .respond_with(ResponseTemplate::new(204))
                .expect(1),
        );
        let api = client(&server);
        block_on(async move { flush_pending(&api, ACCOUNT).await });
        assert_eq!(read_pending(), []);
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// Watched again since (on another device, say): that watch wins over the report from here.
    #[test]
    fn a_pending_report_is_skipped_when_the_item_was_watched_since() {
        let _config = ConfigDir::new("downloads-pending-skip");
        let server = start();
        serve_user_data(&server, Some("2026-10-15T19:00:00.000000Z"));
        serve_stop(&server, 204, 0);
        write_pending(&[pending(ITEM, "2026-10-14T19:00:00.000000Z")]);
        let api = client(&server);
        block_on(async move { flush_pending(&api, ACCOUNT).await });
        assert_eq!(read_pending(), [], "a skipped report is dropped too");
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    #[test]
    fn a_pending_report_the_server_refuses_is_kept() {
        let _config = ConfigDir::new("downloads-pending-kept");
        let server = start();
        serve_user_data(&server, None);
        serve_stop(&server, 500, 1);
        let report = pending(ITEM, "2026-10-14T19:00:00.000000Z");
        write_pending(std::slice::from_ref(&report));
        let api = client(&server);
        block_on(async move { flush_pending(&api, ACCOUNT).await });
        assert_eq!(read_pending(), [report], "it waits for the next sign-in");
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// A report whose item the server no longer has can never be sent: it's dropped rather than
    /// retried forever.
    #[test]
    fn a_pending_report_for_an_item_the_server_forgot_is_dropped() {
        let _config = ConfigDir::new("downloads-pending-gone");
        let server = start();
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}")))
                .respond_with(ResponseTemplate::new(404)),
        );
        serve_stop(&server, 204, 0);
        write_pending(&[pending(ITEM, "2026-10-14T19:00:00.000000Z")]);
        let api = client(&server);
        block_on(async move { flush_pending(&api, ACCOUNT).await });
        assert_eq!(read_pending(), []);
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// A report added while a flush has its requests out is kept: the flush takes out what it
    /// sent, not what it read.
    #[test]
    fn a_report_added_during_a_flush_survives_it() {
        let _config = ConfigDir::new("downloads-pending-race");
        let server = start();
        serve_user_data(&server, None);
        mount(
            &server,
            Mock::given(method("POST"))
                .and(path("/Sessions/Playing/Stopped"))
                .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_millis(600)))
                .expect(1),
        );
        write_pending(&[pending(ITEM, "2026-10-14T19:00:00.000000Z")]);
        let (finished, flushed) = std::sync::mpsc::channel();
        let api = client(&server);
        crate::runtime::detach(async move {
            flush_pending(&api, ACCOUNT).await;
            let _ = finished.send(());
        });
        // Once the stop report is on its way, the flush has read the file.
        let start = Instant::now();
        loop {
            let asked = server.clone();
            let requests = block_on(async move { asked.received_requests().await.expect("recorded") });
            if requests.iter().any(|request| request.method.as_str() == "POST") {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "the flush sent nothing");
            std::thread::sleep(Duration::from_millis(10));
        }
        let later = pending(OTHERS[0], "2026-10-14T19:30:00.000000Z");
        add_pending(ACCOUNT, later.clone());
        flushed.recv_timeout(Duration::from_secs(10)).expect("the flush ended");
        assert_eq!(read_pending(), [later], "the sent one is gone and the new one waits");
    }

    /// Writers of one JSON file each have a temp file of their own, so neither's rename finds
    /// the other's gone, and no temp file is left behind.
    #[test]
    fn concurrent_writes_of_one_json_file_dont_collide() {
        let _config = ConfigDir::new("downloads-write-race");
        let path = account_dir(ACCOUNT).join(PENDING_FILE);
        let writers: Vec<_> = (0..6)
            .map(|writer| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for round in 0..30 {
                        let report = pending(&format!("{writer}-{round}"), "2026-10-14T19:00:00.000000Z");
                        write_json(&path, &vec![report]).expect("a write that doesn't collide");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("a writer");
        }
        assert_eq!(read_pending().len(), 1, "one writer's whole file");
        let left: Vec<_> = std::fs::read_dir(account_dir(ACCOUNT))
            .expect("the account's folder")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from(PENDING_FILE)]);
    }

    /// Signed in without the server, a queued download must not start (or fail): it waits for
    /// the account's next online sign-in.
    #[test]
    fn an_offline_download_starts_no_transfer() {
        let _config = ConfigDir::new("downloads-offline");
        let server = start();
        // Any request at all would be a transfer starting.
        mount(
            &server,
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello"))
                .expect(0),
        );
        mount(
            &server,
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0),
        );
        let ctx = egui::Context::default();
        // Offline on a phone with no network at all: it's the server it waits for, not Wi-Fi.
        let (_network, unmetered) = watch::channel(false);
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.hold();
        downloads.request(&item(ITEM)).expect("queued");
        for _ in 0..5 {
            assert!(downloads.tick().is_empty(), "nothing failed either");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
        assert!(!downloads.waiting_for_wifi());
        drop(downloads);
        let server = server.clone();
        block_on(async move { server.verify().await });
    }

    /// The services of a session signed in without the server connect nothing and download
    /// nothing, however long they're ticked.
    #[test]
    fn offline_services_ask_the_server_nothing() {
        let _config = ConfigDir::new("services-offline");
        let server = crate::ui_tests::FixtureServer::start();
        let ctx = egui::Context::default();
        let session = crate::ui_tests::session(&server);
        let account = session.account_key();
        let mut services = Services::offline(session, &ctx);
        assert!(!services.online);
        services.downloads.request(&item(ITEM)).expect("queued");
        for _ in 0..5 {
            services.tick(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
        // A transfer would start with `PlaybackInfo`, and the websocket with `/socket`.
        assert_eq!(server.requests("POST", &format!("/Items/{ITEM}/PlaybackInfo")), 0);
        assert_eq!(server.requests("GET", "/socket"), 0, "no websocket to keep");
        assert_eq!(services.downloads.state(ITEM), Some(DownloadState::Queued));
        // The queue stays for the account's next online sign-in.
        drop(services);
        let downloads = Downloads::new(client_at("http://127.0.0.1:1/"), &account, &ctx, false);
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
    }

    const OTHERS: [&str; 2] = ["11111111111111111111111111111111", "22222222222222222222222222222222"];

    /// `id`'s media file, "hello", after `delay`.
    fn serve_download(server: &Arc<MockServer>, id: &str, delay: Duration) {
        mount(
            server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{id}/Download")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello").set_delay(delay)),
        );
    }

    /// The workers take the queue's next items themselves: three downloads, one more than run
    /// at once, all finish after a single tick, as they must on Android in the background,
    /// where no frame is drawn.
    #[test]
    fn the_queue_moves_on_without_frames() {
        let _config = ConfigDir::new("downloads-no-frames");
        let server = start();
        let ids = [ITEM, OTHERS[0], OTHERS[1]];
        for id in ids {
            serve_item(&server, id);
            serve_download(&server, id, Duration::ZERO);
        }
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, false);
        for id in ids {
            downloads.request(&item(id)).expect("queued");
        }
        downloads.tick();
        let start = Instant::now();
        while !ids.iter().all(|id| find_local(ACCOUNT, id).is_some()) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "the queue stopped without frames"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // The page catches up on its next frame, and nothing is left running.
        tick_until(&mut downloads, |downloads| {
            ids.iter()
                .all(|id| matches!(downloads.state(id), Some(DownloadState::Done { .. })))
        });
        let start = Instant::now();
        while !lock(&downloads.queue).workers.is_empty() {
            assert!(start.elapsed() < Duration::from_secs(5), "a worker outlived the queue");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(lock(&downloads.queue).activity(), None, "no notice once it's done");
    }

    /// With Wi-Fi only, nothing is fetched on a metered network, and the queue says what it's
    /// waiting for; an unmetered network starts it.
    #[test]
    fn wifi_only_waits_for_an_unmetered_network() {
        let _config = ConfigDir::new("downloads-wifi-only");
        let server = start();
        serve_item(&server, ITEM);
        serve_download(&server, ITEM, Duration::ZERO);
        let (network, unmetered) = watch::channel(false);
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.request(&item(ITEM)).expect("queued");
        for _ in 0..10 {
            assert!(downloads.tick().is_empty(), "nothing failed");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(downloads.state(ITEM), Some(DownloadState::Queued));
        assert!(downloads.waiting_for_wifi());
        // `item.json` is the first thing a transfer writes.
        assert!(!dir(ITEM).join(ITEM_FILE).exists(), "nothing was fetched");
        let notice = lock(&downloads.queue).activity().expect("a notice while it waits");
        assert_eq!(notice.title, "Waiting for Wi-Fi");
        assert_eq!(notice.text, "1 download to come");

        network.send_replace(true);
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert!(!downloads.waiting_for_wifi());
    }

    /// Turning Wi-Fi only off lets a held queue go on the metered network.
    #[test]
    fn turning_wifi_only_off_releases_the_queue() {
        let _config = ConfigDir::new("downloads-wifi-off");
        let server = start();
        serve_item(&server, ITEM);
        serve_download(&server, ITEM, Duration::ZERO);
        let (_network, unmetered) = watch::channel(false);
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.request(&item(ITEM)).expect("queued");
        downloads.tick();
        assert!(downloads.waiting_for_wifi());
        downloads.set_wifi_only(false);
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
    }

    /// Leaving Wi-Fi stops a transfer under way and puts it back at the front of the queue, its
    /// `.part` kept; it starts again once Wi-Fi is back.
    #[test]
    fn leaving_wifi_puts_a_transfer_back_in_the_queue() {
        let _config = ConfigDir::new("downloads-left-wifi");
        let server = start();
        serve_item(&server, ITEM);
        serve_download(&server, ITEM, Duration::from_millis(500));
        let (network, unmetered) = watch::channel(true);
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Downloading { .. }))
        });
        network.send_replace(false);
        tick_until(&mut downloads, |downloads| {
            downloads.state(ITEM) == Some(DownloadState::Queued)
        });
        assert_eq!(
            lock(&downloads.queue)
                .waiting
                .front()
                .map(|item| item.id.clone())
                .as_deref(),
            Some(ITEM)
        );
        assert!(find_local(ACCOUNT, ITEM).is_none());

        network.send_replace(true);
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
    }

    /// Leaving Wi-Fi breaks the connection a moment before the phone says so: a transfer that
    /// fails just before the network turns metered waits for Wi-Fi rather than failing.
    #[test]
    fn a_failure_as_the_phone_leaves_wifi_waits_for_it() {
        let _config = ConfigDir::new("downloads-wifi-race");
        let server = start();
        serve_item(&server, ITEM);
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path(format!("/Items/{ITEM}/Download")))
                .respond_with(ResponseTemplate::new(503)),
        );
        let (network, unmetered) = watch::channel(true);
        let ctx = egui::Context::default();
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx, true).with_network(unmetered);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Downloading { .. }))
        });
        // After the transfer failed, as the callback arrives after the connection broke.
        std::thread::sleep(Duration::from_millis(300));
        network.send_replace(false);
        let notices = tick_until(&mut downloads, |downloads| {
            downloads.state(ITEM) == Some(DownloadState::Queued)
        });
        assert!(notices.is_empty(), "no failure toast: {notices:?}");
        assert!(downloads.waiting_for_wifi());
    }

    /// The notice names the transfer and how far along it is, and how many are still to come.
    #[test]
    fn the_notice_says_how_far_along_the_downloads_are() {
        let current = |done, total| Current {
            item_id: ITEM.into(),
            title: "Film".into(),
            done,
            total,
        };
        let mut queue = Queue::default();
        assert_eq!(queue.activity(), None);
        queue.workers.push(Worker {
            id: 0,
            abort: None,
            current: Some(current(500_000_000, Some(2_000_000_000))),
        });
        queue.waiting.push_back(item(OTHERS[0]));
        assert_eq!(
            queue.activity(),
            Some(Activity {
                title: "Film".into(),
                text: "500.0 MB of 2.0 GB · 1 more to come".into(),
                progress: Some(0.25),
            })
        );
        queue.workers[0].current = Some(current(1_000, None));
        queue.waiting.clear();
        let notice = queue.activity().expect("a notice");
        assert_eq!((notice.text.as_str(), notice.progress), ("1.0 kB so far", None));
    }
}
