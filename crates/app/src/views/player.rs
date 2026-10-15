//! Full-window player for embedded mpv: the video plus GPUI controls on top.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType};
use player::libmpv::Frame;

use crate::components::{remote_image, timestamp};
use crate::markers::{self, SegmentKind};
use crate::motion;
use crate::playback::{Adjustments, NowPlaying, PlayOptions, PlaybackManager, SubtitleChoice, UiCommand};
use crate::smooth_scroll::smooth_wheel;
use crate::tracks::{Choice, ChoiceSelect, SUBTITLES_OFF, track_choices};
use crate::trickplay::Trickplay;
use crate::zoom::dp;

actions!(
    player,
    [
        TogglePause,
        SeekForward,
        SeekBackward,
        VolumeUp,
        VolumeDown,
        ToggleMute,
        NextEpisode,
        PreviousEpisode,
        NextChapter,
        PrevChapter,
        Skip,
        ToggleFullscreen,
        ExitPlayer,
        SpeedUp,
        SpeedDown,
        SubtitleDelayUp,
        SubtitleDelayDown
    ]
);

pub const KEY_CONTEXT: &str = "Player";
/// Seconds per seek key press (and remote rewind or fast-forward).
pub const SEEK_STEP: f64 = 10.;
/// Volume percent per key press, wheel notch or remote volume step.
pub const VOLUME_STEP: f64 = 5.;
/// Wheel lines per volume step. X11 and Windows normalise a wheel notch to three lines, so
/// one notch is one step; a trackpad's pixel deltas are turned into lines first.
const SCROLL_LINES_PER_STEP: f32 = 3.;
/// Trackpad pixels per wheel line, for platforms that send pixel deltas (macOS).
const PIXELS_PER_LINE: f32 = 30.;
/// The most volume steps one wheel event makes. Windows scales a notch by the user's lines per
/// notch, and its "one screen at a time" setting turns a notch into billions of lines.
const MAX_STEPS_PER_SCROLL: f32 = 4.;
const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);
const CHAPTER_MARK_H: f32 = 8.;
/// Width of the seek-bar preview thumbnail.
const PREVIEW_W: f32 = 192.;
/// A row of a settings tab's track list.
const TRACK_ROW_H: f32 = 28.;
/// The tracks a list shows before it scrolls.
const TRACK_ROWS: f32 = 5.;

/// The frame on screen.
#[derive(Clone)]
enum Shown {
    /// Software-rendered pixels uploaded as an image.
    Image(Arc<RenderImage>),
    /// A GPU texture shared by the player (Windows, Linux), drawn as a surface.
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    Shared(player::libmpv::SharedFrame),
}

pub struct PlayerPage {
    now_playing: Entity<NowPlaying>,
    focus_handle: FocusHandle,
    frame: Option<Shown>,
    /// `None` until the duration is known.
    seek: Option<Entity<SliderState>>,
    /// The user is dragging the seek bar; don't move it with playback.
    scrubbing: bool,
    volume: Entity<SliderState>,
    /// The user is dragging the volume slider; don't move it with mpv's updates.
    adjusting_volume: bool,
    /// Wheel lines not yet turned into a volume step, so trackpad deltas add up instead of
    /// being ignored.
    scroll_pending: f32,
    /// Whether playback was paused before the last single click on the video, which a double
    /// click restores.
    paused_before_click: bool,
    /// Skip intros and recaps as they start (a setting).
    auto_skip: bool,
    /// The segment last skipped automatically, so seeking back into it doesn't skip again.
    auto_skipped: Option<Duration>,
    /// Created once the item's versions are known, if there's more than one.
    version: Option<ChoiceSelect>,
    /// Seek-bar previews, once the session knows the server has tiles for this version.
    trickplay: Option<Entity<Trickplay>>,
    controls_visible: bool,
    hide_controls: Option<Task<()>>,
    /// The settings menu is open, which keeps the controls up.
    menu_open: bool,
    /// The settings menu's tab, kept while the player is open.
    settings_tab: SettingsTab,
    /// The chapters menu's list, and the settings menu's track list (one tab shows at a time).
    chapter_scroll: ScrollHandle,
    track_scroll: ScrollHandle,
    /// The seek bar's bounds from the last paint, to turn the mouse position into a time.
    seek_bounds: Bounds<Pixels>,
    /// The mouse's x over the seek bar, in window coordinates.
    seek_hover: Option<Pixels>,
    /// Show the time left instead of the time played (a setting, toggled by clicking it).
    show_remaining: bool,
    _subscriptions: Vec<Subscription>,
    _frames: Task<()>,
}

