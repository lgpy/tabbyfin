//! The player page: the video, the bars over it, the seek bar, the settings and chapters menus,
//! the skip button, and the up-next and resume cards.
//!
//! On `main` this was `views/player.rs`, and the session state and commands it draws are P1's
//! (`playback.rs`): pause, seek and stop go through [`Services::control`], so a SyncPlay group
//! gets them as group requests, and tracks, speed and delays through [`Services::change_audio`],
//! [`Services::change_subtitle`] and [`Services::adjust`].
//!
//! The shell makes the page on `AppEvent::PlaybackStarted` and drops it on `PlaybackEnded`, so it
//! lives through autoplay and reconnects; what belongs to one item (the trickplay sheets, the
//! auto-skipped segment, a scrub in progress) starts over when the session changes. The video
//! widget is keyed by `NowPlaying::session`, and the last frame stays up while the next session
//! loads.
//!
//! The page is not on the shell's page stack: playback opens it and closes it, as on `main`. Its
//! keys are `keys::Scope::Player`, routed by the shell; the shell's Back stops playback. The
//! menus are pointer-only (egui's popups take no keys), and every widget here is non-focusable
//! (`Sense::CLICK` without `FOCUSABLE`, U8's rule for its dialogs): a Tab that gave egui the
//! focus would take every key from the player (`keys.rs`).
//!
//! Auto-skip and the clock's "time left" are config settings read when the page opens; the clock
//! saves its choice when clicked.

use std::time::Duration;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Margin, Rect, RectAlign, Response, RichText,
    Sense, Stroke, StrokeKind, TextFormat, TextStyle, Ui, UiBuilder, Vec2, pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ARROW_BACK, ICON_ARROW_DROP_DOWN, ICON_CHECK, ICON_FAST_FORWARD, ICON_FAST_REWIND, ICON_FULLSCREEN,
    ICON_FULLSCREEN_EXIT, ICON_PAUSE, ICON_PLAY_ARROW, ICON_RESTART_ALT, ICON_SETTINGS, ICON_SKIP_NEXT,
    ICON_SKIP_PREVIOUS, ICON_VIEW_LIST, ICON_VOLUME_OFF, ICON_VOLUME_UP,
};
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemKind, ImageType};

use crate::keys::Action;
use crate::markers::{self, Segment, SegmentKind};
use crate::motion;
use crate::playback::{Adjustments, NowPlaying, PlayOptions, SubtitleChoice, UiCommand, Version};
use crate::task;
use crate::theme::{self, over_video};
use crate::tracks::{self, Choice, SUBTITLES_OFF};
use crate::trickplay::Trickplay;
use crate::views::PageCx;
use crate::widgets::mascot;
use crate::widgets::text::{card_text, timestamp};
use crate::widgets::video::Video;

/// Seconds per seek key press (and remote rewind or fast-forward).
pub const SEEK_STEP: f64 = 10.;
/// Volume percent per key press, wheel notch or remote volume step.
pub const VOLUME_STEP: f64 = 5.;
/// Trackpad points per wheel notch, for platforms that send pixel deltas (macOS): `main`'s
/// three lines of 30. egui's line deltas are notches already (X11, Windows and Wayland report
/// one per notch, where GPUI gave three lines), so only the pixel deltas are divided.
pub const PIXELS_PER_NOTCH: f32 = 90.;
/// The most volume steps one wheel event makes, so a fast flick on a free-spinning wheel or a
/// trackpad's burst of pixels can't jump the volume from silent to full.
pub const MAX_STEPS_PER_SCROLL: f32 = 4.;

/// The mascot's size over the video, as on `main`.
const LOADER_SIZE: f32 = 128.;

/// How long the controls stay up without the mouse moving (and while nothing is paused).
const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);
/// The top bar's height: its padding and a control button.
const TOP_BAR_H: f32 = 2. * PADDING + CONTROL;
const PADDING: f32 = 16.;
/// The gap between a bar's controls, `main`'s `gap_3`.
const GAP: f32 = theme::SPACE.md;
/// The seek bar's row: gpui-kit's slider track is 24 points high around its track.
const SEEK_H: f32 = 24.;
/// The slider track inside that row, and its thumb: a 16-point circle with a 1-point ring.
const TRACK_H: f32 = 6.;
const THUMB_R: f32 = 8.;
const THUMB_RING: f32 = 1.;
/// The bottom bar's height: its top padding, the seek bar, the gap below it, the controls and
/// the bottom padding.
const BOTTOM_BAR_H: f32 = 32. + SEEK_H + 8. + PLAY_CONTROL + PADDING;
/// A control's square, and play/pause's larger one; icons are this share of a side.
const CONTROL: f32 = 40.;
const PLAY_CONTROL: f32 = 48.;
const CONTROL_ICON: f32 = 0.52;
/// The volume slider's width, and the row it sits in (a slider's 24-point track).
const VOLUME_W: f32 = 96.;
const VOLUME_H: f32 = 24.;
/// The version picker: a small select, 140 points wide as `main`'s was.
const VERSION_W: f32 = 140.;
const VERSION_H: f32 = 24.;
/// The chapter marks' height, and the width of the seek-bar preview thumbnail.
const CHAPTER_MARK_H: f32 = 8.;
const PREVIEW_W: f32 = 192.;
/// The prompt's distance from the window's right edge, and from its bottom: above the controls
/// when they show, near the bottom edge otherwise.
const PROMPT_MARGIN: f32 = 24.;
const PROMPT_ABOVE_CONTROLS: f32 = 120.;
const PROMPT_AT_THE_EDGE: f32 = 32.;
const UP_NEXT_W: f32 = 320.;
/// A chapter row's thumbnail, and the chapters menu's width and scrolling height.
const CHAPTER_THUMB_W: f32 = 128.;
const CHAPTER_THUMB_H: f32 = 72.;
const CHAPTERS_W: f32 = 340.;
const CHAPTERS_MAX_H: f32 = 420.;
const SETTINGS_W: f32 = 380.;
/// A row of a settings tab's track list, and how many show before it scrolls.
const TRACK_ROW_H: f32 = 28.;
const TRACK_ROWS: f32 = 5.;

/// What the player's one prompt shows. Each kind slides in when it appears, as `main`'s ids did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptKind {
    Skip,
    UpNext,
    Resume,
}

/// The player, while a playback session shows.
pub struct PlayerPage {
    /// The playing session's video, with the session's number: a new session (the next episode,
    /// a reconnect, another version) brings a new video output. Kept between sessions, so the
    /// last frame stays up while the next one loads.
    video: Option<(u64, Video)>,
    /// The session the per-item state below belongs to.
    session: Option<u64>,
    /// The user is dragging the seek bar: the bar follows the pointer, not playback.
    scrub: Option<Duration>,
    /// The volume shown while the user drags the slider; playback's updates leave it alone then.
    adjusting_volume: bool,
    volume: f64,
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
    /// Seek-bar previews, once the session knows the server has tiles for this version.
    trickplay: Option<Trickplay>,
    controls_visible: bool,
    /// The controls appeared since the last frame drawn, so they fade in (as `main`'s did each
    /// time they showed), and when they did.
    controls_appeared: bool,
    controls_since: Option<f64>,
    /// The prompt showing, and when it appeared: each kind slides in when it appears, as
    /// `main`'s did.
    prompt_in: Option<(PromptKind, f64)>,
    hide_controls: task::Deadline,
    /// A menu is open, which keeps the controls up.
    menu_open: bool,
    /// The settings menu's tab, kept while the player is open.
    settings_tab: SettingsTab,
    /// Show the time left instead of the time played (a setting, toggled by clicking the clock).
    show_remaining: bool,
}

impl PlayerPage {
    pub fn new() -> Self {
        let config = crate::config::Config::load();
        let mut this = Self {
            video: None,
            session: None,
            scrub: None,
            adjusting_volume: false,
            volume: 0.,
            scroll_pending: 0.,
            paused_before_click: false,
            auto_skip: config.auto_skip_intro.unwrap_or(false),
            auto_skipped: None,
            trickplay: None,
            controls_visible: false,
            controls_appeared: false,
            controls_since: None,
            prompt_in: None,
            hide_controls: task::Deadline::default(),
            menu_open: false,
            settings_tab: SettingsTab::Audio,
            show_remaining: config.show_remaining_time.unwrap_or(false),
        };
        this.show_controls();
        this
    }

