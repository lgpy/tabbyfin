//! The player page while casting: a remote for the TV rather than a player (`docs/chromecast.md`,
//! *UI*).
//!
//! There's no picture on this screen, so the page drops the player's forced dark and draws in the
//! app's own theme: the backdrop and its glow, `theme::tokens_of`'s colours, the app's text
//! sizes. On the desktop it's one centred column — the item's backdrop as a 16:9 card, its
//! title and meta line, the seek bar with its times, the transport, the TV's volume and a chip
//! per track choice. On a phone in portrait the backdrop runs across the top and fades into a
//! sheet under it, with the transport over a wide play button and the track choices as full-width
//! rows. A phone in landscape gets the desktop's column, inside its safe area.
//!
//! Every control is the cast's: `Services::control` sends pause, seeks, the TV's volume and mute
//! to the receiver, `play_next` and `play_previous` skip its queue, and the chips and rows open
//! the player's own menus (`settings_menu`, `chapters_menu`, `versions_menu`) or the phone's
//! sheets (`phone::Sheet`). The receiver has no speed, delay, subtitle size or fullscreen, so
//! none of those are here, and the controls never hide: there's no video to watch behind them.
//!
//! Like the rest of the player, the widgets take no keyboard focus (`Sense::CLICK`, `Focus::Skip`)
//! and name themselves to AccessKit from the `player-*` messages, which is how the tests find
//! them. The page's keys stay `casting_key`'s.

use egui::accesskit::Role;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ARROW_BACK, ICON_AUDIOTRACK, ICON_CAST_CONNECTED, ICON_CLOSED_CAPTION, ICON_EXPAND_MORE, ICON_PAUSE,
    ICON_PLAY_ARROW, ICON_SKIP_NEXT, ICON_SKIP_PREVIOUS, ICON_TV, ICON_VIEW_LIST, ICON_VOLUME_OFF, ICON_VOLUME_UP,
};
use jellyfin::models::BaseItemKind;

use super::{
    CastView, Menus, PlayerPage, SettingsTab, View, chapters_id, loader, phone, seek_icons, settings_id, time_at,
    touch, versions_id,
};
use crate::i18n::tr;
use crate::playback::UiCommand;
use crate::theme;
use crate::views::PageCx;
use crate::widgets::backdrop;
use crate::widgets::button::{self, Focus, State};
use crate::widgets::image;
use crate::widgets::popup::{Opened, PopupMenu};
use crate::widgets::round_button::{self, RoundButton};
use crate::widgets::text::timestamp;

/// The desktop column's width (the seek bar's), and the card's, at full size.
const COLUMN_W: f32 = 560.;
const CARD_W: f32 = 480.;
/// The seek bar's row: the track and, under it, the elapsed and remaining times.
const SEEK_ROW_H: f32 = 40.;
/// A chip's height, and the volume block's width beside them.
const CHIP_H: f32 = 36.;
const VOLUME_W: f32 = 280.;
/// The top row's pill.
const PILL_H: f32 = 34.;
/// The desktop column at full size: from the window's top to its last row. Other window sizes
/// scale it down so it stays whole.
const COLUMN_H: f32 = 760.;
/// Where the column starts, and the card, at full size.
const COLUMN_TOP: f32 = 90.;
/// The phone's backdrop across the top, the sheet's top as a share of the window, and its
/// corners.
const HERO_SHARE: f32 = 0.43;
const SHEET_SHARE: f32 = 0.33;
const SHEET_RADIUS: u8 = 28;
/// The tracks' rows on the phone, and the Stop casting button under them.
const ROW_H: f32 = 48.;
const STOP_H: f32 = 42.;
/// The phone content at full size, to scale a short screen down to.
const PHONE_H: f32 = 560.;

