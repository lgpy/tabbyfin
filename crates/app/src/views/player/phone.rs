//! The player's controls on a touch screen: a phone in landscape, full screen (docs/android.md,
//! *Touch UI*; docs/DESIGN.md has the layout).
//!
//! Along the top, Back with the title and the episode line beside it, and at the right the
//! chapters, a subtitles toggle and the settings. In the middle, play/pause on a soft disc with
//! the 10-second skips beside it and the previous and next episode further out. Along the
//! bottom, the clock over the seek bar's left end, and the seek bar across the safe area. The
//! menus open as a side sheet at the right ([`Sheet`]): the settings menu's tabs and rows, with
//! the versions under General, or the chapters.
//!
//! The video runs edge to edge, under the display cutout and the gesture bar; the controls keep
//! to the safe area, inset as far on both sides so they stay centred on the picture
//! ([`safe_area`]).

use egui::{
    Align, Align2, CornerRadius, Id, Layout, Rect, RichText, ScrollArea, Sense, Stroke, UiBuilder, Vec2, pos2, vec2,
};
use egui_material_icons::icons::{
    ICON_ARROW_BACK, ICON_CLOSE, ICON_CLOSED_CAPTION, ICON_FORWARD_10, ICON_PAUSE, ICON_PLAY_ARROW, ICON_REPLAY_10,
    ICON_SKIP_NEXT, ICON_SKIP_PREVIOUS, ICON_TUNE, ICON_VIEW_LIST,
};
use jellyfin::ImageRequest;
use jellyfin::models::ImageType;

use super::{
    PlayerPage, SEEK_STEP, SettingsChoice, SettingsTab, View, chapter_colours, control, gradient, label_badge,
    settings_body,
};
use crate::playback::{PlayOptions, SubtitleChoice, UiCommand};
use crate::theme::{self, over_video};
use crate::views::PageCx;
use crate::widgets::popup::{self, Entry};
use crate::widgets::text::timestamp;
use crate::widgets::{label, media_row};

/// Between the safe area's edge and the controls.
const PAD: f32 = 12.;
/// The bars' buttons, and the middle's: play/pause, the skips and the episodes.
const BUTTON: f32 = 44.;
const PLAY: f32 = 72.;
const SKIP: f32 = 56.;
const EPISODE: f32 = 48.;
/// How far from the middle the skips and the episodes sit, on a phone wide enough.
const SKIP_DX: f32 = 112.;
const EPISODE_DX: f32 = 212.;
/// The seek bar's row: tall enough for a finger, its track in the middle.
const SEEK_ROW: f32 = 40.;
/// The clock's row over the seek bar.
const CLOCK_ROW: f32 = 28.;
/// The prompt's distance above the safe area's bottom while the controls show: over the clock.
pub(super) const PROMPT_ABOVE: f32 = SEEK_ROW + CLOCK_ROW + 12.;
/// The up-next card's room inside, and its name's column beside the buttons: one row, low
/// enough to fit between the middle's buttons and the clock.
pub(super) const UP_NEXT_PAD: f32 = 8.;
pub(super) const UP_NEXT_NAME_W: f32 = 160.;
/// The sheet's width, the room inside it and its outer corners.
const SHEET_W: f32 = 360.;
const SHEET_PAD: f32 = 16.;
const SHEET_RADIUS: u8 = 16;

/// A phone's menu, open as a side sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sheet {
    /// The settings menu's tabs: audio, subtitles, and the speed and versions.
    Settings,
    Chapters,
}

/// Where the controls go in `window` when the page lays out in `content` (inside the system's
/// bars and the cutout): the larger of the two side insets on both sides, so the controls stay
/// centred on the picture, and [`PAD`] inside that.
pub(super) fn safe_area(window: Rect, content: Rect) -> Rect {
    let side = (content.left() - window.left()).max(window.right() - content.right());
    Rect::from_min_max(
        pos2(window.left() + side + PAD, content.top() + PAD),
        pos2(window.right() - side - PAD, content.bottom() - PAD),
    )
}

fn sheet_id() -> Id {
    Id::unique("player_sheet")
}

