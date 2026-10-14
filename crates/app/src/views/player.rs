//! Full-window player for embedded mpv: the video plus GPUI controls on top.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use player::libmpv::Frame;

use crate::components::timestamp;
use crate::playback::{NowPlaying, SubtitleChoice, UiCommand};
use crate::tracks::{Choice, ChoiceSelect, SUBTITLES_OFF, track_pickers};

actions!(player, [TogglePause, SeekForward, SeekBackward, ToggleFullscreen, ExitPlayer]);

pub const KEY_CONTEXT: &str = "Player";
const SEEK_STEP: f64 = 10.;
const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);

/// The frame on screen.
#[derive(Clone)]
enum Shown {
    /// Software-rendered pixels uploaded as an image.
    Image(Arc<RenderImage>),
    /// A GPU texture shared by the player (Windows), drawn as a surface.
    #[cfg_attr(not(windows), allow(dead_code))]
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
        let (audio, subtitle) = track_pickers(&source, state.audio, subtitle_index, window, cx);

        let mut subscriptions = vec![cx.observe_in(&now_playing, window, |this, _, window, cx| {
            this.sync_seek_bar(window, cx);
            cx.notify();
        })];
        if let Some(select) = &audio {
            subscriptions.push(cx.subscribe(select, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    this.send(UiCommand::Audio(*index), cx);
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
                    this.send(UiCommand::Subtitle(choice), cx);
                }
            }));
        }

        let mut frames = now_playing.read(cx).video.frames();
        let _frames = cx.spawn_in(window, async move |this, cx| {
            while frames.changed().await.is_ok() {
                let Some(frame) = frames.borrow_and_update().clone() else { continue };
                let shown = match frame {
                    Frame::Pixels(frame) => {
                        let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels.clone())
                        else {
                            continue;
                        };
                        Shown::Image(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(buffer)])))
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

    fn send(&self, command: UiCommand, cx: &App) {
        self.now_playing.read(cx).send(command);
    }

    /// Creates the seek bar once the duration is known, then follows playback unless the user
    /// is dragging it.
    fn sync_seek_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (position, duration) = {
            let state = self.now_playing.read(cx);
            (state.position.as_secs_f64() as f32, state.duration)
        };
        let Some(duration) = duration.filter(|d| !d.is_zero()) else { return };
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
                self._subscriptions.push(cx.subscribe(&seek, |this, _, event: &SliderEvent, cx| match event {
                    SliderEvent::Change(_) => this.scrubbing = true,
                    SliderEvent::Release(value) => {
                        this.scrubbing = false;
                        this.send(UiCommand::SeekTo(Duration::from_secs_f64(value.start().max(0.) as f64)), cx);
                    }
                }));
                self.seek = Some(seek);
            }
        }
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

    /// Esc: leave fullscreen first, then stop playback (which closes the player).
    fn exit_player(&mut self, _: &ExitPlayer, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() {
            window.toggle_fullscreen();
        } else {
            self.send(UiCommand::Stop, cx);
        }
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.now_playing.read(cx);
        let paused = state.paused;
        let time = format!(
            "{} / {}",
            timestamp(state.position),
            state.duration.map(timestamp).unwrap_or_else(|| "--:--".into())
        );
        let title = crate::components::card_text(&state.item);
        let title = if state.item.kind == jellyfin::models::BaseItemKind::Episode {
            format!("{} · {}", title.0, title.1)
        } else {
            title.0
        };

        let top = h_flex()
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
            .child(div().text_lg().text_color(white()).truncate().child(title));

        let bottom = v_flex()
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
            .children(self.seek.as_ref().map(|seek| Slider::new(seek).w_full()))
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
                            .icon(IconName::Play)
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
                    .child(div().flex_1())
                    .children(self.audio.as_ref().map(|s| Select::new(s).small().w(px(200.)).menu_width(px(320.))))
                    .children(self.subtitle.as_ref().map(|s| Select::new(s).small().w(px(200.)).menu_width(px(320.))))
                    .child(
                        Button::new("fullscreen")
                            .ghost()
                            .label("Fullscreen")
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_fullscreen(&ToggleFullscreen, window, cx))),
                    ),
            );

        div().absolute().inset_0().child(top).child(bottom)
    }
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
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::exit_player))
            .on_mouse_move(cx.listener(|this, _, window, cx| this.show_controls(window, cx)))
            .children(self.frame.clone().map(|frame| {
                div()
                    .id("video")
                    .absolute()
                    .inset_0()
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_pause(&TogglePause, window, cx)))
                    .child(match frame {
                        Shown::Image(image) => {
                            img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Contain).into_any_element()
                        }
                        #[cfg(windows)]
                        Shown::Shared(texture) => surface(SharedTexture {
                            handle: texture.handle,
                            width: texture.width,
                            height: texture.height,
                        })
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                        #[cfg(not(windows))]
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
    }
}