impl PlayerPage {
    pub fn new(now_playing: Entity<NowPlaying>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let initial_volume = now_playing.read(cx).volume as f32;
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(initial_volume)
        });

        let subscriptions = vec![
            cx.observe_in(&now_playing, window, |this, _, window, cx| {
                this.sync_seek_bar(window, cx);
                this.sync_volume(window, cx);
                this.ensure_version_picker(window, cx);
                this.ensure_trickplay(cx);
                this.auto_skip(cx);
                cx.notify();
            }),
            cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| {
                // Applied while dragging, so the change is audible right away.
                let value = match event {
                    SliderEvent::Change(value) => {
                        this.adjusting_volume = true;
                        value.start()
                    }
                    SliderEvent::Release(value) => {
                        this.adjusting_volume = false;
                        value.start()
                    }
                };
                this.send(UiCommand::SetVolume(value as f64), cx);
            }),
        ];
        let mut frames = now_playing.read(cx).video.frames();
        let _frames = cx.spawn_in(window, async move |this, cx| {
            while frames.changed().await.is_ok() {
                let Some(frame) = frames.borrow_and_update().clone() else {
                    continue;
                };
                let shown = match frame {
                    Frame::Pixels(frame) => {
                        let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels.clone())
                        else {
                            continue;
                        };
                        Shown::Image(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                            buffer
                        )])))
                    }
                    // Already on the GPU: just repaint with the new texture.
                    Frame::Shared(frame) => Shown::Shared(frame),
                };
                let updated = this.update_in(cx, |this, window, cx| {
                    // Each software frame is a new texture; free the previous one from GPUI's
                    // atlas.
                    if let Some(Shown::Image(old)) = this.frame.replace(shown) {
                        let _ = window.drop_image(old);
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            now_playing,
            focus_handle,
            frame: None,
            seek: None,
            scrubbing: false,
            volume,
            adjusting_volume: false,
            scroll_pending: 0.,
            paused_before_click: false,
            auto_skip: crate::config::Config::load().auto_skip_intro.unwrap_or(false),
            auto_skipped: None,
            version: None,
            trickplay: None,
            controls_visible: true,
            hide_controls: None,
            menu_open: false,
            settings_tab: SettingsTab::Audio,
            chapter_scroll: ScrollHandle::new(),
            track_scroll: ScrollHandle::new(),
            seek_bounds: Bounds::default(),
            seek_hover: None,
            show_remaining: crate::config::Config::load().show_remaining_time.unwrap_or(false),
            _subscriptions: subscriptions,
            _frames,
        };
        this.show_controls(window, cx);
        this
    }

    /// A control: acts on the player, or asks the SyncPlay group while one is in charge.
    fn send(&self, command: UiCommand, cx: &mut App) {
        self.now_playing.update(cx, |state, cx| state.control(command, cx));
    }

    /// Creates the seek bar once the duration is known, then follows playback unless the user
    /// is dragging it.
    fn sync_seek_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (position, duration) = {
            let state = self.now_playing.read(cx);
            (state.position.as_secs_f64() as f32, state.duration)
        };
        let Some(duration) = duration.filter(|d| !d.is_zero()) else {
            return;
        };
        match &self.seek {
            Some(seek) if !self.scrubbing => seek.update(cx, |seek, cx| seek.set_value(position, window, cx)),
            Some(_) => {}
            None => {
                let seek = cx.new(|_| {
                    SliderState::new()
                        .min(0.)
                        .max(duration.as_secs_f64() as f32)
                        .step(1.)
                        .default_value(position)
                });
                self._subscriptions
                    .push(cx.subscribe(&seek, |this, _, event: &SliderEvent, cx| match event {
                        SliderEvent::Change(_) => this.scrubbing = true,
                        SliderEvent::Release(value) => {
                            this.scrubbing = false;
                            this.send(
                                UiCommand::SeekTo(Duration::from_secs_f64(value.start().max(0.) as f64)),
                                cx,
                            );
                        }
                    }));
                self.seek = Some(seek);
            }
        }
    }

    fn sync_volume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adjusting_volume {
            return;
        }
        let volume = self.now_playing.read(cx).volume as f32;
        if self.volume.read(cx).value().start() != volume {
            self.volume
                .update(cx, |slider, cx| slider.set_value(volume, window, cx));
        }
    }

    /// The version picker, once the session knows the item's versions.
    fn ensure_version_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if self.version.is_some() || state.versions.len() < 2 {
            return;
        }
        let choices: Vec<Choice> = state
            .versions
            .iter()
            .enumerate()
            .map(|(i, version)| Choice {
                label: version.name.clone().into(),
                value: i as i32,
            })
            .collect();
        let current = state.versions.iter().position(|v| v.id == state.source.id).unwrap_or(0);
        let select = cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(current)), window, cx));
        self._subscriptions
            .push(cx.subscribe(&select, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    this.switch_version(*index as usize, cx);
                }
            }));
        self.version = Some(select);
    }

    /// The preview thumbnails, once the session knows the playing version's tiles. The
    /// narrowest set is used: they're shown small.
    fn ensure_trickplay(&mut self, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if self.trickplay.is_some() {
            return;
        }
        let Some(info) = state
            .trickplay
            .get(&state.source.id)
            .and_then(|sets| sets.values().min_by_key(|info| info.width))
            .copied()
        else {
            return;
        };
        let client = PlaybackManager::global(cx).read(cx).client().clone();
        let (item_id, source_id) = (state.item.id.clone(), state.source.id.clone());
        let trickplay = cx.new(|cx| Trickplay::new(client, item_id, source_id, info, cx));
        self._subscriptions.push(cx.observe(&trickplay, |_, _, cx| cx.notify()));
        self.trickplay = Some(trickplay);
    }

    /// Restarts playback of the same item with another version, at the current position.
    fn switch_version(&mut self, index: usize, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        let Some(version) = state.versions.get(index).filter(|v| v.id != state.source.id) else {
            return;
        };
        let options = PlayOptions {
            start: Some(jellyfin::Ticks::from(state.position)),
            media_source_id: Some(version.id.clone()),
            ..Default::default()
        };
        let item = state.item.clone();
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
    }

    /// Enter: skip the intro (or recap, preview…), or play the next episode from the up-next
    /// card.
    fn skip(&mut self, _: &Skip, _: &mut Window, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if let Some(segment) = state.skippable() {
            self.send(UiCommand::SeekTo(segment.end), cx);
        } else if state.show_up_next() {
            self.now_playing.update(cx, |state, cx| state.play_next(cx));
        } else if state.resume_prompt().is_some() {
            self.now_playing.update(cx, |state, cx| state.start_over(cx));
        }
    }

    /// With the setting on, skips each intro or recap once when it starts. Not in a SyncPlay
    /// group, where every member would ask the group to seek.
    fn auto_skip(&mut self, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if !self.auto_skip || state.group_item.is_some() {
            return;
        }
        if let Some(segment) = state.skippable()
            && matches!(segment.kind, SegmentKind::Intro | SegmentKind::Recap)
            && self.auto_skipped != Some(segment.start)
        {
            self.auto_skipped = Some(segment.start);
            self.send(UiCommand::SeekTo(segment.end), cx);
        }
    }

    fn cancel_up_next(&mut self, cx: &mut Context<Self>) {
        self.now_playing.update(cx, |state, cx| {
            state.next_cancelled = true;
            cx.notify();
        });
    }

    fn next_chapter(&mut self, _: &NextChapter, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if let Some(start) = markers::next_chapter(&state.chapters, state.position) {
            self.send(UiCommand::SeekTo(start), cx);
        }
        self.show_controls(window, cx);
    }

    fn prev_chapter(&mut self, _: &PrevChapter, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.now_playing.read(cx);
        if let Some(start) = markers::prev_chapter(&state.chapters, state.position) {
            self.send(UiCommand::SeekTo(start), cx);
        }
        self.show_controls(window, cx);
    }

    /// Shift+N: the next episode, collection movie or queued item.
    fn next_episode(&mut self, _: &NextEpisode, _: &mut Window, cx: &mut Context<Self>) {
        self.now_playing.update(cx, |state, cx| state.play_next(cx));
    }

    /// Shift+P: the previous episode or collection movie.
    fn previous_episode(&mut self, _: &PreviousEpisode, _: &mut Window, cx: &mut Context<Self>) {
        self.now_playing.update(cx, |state, cx| state.play_previous(cx));
    }

    fn change_volume(&mut self, delta: f64, window: &mut Window, cx: &mut Context<Self>) {
        let volume = (self.now_playing.read(cx).volume + delta).clamp(0., 100.);
        self.send(UiCommand::SetVolume(volume), cx);
        self.show_controls(window, cx);
    }

    fn volume_up(&mut self, _: &VolumeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.change_volume(VOLUME_STEP, window, cx);
    }

    fn volume_down(&mut self, _: &VolumeDown, window: &mut Window, cx: &mut Context<Self>) {
        self.change_volume(-VOLUME_STEP, window, cx);
    }

    fn toggle_mute(&mut self, _: &ToggleMute, window: &mut Window, cx: &mut Context<Self>) {
        self.send(UiCommand::ToggleMute, cx);
        self.show_controls(window, cx);
    }

    /// Shows the controls and hides them again after a few seconds of no mouse movement,
    /// unless paused.
    fn show_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.controls_visible = true;
        cx.notify();
        self.hide_controls = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(HIDE_CONTROLS_AFTER).await;
            this.update(cx, |this, cx| {
                if !this.now_playing.read(cx).paused && !this.menu_open {
                    this.controls_visible = false;
                    cx.hide_cursor_until_mouse_moves();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn toggle_pause(&mut self, _: &TogglePause, window: &mut Window, cx: &mut Context<Self>) {
        self.send(UiCommand::TogglePause, cx);
        self.show_controls(window, cx);
    }

    /// Clicking the video toggles pause. A double click toggles fullscreen; its first click
    /// already toggled pause, so the second restores the state from before it. Not a second
    /// toggle: in a SyncPlay group the first pause hasn't come back from the server yet, so
    /// both toggles would ask the group to pause.
    fn click_video(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.click_count() {
            1 => {
                self.paused_before_click = self.now_playing.read(cx).paused;
                self.toggle_pause(&TogglePause, window, cx);
            }
            2 => {
                self.send(UiCommand::SetPause(self.paused_before_click), cx);
                self.toggle_fullscreen(&ToggleFullscreen, window, cx);
                self.show_controls(window, cx);
            }
            _ => {}
        }
    }

    /// A wheel notch over the video changes the volume by one step; trackpad deltas add up
    /// until they make a whole notch.
    fn scroll_volume(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => delta.y.as_f32() / PIXELS_PER_LINE,
        };
        let (pending, steps) = scroll_steps(self.scroll_pending, lines);
        self.scroll_pending = pending;
        if steps != 0 {
            self.change_volume(f64::from(steps) * VOLUME_STEP, window, cx);
        }
    }

    fn seek_forward(&mut self, _: &SeekForward, window: &mut Window, cx: &mut Context<Self>) {
        self.send(UiCommand::SeekBy(SEEK_STEP), cx);
        self.show_controls(window, cx);
    }

    fn seek_backward(&mut self, _: &SeekBackward, window: &mut Window, cx: &mut Context<Self>) {
        self.send(UiCommand::SeekBy(-SEEK_STEP), cx);
        self.show_controls(window, cx);
    }

    fn adjust(&mut self, change: impl FnOnce(&mut Adjustments), window: &mut Window, cx: &mut Context<Self>) {
        let mut adjustments = self.now_playing.read(cx).adjustments;
        change(&mut adjustments);
        self.now_playing.update(cx, |state, cx| state.adjust(adjustments, cx));
        self.show_controls(window, cx);
    }

    fn speed_up(&mut self, _: &SpeedUp, window: &mut Window, cx: &mut Context<Self>) {
        self.adjust(|a| a.speed = a.step_speed(true), window, cx);
    }

    fn speed_down(&mut self, _: &SpeedDown, window: &mut Window, cx: &mut Context<Self>) {
        self.adjust(|a| a.speed = a.step_speed(false), window, cx);
    }

    fn subtitle_delay_up(&mut self, _: &SubtitleDelayUp, window: &mut Window, cx: &mut Context<Self>) {
        self.adjust(|a| a.subtitle_delay += Adjustments::DELAY_STEP, window, cx);
    }

    fn subtitle_delay_down(&mut self, _: &SubtitleDelayDown, window: &mut Window, cx: &mut Context<Self>) {
        self.adjust(|a| a.subtitle_delay -= Adjustments::DELAY_STEP, window, cx);
    }

    /// Switches the clock between the time played and the time left, and remembers the choice.
    fn toggle_remaining(&mut self, cx: &mut Context<Self>) {
        self.show_remaining = !self.show_remaining;
        let mut config = crate::config::Config::load();
        config.show_remaining_time = Some(self.show_remaining);
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
        cx.notify();
    }

    fn toggle_fullscreen(&mut self, _: &ToggleFullscreen, window: &mut Window, _: &mut Context<Self>) {
        window.toggle_fullscreen();
    }

    /// Esc: leave fullscreen first, then dismiss the up-next card, then stop playback (which
    /// closes the player).
    fn exit_player(&mut self, _: &ExitPlayer, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() {
            window.toggle_fullscreen();
        } else if self.now_playing.read(cx).show_up_next() {
            self.cancel_up_next(cx);
        } else {
            self.send(UiCommand::Stop, cx);
        }
    }

    /// Where the mouse is over the seek bar (from its left edge) and the time there.
    fn seek_hover(&self, cx: &App) -> Option<(Pixels, Duration)> {
        let (x, duration) = (self.seek_hover?, self.now_playing.read(cx).duration?);
        let width = self.seek_bounds.size.width;
        if width <= px(0.) {
            return None;
        }
        let offset = (x - self.seek_bounds.left()).clamp(px(0.), width);
        Some((offset, duration.mul_f32(offset / width)))
    }

    fn render_controls(&self, fullscreen: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = self.seek_hover(cx);
        let preview = match (hover, &self.trickplay) {
            (Some((_, time)), Some(trickplay)) => trickplay
                .update(cx, |trickplay, cx| trickplay.thumbnail(time, cx))
                .map(|image| (image, trickplay.read(cx).aspect_ratio())),
            _ => None,
        };
        let state = self.now_playing.read(cx);
        let paused = state.paused;
        let muted = state.muted || state.volume == 0.;
        let previous_tip = state
            .previous
            .as_ref()
            .map(|item| format!("Previous {}(Shift+P)", kind_word(item)));
        let next_tip = state
            .next
            .as_ref()
            .map(|item| format!("Next {}(Shift+N)", kind_word(item)));
        let mut time = match state.duration {
            Some(duration) if self.show_remaining => format!(
                "-{} / {}",
                timestamp(duration.saturating_sub(state.position)),
                timestamp(duration)
            ),
            duration => format!(
                "{} / {}",
                timestamp(state.position),
                duration.map(timestamp).unwrap_or_else(|| "--:--".into())
            ),
        };
        if let Some(name) = markers::current_chapter(&state.chapters, state.position).and_then(|c| c.name.as_ref()) {
            time = format!("{time} · {name}");
        }
        // Chapter starts as fractions of the seek bar.
        let chapter_marks: Vec<f32> = match state.duration.filter(|d| !d.is_zero()) {
            Some(duration) => state
                .chapters
                .iter()
                .filter(|c| !c.start.is_zero() && c.start < duration)
                .map(|c| c.start.as_secs_f32() / duration.as_secs_f32())
                .collect(),
            None => Vec::new(),
        };
        let speed = state.adjustments.speed;
        let title = crate::components::card_text(&state.item);
        let title = if state.item.kind == jellyfin::models::BaseItemKind::Episode {
            format!("{} · {}", title.0, title.1)
        } else {
            title.0
        };

        // Both bars occlude the mouse, so clicks on them don't reach the video below, whose
        // click toggles pause.
        let top = h_flex()
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .p_4()
            .gap_3()
            .bg(linear_gradient(
                180.,
                linear_color_stop(black().opacity(0.7), 0.),
                linear_color_stop(black().opacity(0.), 1.),
            ))
            .child(
                Button::new("exit-player")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .tooltip("Back (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| this.exit_player(&ExitPlayer, window, cx))),
            )
            .child(div().flex_1().text_lg().text_color(white()).truncate().child(title))
            .when(state.group_item.is_some(), |el| el.child(badge("SyncPlay")))
            .when(state.is_transcoding(), |el| el.child(badge("Transcoding")));

        let bottom = v_flex()
            .occlude()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px_4()
            .pb_4()
            .pt_8()
            .gap_2()
            .bg(linear_gradient(
                0.,
                linear_color_stop(black().opacity(0.8), 0.),
                linear_color_stop(black().opacity(0.), 1.),
            ))
            .children(self.seek.as_ref().map(|seek| {
                let page = cx.entity();
                div()
                    .id("seek-bar")
                    .relative()
                    .w_full()
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.seek_hover = Some(event.position.x);
                        cx.notify();
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !hovered {
                            this.seek_hover = None;
                            cx.notify();
                        }
                    }))
                    .child(
                        canvas(
                            move |bounds, _, cx| page.update(cx, |this, _| this.seek_bounds = bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(Slider::new(seek).w_full())
                    .children(chapter_marks.into_iter().map(|fraction| {
                        div()
                            .absolute()
                            .top(relative(0.5))
                            .mt(dp(-CHAPTER_MARK_H / 2.))
                            .left(relative(fraction))
                            .w(dp(2.))
                            .h(dp(CHAPTER_MARK_H))
                            .bg(white().opacity(0.6))
                    }))
                    .when_some(hover, |el, (offset, time)| {
                        // Centred on the mouse, but kept over the bar.
                        let half = if preview.is_some() { PREVIEW_W / 2. } else { 24. };
                        let max = (self.seek_bounds.size.width - px(half)).max(px(half));
                        let left = offset.clamp(px(half), max);
                        el.child(
                            v_flex()
                                .absolute()
                                .bottom(px(18.))
                                .left(left)
                                .ml(px(-half))
                                .w(px(half * 2.))
                                .items_center()
                                .gap_1()
                                .when_some(preview.clone(), |el, (image, aspect_ratio)| {
                                    el.child(
                                        img(ImageSource::Render(image))
                                            .w(px(PREVIEW_W))
                                            .h(px(PREVIEW_W / aspect_ratio))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(white().opacity(0.6)),
                                    )
                                })
                                .child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .bg(black().opacity(0.8))
                                        .text_xs()
                                        .text_color(white())
                                        .child(timestamp(time)),
                                ),
                        )
                    })
            }))
            .child(
                h_flex()
                    .gap_1()
                    .text_color(white())
                    .child(
                        Button::new("play-pause")
                            .ghost()
                            .large()
                            .icon(if paused { Lucide::Play } else { Lucide::Pause })
                            .tooltip(if paused { "Play (Space)" } else { "Pause (Space)" })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_pause(&TogglePause, window, cx))),
                    )
                    .child(
                        Button::new("back-10")
                            .ghost()
                            .icon(Lucide::Rewind)
                            .tooltip("Back 10 s (←)")
                            .on_click(cx.listener(|this, _, window, cx| this.seek_backward(&SeekBackward, window, cx))),
                    )
                    .child(
                        Button::new("forward-10")
                            .ghost()
                            .icon(Lucide::FastForward)
                            .tooltip("Forward 10 s (→)")
                            .on_click(cx.listener(|this, _, window, cx| this.seek_forward(&SeekForward, window, cx))),
                    )
                    .child(
                        Button::new("mute")
                            .ghost()
                            .icon(if muted { Lucide::VolumeX } else { Lucide::Volume2 })
                            .tooltip(if muted { "Unmute (M)" } else { "Mute (M)" })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_mute(&ToggleMute, window, cx))),
                    )
                    .child(div().flex_none().w(dp(96.)).child(Slider::new(&self.volume).w_full()))
                    .child(
                        div()
                            .id("time")
                            .ml_3()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_remaining(cx)))
                            .child(time),
                    )
                    .when(speed != 1., |el| el.child(badge(format!("{speed}×"))))
                    .child(div().flex_1())
                    .when_some(previous_tip, |el, tip| {
                        el.child(
                            Button::new("previous-item")
                                .ghost()
                                .icon(Lucide::SkipBack)
                                .tooltip(tip)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.previous_episode(&PreviousEpisode, window, cx)
                                })),
                        )
                    })
                    .when_some(next_tip, |el, tip| {
                        el.child(
                            Button::new("next-item")
                                .ghost()
                                .icon(Lucide::SkipForward)
                                .tooltip(tip)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.next_episode(&NextEpisode, window, cx)),
                                ),
                        )
                    })
                    .children(self.version.as_ref().map(|s| picker(s, 140., 240.)))
                    .children(self.render_chapters(cx))
                    .child(self.render_settings(cx))
                    .child(
                        Button::new("fullscreen")
                            .ghost()
                            .icon(if fullscreen { Lucide::Minimize } else { Lucide::Maximize })
                            .tooltip(if fullscreen {
                                "Exit fullscreen (F)"
                            } else {
                                "Fullscreen (F)"
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.toggle_fullscreen(&ToggleFullscreen, window, cx)
                                }),
                            ),
                    ),
            );

        div().absolute().inset_0().child(top).child(bottom)
    }

    /// The chapter list: each chapter's image (when the server extracted them), name and start,
    /// the current one highlighted. A click seeks there. (PgUp and PgDn step through them.)
    fn render_chapters(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.now_playing.read(cx).chapters.len() < 2 {
            return None;
        }
        let page = cx.entity();
        Some(
            Popover::new("player-chapters")
                .anchor(Anchor::BottomRight)
                .on_open_change({
                    let page = page.clone();
                    move |open, window, cx| {
                        let open = *open;
                        page.update(cx, |this, cx| {
                            this.menu_open = open;
                            this.show_controls(window, cx);
                        });
                    }
                })
                .trigger(
                    Button::new("chapters")
                        .ghost()
                        .icon(Lucide::ListVideo)
                        .tooltip("Chapters"),
                )
                .content(move |_, _, cx| chapters_panel(&page, cx)),
        )
    }

    /// The settings menu: speed, subtitle and audio delay, subtitle size.
    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = cx.entity();
        Popover::new("player-settings")
            .anchor(Anchor::BottomRight)
            .on_open_change({
                let page = page.clone();
                move |open, window, cx| {
                    let open = *open;
                    page.update(cx, |this, cx| {
                        this.menu_open = open;
                        this.show_controls(window, cx);
                    });
                }
            })
            .trigger(
                Button::new("settings")
                    .ghost()
                    .icon(Lucide::Settings2)
                    .tooltip("Playback settings"),
            )
            .content(move |_, _, cx| settings_panel(&page, cx))
    }

    /// The skip button or the up-next card, bottom right. Shown with or without the controls
    /// (above them when they're up).
    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let state = self.now_playing.read(cx);
        let (kind, prompt) = if let Some(segment) = state.skippable() {
            let skip = Button::new("skip-segment")
                .large()
                .icon(Lucide::SkipForward)
                .label(segment.kind.skip_label())
                .on_click(cx.listener(|this, _, window, cx| this.skip(&Skip, window, cx)));
            ("skip", skip.into_any_element())
        } else if state.show_up_next() {
            let next = state.next.as_ref()?;
            let (series, episode) = crate::components::card_text(next);
            let label = if next.kind == jellyfin::models::BaseItemKind::Episode {
                episode
            } else {
                series
            };
            let remaining = state.duration.map(|d| d.saturating_sub(state.position));
            let card = v_flex()
                .w(dp(320.))
                .p_3()
                .gap_2()
                .rounded_lg()
                .bg(black().opacity(0.8))
                .text_color(white())
                .child(div().text_sm().text_color(white().opacity(0.7)).child("Up next"))
                .child(div().truncate().child(label))
                .when_some(remaining.filter(|_| state.autoplay), |el, remaining| {
                    el.child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.7))
                            .child(format!("Starts in {} s", remaining.as_secs())),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("play-next")
                                .primary()
                                .icon(Lucide::Play)
                                .label("Play now")
                                .on_click(cx.listener(|this, _, window, cx| this.skip(&Skip, window, cx))),
                        )
                        .child(
                            Button::new("cancel-next")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_up_next(cx))),
                        ),
                );
            ("up-next", card.into_any_element())
        } else {
            let from = state.resume_prompt()?;
            let card = h_flex()
                .p_3()
                .gap_3()
                .rounded_lg()
                .bg(black().opacity(0.8))
                .text_color(white())
                .child(div().text_sm().child(format!("Resumed from {}", timestamp(from))))
                .child(
                    Button::new("start-over")
                        .primary()
                        .icon(Lucide::RotateCcw)
                        .label("Start over")
                        .on_click(cx.listener(|this, _, window, cx| this.skip(&Skip, window, cx))),
                );
            ("resume", card.into_any_element())
        };
        let bottom = if self.controls_visible { 120. } else { 32. };
        Some(
            div()
                .occlude()
                .absolute()
                .right(dp(24.))
                .bottom(dp(bottom))
                // Each kind slides in when it appears.
                .child(motion::slide_in(div().child(prompt), kind)),
        )
    }
}

