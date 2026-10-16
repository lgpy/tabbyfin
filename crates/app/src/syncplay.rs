//! SyncPlay (watch together) in the app: the group this session is in, and playback while the
//! group controls it.
//!
//! The server owns the group's queue and state. When the queue's current item changes, every
//! member loads it paused and reports `Ready`; the server then sends scheduled commands
//! (unpause, pause, seek, stop) with a `When` in server time, which [`ServerClock`] turns into
//! local time. Local pause, seek and stop become requests ([`GroupRequest`]) instead of acting
//! directly. See `docs/ROADMAP.md` §2.4.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{
    BufferReport, Client, Command, CommandKind, GroupInfo, GroupUpdate, ItemsQuery, PlayQueue, RepeatMode, ServerClock,
    ShuffleMode, Ticks,
};

use crate::playback::{NowPlaying, PlayOptions, PlaybackEvent, PlaybackManager, UiCommand};
use crate::runtime;

/// How often the clock is re-sampled (and the ping reported) while in a group.
const CLOCK_INTERVAL: Duration = Duration::from_secs(60);
/// Samples taken on joining, to start with a good estimate.
const FIRST_SAMPLES: usize = 4;

pub enum SyncPlayEvent {
    /// Something to tell the user, e.g. who joined.
    Notice(SharedString),
}

/// A local control, sent to the group instead.
#[derive(Debug, Clone, Copy)]
pub enum GroupRequest {
    SetPause(bool),
    Seek(Duration),
    Stop,
}

pub struct SyncPlay {
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
    /// The group session's player state, and the restart count when a `Ready` was last due.
    now_playing: Option<(Entity<NowPlaying>, u64)>,
    /// Report `Ready` after the player's next restart (load or seek finished).
    ready_due: bool,
    /// The pending scheduled command; replacing it cancels the previous one.
    scheduled: Option<Task<()>>,
    clock_task: Option<Task<()>>,
    /// Observes the group session's player.
    player_watch: Option<Subscription>,
    /// The group's timeline while playing: the position it's at, at a local instant.
    timeline: Option<(Instant, Duration)>,
    /// The playback speed set for drift correction.
    speed: f64,
    drift_task: Option<Task<()>>,
    _playback: Subscription,
}

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

struct GlobalSyncPlay(Entity<SyncPlay>);

impl Global for GlobalSyncPlay {}

impl EventEmitter<SyncPlayEvent> for SyncPlay {}

impl SyncPlay {
    /// Installs SyncPlay for a signed-in session. Needs [`PlaybackManager::init`] first.
    pub fn init(client: Client, cx: &mut App) {
        let syncplay = cx.new(|cx| SyncPlay {
            client,
            group: None,
            groups: Vec::new(),
            clock: ServerClock::default(),
            queue: None,
            names: HashMap::new(),
            loaded: None,
            now_playing: None,
            ready_due: false,
            scheduled: None,
            clock_task: None,
            player_watch: None,
            timeline: None,
            speed: 1.,
            drift_task: None,
            _playback: cx.subscribe(&PlaybackManager::global(cx), Self::on_playback),
        });
        #[cfg(debug_assertions)]
        if let Ok(name) = std::env::var("TABBYFIN_SYNCPLAY") {
            syncplay.update(cx, |syncplay, cx| syncplay.debug_join(name, cx));
        }
        cx.set_global(GlobalSyncPlay(syncplay));
    }