impl PlayerPage {
    /// The controls over the video, in `safe` inside the whole `window`. Returns the top and
    /// bottom bars, whose taps are theirs rather than the video's.
    pub(super) fn phone_controls(
        &mut self,
        ui: &mut egui::Ui,
        view: &View,
        cx: &mut PageCx,
        window: Rect,
        safe: Rect,
    ) -> [Rect; 2] {
        let painter = ui.painter();
        painter.rect_filled(window, 0., over_video::VEIL);
        let top_band = Rect::from_min_max(window.min, pos2(window.right(), safe.top() + BUTTON + 32.));
        gradient(painter, top_band, over_video::SCRIM, over_video::CLEAR);
        let bottom_band = Rect::from_min_max(
            pos2(window.left(), safe.bottom() - SEEK_ROW - CLOCK_ROW - 48.),
            window.max,
        );
        gradient(painter, bottom_band, over_video::CLEAR, over_video::SCRIM_STRONG);
        let top = Rect::from_min_max(window.min, pos2(window.right(), safe.top() + BUTTON));
        let bottom = Rect::from_min_max(pos2(window.left(), safe.bottom() - SEEK_ROW - CLOCK_ROW), window.max);
        // The bars take the taps meant for the video under them; their buttons, drawn after,
        // still get theirs.
        ui.interact(top, Id::unique("player_top_bar"), Sense::CLICK);
        ui.interact(bottom, Id::unique("player_controls"), Sense::CLICK);
        self.phone_top(ui, view, cx, Rect::from_min_size(safe.min, vec2(safe.width(), BUTTON)));
        self.phone_middle(ui, view, cx, pos2(safe.center().x, window.center().y), safe.width());
        self.phone_bottom(ui, view, cx, safe);
        [top, bottom]
    }