    /// Draws the player into the whole window and handles its mouse and wheel input.
    pub fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        // Dark over the video in either theme, as `main` had it.
        ui.style_mut().visuals = egui::Visuals::dark();
        ui.style_mut().visuals.override_text_color = Some(over_video::FOREGROUND);
        // Any mouse movement brings the controls (and the cursor) back.
        if ui.input(|input| input.pointer.delta() != Vec2::ZERO) {
            self.show_controls();
        }
        self.sync_video(cx);
        if let Some(session) = cx.services.playback.now_playing().map(|state| state.session) {
            self.follow_session(ui.ctx(), session);
            self.auto_skip(cx);
            self.ensure_trickplay(cx);
            if let Some(trickplay) = &mut self.trickplay {
                trickplay.poll();
            }
        }
        let menus = Menus {
            chapters: egui::Popup::is_id_open(ui.ctx(), chapters_id()),
            settings: egui::Popup::is_id_open(ui.ctx(), settings_id()),
        };
        let view = cx
            .services
            .playback
            .now_playing()
            .map(|state| View::read(state, menus, self.show_remaining));

        let now = ui.input(|input| input.time);
        if std::mem::take(&mut self.controls_appeared) || self.controls_since.is_none() {
            self.controls_since = Some(now);
        }

        let rect = ui.max_rect();
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_BAR_H));
        let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BOTTOM_BAR_H), rect.right_bottom());
        let (response, has_frame, import_failed) = self.draw_video(ui);
        if import_failed {
            cx.services.fall_back_to_pixels();
        }
        let Some(view) = view else {
            // Between sessions (a reconnect, or the next item still loading): the last frame
            // stays, with the loader over it and a bar to leave by.
            loader(ui, rect);
            self.reconnecting_bar(ui, cx, top);
            self.settle(ui, false);
            return;
        };
        if view.loading || !has_frame {
            loader(ui, rect);
        }
        if self.controls_visible {
            let alpha = self.controls_alpha(ui.ctx(), now);
            motion::faded(ui, alpha, |ui| {
                self.top_bar(ui, &view, cx, top);
                self.controls(ui, &view, cx, bottom);
            });
        }
        self.prompt(ui, &view, cx);
        if let Some(response) = response {
            let bars = self.controls_visible.then_some([top, bottom]);
            self.video_input(&response, ui, &view, cx, bars);
        }
        self.settle(ui, view.paused);
    }

    /// The video widget for the playing session, made when the session changes.
    fn sync_video(&mut self, cx: &mut PageCx) {
        let Some(render_state) = cx.render_state else { return };
        let Some(state) = cx.services.playback.now_playing() else {
            return;
        };
        if self.video.as_ref().map(|(session, _)| *session) != Some(state.session) {
            self.video = Some((state.session, Video::new(cx.ctx, state.video.clone(), render_state)));
        }
    }

    /// Draws the video (or the last session's last frame), reporting whether a frame shows and
    /// whether a GPU frame couldn't be imported.
    fn draw_video(&mut self, ui: &mut egui::Ui) -> (Option<Response>, bool, bool) {
        let Some((_, video)) = &mut self.video else {
            return (None, false, false);
        };
        let response = video.show(ui);
        (Some(response), video.has_frame(), video.import_failed())
    }

    /// The controls' opacity at `now`: they fade in over [`motion::FADE`] after they show, as
    /// `main`'s did. Asks for the frames while they fade.
    fn controls_alpha(&self, ctx: &egui::Context, now: f64) -> f32 {
        if !theme::animations(ctx) {
            return 1.;
        }
        let alpha = motion::progress(self.controls_since.unwrap_or(now), now, 0., motion::FADE);
        if alpha < 1. {
            ctx.request_repaint();
        }
        alpha
    }

    /// The bar while no session is running (a reconnect, or the next item loading): Back stops
    /// playback, cancelling the reconnect, as Esc does.
    fn reconnecting_bar(&mut self, ui: &mut egui::Ui, cx: &mut PageCx, rect: Rect) {
        let painter = ui.painter();
        gradient(painter, rect, over_video::SCRIM, over_video::CLEAR);
        ui.interact(rect, Id::unique("player_top_bar"), Sense::CLICK);
        let inner = rect.shrink2(vec2(PADDING, 0.));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                if control_button(ui, ICON_ARROW_BACK, "Back (Esc)").clicked() {
                    self.exit(cx);
                }
                ui.label("Starting playback…");
            },
        );
    }

    /// The session changed: what belongs to one item starts over, and the menus close, as they
    /// did when `main` made a new page for each session.
    fn follow_session(&mut self, ctx: &egui::Context, session: u64) {
        if self.session == Some(session) {
            return;
        }
        self.session = Some(session);
        self.scrub = None;
        self.trickplay = None;
        self.auto_skipped = None;
        self.adjusting_volume = false;
        self.paused_before_click = false;
        self.settings_tab = SettingsTab::Audio;
        egui::Popup::close_id(ctx, settings_id());
        egui::Popup::close_id(ctx, chapters_id());
        egui::Popup::close_id(ctx, versions_id());
        self.menu_open = false;
        // The settings belong to the player, not to one item: read them again as `main` did
        // when it made a new page for each.
        let config = crate::config::Config::load();
        self.auto_skip = config.auto_skip_intro.unwrap_or(false);
        self.show_remaining = config.show_remaining_time.unwrap_or(false);
        self.show_controls();
        // A new item's prompt slides in again, as it did under `main`'s page per session.
        self.prompt_in = None;
    }

    /// With the setting on, skips each intro or recap once when it starts. Not in a SyncPlay
    /// group, where every member would ask the group to seek.
    fn auto_skip(&mut self, cx: &mut PageCx) {
        if !self.auto_skip {
            return;
        }
        let segment = {
            let Some(state) = cx.services.playback.now_playing() else {
                return;
            };
            if state.group_item.is_some() {
                return;
            }
            state.skippable()
        };
        if let Some(segment) = segment
            && matches!(segment.kind, SegmentKind::Intro | SegmentKind::Recap)
            && self.auto_skipped != Some(segment.start)
        {
            self.auto_skipped = Some(segment.start);
            cx.services.control(UiCommand::SeekTo(segment.end));
        }
    }

    /// The preview thumbnails, once the session knows the playing version's tiles. The
    /// narrowest set is used: they're shown small.
    fn ensure_trickplay(&mut self, cx: &mut PageCx) {
        if self.trickplay.is_some() {
            return;
        }
        let Some(state) = cx.services.playback.now_playing() else {
            return;
        };
        let Some(info) = state
            .trickplay
            .get(&state.source.id)
            .and_then(|sets| sets.values().min_by_key(|info| info.width))
            .copied()
        else {
            return;
        };
        let client = cx.services.playback.client().clone();
        let (item_id, source_id) = (state.item.id.clone(), state.source.id.clone());
        self.trickplay = Some(Trickplay::new(client, item_id, source_id, info, cx.ctx));
    }

    /// Shows the controls, and hides them again after a few seconds without the mouse moving,
    /// unless paused or a menu is open.
    fn show_controls(&mut self) {
        if !self.controls_visible {
            self.controls_appeared = true;
        }
        self.controls_visible = true;
        self.hide_controls.set(HIDE_CONTROLS_AFTER);
    }

    /// Hides the controls once their timer is up, keeps egui from drawing the cursor then, and
    /// follows the menus opening and closing (which keep the controls up).
    fn settle(&mut self, ui: &egui::Ui, paused: bool) {
        let ctx = ui.ctx();
        let open = egui::Popup::is_id_open(ctx, settings_id())
            || egui::Popup::is_id_open(ctx, chapters_id())
            || egui::Popup::is_id_open(ctx, versions_id());
        if open != self.menu_open {
            self.menu_open = open;
            self.show_controls();
        }
        // A scrub whose release never reached the bar (the window lost the pointer) mustn't
        // leave the bar at the pointer's last time for ever.
        if self.scrub.is_some() && !ui.input(|input| input.pointer.any_down()) {
            self.scrub = None;
        }
        if self.hide_controls.fire(ctx) {
            if self.scrub.is_some() || self.adjusting_volume {
                // Hiding the controls would drop the drag under the user's hand.
                self.hide_controls.set(HIDE_CONTROLS_AFTER);
            } else if !paused && !self.menu_open {
                self.controls_visible = false;
            }
        }
        if !self.controls_visible {
            ctx.set_cursor_icon(CursorIcon::None);
        }
    }

    /// A click on the video toggles pause; a double click toggles fullscreen and puts pause
    /// back. The wheel changes the volume, except over the bars: `bars` is `None` while they're
    /// hidden, so the whole window is the video then.
    fn video_input(&mut self, response: &Response, ui: &Ui, view: &View, cx: &mut PageCx, bars: Option<[Rect; 2]>) {
        let over_bar = ui
            .input(|input| input.pointer.interact_pos())
            .is_some_and(|pos| bars.is_some_and(|bars| bars.iter().any(|bar| bar.contains(pos))));
        if !over_bar && response.double_clicked() {
            // The first click already toggled pause: the second restores it. Not a second
            // toggle: in a SyncPlay group the first pause hasn't come back from the server
            // yet, so both toggles would ask the group to pause.
            cx.services.control(UiCommand::SetPause(self.paused_before_click));
            toggle_fullscreen(cx.ctx);
            self.show_controls();
        } else if !over_bar && response.triple_clicked() {
            // A third click does nothing, as on `main`.
        } else if !over_bar && response.clicked() {
            self.paused_before_click = view.paused;
            cx.services.control(UiCommand::TogglePause);
            self.show_controls();
        }
        self.scroll_volume(ui, over_bar, cx);
    }

    /// A wheel notch over the video changes the volume by one step; trackpad deltas add up
    /// until they make a whole notch. A menu over the video or the bars keep the wheel.
    fn scroll_volume(&mut self, ui: &Ui, over_bar: bool, cx: &mut PageCx) {
        if over_bar {
            return;
        }
        let over_area = ui.input(|input| input.pointer.hover_pos()).is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_some_and(|layer| layer != egui::LayerId::background())
        });
        if over_area {
            return;
        }
        let notches: f32 = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(wheel_notches(*unit, delta.y)),
                    _ => None,
                })
                .sum()
        });
        if notches == 0. {
            return;
        }
        let (pending, steps) = scroll_steps(self.scroll_pending, notches);
        self.scroll_pending = pending;
        if steps != 0 {
            self.change_volume(f64::from(steps) * VOLUME_STEP, cx);
        }
    }

    fn change_volume(&mut self, delta: f64, cx: &mut PageCx) {
        let Some(volume) = cx.services.playback.now_playing().map(|state| state.volume) else {
            return;
        };
        cx.services
            .control(UiCommand::SetVolume((volume + delta).clamp(0., 100.)));
        self.show_controls();
    }

    /// Esc (or Back): leave fullscreen first, then dismiss the up-next card, then stop playback
    /// (which closes the player).
    fn exit(&mut self, cx: &mut PageCx) {
        if fullscreen(cx.ctx) {
            cx.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        } else if cx
            .services
            .playback
            .now_playing()
            .is_some_and(|state| state.show_up_next())
        {
            if let Some(state) = cx.services.playback.now_playing_mut() {
                state.next_cancelled = true;
            }
        } else {
            cx.services.control(UiCommand::Stop);
        }
        cx.ctx.request_repaint();
    }

    /// Applies a change to the adjustments, as the settings menu and the speed and delay keys
    /// do; the playback manager clamps it and tells the session.
    fn adjust(&mut self, cx: &mut PageCx, change: impl FnOnce(&mut Adjustments)) {
        let Some(mut adjustments) = cx.services.playback.now_playing().map(|state| state.adjustments) else {
            return;
        };
        change(&mut adjustments);
        cx.services.adjust(adjustments);
        self.show_controls();
    }

    /// Switches the clock between the time played and the time left, and remembers the choice.
    fn toggle_remaining(&mut self) {
        self.show_remaining = !self.show_remaining;
        let mut config = crate::config::Config::load();
        config.show_remaining_time = Some(self.show_remaining);
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
    }

    /// The player's keys (`keys::Scope::Player`), acting on the running session. Returns whether
    /// the key was used.
    pub fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        let Some(state) = cx.services.playback.now_playing().map(Keys::read) else {
            // Between sessions or waiting to reconnect: Esc still leaves fullscreen, then stops
            // (cancelling a reconnect).
            return match action {
                Action::ExitPlayer if fullscreen(cx.ctx) => {
                    cx.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    true
                }
                Action::ExitPlayer => {
                    cx.services.control(UiCommand::Stop);
                    true
                }
                Action::ToggleFullscreen => {
                    toggle_fullscreen(cx.ctx);
                    true
                }
                _ => false,
            };
        };
        match action {
            Action::TogglePause => {
                cx.services.control(UiCommand::TogglePause);
                self.show_controls();
            }
            Action::SeekForward => {
                cx.services.control(UiCommand::SeekBy(SEEK_STEP));
                self.show_controls();
            }
            Action::SeekBackward => {
                cx.services.control(UiCommand::SeekBy(-SEEK_STEP));
                self.show_controls();
            }
            Action::VolumeUp => self.change_volume(VOLUME_STEP, cx),
            Action::VolumeDown => self.change_volume(-VOLUME_STEP, cx),
            Action::ToggleMute => {
                cx.services.control(UiCommand::ToggleMute);
                self.show_controls();
            }
            // Shift+N: the next episode, collection movie or queued item.
            Action::NextEpisode => cx.services.play_next(),
            // Shift+P: the previous episode or collection movie.
            Action::PreviousEpisode => cx.services.play_previous(),
            Action::NextChapter => {
                if let Some(start) = state.next_chapter {
                    cx.services.control(UiCommand::SeekTo(start));
                }
                self.show_controls();
            }
            Action::PrevChapter => {
                if let Some(start) = state.prev_chapter {
                    cx.services.control(UiCommand::SeekTo(start));
                }
                self.show_controls();
            }
            // Enter or S: skip the intro (or recap, preview…), or play the next episode from
            // the up-next card, or start over from the resume prompt.
            Action::Skip => {
                if let Some(segment) = state.skippable {
                    cx.services.control(UiCommand::SeekTo(segment.end));
                } else if state.show_up_next {
                    cx.services.play_next();
                } else if state.resume.is_some() {
                    cx.services.start_over();
                }
            }
            Action::ToggleFullscreen => toggle_fullscreen(cx.ctx),
            Action::ExitPlayer => self.exit(cx),
            Action::SpeedUp | Action::SpeedDown => {
                self.adjust(cx, |a| a.speed = a.step_speed(action == Action::SpeedUp));
            }
            Action::SubtitleDelayUp => {
                self.adjust(cx, |a| a.subtitle_delay += Adjustments::DELAY_STEP);
            }
            Action::SubtitleDelayDown => {
                self.adjust(cx, |a| a.subtitle_delay -= Adjustments::DELAY_STEP);
            }
            _ => return false,
        }
        true
    }

    /// The bar at the top: Back, the title, and whether a group or the transcoder is in charge.
    fn top_bar(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, rect: Rect) {
        let painter = ui.painter();
        gradient(painter, rect, over_video::SCRIM, over_video::CLEAR);
        // The bar takes clicks meant for the video below it (`main`'s `occlude`), while its
        // buttons, drawn after it, still get theirs.
        ui.interact(rect, Id::unique("player_top_bar"), Sense::CLICK);
        let badges = view.badges();
        let badge_w: f32 = badges.iter().map(|text| text_width(ui, text) + 16. + GAP).sum();
        let inner = rect.shrink2(vec2(PADDING, 0.));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                if control_button(ui, ICON_ARROW_BACK, "Back (Esc)").clicked() {
                    self.exit(cx);
                }
                let title_w = (ui.available_width() - badge_w).max(0.);
                ui.allocate_ui_with_layout(vec2(title_w, TOP_BAR_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&view.title).heading()).truncate());
                });
                for badge in badges {
                    label_badge(ui, badge);
                }
            },
        );
    }

    /// The bar at the bottom: the seek bar (with the hover preview) and the controls.
    fn controls(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, rect: Rect) {
        let painter = ui.painter();
        gradient(painter, rect, over_video::CLEAR, over_video::SCRIM_STRONG);
        ui.interact(rect, Id::unique("player_controls"), Sense::CLICK);
        let inner = rect.shrink2(vec2(PADDING, 0.));
        let seek = Rect::from_min_size(pos2(inner.left(), rect.top() + 32.), vec2(inner.width(), SEEK_H));
        self.seek_bar(ui, view, cx, seek);
        let row = Rect::from_min_size(
            pos2(inner.left(), seek.bottom() + 8.),
            vec2(inner.width(), PLAY_CONTROL),
        );

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(row)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                let tip = if view.paused { "Play (Space)" } else { "Pause (Space)" };
                let icon = if view.paused { ICON_PLAY_ARROW } else { ICON_PAUSE };
                if sized_control(ui, icon, PLAY_CONTROL, tip, false).clicked() {
                    cx.services.control(UiCommand::TogglePause);
                    self.show_controls();
                }
                if control_button(ui, ICON_FAST_REWIND, "Back 10 s (←)").clicked() {
                    cx.services.control(UiCommand::SeekBy(-SEEK_STEP));
                    self.show_controls();
                }
                if control_button(ui, ICON_FAST_FORWARD, "Forward 10 s (→)").clicked() {
                    cx.services.control(UiCommand::SeekBy(SEEK_STEP));
                    self.show_controls();
                }
                let muted = view.muted || view.volume == 0.;
                let icon = if muted { ICON_VOLUME_OFF } else { ICON_VOLUME_UP };
                let tip = if muted { "Unmute (M)" } else { "Mute (M)" };
                if control_button(ui, icon, tip).clicked() {
                    cx.services.control(UiCommand::ToggleMute);
                    self.show_controls();
                }
                self.volume_slider(ui, view, cx);
                // `main`'s `ml_2` on the clock, on top of the bar's gap.
                ui.add_space(8.);
                let clock = ui.add(
                    egui::Label::new(RichText::new(&view.time).small())
                        .sense(Sense::CLICK)
                        .selectable(false),
                );
                if clock.clicked() {
                    self.toggle_remaining();
                }
                if (view.speed - 1.).abs() > 1e-6 {
                    label_badge(ui, format!("{}×", view.speed));
                }
            },
        );

        let mut chapters_button = None;
        let mut settings_button = None;
        let mut versions_button = None;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(row)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                let fullscreen = fullscreen(ui.ctx());
                let icon = if fullscreen {
                    ICON_FULLSCREEN_EXIT
                } else {
                    ICON_FULLSCREEN
                };
                let tip = if fullscreen {
                    "Exit fullscreen (F)"
                } else {
                    "Fullscreen (F)"
                };
                if control_button(ui, icon, tip).clicked() {
                    toggle_fullscreen(ui.ctx());
                }
                let open = egui::Popup::is_id_open(ui.ctx(), settings_id());
                let settings = sized_control(ui, ICON_SETTINGS, CONTROL, "Playback settings", open);
                if settings.clicked() {
                    toggle_menu(ui.ctx(), settings_id());
                }
                settings_button = Some(settings);
                if view.chapter_count >= 2 {
                    let open = egui::Popup::is_id_open(ui.ctx(), chapters_id());
                    let chapters = sized_control(ui, ICON_VIEW_LIST, CONTROL, "Chapters", open);
                    if chapters.clicked() {
                        toggle_menu(ui.ctx(), chapters_id());
                    }
                    chapters_button = Some(chapters);
                }
                if let Some(name) = view.version_picker() {
                    let picker = version_picker(ui, name);
                    if picker.clicked() {
                        toggle_menu(ui.ctx(), versions_id());
                    }
                    versions_button = Some(picker);
                }
                if let Some(tip) = &view.next_tip
                    && control_button(ui, ICON_SKIP_NEXT, tip).clicked()
                {
                    cx.services.play_next();
                }
                if let Some(tip) = &view.previous_tip
                    && control_button(ui, ICON_SKIP_PREVIOUS, tip).clicked()
                {
                    cx.services.play_previous();
                }
            },
        );

        if let Some(response) = &chapters_button {
            self.chapters_menu(response, view, cx);
        }
        if let Some(response) = &settings_button {
            self.settings_menu(response, view, cx);
        }
        if let Some(response) = &versions_button {
            self.versions_menu(response, view, cx);
        }
    }

    /// The seek bar: chapter marks, the hover time and thumbnail, and dragging to seek. The bar
    /// follows the pointer while it's dragged, not playback, and seeks when it's released.
    fn seek_bar(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, rect: Rect) {
        let Some(duration) = view.duration.filter(|duration| !duration.is_zero()) else {
            return;
        };
        let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
        let shown = self.scrub.unwrap_or(view.position);
        let fraction = (shown.as_secs_f64() / duration.as_secs_f64()).clamp(0., 1.) as f32;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), TRACK_H));
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(TRACK_H as u8 / 2), over_video::SEEK_TRACK);
        let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * fraction, track.bottom()));
        painter.rect_filled(filled, CornerRadius::same(TRACK_H as u8 / 2), over_video::FOREGROUND);
        for mark in &view.chapter_marks {
            let x = track.left() + track.width() * mark;
            let half = CHAPTER_MARK_H / 2.;
            painter.line_segment(
                [pos2(x, track.center().y - half), pos2(x, track.center().y + half)],
                Stroke::new(2., over_video::MARK),
            );
        }
        slider_thumb(painter, pos2(filled.right(), track.center().y));

        // Dragging: the bar follows the pointer. The seek goes out when the button comes up, so
        // playback doesn't fight the bar.
        if let Some(pos) = response.interact_pointer_pos()
            && (response.is_pointer_button_down_on() || response.clicked())
        {
            self.scrub = Some(time_at(rect, duration, pos.x));
        }
        if response.clicked() || response.drag_stopped() {
            if let Some(time) = self.scrub.take() {
                cx.services.control(UiCommand::SeekTo(time));
            }
            self.show_controls();
        }

        let hover = response.hover_pos();
        let Some(pos) = hover else { return };
        let offset = (pos.x - rect.left()).clamp(0., rect.width());
        let time = time_at(rect, duration, rect.left() + offset);
        let preview = self.trickplay.as_mut().and_then(|trickplay| {
            let aspect = trickplay.aspect_ratio();
            trickplay.thumbnail(time).map(|texture| (texture, aspect))
        });
        // Centred on the mouse, but kept over the bar.
        let half = if preview.is_some() { PREVIEW_W / 2. } else { 24. };
        let max = (rect.width() - half).max(half);
        let center_x = rect.left() + offset.clamp(half, max);
        let painter = ui.painter();
        // `main`'s v_flex sat 18 points above the seek row's bottom, i.e. the time just over the
        // track and the thumbnail above the time.
        let galley = painter.layout_no_wrap(timestamp(time), theme::text::font(ui, "xs"), over_video::FOREGROUND);
        let time_h = galley.size().y;
        let label = Rect::from_min_size(
            pos2(center_x - galley.size().x / 2. - 4., rect.top() + 6. - time_h),
            galley.size() + vec2(8., 0.),
        );
        if let Some((texture, aspect)) = preview {
            let height = PREVIEW_W / aspect;
            let image = Rect::from_min_size(
                pos2(center_x - PREVIEW_W / 2., label.top() - 4. - height),
                vec2(PREVIEW_W, height),
            );
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                Color32::WHITE,
            );
            painter.rect_stroke(
                image,
                CornerRadius::same(4),
                Stroke::new(1., over_video::MARK),
                StrokeKind::Outside,
            );
        }
        painter.rect_filled(label, CornerRadius::same(4), over_video::SURFACE);
        painter.galley(label.center() - galley.size() / 2., galley, over_video::FOREGROUND);
    }

    /// The volume slider: a drag bar from 0 to 100, as `main`'s slider was. egui's `Slider` is
    /// focusable, which would let Tab take the player's keys from it.
    fn volume_slider(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx) {
        let (rect, response) = ui.allocate_exact_size(vec2(VOLUME_W, VOLUME_H), Sense::CLICK | Sense::DRAG);
        let volume = if self.adjusting_volume {
            self.volume
        } else {
            view.volume
        };
        let fraction = (volume / 100.).clamp(0., 1.) as f32;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), TRACK_H));
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(TRACK_H as u8 / 2), over_video::SEEK_TRACK);
        let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * fraction, track.bottom()));
        painter.rect_filled(filled, CornerRadius::same(TRACK_H as u8 / 2), over_video::FOREGROUND);
        slider_thumb(painter, pos2(filled.right(), track.center().y));

        let pressed = response.is_pointer_button_down_on();
        let released = response.clicked() || response.drag_stopped();
        if !self.adjusting_volume {
            self.volume = view.volume;
        }
        // Applied while dragging, so the change is audible right away, but only when the value
        // moved (as `main`'s slider did on `Change`), and once more on release.
        if let Some(pos) = response.interact_pointer_pos()
            && (pressed || released)
        {
            let value = (((pos.x - rect.left()) / rect.width()).clamp(0., 1.) * 100.).round() as f64;
            if value != self.volume {
                self.volume = value;
                cx.services.control(UiCommand::SetVolume(value));
            }
            if released {
                cx.services.control(UiCommand::SetVolume(self.volume));
            }
            self.adjusting_volume = true;
        }
        if !pressed && !response.dragged() {
            self.adjusting_volume = false;
        }
    }

    /// The skip button or the up-next card, bottom right, above the controls when they show.
    /// Each kind slides in when it appears, as `main`'s did.
    fn prompt(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx) {
        let kind = if view.skippable.is_some() {
            PromptKind::Skip
        } else if view.show_up_next {
            PromptKind::UpNext
        } else if view.resume.is_some() {
            PromptKind::Resume
        } else {
            self.prompt_in = None;
            return;
        };
        let now = ui.input(|input| input.time);
        let started = match self.prompt_in {
            Some((shown, started)) if shown == kind => started,
            _ => now,
        };
        self.prompt_in = Some((kind, started));
        // Sliding in from the right while it fades, as `main`'s `motion::slide_in` did.
        let (slide, alpha) = if theme::animations(ui.ctx()) {
            let t = motion::progress(started, now, 0., motion::ENTER);
            if t < 1. {
                ui.ctx().request_repaint();
            }
            (motion::LIFT * 2. * (1. - t), t)
        } else {
            (0., 1.)
        };
        let bottom = if self.controls_visible {
            PROMPT_ABOVE_CONTROLS
        } else {
            PROMPT_AT_THE_EDGE
        };
        egui::Area::new(prompt_id())
            .anchor(Align2::RIGHT_BOTTOM, vec2(-PROMPT_MARGIN + slide, -bottom))
            .order(egui::Order::Middle)
            // The slide starts off the window's edge; without this egui holds the card on
            // screen and nothing moves.
            .constrain(false)
            // Takes clicks meant for the video behind it.
            .sense(Sense::CLICK)
            .show(ui.ctx(), |ui| {
                ui.style_mut().visuals = egui::Visuals::dark();
                ui.style_mut().visuals.override_text_color = Some(over_video::FOREGROUND);
                if alpha < 1. {
                    ui.set_opacity(alpha);
                }
                if let Some(segment) = view.skippable {
                    let button =
                        egui::Button::new(icon_text(ui, ICON_SKIP_NEXT, segment.kind.skip_label())).sense(Sense::CLICK);
                    if ui.add(button).clicked() {
                        cx.services.control(UiCommand::SeekTo(segment.end));
                    }
                } else if view.show_up_next {
                    self.up_next_card(ui, view, cx);
                } else if let Some(from) = view.resume {
                    egui::Frame::new()
                        .fill(over_video::SURFACE)
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(format!("Resumed from {}", timestamp(from)));
                                let start_over = egui::Button::new(icon_text(ui, ICON_RESTART_ALT, "Start over"))
                                    .sense(Sense::CLICK);
                                if ui.add(start_over).clicked() {
                                    cx.services.start_over();
                                }
                            });
                        });
                }
            });
    }

    /// What plays next: its name, when it starts (with autoplay), and Play now or Cancel.
    fn up_next_card(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx) {
        let Some(label) = &view.next_label else { return };
        egui::Frame::new()
            .fill(over_video::SURFACE)
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(UP_NEXT_W);
                ui.label(RichText::new("Up next").small().color(over_video::MUTED_FOREGROUND));
                ui.add(egui::Label::new(RichText::new(label)).truncate());
                if view.autoplay
                    && let Some(remaining) = view.remaining
                {
                    let text = format!("Starts in {} s", remaining.as_secs());
                    ui.label(RichText::new(text).small().color(over_video::MUTED_FOREGROUND));
                }
                ui.horizontal(|ui| {
                    let play = egui::Button::new(icon_text(ui, ICON_PLAY_ARROW, "Play now")).sense(Sense::CLICK);
                    if ui.add(play).clicked() {
                        cx.services.play_next();
                    }
                    if ui.add(egui::Button::new("Cancel").sense(Sense::CLICK)).clicked()
                        && let Some(state) = cx.services.playback.now_playing_mut()
                    {
                        state.next_cancelled = true;
                    }
                });
            });
    }

    /// The chapters list: each chapter's image (when the server extracted them), name and start,
    /// the current one highlighted. A click seeks there and closes the menu.
    fn chapters_menu(&mut self, response: &Response, view: &View, cx: &mut PageCx) {
        let Some(menu) = &view.chapters else { return };
        let mut chosen = None;
        egui::Popup::menu(response)
            .id(chapters_id())
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::TOP_END)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(CHAPTERS_W)
            .show(|ui| {
                egui::ScrollArea::vertical().max_height(CHAPTERS_MAX_H).show(ui, |ui| {
                    for chapter in &menu.rows {
                        let request = chapter.image_tag.as_ref().map(|tag| ImageRequest {
                            index: Some(chapter.index as u32),
                            ..ImageRequest::new(&menu.item_id, ImageType::Chapter, Some(tag.clone()))
                                .fill_width((CHAPTER_THUMB_W * 2.) as u32)
                        });
                        let (rect, row) =
                            ui.allocate_exact_size(vec2(ui.available_width(), CHAPTER_THUMB_H + 8.), Sense::CLICK);
                        let current = menu.current == Some(chapter.index);
                        if current {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), theme::tokens_of(ui).accent);
                        } else if row.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(6),
                                ui.visuals().widgets.hovered.weak_bg_fill,
                            );
                        }
                        let image =
                            Rect::from_min_size(rect.min + vec2(4., 4.), vec2(CHAPTER_THUMB_W, CHAPTER_THUMB_H));
                        crate::widgets::image::show(ui, &mut cx.services.images, image, request.as_ref(), None, 4);
                        let name = chapter
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("Chapter {}", chapter.index + 1));
                        ui.painter().text(
                            pos2(image.right() + 12., image.top() + 2.),
                            Align2::LEFT_TOP,
                            name,
                            TextStyle::Body.resolve(ui.style()),
                            ui.visuals().text_color(),
                        );
                        ui.painter().text(
                            pos2(image.right() + 12., image.top() + 24.),
                            Align2::LEFT_TOP,
                            timestamp(chapter.start),
                            TextStyle::Small.resolve(ui.style()),
                            over_video::MUTED_FOREGROUND,
                        );
                        if row.clicked() {
                            chosen = Some(chapter.start);
                        }
                    }
                });
            });
        if let Some(start) = chosen {
            egui::Popup::close_id(cx.ctx, chapters_id());
            cx.services.control(UiCommand::SeekTo(start));
        }
    }

    /// The settings menu: audio, subtitle and general tabs, with the tracks, the delays, the
    /// subtitle size and the speeds.
    fn settings_menu(&mut self, response: &Response, view: &View, cx: &mut PageCx) {
        let Some(menu) = &view.settings else { return };
        let mut tab = self.settings_tab;
        let mut chosen_audio = None;
        let mut chosen_subtitle = None;
        let mut adjustments = menu.adjustments;
        let mut adjusted = false;
        egui::Popup::menu(response)
            .id(settings_id())
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::TOP_END)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(SETTINGS_W)
            .show(|ui| {
                ui.set_min_width(SETTINGS_W - 16.);
                ui.horizontal(|ui| {
                    for choice in SettingsTab::ALL {
                        if menu_button(ui, choice.label(), tab == choice).clicked() {
                            tab = choice;
                        }
                    }
                });
                ui.add_space(6.);
                match tab {
                    SettingsTab::Audio => {
                        section_label(ui, "Track");
                        chosen_audio = track_list(
                            ui,
                            "player-audio",
                            "No audio tracks",
                            &menu.audio_choices,
                            menu.audio.or(menu.default_audio),
                        );
                        if let Some(step) =
                            stepper(ui, "Audio delay", delay_text(adjustments.audio_delay), "−0.1", "+0.1")
                        {
                            match step {
                                Step::Down => adjustments.audio_delay -= Adjustments::DELAY_STEP,
                                Step::Up => adjustments.audio_delay += Adjustments::DELAY_STEP,
                                Step::Reset => adjustments.audio_delay = 0.,
                            }
                            adjusted = true;
                        }
                    }
                    SettingsTab::Subtitles => {
                        section_label(ui, "Track");
                        let selected = match menu.subtitle {
                            SubtitleChoice::Stream(index) => index,
                            SubtitleChoice::Off => SUBTITLES_OFF,
                            SubtitleChoice::Auto => menu.default_subtitle.unwrap_or(SUBTITLES_OFF),
                        };
                        chosen_subtitle = track_list(
                            ui,
                            "player-subtitles",
                            "No subtitles",
                            &menu.subtitle_choices,
                            Some(selected),
                        )
                        .map(|value| match value {
                            SUBTITLES_OFF => SubtitleChoice::Off,
                            index => SubtitleChoice::Stream(index),
                        });
                        if let Some(step) = stepper(
                            ui,
                            "Subtitle delay",
                            delay_text(adjustments.subtitle_delay),
                            "−0.1",
                            "+0.1",
                        ) {
                            match step {
                                Step::Down => adjustments.subtitle_delay -= Adjustments::DELAY_STEP,
                                Step::Up => adjustments.subtitle_delay += Adjustments::DELAY_STEP,
                                Step::Reset => adjustments.subtitle_delay = 0.,
                            }
                            adjusted = true;
                        }
                        let size = format!("{:.0}%", adjustments.subtitle_scale * 100.);
                        if let Some(step) = stepper(ui, "Subtitle size", size, "−", "+") {
                            match step {
                                Step::Down => adjustments.subtitle_scale -= Adjustments::SCALE_STEP,
                                Step::Up => adjustments.subtitle_scale += Adjustments::SCALE_STEP,
                                Step::Reset => adjustments.subtitle_scale = 1.,
                            }
                            adjusted = true;
                        }
                    }
                    SettingsTab::General => {
                        ui.horizontal(|ui| {
                            ui.add_sized(vec2(110., 22.), egui::Label::new("Speed"));
                            if menu.in_group {
                                ui.label(RichText::new("Normal in SyncPlay").color(over_video::MUTED_FOREGROUND));
                            } else {
                                for &speed in &Adjustments::SPEEDS {
                                    let current = (speed - adjustments.speed).abs() < 1e-6;
                                    if menu_button(ui, format!("{speed}×"), current).clicked() {
                                        adjustments.speed = speed;
                                        adjusted = true;
                                    }
                                }
                            }
                        });
                    }
                }
                let hint = match tab {
                    SettingsTab::Audio => None,
                    SettingsTab::Subtitles => Some("Keys: z and x change the subtitle delay."),
                    SettingsTab::General => Some("Keys: [ and ] change the speed."),
                };
                if let Some(hint) = hint {
                    ui.add_space(4.);
                    ui.label(RichText::new(hint).small().color(over_video::MUTED_FOREGROUND));
                }
            });
        self.settings_tab = tab;
        if let Some(index) = chosen_audio {
            cx.services.change_audio(index);
        }
        if let Some(choice) = chosen_subtitle {
            cx.services.change_subtitle(choice);
        }
        if adjusted {
            cx.services.adjust(adjustments);
            self.show_controls();
        }
    }

    /// The version picker's menu: the item's versions, the playing one checked. Another version
    /// restarts the stream at the current position.
    fn versions_menu(&mut self, response: &Response, view: &View, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, versions_id()) {
            return;
        }
        let mut chosen = None;
        egui::Popup::menu(response)
            .id(versions_id())
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::TOP_END)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(240.)
            .show(|ui| {
                for version in &view.versions {
                    let current = version.id == view.source_id;
                    if menu_button(ui, &version.name, current).clicked() {
                        chosen = Some(version.id.clone());
                    }
                }
            });
        if let Some(id) = chosen {
            egui::Popup::close_id(cx.ctx, versions_id());
            cx.services.reload(PlayOptions {
                media_source_id: Some(id),
                ..Default::default()
            });
        }
    }
}