impl PlayerPage {
    /// The page while casting: the controller, in the app's own theme (there's no video to keep
    /// dark for).
    pub(super) fn show_casting(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let ctx = ui.ctx().clone();
        ui.style_mut().visuals = ctx.style_of(ctx.theme()).visuals.clone();
        let menus = Menus {
            chapters: egui::Popup::is_id_open(&ctx, chapters_id()) || self.sheet == Some(phone::Sheet::Chapters),
            settings: egui::Popup::is_id_open(&ctx, settings_id()) || self.sheet == Some(phone::Sheet::Settings),
        };
        let seek_step = cx.services.playback.seek_step();
        let Some(session) = cx.services.cast.session.as_ref() else {
            return;
        };
        let view = View::read_cast(
            &session.casting,
            &session.device,
            session.reconnecting.is_some(),
            menus,
            seek_step,
        );
        let window = ui.max_rect();
        if touch::on(&ctx) {
            let safe = phone::safe_area(window, ctx.content_rect());
            if window.height() >= window.width() {
                self.cast_phone(ui, &view, cx, window, safe);
            } else {
                // A phone on its side has the room for the desktop's column, inside its safe area.
                self.cast_desktop(ui, &view, cx, safe);
            }
            self.phone_sheet(ui, &view, cx);
        } else {
            self.cast_desktop(ui, &view, cx, window);
        }
        // A scrub whose release never reached the bar (the pointer left the window) mustn't
        // leave the bar at the pointer's last time for ever.
        if self.scrub.is_some() && !ui.input(|input| input.pointer.any_down()) {
            self.scrub = None;
        }
        self.scroll_volume(ui, false, cx);
    }

    /// The desktop's controller: one column in the middle, the TV named at the top.
    fn cast_desktop(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, bounds: Rect) {
        let ctx = ui.ctx().clone();
        let tokens = theme::tokens_of(ui);
        backdrop::fill(ui.painter(), ui.max_rect(), tokens);
        backdrop::glow(ui.painter(), pos2(bounds.center().x, bounds.top() + 200.), 420., tokens);
        self.cast_top(ui, view, cx, bounds, false);

        let cast = view.cast.clone().unwrap_or_default();
        let scale = ((bounds.height() - 24.) / COLUMN_H)
            .min((bounds.width() - 48.) / COLUMN_W)
            .clamp(0.5, 1.);
        let column_w = COLUMN_W * scale;
        let top = bounds.top() + COLUMN_TOP * scale;
        let column_rect = Rect::from_min_size(
            pos2(bounds.center().x - column_w / 2., top),
            vec2(column_w, (bounds.bottom() - top).max(0.)),
        );
        let mut column = ui.new_child(
            UiBuilder::new()
                .max_rect(column_rect)
                .layout(Layout::top_down(Align::Center)),
        );
        column.spacing_mut().item_spacing = Vec2::ZERO;

        // The item's backdrop as a 16:9 card, with its shadow.
        let card = Rect::from_min_size(
            column_rect.center_top(),
            vec2(CARD_W * scale, CARD_W * scale * 9. / 16.),
        );
        let (card, _) = column.allocate_exact_size(card.size(), Sense::hover());
        column.painter().add(
            egui::Shadow {
                offset: [0, (10. * scale).round() as i8],
                blur: (36. * scale).round() as u8,
                spread: 0,
                color: tokens.surface_shadow,
            }
            .as_shape(card, CornerRadius::same(tokens.radius_surface)),
        );
        image::show(
            &column,
            &mut cx.services.images,
            card,
            cast.backdrop.as_ref(),
            cast.item.as_ref(),
            tokens.radius_surface,
        );
        // Until the TV reports the item playing.
        if view.loading {
            loader(&mut column, card);
        }

        column.add_space(24. * scale);
        let title = RichText::new(cast_title(view))
            .font(theme::text::font(&column, "2xl"))
            .color(tokens.ink);
        column.add(egui::Label::new(title).truncate().selectable(false));
        let meta = cast_meta(&cast);
        if !meta.is_empty() {
            column.add_space(4. * scale);
            let meta = RichText::new(meta)
                .font(theme::text::font(&column, "sm"))
                .color(tokens.ink_muted);
            column.add(egui::Label::new(meta).truncate().selectable(false));
        }

        column.add_space(28. * scale);
        let (seek, _) = column.allocate_exact_size(vec2(column_w, SEEK_ROW_H * scale), Sense::hover());
        self.cast_seek(&mut column, view, cx, seek, scale, cast.can_seek);

        column.add_space(32. * scale);
        let (transport, _) = column.allocate_exact_size(vec2(column_w, 68. * scale), Sense::hover());
        cast_transport(&mut column, view, cx, transport, scale, (68., 26.), cast.can_seek);

        column.add_space(24. * scale);
        let chips = cast_chips(view);
        let chips_w: f32 = chips
            .iter()
            .map(|(_, value, _)| chip_width(&column, value))
            .sum::<f32>()
            + (chips.len().saturating_sub(1) as f32) * 8.;
        let volume_w = VOLUME_W * scale;
        let room = (bounds.width() - 48.).max(0.);
        let mut responses = ChipResponses::default();
        if volume_w + 32. * scale + chips_w <= room {
            // The TV's volume and the chips on one row, centred as one block (the mockup's).
            let width = volume_w + 32. * scale + chips_w;
            let top = column.cursor().top();
            let row = Rect::from_min_size(
                pos2(column_rect.center().x - width / 2., top),
                vec2(width, CHIP_H * scale),
            );
            column.allocate_exact_size(vec2(column_w, CHIP_H * scale), Sense::hover());
            self.volume_at(
                ui,
                view,
                cx,
                Rect::from_min_size(row.min, vec2(volume_w, row.height())),
                scale,
            );
            let chips_rect = Rect::from_min_max(pos2(row.left() + volume_w + 32. * scale, row.top()), row.max);
            self.chip_row(ui, chips_rect, scale, &chips, &mut responses);
        } else {
            // Too narrow for one row: the volume, then the chips under it.
            let top = column.cursor().top();
            self.volume_at(
                ui,
                view,
                cx,
                Rect::from_min_size(
                    pos2(column_rect.center().x - volume_w / 2., top),
                    vec2(volume_w, 32. * scale),
                ),
                scale,
            );
            column.allocate_exact_size(vec2(column_w, 32. * scale + 8. * scale), Sense::hover());
            let width = chips_w.min(room);
            let top = column.cursor().top();
            let chips_rect = Rect::from_min_size(
                pos2(column_rect.center().x - width / 2., top),
                vec2(width, CHIP_H * scale),
            );
            column.allocate_exact_size(vec2(column_w, CHIP_H * scale), Sense::hover());
            self.chip_row(ui, chips_rect, scale, &chips, &mut responses);
        }

        // The menus hang from their chips. The settings menu keeps to the tab it was opened
        // on, whichever chip that is.
        let anchor = match self.settings_tab {
            SettingsTab::Audio => responses.audio.clone(),
            SettingsTab::Subtitles => responses.subtitle.clone(),
            SettingsTab::General => None,
        };
        if let Some(anchor) = anchor.or(responses.audio.clone()).or(responses.subtitle.clone()) {
            self.settings_menu(&anchor, view, cx);
        }
        if let Some(chapters) = &responses.chapters {
            self.chapters_menu(chapters, view, cx);
        }
        if let Some(versions) = &responses.version {
            self.versions_menu(versions, view, cx);
        }
        // After the menus: a click on a chip is a click outside the open menu, which closes it
        // just above.
        if responses.reopen {
            egui::Popup::open_id(&ctx, settings_id());
            ctx.request_repaint();
        }
    }

