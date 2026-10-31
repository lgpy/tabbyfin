//! SyncPlay (watch together) in the app: the group this session is in, and playback while the
//! group controls it.
//!
//! The server owns the group's queue and state. When the queue's current item changes, every
//! member loads it paused and reports `Ready`; the server then sends scheduled commands
//! (unpause, pause, seek, stop) with a `When` in server time, which [`ServerClock`] turns into
//! local time. Local pause, seek and stop become requests ([`GroupRequest`]) instead of acting
//! directly. See `docs/ROADMAP.md` §2.4.
//!
//! [`SyncPlay`] (`services.syncplay`) is the part that needs nothing else: the group, the
//! server's groups, the queue and the requests the SyncPlay panel makes. What involves playback
//! (loading the group's items, carrying out its commands, `Ready` reports, drift correction) is
//! on [`Services`], below. The server's updates and commands come in through
//! [`Services::on_syncplay_update`] and [`Services::on_syncplay_command`], which the session
//! websocket (`live.rs`) calls.
//!
//! **Timing.** A scheduled command waits on tokio, not on the UI loop, and goes straight to the
//! session's player at its time: the UI checks its timers once a frame, which can be 16 ms
//! apart on a 60 Hz screen (far more on a software renderer), while tokio's timer wakes within
//! a millisecond or two. The UI only hears back through `tick`. Drift correction, which
//! compares positions rather than acting at an exact time, runs from `tick` once a second.
//! Measured with the scripted member: tokio sent commands 0.3–1.9 ms late, the UI loop up to
//! 8 ms late while repainting continuously.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{
    BufferReport, Client, ClockSample, Command, CommandKind, GroupInfo, GroupUpdate, ItemsQuery, PlayQueue, RepeatMode,
    ServerClock, ShuffleMode, Ticks,
};
use tokio::sync::mpsc;

use crate::playback::{NowPlaying, PlayOptions, UiCommand};
use crate::runtime;
use crate::services::Services;
use crate::task::{self, Deadline, Task};
use crate::toasts::Toast;
use crate::widgets::text::card_text;

/// How often the clock is re-sampled (and the ping reported) while in a group.
const CLOCK_INTERVAL: Duration = Duration::from_secs(60);
/// Samples taken on joining, to start with a good estimate.
const FIRST_SAMPLES: usize = 4;

/// Off by more than this, a member seeks to the group's position before unpausing.
const SEEK_THRESHOLD: Duration = Duration::from_millis(400);
/// Drift correction: how often it checks, when it starts adjusting the speed, when it seeks
/// instead, the largest speed change, and how fast drift is corrected (speed change per
/// second of drift).
const DRIFT_INTERVAL: Duration = Duration::from_secs(1);
const DRIFT_TOLERANCE: f64 = 0.08;
const DRIFT_SEEK: f64 = 1.5;
const MAX_SPEED_CHANGE: f64 = 0.05;
const SPEED_PER_DRIFT: f64 = 0.25;

/// A local control, sent to the group instead.
#[derive(Debug, Clone, Copy)]
pub enum GroupRequest {
    SetPause(bool),
    Seek(Duration),
    Stop,
}

/// A request to the server, run by the request queue ([`SyncPlay::call`]).
type Request = Box<dyn FnOnce(Client) -> BoxFuture<'static, jellyfin::Result<()>> + Send>;

/// A queued request, and who to tell once it's done (quitting waits for the group to be left).
struct Queued {
    request: Request,
    done: Option<std::sync::mpsc::Sender<()>>,
}

/// What background work reports back to the UI, drained in `tick`.
enum Inbound {
    /// A request failed; the user hears about it.
    Failed(jellyfin::Error),
    /// A sample from the clock task.
    Clock(ClockSample),
}

/// The group session's player: which session it is (`NowPlaying::session`), and its restart
/// count when a `Ready` was last due.
#[derive(Debug, Clone, Copy)]
struct Watched {
    session: u64,
    seen: u64,
}

/// The queue's current entry, being fetched to load it.
struct Loading {
    playlist_item_id: String,
    start: Ticks,
    item: Task<jellyfin::Result<BaseItemDto>>,
}