/// The session values the keys act on, read before any command is sent (which borrows the
/// playback manager mutably).
struct Keys {
    skippable: Option<Segment>,
    show_up_next: bool,
    resume: Option<Duration>,
    next_chapter: Option<Duration>,
    prev_chapter: Option<Duration>,
}

impl Keys {
    fn read(state: &NowPlaying) -> Self {
        Self {
            skippable: state.skippable(),
            show_up_next: state.show_up_next(),
            resume: state.resume_prompt(),
            next_chapter: markers::next_chapter(&state.chapters, state.position),
            prev_chapter: markers::prev_chapter(&state.chapters, state.position),
        }
    }
}

/// What the page draws this frame, read from the session once: strings and numbers, so drawing
/// can borrow the page's own state (the menus, the video) and the services (the image cache) at
/// the same time. The heavy parts of a menu's data are read only while it's open.
struct View {
    title: String,
    paused: bool,
    muted: bool,
    loading: bool,
    position: Duration,
    duration: Option<Duration>,
    volume: f64,
    speed: f64,
    /// A SyncPlay group controls playback for this session.
    group: bool,
    transcoding: bool,
    /// The clock's text: played (or left) and the total, and the chapter's name.
    time: String,
    /// Chapter starts as fractions of the duration, for the seek bar's marks.
    chapter_marks: Vec<f32>,
    chapter_count: usize,
    previous_tip: Option<String>,
    next_tip: Option<String>,
    versions: Vec<Version>,
    source_id: String,
    version_name: Option<String>,
    skippable: Option<Segment>,
    show_up_next: bool,
    next_label: Option<String>,
    /// How long the item has left, for the up-next card's countdown.
    remaining: Option<Duration>,
    autoplay: bool,
    resume: Option<Duration>,
    chapters: Option<ChaptersMenu>,
    settings: Option<SettingsMenu>,
}