    /// The TV's volume in a row of its own, inside `rect`.
    fn volume_at(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, rect: Rect, scale: f32) {
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing = Vec2::ZERO;
        self.cast_volume(&mut row, view, cx, rect.width(), scale);
    }

    /// The track chips in `rect`, from its left. What a chip's click opens is handled here; the
    /// responses are kept for their menus.
    fn chip_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        scale: f32,
        chips: &[(MaterialIcon, String, Chip)],
        responses: &mut ChipResponses,
    ) {
        let ctx = ui.ctx().clone();
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing = Vec2::ZERO;
        for (icon, value, kind) in chips {
            let response = chip(&mut row, *icon, value, scale);
            if response.clicked() {
                match kind {
                    Chip::Audio => responses.reopen = self.toggle_settings(&ctx, SettingsTab::Audio),
                    Chip::Subtitles => responses.reopen = self.toggle_settings(&ctx, SettingsTab::Subtitles),
                    Chip::Chapters => toggle_menu(&ctx, chapters_id()),
                    Chip::Version => PopupMenu::toggle(&ctx, versions_id(), Opened::Pointer),
                }
            }
            match kind {
                Chip::Audio => responses.audio = Some(response),
                Chip::Subtitles => responses.subtitle = Some(response),
                Chip::Chapters => responses.chapters = Some(response),
                Chip::Version => responses.version = Some(response),
            }
            row.add_space(8.);
        }
    }

    /// The phone's controller: the backdrop across the top, fading into a sheet with everything
    /// else, laid out for a thumb.
    fn cast_phone(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, window: Rect, safe: Rect) {
        let tokens = theme::tokens_of(ui);
        backdrop::fill(ui.painter(), window, tokens);
        let cast = view.cast.clone().unwrap_or_default();
        let hero = Rect::from_min_size(window.min, vec2(window.width(), window.height() * HERO_SHARE));
        image::show(
            ui,
            &mut cx.services.images,
            hero,
            cast.backdrop.as_ref(),
            cast.item.as_ref(),
            0,
        );
        let lower = Rect::from_min_max(pos2(hero.left(), hero.top() + hero.height() * 0.44), hero.max);
        ui.painter().add(backdrop::fade_into(lower, window, 0., tokens));
        if view.loading {
            loader(ui, hero);
        }
        self.cast_top(ui, view, cx, window, true);

        // The sheet, from about a third down.
        let sheet = Rect::from_min_max(
            pos2(window.left(), window.top() + window.height() * SHEET_SHARE),
            window.max,
        );
        ui.painter().rect(
            sheet,
            CornerRadius {
                nw: SHEET_RADIUS,
                ne: SHEET_RADIUS,
                sw: 0,
                se: 0,
            },
            tokens.surface,
            Stroke::new(1., tokens.surface_border),
            StrokeKind::Inside,
        );
        let side = (safe.left() - window.left() + 12.).max(16.);
        let inner_w = (window.width() - 2. * side).max(0.);
        let scale = (sheet.height() / PHONE_H).min(1.);
        let content = Rect::from_min_size(pos2(window.left() + side, sheet.top()), vec2(inner_w, sheet.height()));
        let mut sheet_ui = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)));
        sheet_ui.spacing_mut().item_spacing = Vec2::ZERO;
        sheet_ui.add_space(28. * scale);
        let title = RichText::new(cast_title(view))
            .font(theme::text::font(&sheet_ui, "xl"))
            .color(tokens.ink);
        sheet_ui.add(egui::Label::new(title).truncate().selectable(false));
        let meta = cast_meta(&cast);
        if !meta.is_empty() {
            sheet_ui.add_space(2. * scale);
            let meta = RichText::new(meta)
                .font(theme::text::font(&sheet_ui, "sm"))
                .color(tokens.ink_muted);
            sheet_ui.add(egui::Label::new(meta).truncate().selectable(false));
        }
        sheet_ui.add_space(24. * scale);
        let (seek, _) = sheet_ui.allocate_exact_size(vec2(inner_w, SEEK_ROW_H * scale), Sense::hover());
        self.cast_seek(&mut sheet_ui, view, cx, seek, scale, cast.can_seek);
        sheet_ui.add_space(16. * scale);
        let (transport, _) = sheet_ui.allocate_exact_size(vec2(inner_w, 76. * scale), Sense::hover());
        cast_transport(&mut sheet_ui, view, cx, transport, scale, (76., 6.), cast.can_seek);
        sheet_ui.add_space(16. * scale);
        let (volume, _) = sheet_ui.allocate_exact_size(vec2(inner_w, 24. * scale), Sense::hover());
        self.volume_at(ui, view, cx, volume, scale);

        // The tracks as full-width rows, under the volume, and Stop casting at the bottom,
        // inside the safe area. A short screen squeezes the rows up against the button.
        let flow = sheet_ui.cursor().top();
        let mut rows: Vec<(MaterialIcon, String, String, Chip)> = Vec::new();
        if view.audio_button {
            let value = cast.audio.clone().unwrap_or_default();
            rows.push((ICON_AUDIOTRACK, tr!("player-audio"), value, Chip::Audio));
        }
        if view.subtitles_button {
            let value = cast.subtitle.clone().unwrap_or_default();
            rows.push((ICON_CLOSED_CAPTION, tr!("player-subtitles"), value, Chip::Subtitles));
        }
        if view.chapter_count >= 2 {
            let value = cast.chapter.clone().unwrap_or_else(|| tr!("player-chapters"));
            rows.push((ICON_VIEW_LIST, tr!("player-chapters"), value, Chip::Chapters));
        }
        let stop_h = STOP_H;
        let rows_h = rows.len() as f32 * ROW_H * scale;
        let room = safe.bottom() - stop_h - 12. - rows_h;
        let mut y = (flow + 24. * scale).min(room);
        for (icon, name, value, kind) in rows {
            let row = Rect::from_min_size(pos2(window.left() + side, y), vec2(inner_w, ROW_H * scale));
            if cast_row(ui, row, icon, &name, &value).clicked() {
                match kind {
                    Chip::Audio => {
                        self.settings_tab = SettingsTab::Audio;
                        self.sheet = Some(phone::Sheet::Settings);
                    }
                    Chip::Subtitles => {
                        self.settings_tab = SettingsTab::Subtitles;
                        self.sheet = Some(phone::Sheet::Settings);
                    }
                    Chip::Chapters => self.sheet = Some(phone::Sheet::Chapters),
                    Chip::Version => {}
                }
            }
            y += ROW_H * scale;
        }
        let stop = Rect::from_min_size(
            pos2(window.left() + side, safe.bottom() - stop_h),
            vec2(inner_w, stop_h),
        );
        cast_stop(ui, stop, cx);
    }

    /// The top row: Back (or the phone's chevron), the TV's name on a pill in the middle, and on
    /// the desktop *Stop casting* at the right.
    fn cast_top(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, bounds: Rect, phone: bool) {
        let cast = view.cast.clone().unwrap_or_default();
        let (icon, tip) = if phone {
            (ICON_EXPAND_MORE, tr!("player-close"))
        } else {
            (ICON_ARROW_BACK, tr!("player-back"))
        };
        let (size, icon_size) = if phone { (44., 28.) } else { (40., 24.) };
        let center = if phone {
            pos2(bounds.left() + 32., bounds.top() + 44.)
        } else {
            pos2(bounds.left() + 36., bounds.top() + 36.)
        };
        let back = RoundButton::icon(icon, name(&tip))
            .size(size, icon_size)
            .tooltip(&tip)
            .colours(controller_colours(ui))
            .focus(Focus::Skip)
            .show_at(ui, center, Id::unique("player_cast_back"));
        if back.clicked() {
            self.exit(cx);
        }
        let text = if cast.reconnecting {
            tr!("cast-bar-reconnecting", device = cast.device.as_str())
        } else {
            cast.device.clone()
        };
        let max_w = bounds.width() - if phone { 200. } else { 440. };
        pill(ui, pos2(bounds.center().x, center.y), &text, max_w);
        if !phone {
            let stop = Rect::from_min_size(pos2(bounds.right() - 24. - 150., bounds.top() + 16.), vec2(150., 42.));
            cast_stop(ui, stop, cx);
        }
    }

    /// The seek bar: the track, the elapsed and remaining times under its ends, and dragging to
    /// seek. The bar follows the pointer while it's dragged, not playback, and seeks when it's
    /// released.
    fn cast_seek(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, rect: Rect, scale: f32, can_seek: bool) {
        let tokens = theme::tokens_of(ui);
        let Some(duration) = view.duration.filter(|duration| !duration.is_zero()) else {
            return;
        };
        let sense = if can_seek {
            Sense::CLICK | Sense::DRAG
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, Id::unique("player_cast_seek"), sense);
        let shown = self.scrub.unwrap_or(view.position).min(duration);
        // A slider to AccessKit, in seconds; the pointer is how it moves.
        let name = tr!("player-seek");
        response.widget_info(|| WidgetInfo::slider(can_seek, shown.as_secs_f64(), &name));
        let track_h = 4. * scale;
        let track = Rect::from_min_size(pos2(rect.left(), rect.top() + 8. * scale), vec2(rect.width(), track_h));
        let fraction = (shown.as_secs_f64() / duration.as_secs_f64()).clamp(0., 1.) as f32;
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(2), tokens.inset);
        let filled = Rect::from_min_size(track.min, vec2(track.width() * fraction, track_h));
        painter.rect_filled(filled, CornerRadius::same(2), tokens.brand);
        painter.circle_filled(pos2(filled.right(), track.center().y), 7. * scale, tokens.brand);
        let font = theme::text::font(ui, "xs");
        painter.text(
            pos2(rect.left(), rect.top() + 22. * scale),
            Align2::LEFT_TOP,
            timestamp(shown),
            font.clone(),
            tokens.ink_muted,
        );
        let left = format!("-{}", timestamp(duration.saturating_sub(shown)));
        painter.text(
            pos2(rect.right(), rect.top() + 22. * scale),
            Align2::RIGHT_TOP,
            left,
            font,
            tokens.ink_muted,
        );
        if !can_seek {
            return;
        }
        if let Some(pos) = response.interact_pointer_pos()
            && (response.is_pointer_button_down_on() || response.clicked())
        {
            self.scrub = Some(time_at(rect, duration, pos.x));
        }
        if (response.clicked() || response.drag_stopped())
            && let Some(time) = self.scrub.take()
        {
            cx.services.control(UiCommand::SeekTo(time));
        }
    }

    /// The TV's volume: mute, a slider and the level. Applied while dragging, so the change is
    /// heard right away.
    fn cast_volume(&mut self, ui: &mut egui::Ui, view: &View, cx: &mut PageCx, width: f32, scale: f32) {
        let tokens = theme::tokens_of(ui);
        let muted = view.muted || view.volume == 0.;
        let (icon, tip) = if muted {
            (ICON_VOLUME_OFF, tr!("player-unmute"))
        } else {
            (ICON_VOLUME_UP, tr!("player-mute"))
        };
        let button = RoundButton::icon(icon, name(&tip))
            .size(32. * scale, 20. * scale)
            .tooltip(&tip)
            .colours(controller_colours(ui))
            .focus(Focus::Skip)
            .show(ui);
        if button.clicked() {
            cx.services.control(UiCommand::ToggleMute);
        }
        ui.add_space(4. * scale);
        let slider_w = (width - 32. * scale - 4. * scale - 40. * scale).max(80.);
        let (rect, response) = ui.allocate_exact_size(vec2(slider_w, 24. * scale), Sense::CLICK | Sense::DRAG);
        let volume = if self.adjusting_volume {
            self.volume
        } else {
            view.volume
        };
        let name = tr!("player-volume");
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), volume, &name));
        let track_h = 4. * scale;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), track_h));
        let fraction = (volume / 100.).clamp(0., 1.) as f32;
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(2), tokens.inset);
        let filled = Rect::from_min_size(track.min, vec2(track.width() * fraction, track_h));
        painter.rect_filled(filled, CornerRadius::same(2), tokens.brand);
        painter.circle_filled(pos2(filled.right(), track.center().y), 7. * scale, tokens.brand);

        let pressed = response.is_pointer_button_down_on();
        let released = response.clicked() || response.drag_stopped();
        if !self.adjusting_volume {
            self.volume = view.volume;
        }
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
        ui.add_space(4. * scale);
        let level = if self.adjusting_volume {
            self.volume
        } else {
            view.volume
        };
        let level = RichText::new(format!("{level:.0}"))
            .font(theme::text::font(ui, "sm"))
            .color(tokens.ink_muted);
        ui.label(level);
    }

    /// Opens the settings menu on `tab`, or closes it when it's already there. Returns whether
    /// it has to be (re)opened after the menus are drawn: the click that opened it is a click
    /// outside the menu, which closes it first.
    fn toggle_settings(&mut self, ctx: &egui::Context, tab: SettingsTab) -> bool {
        if egui::Popup::is_id_open(ctx, settings_id()) && self.settings_tab == tab {
            egui::Popup::close_id(ctx, settings_id());
            return false;
        }
        self.settings_tab = tab;
        true
    }
}