/// "episode " or "movie " for a button's tooltip, or nothing for other kinds.
fn kind_word(item: &BaseItemDto) -> &'static str {
    match item.kind {
        BaseItemKind::Episode => "episode ",
        BaseItemKind::Movie => "movie ",
        _ => "",
    }
}

/// The settings menu's rows. Each button changes one adjustment through the page.
const CHAPTER_THUMB_W: f32 = 128.;
const CHAPTER_THUMB_H: f32 = 72.;

fn chapters_panel(page: &Entity<PlayerPage>, cx: &mut Context<PopoverState>) -> AnyElement {
    let scroll = page.read(cx).chapter_scroll.clone();
    let popover = cx.entity();
    let state = page.read(cx).now_playing.read(cx);
    let item_id = state.item.id.clone();
    let current = markers::current_chapter(&state.chapters, state.position).map(|c| c.index);
    let chapters = state.chapters.clone();
    let rows: Vec<AnyElement> = chapters
        .iter()
        .map(|chapter| {
            let image = chapter.image_tag.clone().map(|tag| ImageRequest {
                index: Some(chapter.index as u32),
                ..ImageRequest::new(&item_id, ImageType::Chapter, Some(tag)).fill_width((CHAPTER_THUMB_W * 2.) as u32)
            });
            let name = chapter
                .name
                .clone()
                .unwrap_or_else(|| format!("Chapter {}", chapter.index + 1));
            let (page, popover, start) = (page.clone(), popover.clone(), chapter.start);
            h_flex()
                .id(("chapter", chapter.index))
                .gap_3()
                .p_1()
                .rounded_md()
                .cursor_pointer()
                .when(current == Some(chapter.index), |el| el.bg(cx.theme().accent))
                .hover(|style| style.bg(cx.theme().muted))
                .on_click(move |_, window, cx| {
                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    page.update(cx, |this, cx| this.send(UiCommand::SeekTo(start), cx));
                })
                .child(
                    div()
                        .flex_none()
                        .w(dp(CHAPTER_THUMB_W))
                        .h(dp(CHAPTER_THUMB_H))
                        .rounded_sm()
                        .overflow_hidden()
                        .bg(cx.theme().muted)
                        .child(remote_image(image, None, cx)),
                )
                .child(
                    v_flex().min_w_0().child(div().text_sm().truncate().child(name)).child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(timestamp(chapter.start)),
                    ),
                )
                .into_any_element()
        })
        .collect();
    v_flex()
        .id("chapter-list")
        .w(dp(340.))
        .max_h(dp(420.))
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .on_scroll_wheel(smooth_wheel("chapter-list", &scroll))
        .gap_1()
        .children(rows)
        .into_any_element()
}