struct ChaptersMenu {
    item_id: String,
    rows: Vec<markers::Chapter>,
    /// The index of the chapter being played, if any.
    current: Option<usize>,
}

struct SettingsMenu {
    adjustments: Adjustments,
    in_group: bool,
    audio: Option<i32>,
    subtitle: SubtitleChoice,
    audio_choices: Vec<Choice>,
    subtitle_choices: Vec<Choice>,
    default_audio: Option<i32>,
    default_subtitle: Option<i32>,
}

#[derive(Clone, Copy, Default)]
struct Menus {
    chapters: bool,
    settings: bool,
}

impl View {
    fn read(state: &NowPlaying, menus: Menus, show_remaining: bool) -> Self {
        let (name, subtitle) = card_text(&state.item);
        let title = if state.item.kind == BaseItemKind::Episode {
            format!("{name} · {subtitle}")
        } else {
            name
        };
        let mut time = match state.duration {
            Some(duration) if show_remaining => format!(
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
        let chapter_marks = match state.duration.filter(|duration| !duration.is_zero()) {
            Some(duration) => state
                .chapters
                .iter()
                .filter(|chapter| !chapter.start.is_zero() && chapter.start < duration)
                .map(|chapter| chapter.start.as_secs_f32() / duration.as_secs_f32())
                .collect(),
            None => Vec::new(),
        };
        let version_name = state
            .versions
            .iter()
            .find(|version| version.id == state.source.id)
            .or_else(|| state.versions.first())
            .map(|version| version.name.clone());
        let next_label = state.next.as_ref().map(|next| {
            let (series, episode) = card_text(next);
            if next.kind == BaseItemKind::Episode {
                episode
            } else {
                series
            }
        });
        Self {
            title,
            paused: state.paused,
            muted: state.muted,
            loading: state.loading,
            position: state.position,
            duration: state.duration,
            volume: state.volume,
            speed: state.adjustments.speed,
            group: state.group_item.is_some(),
            transcoding: state.is_transcoding(),
            time,
            chapter_marks,
            chapter_count: state.chapters.len(),
            previous_tip: state
                .previous
                .as_ref()
                .map(|item| format!("Previous {}(Shift+P)", kind_word(item))),
            next_tip: state
                .next
                .as_ref()
                .map(|item| format!("Next {}(Shift+N)", kind_word(item))),
            versions: state.versions.clone(),
            source_id: state.source.id.clone(),
            version_name,
            skippable: state.skippable(),
            show_up_next: state.show_up_next(),
            next_label,
            remaining: state.duration.map(|duration| duration.saturating_sub(state.position)),
            autoplay: state.autoplay,
            resume: state.resume_prompt(),
            chapters: menus.chapters.then(|| ChaptersMenu {
                item_id: state.item.id.clone(),
                rows: state.chapters.clone(),
                current: markers::current_chapter(&state.chapters, state.position).map(|chapter| chapter.index),
            }),
            settings: menus.settings.then(|| {
                let (audio_choices, subtitle_choices) = tracks::track_choices(&state.source);
                SettingsMenu {
                    adjustments: state.adjustments,
                    in_group: state.group_item.is_some(),
                    audio: state.audio,
                    subtitle: state.subtitle,
                    audio_choices,
                    subtitle_choices,
                    default_audio: state.source.default_audio_stream_index,
                    default_subtitle: state.source.default_subtitle_stream_index,
                }
            }),
        }
    }

    /// The labels at the end of the top bar, in order.
    fn badges(&self) -> Vec<String> {
        let mut badges = Vec::new();
        if self.group {
            badges.push("SyncPlay".to_owned());
        }
        if self.transcoding {
            badges.push("Transcoding".to_owned());
        }
        badges
    }

    /// The version picker's label, when there's a version to switch to.
    fn version_picker(&self) -> Option<&str> {
        (self.versions.len() >= 2).then(|| self.version_name.as_deref().unwrap_or("Version"))
    }
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

/// Which button of a stepper was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Down,
    Up,
    Reset,
}

/// "episode " or "movie " for a button's tooltip, or nothing for other kinds.
fn kind_word(item: &jellyfin::models::BaseItemDto) -> &'static str {
    match item.kind {
        BaseItemKind::Episode => "episode ",
        BaseItemKind::Movie => "movie ",
        _ => "",
    }
}

/// A settings value's step: "0.0 s" at zero, "+0.3 s" and "−0.1 s" otherwise.
fn delay_text(seconds: f64) -> String {
    if seconds.abs() < 0.05 {
        "0.0 s".to_owned()
    } else {
        format!("{seconds:+.1} s")
    }
}

/// The player's settings menu, in egui's popup memory.
fn settings_id() -> Id {
    Id::unique("player_settings")
}

/// The player's chapters menu.
fn chapters_id() -> Id {
    Id::unique("player_chapters")
}

/// The version picker's menu.
fn versions_id() -> Id {
    Id::unique("player_versions")
}

/// The skip button's, up-next card's or resume card's area.
fn prompt_id() -> Id {
    Id::unique("player_prompt")
}

/// `egui::Popup` with a fixed id: clicking the button opens it, or closes it when it's open.
fn toggle_menu(ctx: &egui::Context, id: Id) {
    if egui::Popup::is_id_open(ctx, id) {
        egui::Popup::close_id(ctx, id);
    } else {
        // One menu at a time, as egui's popups keep it.
        egui::Popup::open_id(ctx, id);
    }
    ctx.request_repaint();
}

/// Whether the window is fullscreen.
fn fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.viewport().fullscreen).unwrap_or(false)
}

