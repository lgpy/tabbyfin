//! Full-window player for embedded mpv: the video plus GPUI controls on top.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use player::libmpv::Frame;

use crate::components::timestamp;
use crate::markers::{self, SegmentKind};
use crate::playback::{NowPlaying, PlayOptions, PlaybackManager, SubtitleChoice, UiCommand};
use crate::tracks::{Choice, ChoiceSelect, SUBTITLES_OFF, track_pickers};

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
        NextChapter,
        PrevChapter,
        Skip,
        ToggleFullscreen,
        ExitPlayer
    ]
);

pub const KEY_CONTEXT: &str = "Player";
const SEEK_STEP: f64 = 10.;
const VOLUME_STEP: f64 = 5.;
/// Wheel lines per volume step. X11 and Windows normalise a wheel notch to three lines, so
/// one notch is one step; a trackpad's pixel deltas are turned into lines first.
const SCROLL_LINES_PER_STEP: f32 = 3.;
/// Trackpad pixels per wheel line, for platforms that send pixel deltas (macOS).
const PIXELS_PER_LINE: f32 = 30.;
const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);
const CHAPTER_MARK_H: f32 = 8.;

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
    /// Skip intros and recaps as they start (a setting).
    auto_skip: bool,
    /// The segment last skipped automatically, so seeking back into it doesn't skip again.
    auto_skipped: Option<Duration>,
    /// Created once the item's versions are known, if there's more than one.
    version: Option<ChoiceSelect>,
    audio: Option<ChoiceSelect>,
    subtitle: Option<ChoiceSelect>,
    controls_visible: bool,
    hide_controls: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _frames: Task<()>,
}