/// The SyncPlay part of [`Services`]: the group, its queue, and what keeps the local player in
/// step with it.
pub struct SyncPlay {
    ctx: egui::Context,
    client: Client,
    /// The group this session is in.
    pub group: Option<GroupInfo>,
    /// Groups on the server, as last listed.
    pub groups: Vec<GroupInfo>,
    clock: ServerClock,
    /// The group's queue, as last updated.
    pub queue: Option<PlayQueue>,
    /// Names of the queue's items, by item id (fetched as they're queued).
    pub names: HashMap<String, String>,
    /// The queue entry the local player has loaded (or is loading).
    loaded: Option<String>,
    /// The queue entry's item being fetched before it's loaded. Replacing it cancels the fetch.
    loading: Option<Loading>,
    /// The group session's player, once it has started.
    watched: Option<Watched>,
    /// Report `Ready` after the player's next restart (load or seek finished).
    ready_due: bool,
    /// The pending scheduled command, waiting on tokio; it hands back how late it ran. Replacing
    /// it cancels the previous one.
    scheduled: Option<Task<Duration>>,
    clock_task: Option<Task<()>>,
    /// The group's timeline while playing: the position it's at, at a local instant.
    timeline: Option<(Instant, Duration)>,
    /// The playback speed set for drift correction.
    speed: f64,
    /// The next drift check, while a group session plays.
    drift: Deadline,
    listing: Option<Task<jellyfin::Result<Vec<GroupInfo>>>>,
    naming: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    /// Requests run one at a time, in the order they're made: a `Ready` never overtakes the
    /// `Buffering` before it, nor an unpause the pause before it.
    requests: mpsc::UnboundedSender<Queued>,
    inbox: task::Receiver<Inbound>,
    inbox_sender: task::Sender<Inbound>,
    /// Dev aid: the group `TABBYFIN_SYNCPLAY` joins once the session websocket is connected.
    #[cfg(debug_assertions)]
    debug_join: Option<String>,
}

impl SyncPlay {
    pub fn new(client: Client, ctx: &egui::Context) -> Self {
        let (inbox_sender, inbox) = task::channel(ctx);
        Self {
            ctx: ctx.clone(),
            requests: request_queue(client.clone(), inbox_sender.clone()),
            client,
            group: None,
            groups: Vec::new(),
            clock: ServerClock::default(),
            queue: None,
            names: HashMap::new(),
            loaded: None,
            loading: None,
            watched: None,
            ready_due: false,
            scheduled: None,
            clock_task: None,
            timeline: None,
            speed: 1.,
            drift: Deadline::default(),
            listing: None,
            naming: None,
            inbox,
            inbox_sender,
            #[cfg(debug_assertions)]
            debug_join: std::env::var("TABBYFIN_SYNCPLAY").ok(),
        }
    }