fn toggle_fullscreen(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen(ctx)));
}

/// A control-bar button: an icon in a square, filled with the accent while its menu is open and
/// half of it under the pointer, as gpui-kit's ghost button is. Not focusable: egui's focus would
/// take every key from the player (`keys.rs`), as U8's dialogs avoid for their buttons.
fn sized_control(ui: &mut egui::Ui, icon: MaterialIcon, size: f32, tooltip: &str, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::CLICK);
    let tokens = theme::tokens_of(ui);
    let fill = if active {
        tokens.accent
    } else if response.hovered() {
        ghost_hover(ui)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(tokens.radius), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon.codepoint,
        FontId::new(size * CONTROL_ICON, icon.font_family()),
        over_video::FOREGROUND,
    );
    response.on_hover_text(tooltip)
}

/// `main`'s ghost button under the pointer: gpui-kit's accent at half strength in the dark theme,
/// which is what the player draws in either theme.
fn ghost_hover(ui: &egui::Ui) -> Color32 {
    let [r, g, b, _] = theme::tokens_of(ui).accent.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, 128)
}

/// A slider's thumb, as gpui-kit draws one: a 16-point circle whose 1-point ring is the bar's
/// colour at half strength around a dark middle.
fn slider_thumb(painter: &egui::Painter, center: egui::Pos2) {
    painter.circle_filled(center, THUMB_R, over_video::THUMB_RING);
    painter.circle_filled(center, THUMB_R - THUMB_RING, over_video::THUMB);
}