/// What a chip or row chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chip {
    Audio,
    Subtitles,
    Chapters,
    Version,
}

/// The chips drawn this frame, for their menus' anchors, and whether the settings menu has to
/// be reopened after them.
#[derive(Default)]
struct ChipResponses {
    audio: Option<Response>,
    subtitle: Option<Response>,
    chapters: Option<Response>,
    version: Option<Response>,
    reopen: bool,
}

/// The controller's track chips, in the order they show: each a value with its icon, and what it
/// opens.
fn cast_chips(view: &View) -> Vec<(MaterialIcon, String, Chip)> {
    let cast = view.cast.clone().unwrap_or_default();
    let mut chips = Vec::new();
    if view.audio_button {
        chips.push((
            ICON_AUDIOTRACK,
            cast.audio.unwrap_or_else(|| tr!("player-audio")),
            Chip::Audio,
        ));
    }
    if view.subtitles_button {
        chips.push((
            ICON_CLOSED_CAPTION,
            cast.subtitle.unwrap_or_else(|| tr!("player-subtitles")),
            Chip::Subtitles,
        ));
    }
    if view.chapter_count >= 2 {
        chips.push((ICON_VIEW_LIST, tr!("player-chapters"), Chip::Chapters));
    }
    if view.versions.len() >= 2 {
        let name = view.version_name.clone().unwrap_or_else(|| tr!("player-version"));
        chips.push((ICON_TV, name, Chip::Version));
    }
    chips
}

