//! Downloads of an item's original file, and the local copies playback finds.
//!
//! Everything lives under [`config::downloads_dir`], one folder per account and one per item:
//!
//! ```text
//! <account>/index.json             the queue, in the order it was asked for
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
//! Transfers run on tokio, at most [`MAX_TRANSFERS`] at a time, each with an abort handle that
//! dropping [`Downloads`] fires, leaving its `.part` for the next sign-in. They report through a
//! [`task::channel`] that [`Downloads::tick`] drains; the UI reads [`Downloads::state`] and
//! [`Downloads::entries`], never the disk.

use std::io::SeekFrom;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::FutureExt;

use jellyfin::models::{
    BaseItemDto, BaseItemKind, ChapterInfo, ItemField, MediaSegmentDto, MediaSourceInfo, MediaStream, MediaStreamType,
};
use jellyfin::{Client, ImageRequest, PlaybackInfoRequest};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use url::Url;

use crate::config;
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
/// Image widths kept for offline pages: a poster for a card, a backdrop for a header (twice
/// what's drawn, as `images.rs` asks for).
const PRIMARY_WIDTH: u32 = 600;
const BACKDROP_WIDTH: u32 = 1280;

const INDEX_FILE: &str = "index.json";
const ITEM_FILE: &str = "item.json";

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
    /// Its size in bytes, as the server gave it; `0` when it didn't.
    pub size: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalSubtitle {
    /// The Jellyfin stream index, as [`MediaStream::index`].
    pub index: i32,
    pub file: String,
}

/// A finished download, as playback finds it.
#[derive(Debug, Clone)]
pub struct LocalMedia {
    pub path: PathBuf,
    pub source: MediaSourceInfo,
    /// Jellyfin stream index and the file holding it, for the external subtitles that are there.
    pub subtitles: Vec<(i32, PathBuf)>,
    // For the player's chapters and skip buttons without the server; online, `playback::details`
    // still asks the server, whose answer is fresher.
    #[allow(dead_code)]
    pub chapters: Vec<ChapterInfo>,
    #[allow(dead_code)]
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
    /// The transfers running now, at most [`MAX_TRANSFERS`].
    running: Vec<Running>,
    /// Series and season expansions being fetched.
    expanding: Vec<task::Task<()>>,
    /// [`Downloads::pause`] ran: the queue stays where it is until this account signs in again
    /// (a new `Downloads`), even if a frame still ticks this one.
    stopped: bool,
    updates: task::Receiver<Update>,
    updates_from: task::Sender<Update>,
}

struct Entry {
    item: BaseItemDto,
    state: DownloadState,
}