fn control_button(ui: &mut egui::Ui, icon: MaterialIcon, tooltip: &str) -> Response {
    sized_control(ui, icon, CONTROL, tooltip, false)
}

/// A button with an icon before its label, in the prompt's cards.
fn icon_text(ui: &egui::Ui, icon: MaterialIcon, text: &str) -> egui::WidgetText {
    let body = TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::default();
    job.append(
        icon.codepoint,
        0.,
        TextFormat {
            font_id: FontId::new(body.size + 4., icon.font_family()),
            color: over_video::FOREGROUND,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {text}"),
        0.,
        TextFormat {
            font_id: body,
            color: over_video::FOREGROUND,
            ..Default::default()
        },
    );
    egui::WidgetText::LayoutJob(job.into())
}

/// A small label in the player's bars (SyncPlay, Transcoding, the speed).
fn label_badge(ui: &mut egui::Ui, text: impl Into<String>) {
    egui::Frame::new()
        .fill(over_video::SURFACE_SUBTLE)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).small().color(over_video::MUTED_FOREGROUND));
        });
}

/// A button of the settings menu's tabs and speeds, and its tracks: selected when it's the
/// current choice, and never focusable.
fn menu_button(ui: &mut egui::Ui, text: impl Into<String>, selected: bool) -> Response {
    let button = egui::Button::new(RichText::new(text.into()).small()).sense(Sense::CLICK);
    ui.add(theme::selected(ui, selected, button))
}