fn settings_panel(page: &Entity<PlayerPage>, cx: &mut App) -> AnyElement {
    let state = page.read(cx).now_playing.read(cx);
    let adjustments = state.adjustments;
    let in_group = state.group_item.is_some();
    let button = |id: &'static str, label: String, change: Box<dyn Fn(&mut Adjustments)>| {
        let page = page.clone();
        Button::new(id)
            .small()
            .ghost()
            .label(label)
            .on_click(move |_, window, cx| {
                page.update(cx, |this, cx| this.adjust(|a| change(a), window, cx));
            })
    };
    let stepper = |name: &'static str, value: String, down: Button, up: Button, reset: Button| {
        h_flex()
            .gap_1()
            .child(div().w(dp(110.)).text_sm().child(name))
            .child(down)
            .child(div().w(dp(56.)).text_sm().text_center().child(value))
            .child(up)
            .child(reset)
    };
    let delay = |seconds: f64| {
        if seconds.abs() < 0.05 {
            "0.0 s".to_string()
        } else {
            format!("{seconds:+.1} s")
        }
    };

    let speed_row = h_flex()
        .gap_1()
        .child(div().w(dp(110.)).text_sm().child("Speed"))
        .children(if in_group {
            vec![
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Normal in SyncPlay")
                    .into_any_element(),
            ]
        } else {
            Adjustments::SPEEDS
                .into_iter()
                .enumerate()
                .map(|(i, speed)| {
                    let page = page.clone();
                    Button::new(("speed", i))
                        .small()
                        .label(format!("{speed}×"))
                        .when(speed == adjustments.speed, |b| b.primary())
                        .when(speed != adjustments.speed, |b| b.ghost())
                        .on_click(move |_, window, cx| {
                            page.update(cx, |this, cx| this.adjust(|a| a.speed = speed, window, cx));
                        })
                        .into_any_element()
                })
                .collect()
        });
    let tab = page.read(cx).settings_tab;
    // From the playing version every time, so the lists follow a version switch, and the
    // selection follows track changes made from a remote.
    let (audio_choices, subtitle_choices) = track_choices(&state.source);
    let audio_selected = state.audio.or(state.source.default_audio_stream_index);
    let subtitle_selected = match state.subtitle {
        SubtitleChoice::Stream(index) => index,
        SubtitleChoice::Off => SUBTITLES_OFF,
        SubtitleChoice::Auto => state.source.default_subtitle_stream_index.unwrap_or(SUBTITLES_OFF),
    };
    let track_scroll = page.read(cx).track_scroll.clone();
    let audio_list = {
        let page = page.clone();
        track_list(
            "audio-tracks",
            "No audio tracks",
            &audio_choices,
            audio_selected,
            &track_scroll,
            cx,
            move |value, cx| {
                page.update(cx, |this, cx| {
                    this.now_playing.update(cx, |state, cx| state.change_audio(value, cx));
                });
            },
        )
    };
    let subtitle_list = {
        let page = page.clone();
        track_list(
            "subtitle-tracks",
            "No subtitles",
            &subtitle_choices,
            Some(subtitle_selected),
            &track_scroll,
            cx,
            move |value, cx| {
                let choice = match value {
                    SUBTITLES_OFF => SubtitleChoice::Off,
                    index => SubtitleChoice::Stream(index),
                };
                page.update(cx, |this, cx| {
                    this.now_playing
                        .update(cx, |state, cx| state.change_subtitle(choice, cx));
                });
            },
        )
    };
    let content: Vec<AnyElement> = match tab {
        SettingsTab::Audio => vec![
            section_label("Track", cx),
            audio_list,
            stepper(
                "Audio delay",
                delay(adjustments.audio_delay),
                button(
                    "audio-delay-down",
                    "−0.1".into(),
                    Box::new(|a| a.audio_delay -= Adjustments::DELAY_STEP),
                ),
                button(
                    "audio-delay-up",
                    "+0.1".into(),
                    Box::new(|a| a.audio_delay += Adjustments::DELAY_STEP),
                ),
                button("audio-delay-reset", "Reset".into(), Box::new(|a| a.audio_delay = 0.)),
            )
            .into_any_element(),
        ],
        SettingsTab::Subtitles => vec![
            section_label("Track", cx),
            subtitle_list,
            stepper(
                "Subtitle delay",
                delay(adjustments.subtitle_delay),
                button(
                    "sub-delay-down",
                    "−0.1".into(),
                    Box::new(|a| a.subtitle_delay -= Adjustments::DELAY_STEP),
                ),
                button(
                    "sub-delay-up",
                    "+0.1".into(),
                    Box::new(|a| a.subtitle_delay += Adjustments::DELAY_STEP),
                ),
                button("sub-delay-reset", "Reset".into(), Box::new(|a| a.subtitle_delay = 0.)),
            )
            .into_any_element(),
            stepper(
                "Subtitle size",
                format!("{:.0}%", adjustments.subtitle_scale * 100.),
                button(
                    "sub-size-down",
                    "−".into(),
                    Box::new(|a| a.subtitle_scale -= Adjustments::SCALE_STEP),
                ),
                button(
                    "sub-size-up",
                    "+".into(),
                    Box::new(|a| a.subtitle_scale += Adjustments::SCALE_STEP),
                ),
                button("sub-size-reset", "Reset".into(), Box::new(|a| a.subtitle_scale = 1.)),
            )
            .into_any_element(),
        ],
        SettingsTab::General => vec![speed_row.into_any_element()],
    };
    let hint = match tab {
        SettingsTab::Audio => None,
        SettingsTab::Subtitles => Some("Keys: z and x change the subtitle delay."),
        SettingsTab::General => Some("Keys: [ and ] change the speed."),
    };
    let tabs_page = page.clone();
    v_flex()
        .p_1()
        .gap_3()
        .min_w(dp(380.))
        .child(
            TabBar::new("player-settings-tabs")
                .segmented()
                .small()
                .selected_index(tab as usize)
                .children(SettingsTab::ALL.map(|tab| Tab::new().label(tab.label())))
                .on_click(move |index, _, cx| {
                    tabs_page.update(cx, |this, cx| {
                        this.settings_tab = SettingsTab::ALL[*index];
                        cx.notify();
                    });
                }),
        )
        .child(v_flex().gap_2().children(content))
        .when_some(hint, |el, hint| {
            el.child(div().text_xs().text_color(cx.theme().muted_foreground).child(hint))
        })
        .into_any_element()
}