    /// Dev aid: `TABBYFIN_SYNCPLAY=<group name>` joins that group at startup, creating it
    /// if there's none. Called once the session websocket is connected, since the server
    /// confirms over it.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_join(&mut self) {
        let Some(name) = self.debug_join.take() else {
            return;
        };
        tracing::info!("TABBYFIN_SYNCPLAY: joining {name:?}");
        self.call(move |client| async move {
            let groups = client.syncplay_groups().await?;
            match groups.iter().find(|g| g.group_name == name) {
                Some(group) => client.syncplay_join(&group.group_id).await,
                None => client.syncplay_new(&name).await,
            }
        });
    }

    /// Fetches the server's groups into [`Self::groups`].
    pub fn refresh_groups(&mut self) {
        let client = self.client.clone();
        self.listing = Some(Task::spawn(&self.ctx, async move { client.syncplay_groups().await }));
    }

    pub fn create(&self, name: String) {
        self.call(move |client| async move { client.syncplay_new(&name).await });
    }

    pub fn join(&self, group_id: String) {
        self.call(move |client| async move { client.syncplay_join(&group_id).await });
    }

    pub fn leave(&self) {
        self.call(|client| async move { client.syncplay_leave().await });
    }

    /// Quitting the app: leaves the group, if in one, so the others don't wait for this
    /// member. Returns what to wait on for the request to finish, since the process ends right
    /// after.
    pub(crate) fn leave_for_exit(&self) -> Option<std::sync::mpsc::Receiver<()>> {
        self.group.as_ref()?;
        let (done, finished) = std::sync::mpsc::channel();
        let request: Request = Box::new(|client| async move { client.syncplay_leave().await }.boxed());
        self.queue_request(request, Some(done));
        Some(finished)
    }

    /// Plays `items` in order for the whole group.
    pub fn start_items(&self, items: &[BaseItemDto]) {
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        self.call(move |client| async move { client.syncplay_set_queue(&ids, 0, Ticks::default()).await });
    }

    /// Plays `item` for the whole group, with the next episode queued after it.
    pub fn start_queue(&self, item: &BaseItemDto, start: Ticks) {
        let item = item.clone();
        self.call(move |client| async move {
            let mut ids = vec![item.id.clone()];
            if let (BaseItemKind::Episode, Some(series_id)) = (item.kind, &item.series_id)
                && let Ok(Some(next)) = client.next_episode(series_id, &item.id).await
            {
                ids.push(next.id);
            }
            client.syncplay_set_queue(&ids, 0, start).await
        });
    }

    /// Adds `items` to the group's queue: after the playing entry with `next`, otherwise at
    /// the end.
    pub fn enqueue(&self, items: &[BaseItemDto], next: bool) {
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        self.call(move |client| async move { client.syncplay_queue(&ids, next).await });
    }

    pub fn request(&self, request: GroupRequest) {
        self.call(move |client| async move {
            match request {
                GroupRequest::SetPause(true) => client.syncplay_pause().await,
                GroupRequest::SetPause(false) => client.syncplay_unpause().await,
                GroupRequest::Seek(position) => client.syncplay_seek(Ticks::from(position)).await,
                GroupRequest::Stop => client.syncplay_stop().await,
            }
        });
    }

    /// Plays another entry of the group's queue (for everyone).
    pub fn play_entry(&self, playlist_item_id: String) {
        self.call(move |client| async move { client.syncplay_set_playlist_item(&playlist_item_id).await });
    }

    pub fn remove_entry(&self, playlist_item_id: String) {
        self.call(move |client| async move { client.syncplay_remove(&[playlist_item_id]).await });
    }

    /// Moves an entry up (`-1`) or down (`1`) the queue.
    pub fn move_entry(&self, playlist_item_id: String, by: isize) {
        let Some(queue) = &self.queue else { return };
        let Some(index) = queue
            .playlist
            .iter()
            .position(|e| e.playlist_item_id == playlist_item_id)
        else {
            return;
        };
        let Some(to) = index.checked_add_signed(by).filter(|to| *to < queue.playlist.len()) else {
            return;
        };
        self.call(move |client| async move { client.syncplay_move(&playlist_item_id, to).await });
    }

    pub fn toggle_shuffle(&self) {
        let mode = match self.queue.as_ref().map(|q| q.shuffle_mode) {
            Some(ShuffleMode::Shuffle) => ShuffleMode::Sorted,
            _ => ShuffleMode::Shuffle,
        };
        self.call(move |client| async move { client.syncplay_set_shuffle(mode).await });
    }

    /// None, all, one.
    pub fn cycle_repeat(&self) {
        let mode = self
            .queue
            .as_ref()
            .map_or(RepeatMode::RepeatNone, |q| q.repeat_mode)
            .next();
        self.call(move |client| async move { client.syncplay_set_repeat(mode).await });
    }

    /// Queues a request, whose failure the user hears about (a toast, from `tick`). Results
    /// arrive as websocket updates.
    fn call<F, Fut>(&self, request: F)
    where
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = jellyfin::Result<()>> + Send + 'static,
    {
        self.queue_request(Box::new(move |client| request(client).boxed()), None);
    }

    fn queue_request(&self, request: Request, done: Option<std::sync::mpsc::Sender<()>>) {
        // The queue runs until `self` is gone, so this doesn't fail.
        let _ = self.requests.send(Queued { request, done });
    }

    /// `Buffering` (`ready: false`) or `Ready` for the loaded queue entry.
    fn report(&self, ready: bool, position: Ticks, is_playing: bool) {
        let Some(playlist_item_id) = self.loaded.clone() else {
            return;
        };
        let report = BufferReport {
            when: self.clock.server_now(),
            position,
            is_playing,
            playlist_item_id,
        };
        tracing::debug!(ready, position = position.as_secs_f64(), "SyncPlay report");
        self.call(move |client| async move {
            if ready {
                client.syncplay_ready(&report).await
            } else {
                client.syncplay_buffering(&report).await
            }
        });
    }

    /// Fetches the names of queued items not seen yet, for the queue list. Replacing an earlier
    /// fetch loses nothing: the names it would have brought are still missing, so they're asked
    /// for again.
    fn fetch_names(&mut self, queue: &PlayQueue) {
        let mut ids: Vec<String> = queue
            .playlist
            .iter()
            .map(|entry| entry.item_id.clone())
            .filter(|id| !self.names.contains_key(id))
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        let client = self.client.clone();
        self.naming = Some(Task::spawn(&self.ctx, async move {
            let query = ItemsQuery {
                ids,
                ..Default::default()
            };
            client.items(&query).await.map(|result| result.items)
        }));
    }

    /// Samples the server clock now and every minute while in the group, reporting the ping.
    fn start_clock(&mut self) {
        let (client, inbox) = (self.client.clone(), self.inbox_sender.clone());
        self.clock_task = Some(Task::spawn(&self.ctx, async move {
            // The task's own copy, for the ping; the UI's gets the same samples.
            let mut clock = ServerClock::default();
            let mut samples = FIRST_SAMPLES;
            loop {
                for _ in 0..samples {
                    match client.clock_sample().await {
                        Ok(sample) => {
                            clock.add(sample);
                            if inbox.send(Inbound::Clock(sample)).is_err() {
                                return;
                            }
                        }
                        Err(err) => tracing::warn!("server clock sample failed: {err}"),
                    }
                }
                samples = 1;
                if let Some(best) = clock.best() {
                    let ping = best.round_trip.as_millis() as u64;
                    if let Err(err) = client.syncplay_ping(ping).await {
                        tracing::warn!("SyncPlay ping failed: {err}");
                    }
                }
                tokio::time::sleep(CLOCK_INTERVAL).await;
            }
        }));
    }

    /// Sends `commands` to a session's player at the local instant `at`, waiting on tokio (see
    /// the module's doc), or now if that's past. Replaces, so cancels, the command scheduled
    /// before.
    fn schedule(&mut self, at: Instant, player: mpsc::UnboundedSender<UiCommand>, commands: Vec<UiCommand>) {
        if at <= Instant::now() {
            // A timer that's already due still waits for tokio's next tick.
            self.scheduled = None;
            for command in commands {
                let _ = player.send(command);
            }
            return;
        }
        self.scheduled = Some(Task::spawn(&self.ctx, async move {
            tokio::time::sleep_until(at.into()).await;
            let late = at.elapsed();
            for command in commands {
                let _ = player.send(command);
            }
            late
        }));
    }
}