/// The version picker's button: a small select with the playing version's name and a caret, as
/// `main`'s was. Drawn by the page (not an egui widget) so it can't take the keyboard.
fn version_picker(ui: &mut egui::Ui, name: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(VERSION_W, VERSION_H), Sense::CLICK);
    let tokens = theme::tokens_of(ui);
    let painter = ui.painter().clone();
    let fill = if response.hovered() {
        tokens.muted
    } else {
        tokens.input_background()
    };
    painter.rect(
        rect,
        CornerRadius::same(tokens.radius),
        fill,
        Stroke::new(1., tokens.input),
        StrokeKind::Inside,
    );
    let caret = 14.;
    let text = rect.shrink2(vec2(8., 2.));
    let text_w = (text.width() - caret).max(0.);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(text.min, vec2(text_w, text.height())))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.add(egui::Label::new(RichText::new(name).small()).truncate());
    painter.text(
        pos2(rect.right() - 8. - caret / 2., rect.center().y),
        Align2::CENTER_CENTER,
        ICON_ARROW_DROP_DOWN.codepoint,
        FontId::new(caret, ICON_ARROW_DROP_DOWN.font_family()),
        over_video::MUTED_FOREGROUND,
    );
    response.on_hover_text("Version")
}

/// A small heading inside a settings tab.
fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(over_video::MUTED_FOREGROUND));
}