/// A small heading inside a settings tab.
fn section_label(text: &'static str, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// A settings tab's list of tracks to pick from, the selected one checked. A plain list rather
/// than a select: a select's menu would count as a click outside the settings popover and
/// close it.
fn track_list(
    id: &'static str,
    empty: &'static str,
    choices: &[Choice],
    selected: Option<i32>,
    scroll: &ScrollHandle,
    cx: &App,
    choose: impl Fn(i32, &mut App) + 'static,
) -> AnyElement {
    if choices.is_empty() {
        return div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(empty)
            .into_any_element();
    }
    // Without a track saved or a default, the first one plays.
    let selected = selected
        .filter(|value| choices.iter().any(|c| c.value == *value))
        .unwrap_or(choices[0].value);
    let choose = Rc::new(choose);
    let hover = cx.theme().accent;
    let list = v_flex()
        .id(id)
        .max_h(dp(TRACK_ROW_H * TRACK_ROWS))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .on_scroll_wheel(smooth_wheel("track-list", scroll))
        .children(choices.iter().enumerate().map(|(i, choice)| {
            let value = choice.value;
            let choose = choose.clone();
            h_flex()
                .id((id, i))
                .flex_none()
                .h(dp(TRACK_ROW_H))
                .gap_2()
                .px_2()
                .rounded_md()
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, _, cx| choose(value, cx))
                .child(
                    div()
                        .flex_none()
                        .size_4()
                        .when(value == selected, |el| el.child(Icon::new(IconName::Check).small())),
                )
                .child(div().text_sm().truncate().child(choice.label.clone()))
        }));
    div()
        .relative()
        // Sets the track apart from the delays and sizes below it.
        .mb_2()
        .child(list)
        .vertical_scrollbar(scroll)
        .into_any_element()
}