/// Runs SyncPlay's requests one at a time, in order. Not a `Task`: the last ones (leaving the
/// group on sign-out) still go out after the services are gone. It ends once the queue's
/// sender is dropped and everything queued has run.
fn request_queue(client: Client, inbox: task::Sender<Inbound>) -> mpsc::UnboundedSender<Queued> {
    let (sender, mut queue) = mpsc::unbounded_channel::<Queued>();
    runtime::detach(async move {
        while let Some(Queued { request, done }) = queue.recv().await {
            if let Err(err) = request(client.clone()).await {
                tracing::warn!("SyncPlay request failed: {err}");
                let _ = inbox.send(Inbound::Failed(err));
            }
            if let Some(done) = done {
                let _ = done.send(());
            }
        }
    });
    sender
}

/// What drift correction does about `drift` seconds (positive: ahead of the group).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Correction {
    /// Play at this speed (1.0 when close enough).
    Speed(f64),
    /// Too far off to catch up: seek to the group's position.
    Seek,
}

fn correction(drift: f64) -> Correction {
    if drift.abs() > DRIFT_SEEK {
        Correction::Seek
    } else if drift.abs() > DRIFT_TOLERANCE {
        Correction::Speed(1. - (drift * SPEED_PER_DRIFT).clamp(-MAX_SPEED_CHANGE, MAX_SPEED_CHANGE))
    } else {
        Correction::Speed(1.)
    }
}

/// How to carry out an `Unpause`, from the local position, the group's position and how far
/// ahead (`Ok`) or late (`Err`) the command's time is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnpausePlan {
    /// Seek here first.
    seek: Option<Duration>,
    /// Unpause at the command's time this far ahead; `None`: now, it's late.
    after: Option<Duration>,
    /// The group's position as playback starts.
    from: Duration,
}

fn plan_unpause(local: Duration, position: Duration, delay: Result<Duration, Duration>) -> UnpausePlan {
    let off = |target: Duration| local.abs_diff(target) > SEEK_THRESHOLD;
    match delay {
        // Wait at the group's position, then start exactly at `when`.
        Ok(ahead) => UnpausePlan {
            seek: off(position).then_some(position),
            after: Some(ahead),
            from: position,
        },
        // Late: catch up by however late this is.
        Err(late) => {
            let target = position + late;
            UnpausePlan {
                seek: off(target).then_some(target),
                after: None,
                from: target,
            }
        }
    }
}