/// A settings value with its step buttons; `None` unless one was pressed.
fn stepper(ui: &mut egui::Ui, label: &str, value: String, down: &str, up: &str) -> Option<Step> {
    let mut step = None;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110., 22.), egui::Label::new(RichText::new(label).small()));
        if push_button(ui, down) {
            step = Some(Step::Down);
        }
        ui.add_sized(
            vec2(56., 22.),
            egui::Label::new(RichText::new(value).small().monospace()),
        );
        if push_button(ui, up) {
            step = Some(Step::Up);
        }
        if push_button(ui, "Reset") {
            step = Some(Step::Reset);
        }
    });
    step
}

fn push_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(text).small()).sense(Sense::CLICK))
        .clicked()
}

/// A settings tab's list of tracks to pick from, the selected one checked. A plain list rather
/// than a select: a select's menu would count as a click outside the settings popup and close
/// it (as on `main`).
fn track_list(ui: &mut egui::Ui, id: &str, empty: &str, choices: &[Choice], selected: Option<i32>) -> Option<i32> {
    if choices.is_empty() {
        ui.label(RichText::new(empty).small().color(over_video::MUTED_FOREGROUND));
        return None;
    }
    // Without a track saved or a default, the first one plays.
    let selected = selected
        .filter(|value| choices.iter().any(|choice| choice.value == *value))
        .unwrap_or(choices[0].value);
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(TRACK_ROW_H * TRACK_ROWS)
        .show(ui, |ui| {
            for choice in choices {
                if check_row(ui, &choice.label, choice.value == selected).clicked() {
                    chosen = Some(choice.value);
                }
            }
        });
    chosen
}

/// A track row: a check in a fixed slot (so the labels line up), then the name.
fn check_row(ui: &mut egui::Ui, label: &str, selected: bool) -> Response {
    let body = TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.append(
        ICON_CHECK.codepoint,
        0.,
        TextFormat {
            font_id: crate::theme::text::icon(ui, ICON_CHECK.font_family()),
            color: if selected { color } else { Color32::TRANSPARENT },
            ..Default::default()
        },
    );
    job.append(
        &format!("  {label}"),
        0.,
        TextFormat {
            font_id: body,
            color,
            ..Default::default()
        },
    );
    ui.add_sized(
        vec2(ui.available_width(), TRACK_ROW_H),
        theme::selected(
            ui,
            selected,
            egui::Button::new(egui::WidgetText::LayoutJob(job.into())).sense(Sense::CLICK),
        ),
    )
}

/// The text's width at the small style's size, for laying out the top bar.
fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = TextStyle::Small.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// A vertical gradient behind a bar, from `top` at its top edge to `bottom` at its bottom.
fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The mascot in the middle of the video, until the first frame has loaded.
fn loader(ui: &mut egui::Ui, rect: Rect) {
    let loader = Rect::from_center_size(rect.center(), Vec2::splat(LOADER_SIZE));
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(loader)
            .layout(Layout::top_down(Align::Center)),
    );
    mascot::loader(&mut child, LOADER_SIZE);
}

/// The time `x` along `rect` names, clamped to the bar's ends.
fn time_at(rect: Rect, duration: Duration, x: f32) -> Duration {
    if rect.width() <= 0. {
        return Duration::ZERO;
    }
    let fraction = ((x - rect.left()) / rect.width()).clamp(0., 1.);
    Duration::from_secs_f64(fraction as f64 * duration.as_secs_f64())
}

/// egui's wheel delta in wheel notches, positive upwards. One `Line` is one notch: X11,
/// Windows and Wayland all normalise a notch to it (where `main`'s GPUI gave three lines).
/// Pixel deltas are a trackpad's, at `PIXELS_PER_NOTCH` points per notch; a page counts as one.
fn wheel_notches(unit: egui::MouseWheelUnit, delta: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Line => delta,
        egui::MouseWheelUnit::Point => delta / PIXELS_PER_NOTCH,
        egui::MouseWheelUnit::Page => delta,
    }
}

/// Turns wheel notches (positive is scrolling up) into whole volume steps, one notch per step
/// as `main` had it, keeping the leftover notches for the next event. One event makes at most
/// `MAX_STEPS_PER_SCROLL`.
pub fn scroll_steps(pending: f32, notches: f32) -> (f32, i32) {
    let total = (pending + notches).clamp(-MAX_STEPS_PER_SCROLL, MAX_STEPS_PER_SCROLL);
    let steps = total.trunc() as i32;
    (total - steps as f32, steps)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        MAX_STEPS_PER_SCROLL, PIXELS_PER_NOTCH, PlayerPage, SEEK_STEP, VOLUME_STEP, delay_text, scroll_steps, time_at,
        wheel_notches,
    };

    #[test]
    fn the_controls_are_up_on_the_players_first_frame() {
        // `new` reads the player's settings from the config, so it needs a directory of its own.
        let _config = crate::ui_tests::ConfigDir::new("player-first-frame");
        let page = PlayerPage::new();
        // The page opens with them up (and the timer running), so the first session shows them
        // on its first frame; `controls_appeared` is what starts their fade there.
        assert!(page.controls_visible);
        assert!(page.controls_appeared);
        assert!(
            page.controls_since.is_none(),
            "the fade starts on the first frame drawn"
        );
    }

    // `main`'s input gave three lines per notch and its tests counted those lines; egui gives
    // one line per notch, so the same tests count notches.

    #[test]
    fn a_wheel_notch_is_one_volume_step() {
        let (pending, steps) = scroll_steps(0., 1.);
        assert_eq!(steps, 1);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn one_egui_notch_is_one_volume_step() {
        // What egui-winit reports for a notch on X11, Windows and Wayland.
        let notch = wheel_notches(egui::MouseWheelUnit::Line, 1.);
        assert_eq!(notch, 1.);
        assert_eq!(scroll_steps(0., notch), (0., 1));
        // A macOS trackpad's 90 points are one notch too, and its smaller deltas add up.
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Point, PIXELS_PER_NOTCH), 1.);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Point, PIXELS_PER_NOTCH / 2.), 0.5);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Page, 1.), 1.);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Line, -1.), -1.);
    }

    #[test]
    fn a_fast_flick_keeps_every_step() {
        let (pending, steps) = scroll_steps(0., 4.);
        assert_eq!(steps, 4);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn small_deltas_add_up() {
        let (pending, steps) = scroll_steps(0., 0.5);
        assert_eq!((pending, steps), (0.5, 0));
        let (pending, steps) = scroll_steps(pending, 0.5);
        assert_eq!((pending, steps), (0., 1));
    }

    #[test]
    fn scrolling_down_is_negative() {
        let (pending, steps) = scroll_steps(0., -2.);
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
        let (pending, steps) = scroll_steps(0., -0.5);
        assert_eq!(steps, 0);
        let (_, steps) = scroll_steps(pending, 0.5);
        assert_eq!(steps, 0);
    }

    #[test]
    fn delays_read_as_steps() {
        assert_eq!(delay_text(0.), "0.0 s");
        assert_eq!(delay_text(0.3), "+0.3 s");
        assert_eq!(delay_text(-0.1), "-0.1 s");
    }

    #[test]
    fn a_click_on_the_seek_bar_names_that_time() {
        let bar = egui::Rect::from_min_size(egui::pos2(10., 0.), egui::vec2(200., 20.));
        let duration = Duration::from_secs(100);
        assert_eq!(time_at(bar, duration, 10.), Duration::ZERO);
        assert_eq!(time_at(bar, duration, 110.), Duration::from_secs(50));
        assert_eq!(time_at(bar, duration, 210.), duration);
        // Dragged past either end: the first or last moment.
        assert_eq!(time_at(bar, duration, -50.), Duration::ZERO);
        assert_eq!(time_at(bar, duration, 999.), duration);
        assert_eq!(time_at(egui::Rect::NOTHING, duration, 10.), Duration::ZERO);
    }

    #[test]
    fn the_seek_and_volume_steps_are_the_ones_remote_control_uses() {
        assert_eq!(SEEK_STEP, 10.);
        assert_eq!(VOLUME_STEP, 5.);
    }
}