/// The tabs of the player's settings menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Audio,
    Subtitles,
    General,
}

impl SettingsTab {
    const ALL: [Self; 3] = [Self::Audio, Self::Subtitles, Self::General];

    fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Subtitles => "Subtitles",
            Self::General => "General",
        }
    }
}

/// A small label in the player's bars.
fn badge(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_none()
        .px_2()
        .rounded_md()
        .bg(black().opacity(0.5))
        .text_sm()
        .text_color(white().opacity(0.8))
        .child(text.into())
}

/// A fixed-width track or version picker for the control bar.
fn picker(select: &ChoiceSelect, width: f32, menu_width: f32) -> impl IntoElement {
    div()
        .flex_none()
        .w(dp(width))
        .child(Select::new(select).small().w_full().menu_width(dp(menu_width)))
}

impl Focusable for PlayerPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PlayerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Render frames at the size they're shown, in device pixels.
        let viewport = window.viewport_size();
        let fullscreen = window.is_fullscreen();
        let scale = window.scale_factor();
        self.now_playing.read(cx).video.set_target_size(
            (f32::from(viewport.width) * scale) as u32,
            (f32::from(viewport.height) * scale) as u32,
        );
        let loading = self.now_playing.read(cx).loading || self.frame.is_none();

        div()
            .id("player")
            .relative()
            .size_full()
            .bg(black())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_pause))
            .on_action(cx.listener(Self::seek_forward))
            .on_action(cx.listener(Self::seek_backward))
            .on_action(cx.listener(Self::volume_up))
            .on_action(cx.listener(Self::volume_down))
            .on_action(cx.listener(Self::toggle_mute))
            .on_action(cx.listener(Self::next_episode))
            .on_action(cx.listener(Self::previous_episode))
            .on_action(cx.listener(Self::next_chapter))
            .on_action(cx.listener(Self::prev_chapter))
            .on_action(cx.listener(Self::skip))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::exit_player))
            .on_action(cx.listener(Self::speed_up))
            .on_action(cx.listener(Self::speed_down))
            .on_action(cx.listener(Self::subtitle_delay_up))
            .on_action(cx.listener(Self::subtitle_delay_down))
            .on_mouse_move(cx.listener(|this, _, window, cx| this.show_controls(window, cx)))
            .children(self.frame.clone().map(|frame| {
                div()
                    .id("video")
                    .absolute()
                    .inset_0()
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| this.click_video(event, window, cx)))
                    .on_scroll_wheel(
                        cx.listener(|this, event: &ScrollWheelEvent, window, cx| this.scroll_volume(event, window, cx)),
                    )
                    .child(match frame {
                        Shown::Image(image) => img(ImageSource::Render(image))
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        #[cfg(any(windows, target_os = "linux"))]
                        Shown::Shared(texture) => surface(shared_texture(texture))
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        #[cfg(not(any(windows, target_os = "linux")))]
                        Shown::Shared(_) => div().into_any_element(),
                    })
            }))
            .when(loading, |el| {
                el.child(
                    v_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(Spinner::new().large()),
                )
            })
            .when(self.controls_visible, |el| {
                el.child(motion::fade_in(
                    div().absolute().inset_0().child(self.render_controls(fullscreen, cx)),
                    "player-controls",
                ))
            })
            .children(self.render_prompt(cx))
    }
}

