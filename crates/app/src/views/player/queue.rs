//! The player's queue panel: what's playing and what's queued after it, hanging from the queue
//! button over the video like the chapters menu. A click on an entry plays it, its round buttons
//! move or remove it, and the keyboard does the same (it's the player's Q key that opens it):
//! Up and Down move the highlight, Enter plays the highlighted entry, Delete removes it,
//! Alt+Up and Alt+Down move it, and Q or Esc close the panel. The panel reads those keys itself
//! while it's open, as the SyncPlay panel does, since egui keeps the keyboard then (`keys.rs`).
//!
//! Only the solo player has one: in a SyncPlay group the group's queue is in its panel, and a
//! cast's is the TV's.

use egui::accesskit::Role;
use egui::{Key, Modifiers, RectAlign, Response, Ui};

use super::{CHAPTERS_PAD, PlayerPage, queue_id};
use crate::i18n::tr;
use crate::keys;
use crate::playback;
use crate::theme;
use crate::views::PageCx;
use crate::widgets::list_row::{self, Handle, Handles, Leading, ListRow};
use crate::widgets::{label, popup};

/// The panel's width, and how tall its list grows before it scrolls.
const QUEUE_W: f32 = 360.;
const QUEUE_MAX_H: f32 = 420.;
/// The overline's height, as the SyncPlay panel's.
const HEADING_H: f32 = 30.;

/// The panel's own state: where the keyboard is.
#[derive(Default)]
pub(super) struct QueuePanel {
    /// The highlighted entry's place among the queued ones: `None` when a click opened the panel,
    /// until an arrow key picks one.
    highlight: Option<usize>,
    /// The list scrolls to the highlight on the next frame (set when the keyboard moves it).
    scroll: bool,
}

/// What a frame of the panel asked for.
#[derive(Default)]
struct Asked {
    jump: Option<usize>,
    remove: Option<usize>,
    shift: Option<(usize, isize)>,
}

impl PlayerPage {
    /// Opens the queue panel if it's closed, closes it if it's open: the button's click, and the
    /// Q key (which starts the highlight on the first queued entry).
    pub(super) fn toggle_queue(&mut self, cx: &mut PageCx, from_keyboard: bool) {
        if egui::Popup::is_id_open(cx.ctx, queue_id()) {
            self.close_queue(cx.ctx);
            return;
        }
        self.queue.highlight = (from_keyboard && cx.services.playback.queued().len() > 0).then_some(0);
        self.queue.scroll = true;
        egui::Popup::open_id(cx.ctx, queue_id());
        self.show_controls();
        cx.ctx.request_repaint();
    }

    fn close_queue(&mut self, ctx: &egui::Context) {
        egui::Popup::close_id(ctx, queue_id());
        self.queue.highlight = None;
        ctx.request_repaint();
    }