/// SyncPlay where it involves playback, and the seam playback (`playback.rs`) calls: whether
/// this session is in a group, and handing the group what's played and the controls.
impl Services {
    /// Whether this session is in a SyncPlay group, which then decides what plays: playing
    /// something gives the group a new queue instead.
    pub fn in_group(&self) -> bool {
        self.syncplay.group.is_some()
    }

    /// A pause, seek or stop from a group session's controls.
    pub(crate) fn group_request(&mut self, request: GroupRequest) {
        self.syncplay.request(request);
    }

    /// Plays `items` in order for the whole group.
    pub(crate) fn group_start_items(&mut self, items: &[BaseItemDto]) {
        self.syncplay.start_items(items);
    }

    /// Plays `item` for the whole group from `start`, with the next episode queued after it.
    pub(crate) fn group_start_queue(&mut self, item: &BaseItemDto, start: Ticks) {
        self.syncplay.start_queue(item, start);
    }

    /// Adds `items` to the group's queue.
    pub(crate) fn group_enqueue(&mut self, items: &[BaseItemDto], next: bool) {
        self.syncplay.enqueue(items, next);
    }

    /// Loads the group's current item again here only (another version, or a transcode
    /// restarted for other tracks). Reports `Buffering`, so the server pauses the others and
    /// remembers the group's position; loads paused; and reports `Ready` once loaded, so the
    /// server resumes everyone (or has this member seek first, if it landed elsewhere).
    pub(crate) fn group_reload(&mut self, item: &BaseItemDto, mut options: PlayOptions) {
        let syncplay = &mut self.syncplay;
        let Some(group_item) = syncplay.loaded.clone() else {
            return;
        };
        options.group_item = Some(group_item);
        options.start_paused = true;
        syncplay.scheduled = None;
        // The session being replaced counts as loaded; `Ready` is the new one's to report,
        // once it starts (`group_playback_started`).
        syncplay.watched = None;
        syncplay.drift.cancel();
        syncplay.ready_due = true;
        syncplay.report(false, options.start.unwrap_or_default(), false);
        self.play(item, options);
    }

    /// The group's item ended here: move the group on to the next one (the server ignores
    /// the duplicate reports from other members).
    pub(crate) fn group_item_ended(&mut self, playlist_item_id: &str) {
        self.syncplay.loaded = None;
        let id = playlist_item_id.to_owned();
        self.syncplay
            .call(move |client| async move { client.syncplay_next_item(&id).await });
    }

    /// A session started (`self.playback.now_playing()` is its state). The group session's
    /// player is watched for restarts, to report `Ready`, and kept on the group's timeline.
    pub(crate) fn group_playback_started(&mut self) {
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        let Some(group_item) = &state.group_item else {
            return;
        };
        if self.syncplay.loaded.as_ref() != Some(group_item) {
            // The group was left (or stopped) while this loaded: nobody controls it now. One
            // the group moved past is replaced as soon as its successor loads.
            if self.syncplay.loaded.is_none() {
                tracing::info!("stopping a SyncPlay item that loaded after the group stopped");
                self.stop_playback();
            }
            return;
        }
        self.syncplay.watched = Some(Watched {
            session: state.session,
            seen: 0,
        });
        self.syncplay.speed = 1.;
        self.syncplay.drift.set(DRIFT_INTERVAL);
    }

    /// A `SyncPlayGroupUpdate` from the session websocket.
    pub fn on_syncplay_update(&mut self, update: GroupUpdate) {
        tracing::debug!(?update, "SyncPlay update");
        match update {
            GroupUpdate::GroupJoined(group) => {
                self.toast(Toast::info(format!("Joined the SyncPlay group {}", group.group_name)));
                self.syncplay.group = Some(group);
                self.syncplay.start_clock();
            }
            GroupUpdate::GroupLeft | GroupUpdate::NotInGroup => {
                if let Some(group) = self.syncplay.group.take() {
                    self.toast(Toast::info(format!("Left {}", group.group_name)));
                }
                self.leave_group_playback();
            }
            GroupUpdate::GroupDoesNotExist => {
                self.toast(Toast::info("That SyncPlay group no longer exists."));
                self.syncplay.refresh_groups();
            }
            GroupUpdate::LibraryAccessDenied => {
                self.toast(Toast::info("Someone in the group can't access this item."));
            }
            GroupUpdate::UserJoined(user) => {
                if let Some(group) = &mut self.syncplay.group {
                    group.participants.push(user.clone());
                }
                self.toast(Toast::info(format!("{user} joined")));
            }
            GroupUpdate::UserLeft(user) => {
                if let Some(group) = &mut self.syncplay.group
                    && let Some(i) = group.participants.iter().position(|p| *p == user)
                {
                    group.participants.remove(i);
                }
                self.toast(Toast::info(format!("{user} left")));
            }
            GroupUpdate::StateUpdate { state, .. } => {
                if let Some(group) = &mut self.syncplay.group {
                    group.state = state;
                }
            }
            GroupUpdate::PlayQueue(queue) => self.on_group_queue(queue),
        }
    }