struct Running {
    item_id: String,
    /// Fires when it's dropped, aborting the transfer and leaving its `.part` for a retry.
    abort: tokio::task::AbortHandle,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// What a transfer reports to the UI.
enum Update {
    /// A series' or season's episodes, fetched for the queue.
    Expanded(Vec<BaseItemDto>),
    Progress {
        item_id: String,
        done: u64,
        total: Option<u64>,
    },
    Done {
        item_id: String,
        size: u64,
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
    pub fn new(client: Client, account_key: &str, ctx: &egui::Context) -> Self {
        let (updates_from, updates) = task::channel(ctx);
        let mut downloads = Self {
            client,
            account: account_key.to_owned(),
            ctx: ctx.clone(),
            entries: Vec::new(),
            running: Vec::new(),
            expanding: Vec::new(),
            stopped: false,
            updates,
            updates_from,
        };
        downloads.load();
        downloads
    }

    /// The queue as it was saved, with each item's state read from its folder: a finished file
    /// wins over what `index.json` remembers.
    fn load(&mut self) {
        let dir = account_dir(&self.account);
        let index: Index = read_json(&dir.join(INDEX_FILE)).unwrap_or_default();
        for entry in index.entries {
            let state = match finished(&item_dir(&self.account, &entry.item.id)) {
                Some(size) => DownloadState::Done { size },
                None => match entry.state {
                    SavedState::Queued => DownloadState::Queued,
                    SavedState::Paused => DownloadState::Paused,
                    SavedState::Failed { message } => DownloadState::Failed(message),
                },
            };
            self.entries.push(Entry {
                item: entry.item,
                state,
            });
        }
        // Finished downloads the index doesn't know (one it forgot, or one copied in).
        for (item, size) in scan_finished(&dir) {
            if !self.entries.iter().any(|entry| entry.item.id == item.id) {
                self.entries.push(Entry {
                    item,
                    state: DownloadState::Done { size },
                });
            }
        }
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
                    return Err("This item can't be downloaded from this server.".to_owned());
                }
                self.queue(vec![item.clone()]);
                Ok(())
            }
            _ => Err("Only videos, and the series and seasons that hold them, can be downloaded.".to_owned()),
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
            self.entries.push(Entry {
                item,
                state: DownloadState::Queued,
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
        self.stop(item_id);
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
                Update::Progress { item_id, done, total } => {
                    if let Some(entry) = self.entry_mut(&item_id) {
                        entry.state = DownloadState::Downloading { done, total };
                    }
                }
                Update::Done { item_id, size } => {
                    self.running.retain(|run| run.item_id != item_id);
                    let Some(entry) = self.entry_mut(&item_id) else {
                        continue;
                    };
                    entry.state = DownloadState::Done { size };
                    notices.push(Notice::Downloaded {
                        title: playback::title(&entry.item),
                    });
                    settled = true;
                }
                Update::Failed { item_id, message } => {
                    self.running.retain(|run| run.item_id != item_id);
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
        self.running.clear();
        let mut stopped = false;
        for entry in &mut self.entries {
            if matches!(entry.state, DownloadState::Downloading { .. }) {
                entry.state = DownloadState::Queued;
                stopped = true;
            }
        }
        // Reports already on their way would show up as a transfer with nothing behind it.
        self.updates.drain().for_each(drop);
        if stopped {
            self.save();
        }
    }

    fn entry_mut(&mut self, item_id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.item.id == item_id)
    }

    /// Forgets a running transfer: dropping its task aborts it.
    fn stop(&mut self, item_id: &str) {
        self.running.retain(|run| run.item_id != item_id);
    }

    /// Starts transfers until [`MAX_TRANSFERS`] are running, each taking the first queued item
    /// that isn't already.
    fn start_transfers(&mut self) {
        if self.stopped {
            return;
        }
        while self.running.len() < MAX_TRANSFERS {
            let next = self.entries.iter().position(|entry| {
                matches!(entry.state, DownloadState::Queued)
                    && !self.running.iter().any(|run| run.item_id == entry.item.id)
            });
            let Some(index) = next else {
                break;
            };
            let item = self.entries[index].item.clone();
            // The size the item's own sources have, until `PlaybackInfo` says more precisely.
            let total = item
                .media_sources
                .first()
                .and_then(|source| source.size)
                .map(|size| size as u64);
            self.entries[index].state = DownloadState::Downloading { done: 0, total };
            let dir = item_dir(&self.account, &item.id);
            let (client, updates) = (self.client.clone(), self.updates_from.clone());
            let abort = crate::runtime::start(transfer(client, item.clone(), dir, updates)).abort_handle();
            self.running.push(Running {
                item_id: item.id,
                abort,
            });
        }
    }

    /// Writes the queue and each item's persisted state, atomically like the config.
    fn save(&self) {
        let path = account_dir(&self.account).join(INDEX_FILE);
        if self.entries.is_empty() && !path.exists() {
            // Nothing queued yet: don't leave an empty queue behind for every account signed in.
            return;
        }
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
        if let Err(err) = write_json(&path, &index) {
            tracing::warn!("couldn't save the download queue: {err}");
        }
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
                Notice::Downloaded { title } => Toast::success(format!("Downloaded {title}")),
                Notice::Failed { title, message } => Toast::error(format!("Couldn't download {title}: {message}")),
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
    // A finished file is the source of truth: a `.part` isn't a download yet.
    if !path.is_file() {
        return None;
    }
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

/// Downloads one item. A failure is one message: the transfer's own errors are user-facing.
async fn transfer(client: Client, item: BaseItemDto, dir: PathBuf, updates: task::Sender<Update>) {
    let item_id = item.id.clone();
    // A panic in the transfer would otherwise leave the item downloading forever; the UI hears
    // about it like any other failure.
    let result = AssertUnwindSafe(download(&client, &item, &dir, &updates))
        .catch_unwind()
        .await;
    let message = match result {
        Ok(Ok(())) => return,
        Ok(Err(message)) => message,
        Err(_) => "the download stopped unexpectedly".to_owned(),
    };
    let _ = updates.send(Update::Failed { item_id, message });
}

/// `item.json`, its subtitles and images, then the media file, resuming where a `.part` stopped.
async fn download(
    client: &Client,
    item: &BaseItemDto,
    dir: &Path,
    updates: &task::Sender<Update>,
) -> Result<(), String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| format!("couldn't make its folder: {err}"))?;
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
        .ok_or("the server returned no playable media")?;
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
    };
    write_json(&dir.join(ITEM_FILE), &local).map_err(|err| format!("couldn't write {ITEM_FILE}: {err}"))?;
    fetch_media(client, &local, dir, updates).await
}

/// The media file itself: from the end of an unfinished `.part` when there is one, and renamed
/// into place once all of its bytes are here.
async fn fetch_media(
    client: &Client,
    local: &LocalItem,
    dir: &Path,
    updates: &task::Sender<Update>,
) -> Result<(), String> {
    let item_id = &local.item.id;
    let part = dir.join(format!("{}.part", local.file));
    let finished = dir.join(&local.file);
    let mut offset = tokio::fs::metadata(&part).await.map(|meta| meta.len()).unwrap_or(0);
    let mut restart = false;
    if local.size > 0 && offset >= local.size {
        if offset == local.size {
            // Everything arrived but the rename didn't (a crash in between): finish it now.
            return finish(&part, &finished, local.size, updates, item_id).await;
        }
        // Longer than the file can be: a corrupt leftover, not something to resume.
        offset = 0;
        restart = true;
    }
    if local.size > 0 {
        check_space(dir, local.size.saturating_sub(offset))?;
    }
    let mut download = client
        .download(client.download_url(item_id), offset)
        .await
        .map_err(|err| describe_error(&err))?;
    let total = download.total.or((local.size > 0).then_some(local.size));
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&part)
        .await
        .map_err(|err| format!("couldn't open the file: {err}"))?;
    let mut done = offset;
    if restart || (offset > 0 && !download.partial) {
        // The server ignored the range and sent the whole file: start the file over.
        tracing::info!(item = %local.item.name, "starting the file over");
        file.set_len(0)
            .await
            .map_err(|err| format!("couldn't restart the file: {err}"))?;
        file.seek(SeekFrom::Start(0))
            .await
            .map_err(|err| format!("couldn't restart the file: {err}"))?;
        done = 0;
    } else {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| format!("couldn't continue the file: {err}"))?;
    }
    let _ = updates.send(Update::Progress {
        item_id: item_id.clone(),
        done,
        total,
    });
    let mut reported = Instant::now();
    while let Some(chunk) = download.chunk().await.map_err(|err| describe_error(&err))? {
        file.write_all(&chunk)
            .await
            .map_err(|err| format!("couldn't write the file: {err}"))?;
        done += chunk.len() as u64;
        if reported.elapsed() >= PROGRESS_INTERVAL {
            reported = Instant::now();
            let _ = updates.send(Update::Progress {
                item_id: item_id.clone(),
                done,
                total,
            });
        }
    }
    file.flush()
        .await
        .map_err(|err| format!("couldn't write the file: {err}"))?;
    drop(file);
    if let Some(total) = total.filter(|total| done != *total) {
        // The `.part` stays: the retry resumes from here.
        return Err(format!("only {done} of {total} bytes arrived"));
    }
    finish(&part, &finished, done, updates, item_id).await
}

