//! The player's queue panel: what's playing and what's queued after it, hanging from the queue
//! button over the video like the chapters menu, with repeat and shuffle beside its label. A
//! click on an entry plays it, its round buttons move or remove it, and the keyboard does the
//! same (it's the player's Q key that opens it): Up and Down walk the toggles and the entries,
//! Enter plays the highlighted entry or flips the toggle, Delete removes an entry, Alt+Up and
//! Alt+Down move it, and Q or Esc close the panel. The panel reads those keys itself while it's
//! open, as the SyncPlay panel does, since egui keeps the keyboard then (`keys.rs`).
//!
//! Only the solo player has one: in a SyncPlay group the group's queue is in its panel, and a
//! cast's is the TV's.

use egui::accesskit::Role;
use egui::{Key, Modifiers, RectAlign, Response, Ui};
use egui_material_icons::icons::{ICON_REPEAT, ICON_REPEAT_ONE, ICON_SHUFFLE};
use jellyfin::RepeatMode;

use super::{CHAPTERS_PAD, PlayerPage, control, queue_id};
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
/// The overline's height and the toggles' size, as the SyncPlay panel's.
const HEADING_H: f32 = 30.;

/// What the keyboard can be on, in the order Up and Down walk them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Repeat,
    Shuffle,
    /// A queued item, by its place among the queued ones.
    Item(usize),
}

/// The entries for a queue of `queued` items.
fn entries(queued: usize) -> Vec<Entry> {
    [Entry::Repeat, Entry::Shuffle]
        .into_iter()
        .chain((0..queued).map(Entry::Item))
        .collect()
}

/// The panel's own state: where the keyboard is.
#[derive(Default)]
pub(super) struct QueuePanel {
    /// `None` when a click opened the panel, until an arrow key picks an entry.
    highlight: Option<Entry>,
    /// The list scrolls to the highlight on the next frame (set when the keyboard moves it).
    scroll: bool,
}

/// What a frame of the panel asked for.
#[derive(Default)]
struct Asked {
    jump: Option<usize>,
    remove: Option<usize>,
    shift: Option<(usize, isize)>,
    cycle_repeat: bool,
    toggle_shuffle: bool,
}

impl PlayerPage {
    /// Opens the queue panel if it's closed, closes it if it's open: the button's click, and the
    /// Q key (which starts the highlight on the first queued entry, or on the toggles).
    pub(super) fn toggle_queue(&mut self, cx: &mut PageCx, from_keyboard: bool) {
        if egui::Popup::is_id_open(cx.ctx, queue_id()) {
            self.close_queue(cx.ctx);
            return;
        }
        self.queue.highlight = from_keyboard.then(|| match cx.services.playback.queued().len() {
            0 => Entry::Repeat,
            _ => Entry::Item(0),
        });
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
        // The queue can shrink under an entry (it played, or a remote client took it out).
        if let Some(Entry::Item(at)) = self.queue.highlight {
            self.queue.highlight = Some(if len == 0 {
                Entry::Repeat
            } else {
                Entry::Item(at.min(len - 1))
            });
        }
        let alt_up = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowUp);
        let alt_down = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowDown);
        if alt_up | alt_down {
            let by = if alt_up { -1 } else { 1 };
            if let Some(Entry::Item(at)) = self.queue.highlight
                && let Some(to) = cx.services.queue_move(at, by)
            {
                self.queue.highlight = Some(Entry::Item(to));
                self.queue.scroll = true;
            }
            keep_focus_off(ctx);
            return;
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Delete)
            && let Some(Entry::Item(at)) = self.queue.highlight
        {
            cx.services.queue_remove(at);
            self.queue.scroll = true;
            return;
        }
        let down = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowDown);
        let up = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowUp);
        if down | up {
            let all = entries(len);
            let here = self
                .queue
                .highlight
                .and_then(|entry| all.iter().position(|candidate| *candidate == entry));
            let step = if down { 1 } else { -1 };
            self.queue.highlight = popup::step(here, step, all.len()).map(|at| all[at]);
            self.queue.scroll = true;
            keep_focus_off(ctx);
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Enter) {
            match self.queue.highlight {
                Some(Entry::Repeat) => cx.services.cycle_repeat(),
                Some(Entry::Shuffle) => cx.services.toggle_shuffle(),
                Some(Entry::Item(at)) => {
                    self.close_queue(ctx);
                    cx.services.queue_jump(at);
                }
                None => {}
            }
        }
    }

    /// The panel itself, hanging from `button`. Clicks play, move and remove entries and flip the
    /// toggles; the keyboard's go through [`PlayerPage::queue_keys`].
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
        let (repeat, shuffled) = cx.services.playback.modes();
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
                popup::panel_section(ui, &tr!("shell-queue"), HEADING_H, |ui| {
                    toggles(ui, repeat, shuffled, highlight, &mut asked);
                });
                queue_list(ui, playing.as_deref(), &queued, highlight, scroll, &mut asked);
            });
        if asked.cycle_repeat {
            cx.services.cycle_repeat();
        }
        if asked.toggle_shuffle {
            cx.services.toggle_shuffle();
        }
        if let Some((at, by)) = asked.shift
            && let Some(to) = cx.services.queue_move(at, by)
            && self.queue.highlight == Some(Entry::Item(at))
        {
            self.queue.highlight = Some(Entry::Item(to));
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

/// Shuffle and repeat, small round toggles at the label's right end: lit while on, ringed while
/// the keyboard is on them. Off, all, one: each press of repeat moves to the next.
fn toggles(ui: &mut Ui, repeat: RepeatMode, shuffled: bool, highlight: Option<Entry>, asked: &mut Asked) {
    let (icon, tip, on) = match repeat {
        RepeatMode::RepeatNone => (ICON_REPEAT, tr!("shell-queue-repeat-off"), false),
        RepeatMode::RepeatAll => (ICON_REPEAT, tr!("shell-queue-repeat-all"), true),
        RepeatMode::RepeatOne => (ICON_REPEAT_ONE, tr!("shell-queue-repeat-one"), true),
    };
    let ringed = highlight == Some(Entry::Repeat);
    if control(icon, HEADING_H, &tip).on(on).ringed(ringed).show(ui).clicked() {
        asked.cycle_repeat = true;
    }
    let tip = if shuffled {
        tr!("shell-queue-shuffle-on")
    } else {
        tr!("shell-queue-shuffle-off")
    };
    let ringed = highlight == Some(Entry::Shuffle);
    if control(ICON_SHUFFLE, HEADING_H, &tip)
        .on(shuffled)
        .ringed(ringed)
        .show(ui)
        .clicked()
    {
        asked.toggle_shuffle = true;
    }
}

/// The list: the playing item marked, then the queued ones numbered, or a line saying there are
/// none. What a click asked for goes into `asked`.
fn queue_list(
    ui: &mut Ui,
    playing: Option<&str>,
    queued: &[String],
    highlight: Option<Entry>,
    scroll: bool,
    asked: &mut Asked,
) {
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
                    let highlighted = highlight == Some(Entry::Item(at));
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
}

/// Cancels the focus move egui queued for a key the panel reads itself, before a widget behind
/// it can take the focus.
fn keep_focus_off(ctx: &egui::Context) {
    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
}