/// The item's name, or what the receiver reported before the server's item arrives.
fn cast_title(view: &View) -> String {
    view.title.clone()
}

/// The item's meta line (`views::item::meta_line`: year, runtime, rating), with an episode's
/// series before it.
fn cast_meta(cast: &CastView) -> String {
    let Some(item) = &cast.item else {
        return String::new();
    };
    let line = crate::views::item::meta_line(item);
    match (&item.series_name, item.kind) {
        (Some(series), BaseItemKind::Episode) if !series.is_empty() => format!("{series} · {line}"),
        _ => line,
    }
}

/// The transport: previous, back 10 s, a big play/pause, forward 10 s and next, centred on
/// `rect`.
fn cast_transport(
    ui: &mut Ui,
    view: &View,
    cx: &mut PageCx,
    rect: Rect,
    scale: f32,
    (play, gap): (f32, f32),
    can_seek: bool,
) {
    let center = rect.center();
    let playing = !view.loading;
    let play = play * scale;
    if cast_play(ui, center, play, view.paused, playing).clicked() {
        cx.services.control(UiCommand::TogglePause);
    }
    let step = play / 2. + gap * scale + 24. * scale;
    let far = step + 48. * scale + gap * scale * 0.6;
    let seconds = view.seek_step;
    let (back_icon, forward_icon) = seek_icons(seconds);
    let (back_tip, forward_tip) = (
        tr!("player-back-seconds", seconds = seconds),
        tr!("player-forward-seconds", seconds = seconds),
    );
    let back = RoundButton::icon(back_icon, name(&back_tip))
        .size(48. * scale, 30. * scale)
        .tooltip(&back_tip)
        .colours(controller_colours(ui))
        .enabled(can_seek)
        .focus(Focus::Skip)
        .show_at(ui, center - vec2(step, 0.), Id::unique("player_cast_back_10"));
    if back.clicked() {
        cx.services.control(UiCommand::SeekBy(-seconds));
    }
    let forward = RoundButton::icon(forward_icon, name(&forward_tip))
        .size(48. * scale, 30. * scale)
        .tooltip(&forward_tip)
        .colours(controller_colours(ui))
        .enabled(can_seek)
        .focus(Focus::Skip)
        .show_at(ui, center + vec2(step, 0.), Id::unique("player_cast_forward_10"));
    if forward.clicked() {
        cx.services.control(UiCommand::SeekBy(seconds));
    }
    if let Some(tip) = &view.previous_tip {
        let previous = RoundButton::icon(ICON_SKIP_PREVIOUS, name(tip))
            .size(44. * scale, 28. * scale)
            .tooltip(tip)
            .colours(controller_colours(ui))
            .enabled(playing)
            .focus(Focus::Skip)
            .show_at(ui, center - vec2(far, 0.), Id::unique("player_cast_previous"));
        if previous.clicked() {
            cx.services.play_previous();
        }
    }
    if let Some(tip) = &view.next_tip {
        let next = RoundButton::icon(ICON_SKIP_NEXT, name(tip))
            .size(44. * scale, 28. * scale)
            .tooltip(tip)
            .colours(controller_colours(ui))
            .enabled(playing)
            .focus(Focus::Skip)
            .show_at(ui, center + vec2(far, 0.), Id::unique("player_cast_next"));
        if next.clicked() {
            cx.services.play_next();
        }
    }
}