    /// A `SyncPlayCommand` from the session websocket, carried out on the group session's
    /// player at the command's time.
    pub fn on_syncplay_command(&mut self, command: Command) {
        let delay = self
            .syncplay
            .clock
            .to_local(command.when)
            .duration_since(SystemTime::now())
            .map_err(|late| late.duration());
        tracing::info!(
            command = ?command.kind,
            position = command.position.as_secs_f64(),
            lead_ms = match &delay {
                Ok(ahead) => ahead.as_millis() as i64,
                Err(late) => -(late.as_millis() as i64),
            },
            "SyncPlay command"
        );
        if self.syncplay.loaded.as_deref() != Some(command.playlist_item_id.as_str()) {
            tracing::debug!(loaded = ?self.syncplay.loaded, "SyncPlay command for another item");
            return;
        }
        let Some(state) = self.group_player() else {
            tracing::debug!("SyncPlay command before the player started");
            return;
        };
        let (local_position, player) = (state.position, state.commands());
        let send = |command: UiCommand| {
            let _ = player.send(command);
        };
        let position: Duration = command.position.into();
        if command.kind != CommandKind::Unpause {
            self.syncplay.timeline = None;
            self.set_group_speed(1.);
        }
        match command.kind {
            CommandKind::Unpause => {
                let plan = plan_unpause(local_position, position, delay);
                if let Some(target) = plan.seek {
                    send(UiCommand::SeekTo(target));
                }
                let start = Instant::now() + plan.after.unwrap_or_default();
                self.syncplay.timeline = Some((start, plan.from));
                if plan.after.is_some() {
                    let start = start + debug_skew();
                    self.syncplay
                        .schedule(start, player.clone(), vec![UiCommand::SetPause(false)]);
                } else {
                    self.syncplay.scheduled = None;
                    send(UiCommand::SetPause(false));
                }
            }
            CommandKind::Pause => {
                let at = Instant::now() + delay.unwrap_or_default();
                let commands = vec![UiCommand::SetPause(true), UiCommand::SeekTo(position)];
                self.syncplay.schedule(at, player.clone(), commands);
            }
            CommandKind::Seek => {
                self.syncplay.scheduled = None;
                send(UiCommand::SetPause(true));
                send(UiCommand::SeekTo(position));
                self.syncplay.report(false, command.position, false);
                self.syncplay.ready_due = true;
                self.mark_restart_seen();
            }
            CommandKind::Stop => {
                self.syncplay.loaded = None;
                self.syncplay.scheduled = None;
                self.stop_playback();
            }
        }
    }