impl PlayerPage {
    pub fn new(now_playing: Entity<NowPlaying>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let state = now_playing.read(cx);
        let subtitle_index = match state.subtitle {
            SubtitleChoice::Stream(index) => Some(index),
            SubtitleChoice::Off => Some(SUBTITLES_OFF),
            SubtitleChoice::Auto => state.source.default_subtitle_stream_index,
        };
        let source = state.source.clone();
        let initial_volume = state.volume as f32;
        let (audio, subtitle) = track_pickers(&source, state.audio, subtitle_index, window, cx);
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(initial_volume)
        });

        let mut subscriptions = vec![
            cx.observe_in(&now_playing, window, |this, _, window, cx| {
                this.sync_seek_bar(window, cx);
                this.sync_volume(window, cx);
                this.ensure_version_picker(window, cx);
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
        if let Some(select) = &audio {
            subscriptions.push(cx.subscribe(select, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    let index = *index;
                    this.now_playing.update(cx, |state, cx| state.change_audio(index, cx));
                }
            }));
        }
        if let Some(select) = &subtitle {
            subscriptions.push(cx.subscribe(select, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    let choice = match *index {
                        SUBTITLES_OFF => SubtitleChoice::Off,
                        index => SubtitleChoice::Stream(index),
                    };
                    this.now_playing
                        .update(cx, |state, cx| state.change_subtitle(choice, cx));
                }
            }));
        }

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
            auto_skip: crate::config::Config::load().auto_skip_intro.unwrap_or(false),
            auto_skipped: None,
            version: None,
            audio,
            subtitle,
            controls_visible: true,
            hide_controls: None,
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

    fn next_episode(&mut self, _: &NextEpisode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(next) = self.now_playing.read(cx).next.clone() else {
            return;
        };
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play_from_resume(&next, cx));
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
                if !this.now_playing.read(cx).paused {
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
    /// already toggled pause, so the second toggles it back first.
    fn click_video(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.click_count() {
            1 => self.toggle_pause(&TogglePause, window, cx),
            2 => {
                self.send(UiCommand::TogglePause, cx);
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

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.now_playing.read(cx);
        let paused = state.paused;
        let (muted, has_next) = (state.muted || state.volume == 0., state.next.is_some());
        let mut time = format!(
            "{} / {}",
            timestamp(state.position),
            state.duration.map(timestamp).unwrap_or_else(|| "--:--".into())
        );
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
                div()
                    .relative()
                    .w_full()
                    .child(Slider::new(seek).w_full())
                    .children(chapter_marks.into_iter().map(|fraction| {
                        div()
                            .absolute()
                            .top(relative(0.5))
                            .mt(px(-CHAPTER_MARK_H / 2.))
                            .left(relative(fraction))
                            .w(px(2.))
                            .h(px(CHAPTER_MARK_H))
                            .bg(white().opacity(0.6))
                    }))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .text_color(white())
                    .child(
                        Button::new("back-10")
                            .ghost()
                            .label("-10s")
                            .on_click(cx.listener(|this, _, window, cx| this.seek_backward(&SeekBackward, window, cx))),
                    )
                    .child(
                        Button::new("play-pause")
                            .primary()
                            .icon(if paused { Lucide::Play } else { Lucide::Pause })
                            .label(if paused { "Play" } else { "Pause" })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_pause(&TogglePause, window, cx))),
                    )
                    .child(
                        Button::new("forward-10")
                            .ghost()
                            .label("+10s")
                            .on_click(cx.listener(|this, _, window, cx| this.seek_forward(&SeekForward, window, cx))),
                    )
                    .child(div().text_sm().child(time))
                    .child(
                        Button::new("mute")
                            .ghost()
                            .icon(if muted { Lucide::VolumeX } else { Lucide::Volume2 })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_mute(&ToggleMute, window, cx))),
                    )
                    .child(div().flex_none().w(px(96.)).child(Slider::new(&self.volume).w_full()))
                    .child(div().flex_1())
                    .when(has_next, |el| {
                        el.child(
                            Button::new("next-episode")
                                .ghost()
                                .icon(Lucide::SkipForward)
                                .label("Next episode")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.next_episode(&NextEpisode, window, cx)),
                                ),
                        )
                    })
                    .children(self.version.as_ref().map(|s| picker(s, 140., 240.)))
                    .children(self.audio.as_ref().map(|s| picker(s, 200., 320.)))
                    .children(self.subtitle.as_ref().map(|s| picker(s, 200., 320.)))
                    .child(Button::new("fullscreen").ghost().label("Fullscreen").on_click(
                        cx.listener(|this, _, window, cx| this.toggle_fullscreen(&ToggleFullscreen, window, cx)),
                    )),
            );

        div().absolute().inset_0().child(top).child(bottom)
    }

    /// The skip button or the up-next card, bottom right. Shown with or without the controls
    /// (above them when they're up).
    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let state = self.now_playing.read(cx);
        let prompt = if let Some(segment) = state.skippable() {
            Button::new("skip-segment")
                .large()
                .icon(Lucide::SkipForward)
                .label(segment.kind.skip_label())
                .on_click(cx.listener(|this, _, window, cx| this.skip(&Skip, window, cx)))
                .into_any_element()
        } else if state.show_up_next() {
            let next = state.next.as_ref()?;
            let (series, episode) = crate::components::card_text(next);
            let label = if next.kind == jellyfin::models::BaseItemKind::Episode {
                episode
            } else {
                series
            };
            let remaining = state.duration.map(|d| d.saturating_sub(state.position));
            v_flex()
                .w(px(320.))
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
                )
                .into_any_element()
        } else {
            return None;
        };
        let bottom = if self.controls_visible { 120. } else { 32. };
        Some(
            div()
                .occlude()
                .absolute()
                .right(px(24.))
                .bottom(px(bottom))
                .child(prompt),
        )
    }
}

/// A small label in the player's top bar.
fn badge(text: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .px_2()
        .rounded_md()
        .bg(black().opacity(0.5))
        .text_sm()
        .text_color(white().opacity(0.8))
        .child(text)
}

/// A fixed-width track or version picker for the control bar.
fn picker(select: &ChoiceSelect, width: f32, menu_width: f32) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(width))
        .child(Select::new(select).small().w_full().menu_width(px(menu_width)))
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
            .on_action(cx.listener(Self::next_chapter))
            .on_action(cx.listener(Self::prev_chapter))
            .on_action(cx.listener(Self::skip))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::exit_player))
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
            .when(self.controls_visible, |el| el.child(self.render_controls(cx)))
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
/// the leftover lines for the next event.
fn scroll_steps(pending: f32, lines: f32) -> (f32, i32) {
    let total = pending + lines;
    let steps = (total / SCROLL_LINES_PER_STEP).trunc() as i32;
    (total - steps as f32 * SCROLL_LINES_PER_STEP, steps)
}

#[cfg(test)]
mod tests {
    use super::{SCROLL_LINES_PER_STEP, scroll_steps};

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
    fn scrolling_back_cancels() {
        let (pending, steps) = scroll_steps(0., -1.5);
        assert_eq!(steps, 0);
        let (_, steps) = scroll_steps(pending, 1.5);
        assert_eq!(steps, 0);
    }
}