/// The big play/pause: a `brand` circle, its icon in `brand_foreground`, `size` across.
fn cast_play(ui: &mut Ui, center: Pos2, size: f32, paused: bool, enabled: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let (icon, tip) = if paused {
        (ICON_PLAY_ARROW, tr!("player-play"))
    } else {
        (ICON_PAUSE, tr!("player-pause"))
    };
    let label = name(&tip);
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let sense = if enabled { Sense::CLICK } else { Sense::hover() };
    let response = ui.interact(rect, Id::unique("player_cast_play"), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let fill = if !enabled {
        tokens.brand.gamma_multiply(0.4)
    } else if response.hovered() {
        tokens.brand_hover
    } else {
        tokens.brand
    };
    ui.painter().circle_filled(center, size / 2., fill);
    round_button::glyph(ui, center, icon, size * 0.5, tokens.brand_foreground);
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
    } else {
        response
    }
}

/// A track chip: an icon and the value on `inset`, as a button.
fn chip(ui: &mut Ui, icon: MaterialIcon, value: &str, scale: f32) -> Response {
    let tokens = theme::tokens_of(ui);
    let font = theme::text::font(ui, "sm");
    let height = CHIP_H * scale;
    let (rect, response) = ui.allocate_exact_size(vec2(chip_width(ui, value), height), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, value));
    let fill = if response.hovered() {
        tokens.inset_hover
    } else {
        tokens.inset
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((height / 2.) as u8), fill);
    round_button::glyph(
        ui,
        pos2(rect.left() + 21., rect.center().y),
        icon,
        18.,
        tokens.ink_muted,
    );
    painter.text(
        pos2(rect.left() + 36., rect.center().y),
        Align2::LEFT_CENTER,
        value,
        font,
        tokens.ink,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// How wide a chip's text makes it.
fn chip_width(ui: &Ui, value: &str) -> f32 {
    let font = theme::text::font(ui, "sm");
    let text = ui
        .painter()
        .layout_no_wrap(value.to_owned(), font, Color32::PLACEHOLDER);
    12. + 18. + 6. + text.size().x + 14.
}

/// A phone's track row: an icon, the choice's name and its value at the right, full width.
fn cast_row(ui: &mut Ui, rect: Rect, icon: MaterialIcon, name: &str, value: &str) -> Response {
    let tokens = theme::tokens_of(ui);
    let response = ui.interact(rect, Id::unique(("player_cast_row", name)), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(tokens.radius_control), tokens.inset_hover);
    }
    round_button::glyph(
        ui,
        pos2(rect.left() + 12., rect.center().y),
        icon,
        20.,
        tokens.ink_muted,
    );
    ui.painter().text(
        pos2(rect.left() + 40., rect.center().y),
        Align2::LEFT_CENTER,
        name,
        theme::text::font(ui, "base"),
        tokens.ink,
    );
    let font = theme::text::font(ui, "sm");
    let value = short(ui, value, font.clone(), rect.width() * 0.5);
    ui.painter().text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        font,
        tokens.ink_muted,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// *Stop casting*, in the controller's row: `quiet_danger`, the action that leaves the TV.
fn cast_stop(ui: &mut Ui, rect: Rect, cx: &mut PageCx) {
    let label = tr!("cast-stop");
    let stop = ui
        .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            button::quiet_danger(ui, &label, rect.width(), State::Ready, Focus::Skip)
        })
        .inner;
    if stop.clicked() {
        cx.services.stop_cast();
    }
}