    /// SyncPlay's background work, from [`Services::tick`] after playback's (so a session that
    /// just started is known): what the requests and the clock sent back, the fetches, the
    /// scheduled command finishing, `Ready` after a restart, and the drift check.
    pub(crate) fn tick_syncplay(&mut self) {
        let inbound: Vec<Inbound> = self.syncplay.inbox.drain().collect();
        for message in inbound {
            match message {
                Inbound::Failed(err) => {
                    let message = self.handle_error(&err);
                    self.toast(Toast::error(format!("SyncPlay: {message}")));
                }
                Inbound::Clock(sample) => self.syncplay.clock.add(sample),
            }
        }

        let syncplay = &mut self.syncplay;
        if let Some(result) = syncplay.listing.as_mut().and_then(Task::poll) {
            syncplay.listing = None;
            match result {
                Ok(groups) => syncplay.groups = groups,
                Err(err) => tracing::warn!("couldn't list SyncPlay groups: {err}"),
            }
        }
        if let Some(result) = syncplay.naming.as_mut().and_then(Task::poll) {
            syncplay.naming = None;
            match result {
                Ok(items) => {
                    for item in items {
                        // Episodes read "Series · S1:E2 · Name".
                        let (title, detail) = card_text(&item);
                        let name = match item.kind {
                            BaseItemKind::Episode => format!("{title} · {detail}"),
                            _ => title,
                        };
                        syncplay.names.insert(item.id, name);
                    }
                }
                Err(err) => tracing::warn!("couldn't name the group's queue: {err}"),
            }
        }
        if let Some(late) = syncplay.scheduled.as_mut().and_then(Task::poll) {
            syncplay.scheduled = None;
            tracing::debug!(late_us = late.as_micros() as u64, "SyncPlay scheduled command sent");
        }
        self.load_group_item();

        if self.syncplay.watched.is_some() && self.group_player().is_none() {
            // The group session ended or another took its place: nothing left to watch.
            self.syncplay.watched = None;
            self.syncplay.drift.cancel();
        }
        self.check_ready();
        let syncplay = &mut self.syncplay;
        if syncplay.drift.fire(&syncplay.ctx) {
            syncplay.drift.set(DRIFT_INTERVAL);
            self.correct_drift();
        }
    }

    /// Plays the queue's current entry once its item is fetched, paused at the group's
    /// position, unless the group has moved on meanwhile.
    fn load_group_item(&mut self) {
        let Some(result) = self.syncplay.loading.as_mut().and_then(|loading| loading.item.poll()) else {
            return;
        };
        let Some(Loading {
            playlist_item_id,
            start,
            ..
        }) = self.syncplay.loading.take()
        else {
            return;
        };
        match result {
            Ok(item) if self.syncplay.loaded.as_ref() == Some(&playlist_item_id) => {
                self.syncplay.ready_due = true;
                let options = PlayOptions {
                    start: Some(start),
                    start_paused: true,
                    group_item: Some(playlist_item_id),
                    ..Default::default()
                };
                self.play(&item, options);
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!("couldn't load the group's item: {err}");
                let message = self.handle_error(&err);
                self.toast(Toast::error(format!("Couldn't load the group's item: {message}")));
            }
        }
    }

    /// Loads the queue's current item when it changes (paused, at the group's position).
    fn on_group_queue(&mut self, queue: PlayQueue) {
        let syncplay = &mut self.syncplay;
        syncplay.fetch_names(&queue);
        let current = queue.current().cloned();
        let start = queue.start_position_ticks;
        syncplay.queue = Some(queue);
        let Some(current) = current else {
            self.leave_group_playback();
            return;
        };
        if syncplay.loaded.as_ref() == Some(&current.playlist_item_id) {
            return;
        }
        syncplay.loaded = Some(current.playlist_item_id.clone());
        syncplay.scheduled = None;
        syncplay.report(false, start, false);
        let client = syncplay.client.clone();
        let item_id = current.item_id;
        syncplay.loading = Some(Loading {
            playlist_item_id: current.playlist_item_id,
            start,
            item: Task::spawn(&syncplay.ctx, async move { client.item(&item_id, &[]).await }),
        });
    }

    /// Stops following the group: cancels scheduled commands and stops a group playback.
    fn leave_group_playback(&mut self) {
        let syncplay = &mut self.syncplay;
        syncplay.scheduled = None;
        syncplay.queue = None;
        syncplay.loading = None;
        syncplay.ready_due = false;
        syncplay.drift.cancel();
        syncplay.timeline = None;
        let watched = syncplay.watched.take();
        if syncplay.loaded.take().is_some() && watched.is_some() {
            self.stop_playback();
        }
        if self.syncplay.group.is_none() {
            self.syncplay.clock_task = None;
        }
    }

    /// The group session's player state, while it's the one playing.
    fn group_player(&self) -> Option<&NowPlaying> {
        let watched = self.syncplay.watched?;
        self.playback
            .now_playing()
            .filter(|state| state.session == watched.session)
    }

    /// Reports `Ready` once the group session's player has restarted (finished loading or
    /// seeking) since it was due.
    fn check_ready(&mut self) {
        if !self.syncplay.ready_due {
            return;
        }
        let Some(watched) = self.syncplay.watched else {
            return;
        };
        let Some(state) = self.group_player() else {
            return;
        };
        if state.restarts > watched.seen && !state.loading {
            tracing::trace!(
                restarts = state.restarts,
                seen = watched.seen,
                "SyncPlay player restarted"
            );
            let (position, playing, restarts) = (Ticks::from(state.position), !state.paused, state.restarts);
            self.syncplay.ready_due = false;
            self.syncplay.watched = Some(Watched {
                seen: restarts,
                ..watched
            });
            self.syncplay.report(true, position, playing);
        }
    }