#[cfg(windows)]
fn shared_texture(frame: player::libmpv::SharedFrame) -> SharedTexture {
    SharedTexture {
        handle: frame.handle,
        width: frame.width,
        height: frame.height,
    }
}

#[cfg(target_os = "linux")]
fn shared_texture(frame: player::libmpv::SharedFrame) -> SharedTexture {
    SharedTexture {
        id: frame.id,
        fd: frame.fd,
        allocation_size: frame.allocation_size,
        width: frame.width,
        height: frame.height,
    }
}

/// Turns a wheel delta (in lines, positive is scrolling up) into whole volume steps, keeping
/// the leftover lines for the next event. One event makes at most `MAX_STEPS_PER_SCROLL`.
fn scroll_steps(pending: f32, lines: f32) -> (f32, i32) {
    let limit = MAX_STEPS_PER_SCROLL * SCROLL_LINES_PER_STEP;
    let total = (pending + lines).clamp(-limit, limit);
    let steps = (total / SCROLL_LINES_PER_STEP).trunc() as i32;
    (total - steps as f32 * SCROLL_LINES_PER_STEP, steps)
}

#[cfg(test)]
mod tests {
    use super::{MAX_STEPS_PER_SCROLL, SCROLL_LINES_PER_STEP, scroll_steps};