/// Renames an all-there `.part` into place and tells the UI.
async fn finish(
    part: &Path,
    finished: &Path,
    size: u64,
    updates: &task::Sender<Update>,
    item_id: &str,
) -> Result<(), String> {
    tokio::fs::rename(part, finished)
        .await
        .map_err(|err| format!("couldn't finish the file: {err}"))?;
    let _ = updates.send(Update::Done {
        item_id: item_id.to_owned(),
        size,
    });
    Ok(())
}

/// Refuses before any bytes are written when the disk can't take them, with a message saying
/// what it needs and what's there. A filesystem that can't say (or an odd one, such as a
/// network share with no reported quota) only logs.
fn check_space(dir: &Path, needed: u64) -> Result<(), String> {
    match fs4::available_space(dir) {
        Ok(free) if free < needed => Err(format!(
            "there isn't enough free space: {} is needed and {} is free",
            bytes(needed),
            bytes(free)
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

/// `Some(size)` when the item's folder holds its finished media file; the state a done
/// download is in, whatever `index.json` says.
fn finished(dir: &Path) -> Option<u64> {
    let local: LocalItem = read_json(&dir.join(ITEM_FILE))?;
    let meta = std::fs::metadata(dir.join(&local.file)).ok()?;
    meta.is_file().then_some(meta.len())
}

/// Every finished download in the account's folder, for an index that lost some.
fn scan_finished(account: &Path) -> Vec<(BaseItemDto, u64)> {
    let Ok(dir) = std::fs::read_dir(account) else {
        return Vec::new();
    };
    let mut found: Vec<(BaseItemDto, u64)> = dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let dir = entry.path();
            let local: LocalItem = read_json(&dir.join(ITEM_FILE))?;
            let size = finished(&dir)?;
            Some((local.item, size))
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
/// half a file the next launch can't read (as `config.save` does).
fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_vec_pretty(value)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
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
        format!("{value:.1} {unit}")
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
        let info = ClientInfo {
            client: "tabbyfin-tests".into(),
            device: "tests".into(),
            device_id: "test-device".into(),
            version: "0.1.0".into(),
        };
        Client::new(&server.uri(), info)
            .expect("mock server URL")
            .with_token("token")
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
        let playback = json!({
            "MediaSources": [{
                "Id": "source",
                "Container": "mkv",
                "Size": 5,
                "MediaStreams": [{"Index": 0, "Type": "Video", "Codec": "h264"}],
            }],
        });
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
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
        // Done downloads are found on disk, not in the index.
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
        downloads.request(&item(ITEM)).expect("queued");
        tick_until(&mut downloads, |downloads| {
            matches!(downloads.state(ITEM), Some(DownloadState::Done { .. }))
        });
        assert_eq!(find_dir_contents(&dir(ITEM)), b"hello");
        assert!(!dir(ITEM).join("media.mkv.part").exists());
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
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
        let mut downloads = Downloads::new(client(&server), ACCOUNT, &ctx);
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
            }))
            .expect("a local item"),
        )
        .expect("item.json");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        std::fs::write(dir(ITEM).join("media.mkv.part"), b"hel").expect("a partial file");
        assert!(find_local(ACCOUNT, ITEM).is_none());
        std::fs::rename(dir(ITEM).join("media.mkv.part"), dir(ITEM).join("media.mkv")).expect("rename");
        assert!(find_local(ACCOUNT, ITEM).is_some());
    }

    #[test]
    fn account_keys_become_one_folder_name() {
        let _config = ConfigDir::new("downloads-names");
        assert_eq!(account_dir(ACCOUNT), config::downloads_dir().join("server_user"));
        assert_eq!(safe_name("../../etc"), "______etc");
    }
}