/// Where it plays: a `brand` cast mark and the TV's name on a pill, centred on `center`. The
/// text is cut to `max_w`.
fn pill(ui: &Ui, center: Pos2, text: &str, max_w: f32) {
    let tokens = theme::tokens_of(ui);
    let font = theme::text::font(ui, "sm");
    let max_text = (max_w - 16. - 18. - 8. - 16.).max(24.);
    let text = short(ui, text, font.clone(), max_text);
    let galley = ui.painter().layout_no_wrap(text, font, tokens.ink);
    let rect = Rect::from_center_size(center, vec2(16. + 18. + 8. + galley.size().x + 16., PILL_H));
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(17), tokens.surface);
    painter.rect_stroke(
        rect,
        CornerRadius::same(17),
        Stroke::new(1., tokens.surface_border),
        StrokeKind::Inside,
    );
    round_button::glyph(
        ui,
        pos2(rect.left() + 25., center.y),
        ICON_CAST_CONNECTED,
        18.,
        tokens.brand,
    );
    painter.galley(
        pos2(rect.left() + 42., center.y - galley.size().y / 2.),
        galley,
        tokens.ink,
    );
}

/// `text` in `font`, cut with an ellipsis to fit `max_w`.
fn short(ui: &Ui, text: &str, font: FontId, max_w: f32) -> String {
    let width = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    if width(text) <= max_w {
        return text.to_owned();
    }
    let mut fit = String::new();
    for ch in text.chars() {
        let mut candidate = fit.clone();
        candidate.push(ch);
        candidate.push('…');
        if width(&candidate) > max_w {
            break;
        }
        fit.push(ch);
    }
    fit.push('…');
    fit
}

/// The controller's buttons: the theme's ink, as the player's bars are white over the video.
fn controller_colours(ui: &Ui) -> round_button::Colours {
    let tokens = theme::tokens_of(ui);
    round_button::Colours {
        circle: tokens.inset_hover,
        ink: tokens.ink,
        ink_hover: tokens.ink,
        on: tokens.brand,
    }
}

/// A button's AccessKit name: its tooltip without the key hint, so "Back" rather than "Back
/// (Esc)".
fn name(tooltip: &str) -> &str {
    tooltip.split(" (").next().unwrap_or(tooltip)
}

/// `egui::Popup` with a fixed id: clicking the chip opens it, or closes it when it's open.
fn toggle_menu(ctx: &egui::Context, id: Id) {
    if egui::Popup::is_id_open(ctx, id) {
        egui::Popup::close_id(ctx, id);
    } else {
        egui::Popup::open_id(ctx, id);
    }
    ctx.request_repaint();
}