    /// Back, the title and the episode line; at the right the chapters, subtitles and settings,
    /// and the SyncPlay and transcoding badges before them.
    fn phone_top(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, row: Rect) {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(row)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.;
                let settings_open = self.sheet == Some(Sheet::Settings);
                if control(ICON_TUNE, BUTTON, "Settings")
                    .on(settings_open)
                    .show(ui)
                    .clicked()
                {
                    self.toggle_sheet(Sheet::Settings);
                }
                if view.subtitles_button {
                    let on = view.subtitle_shown.is_some();
                    let subtitles = control(ICON_CLOSED_CAPTION, BUTTON, "Subtitles").on(on).show(ui);
                    if subtitles.clicked() {
                        self.toggle_subtitles(view, cx);
                    }
                    // A long press picks which: the settings on their Subtitles tab.
                    if subtitles.long_touched() || subtitles.secondary_clicked() {
                        self.settings_tab = SettingsTab::Subtitles;
                        self.sheet = Some(Sheet::Settings);
                    }
                }
                if view.chapter_count >= 2 {
                    let open = self.sheet == Some(Sheet::Chapters);
                    if control(ICON_VIEW_LIST, BUTTON, "Chapters").on(open).show(ui).clicked() {
                        self.toggle_sheet(Sheet::Chapters);
                    }
                }
                for name in view.badges().iter().rev() {
                    label_badge(ui, name);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.;
                    if control(ICON_ARROW_BACK, BUTTON, "Back (Esc)").show(ui).clicked() {
                        self.exit(cx);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        let title = RichText::new(&view.title)
                            .font(theme::text::font(ui, "lg"))
                            .color(over_video::FOREGROUND);
                        ui.add(egui::Label::new(title).truncate().selectable(false));
                        if let Some(aside) = &view.title_aside {
                            let aside = RichText::new(aside)
                                .font(theme::text::font(ui, "sm"))
                                .color(over_video::MUTED_FOREGROUND);
                            ui.add(egui::Label::new(aside).truncate().selectable(false));
                        }
                    });
                });
            },
        );
    }

    /// Play/pause in the middle, the skips beside it and the previous and next episode further
    /// out, closer in on a narrow screen.
    fn phone_middle(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, center: egui::Pos2, width: f32) {
        let scale = (width / 2. / (EPISODE_DX + EPISODE / 2. + 8.)).min(1.);
        ui.painter()
            .circle_filled(center, PLAY / 2., over_video::SURFACE_SUBTLE);
        let (icon, tip) = if view.paused {
            (ICON_PLAY_ARROW, "Play (Space)")
        } else {
            (ICON_PAUSE, "Pause (Space)")
        };
        if control(icon, PLAY, tip)
            .show_at(ui, center, Id::unique("player_play"))
            .clicked()
        {
            cx.services.control(UiCommand::TogglePause);
            self.show_controls();
        }
        let skips = [
            (-1., ICON_REPLAY_10, "Back 10 s (Left)", -SEEK_STEP),
            (1., ICON_FORWARD_10, "Forward 10 s (Right)", SEEK_STEP),
        ];
        for (side, icon, tip, by) in skips {
            let at = center + vec2(side * SKIP_DX * scale, 0.);
            if control(icon, SKIP, tip).show_at(ui, at, Id::unique(tip)).clicked() {
                cx.services.control(UiCommand::SeekBy(by));
                self.show_controls();
            }
        }
        if let Some(tip) = &view.previous_tip {
            let at = center - vec2(EPISODE_DX * scale, 0.);
            if control(ICON_SKIP_PREVIOUS, EPISODE, tip)
                .show_at(ui, at, Id::unique("player_previous"))
                .clicked()
            {
                cx.services.play_previous();
            }
        }
        if let Some(tip) = &view.next_tip {
            let at = center + vec2(EPISODE_DX * scale, 0.);
            if control(ICON_SKIP_NEXT, EPISODE, tip)
                .show_at(ui, at, Id::unique("player_next"))
                .clicked()
            {
                cx.services.play_next();
            }
        }
    }

    /// The clock (played and total, or what's left; a tap switches them) with the speed when
    /// it isn't 1×, over the seek bar.
    fn phone_bottom(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, safe: Rect) {
        let seek = Rect::from_min_max(pos2(safe.left(), safe.bottom() - SEEK_ROW), safe.max);
        let clock_row = Rect::from_min_max(
            pos2(safe.left(), seek.top() - CLOCK_ROW),
            pos2(safe.right(), seek.top()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(clock_row)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.;
                let clock = ui.add(
                    egui::Label::new(RichText::new(&view.time).font(theme::text::font(ui, "sm")))
                        .sense(Sense::CLICK)
                        .selectable(false),
                );
                if crate::layout::grow(ui, clock, Sense::CLICK).clicked() {
                    self.toggle_remaining();
                }
                if (view.speed - 1.).abs() > 1e-6 {
                    label_badge(ui, &format!("{}×", view.speed));
                }
            },
        );
        self.seek_bar(ui, view, cx, seek);
    }

    /// Opens the settings sheet, for a mockup to show it.
    #[cfg(all(test, feature = "mockup"))]
    pub(crate) fn open_settings_sheet(&mut self) {
        self.sheet = Some(Sheet::Settings);
        self.settings_tab = SettingsTab::Audio;
    }

    /// Opens `sheet`, or closes it when it's the one open.
    fn toggle_sheet(&mut self, sheet: Sheet) {
        self.sheet = if self.sheet == Some(sheet) { None } else { Some(sheet) };
        self.show_controls();
    }

    /// Turns subtitles off, or back on: the track last turned off, else the default one.
    fn toggle_subtitles(&mut self, view: &View, cx: &mut PageCx) {
        let choice = match view.subtitle_shown {
            Some(index) => {
                self.last_subtitle = Some(index);
                SubtitleChoice::Off
            }
            None => match self.last_subtitle.or(view.subtitle_default) {
                Some(index) => SubtitleChoice::Stream(index),
                None => return,
            },
        };
        cx.services.change_subtitle(choice);
        self.show_controls();
    }

    /// The open sheet: the full height at the right, the picture dimmed beside it. A tap beside
    /// it, its close button or Back closes it.
    pub(super) fn phone_sheet(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx) {
        let Some(sheet) = self.sheet else { return };
        let content = ui.ctx().content_rect();
        let height = (content.height() - 2. * SHEET_PAD).max(0.);
        let frame = egui::Frame::NONE
            .fill(over_video::MENU)
            .stroke(Stroke::new(1., over_video::MENU_BORDER))
            .corner_radius(CornerRadius {
                nw: SHEET_RADIUS,
                sw: SHEET_RADIUS,
                ne: 0,
                se: 0,
            })
            .inner_margin(SHEET_PAD as i8);
        let area = egui::Modal::default_area(sheet_id()).anchor(Align2::RIGHT_TOP, Vec2::ZERO);
        let mut tab = self.settings_tab;
        let mut close = false;
        let mut choice = SettingsChoice::default();
        let mut seek_to = None;
        let mut version = None;
        let modal = egui::Modal::new(sheet_id())
            .area(area)
            .frame(frame)
            .backdrop_color(over_video::SHEET_BACKDROP)
            .show(ui.ctx(), |ui| {
                ui.style_mut().visuals = egui::Visuals::dark();
                ui.style_mut().visuals.override_text_color = Some(over_video::FOREGROUND);
                ui.set_width(SHEET_W - 2. * SHEET_PAD);
                ui.set_height(height);
                let title = match (sheet, &view.settings) {
                    (Sheet::Settings, Some(menu)) => {
                        tab = menu.tab(tab);
                        tab.label()
                    }
                    (Sheet::Settings, None) => "Settings",
                    (Sheet::Chapters, _) => "Chapters",
                };
                ui.horizontal(|ui| {
                    let title = RichText::new(title)
                        .font(theme::text::font(ui, "lg"))
                        .color(over_video::FOREGROUND);
                    ui.add(egui::Label::new(title).selectable(false));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if control(ICON_CLOSE, 36., "Close").show(ui).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.);
                ScrollArea::vertical()
                    .id_salt("player_sheet_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match sheet {
                        Sheet::Settings => {
                            let Some(menu) = &view.settings else { return };
                            choice = settings_body(ui, menu, menu.tabs(), &mut tab);
                            if tab == SettingsTab::General && view.versions.len() >= 2 {
                                version = versions(ui, view);
                            }
                        }
                        Sheet::Chapters => {
                            seek_to = chapters(ui, view, cx);
                        }
                    });
            });
        self.settings_tab = tab;
        if modal.should_close() || close {
            self.sheet = None;
        }
        self.apply_settings(choice, cx);
        if let Some(start) = seek_to {
            cx.services.control(UiCommand::SeekTo(start));
            self.sheet = None;
        }
        if let Some(id) = version {
            cx.services.reload(PlayOptions {
                media_source_id: Some(id),
                ..Default::default()
            });
            self.sheet = None;
        }
    }
}