    #[test]
    fn a_wheel_notch_is_one_volume_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP);
        assert_eq!(steps, 1);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_fast_flick_keeps_every_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP * 4.);
        assert_eq!(steps, 4);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn small_deltas_add_up() {
        let (pending, steps) = scroll_steps(0., 1.);
        assert_eq!((pending, steps), (1., 0));
        let (pending, steps) = scroll_steps(pending, 2.);
        assert_eq!((pending, steps), (0., 1));
    }

    #[test]
    fn scrolling_down_is_negative() {
        let (pending, steps) = scroll_steps(0., -2. * SCROLL_LINES_PER_STEP);
        assert_eq!(steps, -2);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_huge_delta_is_capped() {
        let (pending, steps) = scroll_steps(0., 4_294_967_295.);
        assert_eq!(steps, MAX_STEPS_PER_SCROLL as i32);
        assert_eq!(pending, 0.);
        let (_, steps) = scroll_steps(0., -4_294_967_295.);
        assert_eq!(steps, -(MAX_STEPS_PER_SCROLL as i32));
    }

    #[test]
    fn scrolling_back_cancels() {
        let (pending, steps) = scroll_steps(0., -1.5);
        assert_eq!(steps, 0);
        let (_, steps) = scroll_steps(pending, 1.5);
        assert_eq!(steps, 0);
    }
}