    /// The panel's keys, read before the page draws while the panel is open.
    pub(super) fn queue_keys(&mut self, cx: &mut PageCx) {
        let ctx = cx.ctx;
        if !egui::Popup::is_id_open(ctx, queue_id()) {
            self.queue.highlight = None;
            return;
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Escape) | keys::take_exact(ctx, Modifiers::NONE, Key::Q) {
            self.close_queue(ctx);
            return;
        }
        let len = cx.services.playback.queued().len();
        self.queue.highlight = self.queue.highlight.filter(|_| len > 0).map(|at| at.min(len - 1));
        let alt_up = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowUp);
        let alt_down = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowDown);
        if alt_up | alt_down {
            let by = if alt_up { -1 } else { 1 };
            if let Some(at) = self.queue.highlight
                && let Some(to) = cx.services.queue_move(at, by)
            {
                self.queue.highlight = Some(to);
                self.queue.scroll = true;
            }
            keep_focus_off(ctx);
            return;
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Delete)
            && let Some(at) = self.queue.highlight
        {
            cx.services.queue_remove(at);
            self.queue.scroll = true;
            return;
        }
        let down = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowDown);
        let up = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowUp);
        if (down | up) && len > 0 {
            let step = if down { 1 } else { -1 };
            self.queue.highlight = popup::step(self.queue.highlight, step, len);
            self.queue.scroll = true;
            keep_focus_off(ctx);
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Enter)
            && let Some(at) = self.queue.highlight
        {
            self.close_queue(ctx);
            cx.services.queue_jump(at);
        }
    }

    /// The panel itself, hanging from `button`. Clicks play, move and remove entries; the
    /// keyboard's go through [`PlayerPage::queue_keys`].
    pub(super) fn queue_menu(&mut self, button: &Response, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, queue_id()) {
            return;
        }
        // Read before drawing: a row's button changes the queue, which the draw borrows.
        let playing = cx
            .services
            .playback
            .now_playing()
            .map(|state| playback::title(&state.item));
        let queued: Vec<String> = cx.services.playback.queued().map(playback::title).collect();
        let highlight = self.queue.highlight;
        let scroll = std::mem::take(&mut self.queue.scroll);
        let mut asked = Asked::default();
        egui::Popup::menu(button)
            .id(queue_id())
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::TOP_END)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(QUEUE_W + 24.)
            .frame(popup::over_video_frame(CHAPTERS_PAD as i8))
            .show(|ui| {
                ui.set_width(QUEUE_W + 24. - 2. * CHAPTERS_PAD);
                popup::panel_section(ui, &tr!("shell-queue"), HEADING_H, |_| {});
                asked = queue_list(ui, playing.as_deref(), &queued, highlight, scroll);
            });
        if let Some((at, by)) = asked.shift
            && let Some(to) = cx.services.queue_move(at, by)
            && self.queue.highlight == Some(at)
        {
            self.queue.highlight = Some(to);
        }
        if let Some(at) = asked.remove {
            cx.services.queue_remove(at);
        }
        if let Some(at) = asked.jump {
            self.close_queue(cx.ctx);
            cx.services.queue_jump(at);
        }
    }
}

/// The list: the playing item marked, then the queued ones numbered, or a line saying there are
/// none. Returns what a click asked for.
fn queue_list(ui: &mut Ui, playing: Option<&str>, queued: &[String], highlight: Option<usize>, scroll: bool) -> Asked {
    let mut asked = Asked::default();
    list_row::block(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(QUEUE_MAX_H)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                if let Some(name) = playing {
                    let named = tr!("shell-queue-playing", name = name);
                    ListRow::new(name)
                        .compact()
                        .leading(Leading::Playing)
                        .no_hover()
                        .role(Role::MenuItem)
                        .label(&named)
                        .show(ui);
                }
                let last = queued.len().saturating_sub(1);
                for (at, name) in queued.iter().enumerate() {
                    let highlighted = highlight == Some(at);
                    let moves = Handles {
                        up: at > 0,
                        down: at < last,
                    };
                    let row = ListRow::new(name)
                        .compact()
                        .leading(Leading::Number(at + 1))
                        .handles(moves, &tr!("shell-queue-remove"))
                        .highlighted(highlighted)
                        .ringed(highlighted)
                        .role(Role::MenuItem)
                        .show(ui);
                    match row.handle {
                        Some(Handle::Up) => asked.shift = Some((at, -1)),
                        Some(Handle::Down) => asked.shift = Some((at, 1)),
                        Some(Handle::Remove) => asked.remove = Some(at),
                        None if row.row.clicked() => asked.jump = Some(at),
                        None => {}
                    }
                    if highlighted && scroll {
                        row.row.scroll_to_me(Some(egui::Align::Center));
                    }
                }
                if queued.is_empty() {
                    ui.add_space(theme::SPACE.xs);
                    ui.horizontal(|ui| {
                        ui.add_space(theme::SPACE.md);
                        ui.label(label::muted(ui, tr!("player-queue-empty")));
                    });
                    ui.add_space(theme::SPACE.xs);
                }
            });
    });
    asked
}

/// Cancels the focus move egui queued for a key the panel reads itself, before a widget behind
/// it can take the focus.
fn keep_focus_off(ctx: &egui::Context) {
    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
}