/// The versions under the General tab, the playing one ticked. Returns the one picked.
fn versions(ui: &mut egui::Ui, view: &View) -> Option<String> {
    ui.add_space(8.);
    popup::separator_in(ui, over_video::MENU_BORDER);
    ui.add_space(8.);
    ui.label(label::overline(ui, "Version").color(over_video::MUTED_FOREGROUND));
    let mut picked = None;
    for version in &view.versions {
        let entry = Entry::new(version.name.as_str())
            .checked(version.id == view.source_id)
            .tint(over_video::BRAND);
        if popup::row(ui, &entry, true, false, over_video::ROW_HOVER).clicked() && version.id != view.source_id {
            picked = Some(version.id.clone());
        }
    }
    picked
}

/// The chapters as the desktop's menu has them: each one's image, name and start, the current
/// one tinted. Returns where the one picked starts.
fn chapters(ui: &mut egui::Ui, view: &View, cx: &mut PageCx) -> Option<std::time::Duration> {
    let menu = view.chapters.as_ref()?;
    ui.spacing_mut().item_spacing.y = 2.;
    let mut picked = None;
    for chapter in &menu.rows {
        let request = chapter.image_tag.as_ref().map(|tag| ImageRequest {
            index: Some(chapter.index as u32),
            ..ImageRequest::new(&menu.item_id, ImageType::Chapter, Some(tag.clone()))
                .fill_width((media_row::CHAPTER_THUMB_W * 2.) as u32)
        });
        let name = chapter
            .name
            .clone()
            .unwrap_or_else(|| format!("Chapter {}", chapter.index + 1));
        let current = menu.current == Some(chapter.index);
        let row = media_row::chapter(
            ui,
            &mut cx.services.images,
            request.as_ref(),
            &name,
            &timestamp(chapter.start),
            current,
            chapter_colours(),
        );
        if row.clicked() {
            picked = Some(chapter.start);
        }
    }
    picked
}