    /// Restarts so far don't count towards the next `Ready`.
    fn mark_restart_seen(&mut self) {
        let Some(restarts) = self.group_player().map(|state| state.restarts) else {
            return;
        };
        if let Some(watched) = &mut self.syncplay.watched {
            watched.seen = restarts;
        }
    }

    /// Keeps the local player on the group's timeline while playing: small drift is evened
    /// out by playing slightly faster or slower (mpv keeps the pitch), large drift by seeking.
    fn correct_drift(&mut self) {
        let Some((anchor_at, anchor)) = self.syncplay.timeline else {
            return;
        };
        let Some(state) = self.group_player() else {
            return;
        };
        let now = Instant::now();
        // Wait for a position reported since playback started.
        if now < anchor_at || state.position_at <= anchor_at || state.paused || state.loading {
            return;
        }
        let expected = anchor + (now - anchor_at);
        let local = state.position + (now - state.position_at).mul_f64(self.syncplay.speed);
        let drift = local.as_secs_f64() - expected.as_secs_f64();
        tracing::debug!(
            drift_ms = (drift * 1000.) as i64,
            speed = self.syncplay.speed,
            "SyncPlay drift"
        );
        let speed = match correction(drift) {
            Correction::Seek => {
                state.send(UiCommand::SeekTo(expected));
                1.
            }
            Correction::Speed(speed) => speed,
        };
        self.set_group_speed(speed);
    }

    fn set_group_speed(&mut self, speed: f64) {
        if (speed - self.syncplay.speed).abs() < 0.001 {
            return;
        }
        self.syncplay.speed = speed;
        if let Some(state) = self.group_player() {
            state.send(UiCommand::SetSpeed(speed));
        }
    }
}

/// Dev aid: `TABBYFIN_SYNCPLAY_SKEW_MS=800` starts playback that much after the group, to
/// watch drift correction catch up.
fn debug_skew() -> Duration {
    if !cfg!(debug_assertions) {
        return Duration::ZERO;
    }
    std::env::var("TABBYFIN_SYNCPLAY_SKEW_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Correction, UnpausePlan, correction, plan_unpause};

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn small_drift_is_left_alone() {
        assert_eq!(correction(0.05), Correction::Speed(1.));
        assert_eq!(correction(-0.08), Correction::Speed(1.));
    }

    #[test]
    fn drift_is_evened_out_by_speed_up_to_five_percent() {
        // Ahead plays slower, behind faster, by a quarter of the drift per second.
        assert_eq!(correction(0.1), Correction::Speed(1. - 0.025));
        assert_eq!(correction(-0.1), Correction::Speed(1. + 0.025));
        assert_eq!(correction(0.8), Correction::Speed(0.95));
        assert_eq!(correction(-1.5), Correction::Speed(1.05));
    }

    #[test]
    fn large_drift_seeks() {
        assert_eq!(correction(1.6), Correction::Seek);
        assert_eq!(correction(-3.), Correction::Seek);
    }

    #[test]
    fn an_unpause_ahead_waits_at_the_groups_position() {
        // Close enough: no seek.
        let plan = plan_unpause(secs(10.2), secs(10.), Ok(secs(0.5)));
        assert_eq!(
            plan,
            UnpausePlan {
                seek: None,
                after: Some(secs(0.5)),
                from: secs(10.)
            }
        );
        // Further off than 0.4 s: seek there first.
        let plan = plan_unpause(secs(12.), secs(10.), Ok(secs(0.5)));
        assert_eq!(plan.seek, Some(secs(10.)));
        assert_eq!(plan.after, Some(secs(0.5)));
    }

    #[test]
    fn a_late_unpause_catches_up_by_the_lateness() {
        let plan = plan_unpause(secs(10.), secs(10.), Err(secs(1.)));
        assert_eq!(
            plan,
            UnpausePlan {
                seek: Some(secs(11.)),
                after: None,
                from: secs(11.)
            }
        );
        // A little late: start now without seeking, on the group's timeline.
        let plan = plan_unpause(secs(10.), secs(10.), Err(secs(0.1)));
        assert_eq!(plan.seek, None);
        assert_eq!(plan.from, secs(10.1));
    }
}