    /// Dev aid: `TABBYFIN_SYNCPLAY=<group name>` joins that group at startup, creating it
    /// if there's none.
    #[cfg(debug_assertions)]
    fn debug_join(&mut self, name: String, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move {
            let groups = client.syncplay_groups().await?;
            match groups.iter().find(|g| g.group_name == name) {
                Some(group) => client.syncplay_join(&group.group_id).await,
                None => client.syncplay_new(&name).await,
            }
        });
    }

    pub fn global(cx: &App) -> Entity<SyncPlay> {
        cx.global::<GlobalSyncPlay>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<SyncPlay>> {
        cx.try_global::<GlobalSyncPlay>().map(|g| g.0.clone())
    }

    /// Fetches the server's groups into [`Self::groups`].
    pub fn refresh_groups(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let groups = runtime::spawn(async move { client.syncplay_groups().await }).await;
            this.update(cx, |this, cx| {
                match groups {
                    Ok(groups) => this.groups = groups,
                    Err(err) => tracing::warn!("couldn't list SyncPlay groups: {err}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn create(&mut self, name: String, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move { client.syncplay_new(&name).await });
    }

    pub fn join(&mut self, group_id: String, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move { client.syncplay_join(&group_id).await });
    }

    pub fn leave(&mut self, cx: &mut Context<Self>) {
        self.call(cx, |client| async move { client.syncplay_leave().await });
    }

    /// Plays `items` in order for the whole group.
    pub fn start_items(&mut self, items: &[BaseItemDto], cx: &mut Context<Self>) {
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        self.call(cx, move |client| async move {
            client.syncplay_set_queue(&ids, 0, Ticks::default()).await
        });
    }

    /// Plays `item` for the whole group, with the next episode queued after it.
    pub fn start_queue(&mut self, item: &BaseItemDto, start: Ticks, cx: &mut Context<Self>) {
        let item = item.clone();
        self.call(cx, move |client| async move {
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
    pub fn enqueue(&mut self, items: &[BaseItemDto], next: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        self.call(cx, move |client| async move { client.syncplay_queue(&ids, next).await });
    }

    /// Loads the group's current item again here only (another version, or a transcode
    /// restarted for other tracks). Reports `Buffering`, so the server pauses the others and
    /// remembers the group's position; loads paused; and reports `Ready` once loaded, so the
    /// server resumes everyone (or has this member seek first, if it landed elsewhere).
    pub fn reload(&mut self, item: &BaseItemDto, mut options: PlayOptions, cx: &mut Context<Self>) {
        let Some(group_item) = self.loaded.clone() else {
            return;
        };
        options.group_item = Some(group_item);
        options.start_paused = true;
        self.scheduled = None;
        // The session being replaced counts as loaded; `Ready` is the new one's to report,
        // once it starts (`on_playback`).
        self.player_watch = None;
        self.now_playing = None;
        self.drift_task = None;
        self.ready_due = true;
        self.report(false, options.start.unwrap_or_default(), false, cx);
        let item = item.clone();
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
    }

    /// The group's item ended here: move the group on to the next one (the server ignores
    /// the duplicate reports from other members).
    pub fn item_ended(&mut self, playlist_item_id: &str, cx: &mut Context<Self>) {
        self.loaded = None;
        let id = playlist_item_id.to_owned();
        self.call(cx, move |client| async move { client.syncplay_next_item(&id).await });
    }

    pub fn request(&mut self, request: GroupRequest, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move {
            match request {
                GroupRequest::SetPause(true) => client.syncplay_pause().await,
                GroupRequest::SetPause(false) => client.syncplay_unpause().await,
                GroupRequest::Seek(position) => client.syncplay_seek(Ticks::from(position)).await,
                GroupRequest::Stop => client.syncplay_stop().await,
            }
        });
    }

    /// Runs a request on tokio and logs failures. Results arrive as websocket updates.
    fn call<F, Fut>(&mut self, cx: &mut Context<Self>, request: F)
    where
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = jellyfin::Result<()>> + Send + 'static,
    {
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            if let Err(err) = runtime::spawn(async move { request(client).await }).await {
                tracing::warn!("SyncPlay request failed: {err}");
                this.update(cx, |_, cx| {
                    cx.emit(SyncPlayEvent::Notice(format!("SyncPlay: {err}").into()));
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn handle_update(&mut self, update: GroupUpdate, cx: &mut Context<Self>) {
        tracing::debug!(?update, "SyncPlay update");
        match update {
            GroupUpdate::GroupJoined(group) => {
                cx.emit(SyncPlayEvent::Notice(
                    format!("Joined the SyncPlay group {}", group.group_name).into(),
                ));
                self.group = Some(group);
                self.start_clock(cx);
            }
            GroupUpdate::GroupLeft | GroupUpdate::NotInGroup => {
                if let Some(group) = self.group.take() {
                    cx.emit(SyncPlayEvent::Notice(format!("Left {}", group.group_name).into()));
                }
                self.leave_group_playback(cx);
            }
            GroupUpdate::GroupDoesNotExist => {
                cx.emit(SyncPlayEvent::Notice("That SyncPlay group no longer exists.".into()));
                self.refresh_groups(cx);
            }
            GroupUpdate::LibraryAccessDenied => cx.emit(SyncPlayEvent::Notice(
                "Someone in the group can't access this item.".into(),
            )),
            GroupUpdate::UserJoined(user) => {
                if let Some(group) = &mut self.group {
                    group.participants.push(user.clone());
                }
                cx.emit(SyncPlayEvent::Notice(format!("{user} joined").into()));
            }
            GroupUpdate::UserLeft(user) => {
                if let Some(group) = &mut self.group
                    && let Some(i) = group.participants.iter().position(|p| *p == user)
                {
                    group.participants.remove(i);
                }
                cx.emit(SyncPlayEvent::Notice(format!("{user} left").into()));
            }
            GroupUpdate::StateUpdate { state, .. } => {
                if let Some(group) = &mut self.group {
                    group.state = state;
                }
            }
            GroupUpdate::PlayQueue(queue) => self.on_queue(queue, cx),
        }
        cx.notify();
    }

    /// Plays another entry of the group's queue (for everyone).
    pub fn play_entry(&mut self, playlist_item_id: String, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move {
            client.syncplay_set_playlist_item(&playlist_item_id).await
        });
    }

    pub fn remove_entry(&mut self, playlist_item_id: String, cx: &mut Context<Self>) {
        self.call(cx, move |client| async move {
            client.syncplay_remove(&[playlist_item_id]).await
        });
    }

    /// Moves an entry up (`-1`) or down (`1`) the queue.
    pub fn move_entry(&mut self, playlist_item_id: String, by: isize, cx: &mut Context<Self>) {
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
        self.call(cx, move |client| async move {
            client.syncplay_move(&playlist_item_id, to).await
        });
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let mode = match self.queue.as_ref().map(|q| q.shuffle_mode) {
            Some(ShuffleMode::Shuffle) => ShuffleMode::Sorted,
            _ => ShuffleMode::Shuffle,
        };
        self.call(cx, move |client| async move { client.syncplay_set_shuffle(mode).await });
    }

    /// None, all, one.
    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let mode = self
            .queue
            .as_ref()
            .map_or(RepeatMode::RepeatNone, |q| q.repeat_mode)
            .next();
        self.call(cx, move |client| async move { client.syncplay_set_repeat(mode).await });
    }

    /// Fetches the names of queued items not seen yet, for the queue list.
    fn fetch_names(&mut self, queue: &PlayQueue, cx: &mut Context<Self>) {
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
        cx.spawn(async move |this, cx| {
            let query = ItemsQuery {
                ids,
                ..Default::default()
            };
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            match result {
                Ok(result) => {
                    this.update(cx, |this, cx| {
                        for item in result.items {
                            // Episodes read "Series · S1:E2 · Name".
                            let (title, detail) = crate::components::card_text(&item);
                            let name = match item.kind {
                                BaseItemKind::Episode => format!("{title} · {detail}"),
                                _ => title,
                            };
                            this.names.insert(item.id, name);
                        }
                        cx.notify();
                    })
                    .ok();
                }
                Err(err) => tracing::warn!("couldn't name the group's queue: {err}"),
            }
        })
        .detach();
    }

    /// Loads the queue's current item when it changes (paused, at the group's position).
    fn on_queue(&mut self, queue: PlayQueue, cx: &mut Context<Self>) {
        self.fetch_names(&queue, cx);
        let current = queue.current().cloned();
        let start = queue.start_position_ticks;
        self.queue = Some(queue);
        let Some(current) = current else {
            self.leave_group_playback(cx);
            return;
        };
        if self.loaded.as_ref() == Some(&current.playlist_item_id) {
            return;
        }
        self.loaded = Some(current.playlist_item_id.clone());
        self.scheduled = None;
        self.report(false, start, false, cx);
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let item_id = current.item_id.clone();
            let item = runtime::spawn(async move { client.item(&item_id, &[]).await }).await;
            this.update(cx, |this, cx| match item {
                Ok(item) if this.loaded.as_ref() == Some(&current.playlist_item_id) => {
                    this.ready_due = true;
                    let options = PlayOptions {
                        start: Some(start),
                        start_paused: true,
                        group_item: Some(current.playlist_item_id.clone()),
                        ..Default::default()
                    };
                    PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!("couldn't load the group's item: {err}");
                    cx.emit(SyncPlayEvent::Notice(
                        format!("Couldn't load the group's item: {err}").into(),
                    ));
                }
            })
            .ok();
        })
        .detach();
    }

    /// Stops following the group: cancels scheduled commands and stops a group playback.
    fn leave_group_playback(&mut self, cx: &mut Context<Self>) {
        self.scheduled = None;
        self.queue = None;
        self.ready_due = false;
        self.player_watch = None;
        self.drift_task = None;
        self.timeline = None;
        if self.loaded.take().is_some() && self.now_playing.take().is_some() {
            PlaybackManager::global(cx).update(cx, |playback, cx| playback.stop(cx));
        }
        if self.group.is_none() {
            self.clock_task = None;
        }
    }

    pub fn handle_command(&mut self, command: Command, cx: &mut Context<Self>) {
        let delay = self.clock.to_local(command.when).duration_since(SystemTime::now());
        tracing::info!(
            command = ?command.kind,
            position = command.position.as_secs_f64(),
            lead_ms = match &delay {
                Ok(ahead) => ahead.as_millis() as i64,
                Err(late) => -(late.duration().as_millis() as i64),
            },
            "SyncPlay command"
        );
        if self.loaded.as_deref() != Some(command.playlist_item_id.as_str()) {
            tracing::debug!(loaded = ?self.loaded, "SyncPlay command for another item");
            return;
        }
        let Some((now_playing, _)) = self.now_playing.clone() else {
            tracing::debug!("SyncPlay command before the player started");
            return;
        };
        let local_position = now_playing.read(cx).position;
        let send = move |command: UiCommand, cx: &mut App| now_playing.read(cx).send(command);
        let position: Duration = command.position.into();
        let off = |target: Duration| local_position.abs_diff(target) > SEEK_THRESHOLD;
        if command.kind != CommandKind::Unpause {
            self.timeline = None;
            self.set_speed(1., cx);
        }
        match command.kind {
            CommandKind::Unpause => match delay {
                // Wait at the group's position, then start exactly at `when`.
                Ok(ahead) => {
                    if off(position) {
                        send(UiCommand::SeekTo(position), cx);
                    }
                    self.timeline = Some((Instant::now() + ahead, position));
                    let ahead = ahead + debug_skew();
                    self.schedule(ahead, cx, move |cx| send(UiCommand::SetPause(false), cx));
                }
                // Late: catch up by however late this is.
                Err(late) => {
                    let target = position + late.duration();
                    if off(target) {
                        send(UiCommand::SeekTo(target), cx);
                    }
                    send(UiCommand::SetPause(false), cx);
                    self.timeline = Some((Instant::now(), target));
                }
            },
            CommandKind::Pause => {
                let ahead = delay.unwrap_or_default();
                self.schedule(ahead, cx, move |cx| {
                    send(UiCommand::SetPause(true), cx);
                    send(UiCommand::SeekTo(position), cx);
                });
            }
            CommandKind::Seek => {
                self.scheduled = None;
                send(UiCommand::SetPause(true), cx);
                send(UiCommand::SeekTo(position), cx);
                self.report(false, command.position, false, cx);
                self.ready_due = true;
                self.mark_restart_seen(cx);
            }
            CommandKind::Stop => {
                self.loaded = None;
                self.scheduled = None;
                PlaybackManager::global(cx).update(cx, |playback, cx| playback.stop(cx));
            }
        }
    }

    fn schedule(&mut self, after: Duration, cx: &mut Context<Self>, run: impl FnOnce(&mut App) + 'static) {
        self.scheduled = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(after).await;
            cx.update(run);
        }));
    }

    /// The group session's player: watch it for restarts, to report `Ready`.
    fn on_playback(&mut self, _: Entity<PlaybackManager>, event: &PlaybackEvent, cx: &mut Context<Self>) {
        let PlaybackEvent::Started(now_playing) = event else {
            return;
        };
        if now_playing.read(cx).group_item.is_none() || now_playing.read(cx).group_item != self.loaded {
            return;
        }
        self.player_watch = Some(cx.observe(now_playing, |this, now_playing, cx| {
            this.on_player_change(now_playing, cx)
        }));
        self.speed = 1.;
        self.drift_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRIFT_INTERVAL).await;
                if this.update(cx, |this, cx| this.correct_drift(cx)).is_err() {
                    return;
                }
            }
        }));
        self.now_playing = Some((now_playing.clone(), 0));
    }

    /// Keeps the local player on the group's timeline while playing: small drift is evened
    /// out by playing slightly faster or slower (mpv keeps the pitch), large drift by seeking.
    fn correct_drift(&mut self, cx: &mut Context<Self>) {
        let (Some((anchor_at, anchor)), Some((now_playing, _))) = (self.timeline, &self.now_playing) else {
            return;
        };
        let now = Instant::now();
        let state = now_playing.read(cx);
        // Wait for a position reported since playback started.
        if now < anchor_at || state.position_at <= anchor_at || state.paused || state.loading {
            return;
        }
        let expected = anchor + (now - anchor_at);
        let local = state.position + (now - state.position_at).mul_f64(self.speed);
        let drift = local.as_secs_f64() - expected.as_secs_f64();
        tracing::debug!(drift_ms = (drift * 1000.) as i64, speed = self.speed, "SyncPlay drift");
        let speed = if drift.abs() > DRIFT_SEEK {
            now_playing.read(cx).send(UiCommand::SeekTo(expected));
            1.
        } else if drift.abs() > DRIFT_TOLERANCE {
            1. - (drift * SPEED_PER_DRIFT).clamp(-MAX_SPEED_CHANGE, MAX_SPEED_CHANGE)
        } else {
            1.
        };
        self.set_speed(speed, cx);
    }

    fn set_speed(&mut self, speed: f64, cx: &App) {
        if (speed - self.speed).abs() < 0.001 {
            return;
        }
        self.speed = speed;
        if let Some((now_playing, _)) = &self.now_playing {
            now_playing.read(cx).send(UiCommand::SetSpeed(speed));
        }
    }

    fn mark_restart_seen(&mut self, cx: &App) {
        if let Some((now_playing, seen)) = &mut self.now_playing {
            *seen = now_playing.read(cx).restarts;
        }
    }

    fn on_player_change(&mut self, now_playing: Entity<NowPlaying>, cx: &mut Context<Self>) {
        let Some((current, seen)) = &self.now_playing else {
            return;
        };
        if *current != now_playing || !self.ready_due {
            return;
        }
        let state = now_playing.read(cx);
        tracing::trace!(
            restarts = state.restarts,
            seen = *seen,
            loading = state.loading,
            "SyncPlay player change"
        );
        if state.restarts > *seen && !state.loading {
            let (position, playing) = (Ticks::from(state.position), !state.paused);
            self.ready_due = false;
            self.mark_restart_seen(cx);
            self.report(true, position, playing, cx);
        }
    }

    /// `Buffering` (`ready: false`) or `Ready` for the loaded queue entry.
    fn report(&mut self, ready: bool, position: Ticks, is_playing: bool, cx: &mut Context<Self>) {
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
        self.call(cx, move |client| async move {
            if ready {
                client.syncplay_ready(&report).await
            } else {
                client.syncplay_buffering(&report).await
            }
        });
    }

    /// Samples the server clock now and every minute while in the group, reporting the ping.
    fn start_clock(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.clock_task = Some(cx.spawn(async move |this, cx| {
            let mut samples = FIRST_SAMPLES;
            loop {
                for _ in 0..samples {
                    let client = client.clone();
                    match runtime::spawn(async move { client.clock_sample().await }).await {
                        Ok(sample) => {
                            if this.update(cx, |this, _| this.clock.add(sample)).is_err() {
                                return;
                            }
                        }
                        Err(err) => tracing::warn!("server clock sample failed: {err}"),
                    }
                }
                samples = 1;
                let best = this.read_with(cx, |this, _| this.clock.best()).ok().flatten();
                if let Some(best) = best {
                    let client = client.clone();
                    let ping = best.round_trip.as_millis() as u64;
                    if let Err(err) = runtime::spawn(async move { client.syncplay_ping(ping).await }).await {
                        tracing::warn!("SyncPlay ping failed: {err}");
                    }
                }
                cx.background_executor().timer(CLOCK_INTERVAL).await;
            }
        }));
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
