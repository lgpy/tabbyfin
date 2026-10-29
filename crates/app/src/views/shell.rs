//! The signed-in frame: a top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.
//!
//! The bar has the app's icon, Back when there's a page to go back to, the page's title, and
//! on the right the search box, SyncPlay, Settings and the account menu. The keyboard walks
//! it: Up past a page's first row moves into the bar, Left and Right move along, Enter presses,
//! Down or Esc goes back to the page with its cursor where it was. The account menu is a
//! [`PopupMenu`]; the SyncPlay panel is drawn and driven here: its keys are read before egui's
//! focus pass can hand them to a button or card behind it.
//!
//! The SyncPlay panel (`render_syncplay` on `main`) lists the server's groups to join, or the
//! group this session is in: its state, its members and the group's queue, which the keyboard
//! can play, move, remove and toggle (`menu_entries` on `main`).
//!
//! The search box is the bar's only text field: typing opens the results page (or comes back
//! to it from a page opened from them), clearing it closes them, and Enter or Down moves into
//! the results, which hand Up and Shift+Tab back to the box; Esc gives the keyboard back to the
//! page. Closed, the box is a button with the others on the right; opening, it grows out of that
//! button into the middle of the bar, and it shrinks back into it as it closes, as on `main`
//! (`main`'s `motion::morph`, 260 ms eased out, with the button fading the other way).
//!
//! While a playback session shows, [`views::player::PlayerPage`] takes the window: no top bar,
//! the video and its controls filling it, and the player's keys working. Going back stops
//! playback; the player closing leaves fullscreen.

use egui::accesskit::Role;
use egui::{
    Align, Align2, Color32, CornerRadius, Id, Key, Layout, Modifiers, Rect, RectAlign, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ADD, ICON_ARROW_BACK, ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_CLOSE, ICON_GROUPS, ICON_LOGOUT,
    ICON_PLAY_ARROW, ICON_REPEAT, ICON_REPEAT_ONE, ICON_SEARCH, ICON_SETTINGS, ICON_SHUFFLE,
};
use jellyfin::{GroupInfo, GroupState, PlayQueue, RepeatMode, ShuffleMode};

use crate::browse::Move;
use crate::config::{Config, SavedServer};
use crate::keys::{self, Action, Press, Scope};
use crate::motion;
use crate::nav::{NavRequest, Route};
use crate::playback;
use crate::services::AppEvent;
use crate::session::Session;
use crate::syncplay::SyncPlay;
use crate::task::Task;
use crate::theme::{self, over_video, text};
use crate::toasts::{Toast, ToastAction};
use crate::update::{self, Update};
use crate::views::{self, Page, PageCx, player};
use crate::widgets::popup::{self, Entry, Opened, PopupMenu};

/// The bar's height, inside the panel's own margin.
/// `main`'s bar: 72 points tall, 40-point buttons with 24-point icons.
const BAR_H: f32 = 72.;
/// The size of the bar's buttons, and of the slot each takes (so the right-hand side lines up
/// whatever the icon is).
const BAR_BUTTON: f32 = 40.;
const BAR_ICON: f32 = 24.;
/// The app icon's size in the bar.
const LOGO: f32 = 40.;
/// The open search box's width, when the window has room for it.
const SEARCH_BOX_W: f32 = 400.;
/// The search box's id: it gives its focus back when the keyboard leaves it.
const SEARCH_ID: &str = "top_bar_search";
/// The account menu's width, and its id.
const ACCOUNT_MENU_W: f32 = 240.;
const ACCOUNT_MENU: &str = "account_menu";
/// The SyncPlay panel's width, as `main`'s popover had it.
const SYNCPLAY_PANEL_W: f32 = 300.;
/// The SyncPlay panel's id in egui's popup memory (egui keeps the keyboard while it's open
/// only if it is in that memory, as for the account menu), and the id it lives under.
const SYNCPLAY_PANEL: &str = "syncplay_panel";
/// How tall the group's queue may grow before it scrolls: the panel hangs from the bar, so a
/// long queue would otherwise run off the bottom of the window.
const SYNCPLAY_QUEUE_H: f32 = 320.;
/// The queue's shuffle and repeat toggles, and the buttons on a queue row.
const QUEUE_TOGGLE: f32 = 28.;
const QUEUE_BUTTON: f32 = 26.;

/// A choice in the SyncPlay panel, in the order it lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuEntry {
    JoinGroup(String),
    NewGroup,
    ToggleShuffle,
    CycleRepeat,
    /// An entry of the group's queue, by playlist item id: Enter plays it, Delete removes it,
    /// Alt+Up and Alt+Down move it.
    QueueEntry(String),
    LeaveGroup,
}

/// The panel's keyboard highlight: the entry itself, so that a queue change can never slide it
/// onto *Leave group* or a *Join* row, and where that entry was last seen, to find its
/// neighbours when it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Highlight {
    entry: MenuEntry,
    /// The entry's index the last time the list had it.
    index: usize,
}

impl Highlight {
    /// The highlight on `index` of `entries`.
    fn at(entries: &[MenuEntry], index: usize) -> Option<Self> {
        Some(Self {
            entry: entries.get(index)?.clone(),
            index,
        })
    }
}

/// The box opening out of the search button, or closing back into it: `main`'s `motion::morph`,
/// over `motion::ENTER` and its easing. It runs on `InputState::time`, so a frame dropped or a
/// slow one doesn't stretch it, and the shell asks for a repaint only while it runs.
#[derive(Clone, Copy, Debug)]
struct SearchMotion {
    /// Closing (into the button) rather than opening (out of it).
    closing: bool,
    /// When it started, in `egui::InputState::time` seconds.
    started: f64,
    /// The box's rectangle it moves out of: the button's when it opens, the place it was last
    /// drawn when it closes. It moves to the button, or to its place in the middle of the bar.
    from: Rect,
}

impl SearchMotion {
    /// How far it has run, 0 at the start and 1 at the end, eased as `main` eased it.
    fn progress(&self, now: f64) -> f32 {
        motion::progress(self.started, now, 0., motion::ENTER)
    }
}

/// The box's rectangle `t` of the way from `from` to `to`, with the same 40-point height at
/// either end.
fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    Rect::from_min_max(
        egui::pos2(mix(from.min.x, to.min.x), mix(from.min.y, to.min.y)),
        egui::pos2(mix(from.max.x, to.max.x), mix(from.max.y, to.max.y)),
    )
}

/// The top bar's controls, in the order the keyboard walks them from the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarItem {
    Back,
    Search,
    SyncPlay,
    Settings,
    Account,
}

/// The bar's items; Back only while there's a page to go back to.
const BAR: [BarItem; 5] = [
    BarItem::Back,
    BarItem::Search,
    BarItem::SyncPlay,
    BarItem::Settings,
    BarItem::Account,
];
const BAR_WITHOUT_BACK: [BarItem; 4] = [BarItem::Search, BarItem::SyncPlay, BarItem::Settings, BarItem::Account];

/// The bar's items on a page that can go back or not.
fn bar_items(can_go_back: bool) -> &'static [BarItem] {
    if can_go_back { &BAR } else { &BAR_WITHOUT_BACK }
}

/// Where `item` sits among `items`: its own place, or the place of the first item to its right
/// in the bar's order when it has left the bar (Back on the first page), so a step from it
/// lands next to where it was rather than starting over.
fn bar_position(items: &[BarItem], item: BarItem) -> usize {
    let order = |item: BarItem| BAR.iter().position(|entry| *entry == item).unwrap_or(0);
    let target = order(item);
    items
        .iter()
        .position(|entry| order(*entry) >= target)
        .unwrap_or(items.len().saturating_sub(1))
}

/// One step along `items` from `current`: Left and Right stop at either end, and without a
/// current item this is where the keyboard enters the bar. `main`'s `focus_top_bar` started on
/// Back, or on Search when there was nowhere to go back to.
fn bar_step(items: &[BarItem], current: Option<BarItem>, step: i32) -> Option<BarItem> {
    if items.is_empty() {
        return None;
    }
    let index = current.map_or(0, |item| bar_position(items, item)) as i32;
    let last = items.len() as i32 - 1;
    items.get((index + step).clamp(0, last) as usize).copied()
}

pub struct Shell {
    /// Never empty: Home is at the bottom.
    stack: Vec<StackEntry>,
    update_check: Option<Task<Option<Update>>>,
    /// The player, while a session shows: from a session's start until playback ends for good
    /// (it stays through autoplay and reconnects). Leaving it stops playback; it keeps the
    /// session's video, menus and control state.
    player: Option<player::PlayerPage>,
    /// `TABBYFIN_ROUTE=play:…`: the item being fetched, and how to play it.
    #[cfg(debug_assertions)]
    debug_play: Option<(
        Task<jellyfin::Result<jellyfin::models::BaseItemDto>>,
        playback::PlayOptions,
    )>,
    /// The saved accounts, read from the config when the account menu opens, as on `main`.
    accounts: Vec<SavedServer>,
    /// The bar item the keyboard is on; `None` while the page has the keyboard.
    bar_key: Option<BarItem>,
    /// The search box's text.
    search_text: String,
    /// The box is in the bar rather than closed to its button. It also stays while it has
    /// text, as on `main`.
    search_open: bool,
    /// The box takes the keyboard when it draws next (opened from the bar, Ctrl+F or a route).
    want_search_focus: bool,
    /// Where the search button was drawn, and where the open box was: the places the box opens
    /// out of and closes back into (as `main`'s `search_button` and `search_slot`).
    search_button: Option<Rect>,
    search_slot: Option<Rect>,
    /// The box opening out of the button or closing back into it, while it moves.
    search_motion: Option<SearchMotion>,
    /// The box had the keyboard when the last frame ended: what this frame's key routing saw
    /// (`keys::take` runs before egui changes the focus). The box's own keys are read on this
    /// rather than `Memory::has_focus`, which later events in the frame could have changed.
    search_focused: bool,
    /// The SyncPlay panel's keyboard highlight: `None` when a click opened the panel, until an
    /// arrow key picks an entry.
    syncplay_highlight: Option<Highlight>,
    /// The panel scrolls its queue to the highlighted entry next time it draws: set when the
    /// keyboard moves the highlight, so the highlighted row is on screen (a `scroll_to_me` on
    /// every frame would stop the pointer scrolling the queue itself).
    syncplay_scroll: bool,
    /// The app icon at the start of the bar.
    logo: Option<egui::TextureHandle>,
}

/// A page's [`StackEntry::serial`]. Counted for the whole process rather than per shell, so a
/// page never shares ids with an earlier one, not even with the last account's Home after a
/// switch.
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Shell {
    pub fn new(cx: &mut PageCx) -> Self {
        // `mut` is for the debug build's `debug_start` below.
        #[cfg_attr(not(debug_assertions), allow(unused_mut))]
        let mut shell = Self {
            stack: vec![StackEntry {
                serial: next_serial(),
                route: Route::Home,
                page: views::open(Route::Home, cx),
            }],
            update_check: Some(Task::spawn(cx.ctx, update::check())),
            player: None,
            #[cfg(debug_assertions)]
            debug_play: None,
            accounts: Vec::new(),
            bar_key: None,
            search_text: String::new(),
            search_open: false,
            want_search_focus: false,
            search_button: None,
            search_slot: None,
            search_motion: None,
            search_focused: false,
            syncplay_highlight: None,
            syncplay_scroll: false,
            logo: load_logo(cx.ctx),
        };
        // A new session starts with no menu left over from the last one.
        egui::Popup::close_id(cx.ctx, account_menu_id());
        egui::Popup::close_id(cx.ctx, syncplay_panel_id());
        #[cfg(debug_assertions)]
        shell.debug_start(cx);
        shell
    }

    /// The scopes whose keys the app takes from egui (`keys.rs`): the top page's, then the
    /// shell's.
    pub fn scopes(&self) -> Vec<Scope> {
        if self.player.is_some() {
            return vec![Scope::Player, Scope::Shell];
        }
        self.top().scope().into_iter().chain([Scope::Shell]).collect()
    }

    /// A playback session started (`AppEvent::PlaybackStarted`): the player opens, or a new
    /// session takes over the open one. The page is kept: it follows `now_playing` and starts
    /// over what belongs to one item (`PlayerPage`'s `follow_session`), so the last session's
    /// last frame stays up while the next one loads.
    pub fn player_started(&mut self, ctx: &egui::Context) {
        self.player.get_or_insert_with(player::PlayerPage::new);
        // The player takes the window, and the bar's keyboard and menus leave with the bar. The
        // search box would otherwise keep egui's keyboard (and the player's keys) while it's
        // hidden.
        self.bar_key = None;
        self.want_search_focus = false;
        self.search_focused = false;
        // A box on its way out or in stops with the bar gone: it would be stale when the bar
        // comes back.
        self.search_motion = None;
        self.syncplay_highlight = None;
        self.syncplay_scroll = false;
        egui::Popup::close_id(ctx, account_menu_id());
        // A popup left open while its button isn't drawn would keep egui's keyboard from the
        // player.
        egui::Popup::close_id(ctx, syncplay_panel_id());
        ctx.memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
        // The player takes every key: nothing else may keep egui's focus, or egui would keep
        // the keyboard (`keys.rs`) while the player's keys are needed.
        if let Some(focused) = ctx.memory(|memory| memory.focused())
            && focused != Id::unique(SEARCH_ID)
        {
            ctx.memory_mut(|memory| memory.surrender_focus(focused));
        }
        ctx.request_repaint();
    }

    /// Playback ended for good (`AppEvent::PlaybackEnded`): the player closes, leaving
    /// fullscreen.
    pub fn player_ended(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.player).is_some() {
            // `viewport().fullscreen` is a frame stale (a request applies at the next pass), so
            // ask for windowed unconditionally; a windowed window ignores it.
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        ctx.request_repaint();
    }

    /// Routes this frame's keys, then draws the bar and the top page, then applies the pages'
    /// navigation requests.
    pub fn show(&mut self, ui: &mut egui::Ui, keys: Vec<Press>, cx: &mut PageCx) {
        for press in keys {
            self.on_key(press, cx);
        }
        self.type_letters(cx);
        self.notify_update(cx);
        #[cfg(debug_assertions)]
        self.debug_play(cx);
        if let Some(player) = &mut self.player {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(over_video::BACKDROP))
                .show(ui, |ui| player.show(ui, cx));
        } else {
            // `main`'s bar keeps its contents 24 points from the window's edges; the pages own
            // their own 24 (or less, for a grid's own padding), so the panel adds none.
            egui::Panel::top("top_bar")
                .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(24, 0)))
                .show(ui, |ui| self.top_bar(ui, cx));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                .show(ui, |ui| {
                    // Each page's widgets and scroll areas live under its own id scope, so stacked
                    // pages can use the same names for theirs (two `"cast"` shelves, two
                    // `ScrollArea`s).
                    let entry = self.stack.last_mut().expect("the page stack is never empty");
                    let serial = entry.serial;
                    ui.push_id(serial, |ui| {
                        // A page rises in as it opens, as `main`'s did (the player takes the
                        // window on its own, without the entry).
                        motion::enter(ui, ("page", serial), 0.0, |ui| entry.page.show(ui, cx));
                    });
                });
        }
        self.navigate(cx);
    }

    /// A key the app took, in the routing order: the top bar while the keyboard is in it, then
    /// the page, then the shell's own.
    fn on_key(&mut self, press: Press, cx: &mut PageCx) {
        // An item the bar lost with a page (Back on the first page) can't keep the keyboard.
        if self
            .bar_key
            .is_some_and(|item| !bar_items(self.can_go_back()).contains(&item))
        {
            self.bar_key = None;
        }
        if let Some(player) = &mut self.player {
            if let Some(action) = press.action(Scope::Player) {
                player.on_key(action, cx);
                return;
            }
            // Leaving the player stops playback; it closes when the session ends.
            if let Some(Action::Back) = press.action(Scope::Shell) {
                cx.services.stop_playback();
            }
            return;
        }
        if self.bar_key.is_some() {
            // The bar has the keys, so the page keeps still even for the ones the bar ignores.
            if let Some(action) = self.top().scope().and_then(|scope| press.action(scope)) {
                self.bar_key_action(action, cx);
            } else if let Some(action) = press.action(Scope::Shell) {
                // Esc goes back to the page, not a page back, as on `main`.
                match action {
                    Action::Back => self.leave_top_bar(cx),
                    Action::OpenSearch => self.focus_search(cx),
                    _ => {}
                }
            }
            return;
        }
        let page = self.top_mut();
        if let Some(action) = page.scope().and_then(|scope| press.action(scope)) {
            if page.on_key(action, cx) {
                return;
            }
            match action {
                // The results page hands Up and Shift+Tab from its first row back to the box.
                Action::Move(Move::Up | Move::PrevSection) if self.top_is_search() => {
                    self.focus_search(cx);
                    return;
                }
                // Up past the page's first row moves into the top bar.
                Action::Move(Move::Up) => {
                    self.enter_top_bar();
                    return;
                }
                _ => {}
            }
        }
        if let Some(action) = press.action(Scope::Shell) {
            match action {
                Action::Back => cx.nav.push(NavRequest::Back),
                Action::OpenSearch => self.focus_search(cx),
                _ => {}
            }
        }
    }

    /// Text typed while the app has the keyboard goes to the top page (`Page::on_text`), for
    /// pages that read letters: the library's letter jump. Not while egui is busy with keys (a
    /// text field, a menu, a dialog), nor with the keyboard in the bar, which is where a typed
    /// letter would otherwise land behind the bar. The events stay in egui's input; nothing
    /// listens for text while no widget has the focus.
    fn type_letters(&mut self, cx: &mut PageCx) {
        if self.player.is_some() || self.bar_key.is_some() || self.search_focused || keys::egui_wants_keys(cx.ctx) {
            return;
        }
        let texts: Vec<String> = cx.ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        });
        for text in texts {
            self.top_mut().on_text(&text, cx);
        }
    }

    /// A key in the top bar (`keys.rs`'s `Scope::Browse`, which the bar borrows): along it,
    /// pressing what it's on, or back down to the page.
    fn bar_key_action(&mut self, action: Action, cx: &mut PageCx) {
        let items = bar_items(self.can_go_back());
        match action {
            Action::Move(Move::Left) => self.bar_key = bar_step(items, self.bar_key, -1),
            Action::Move(Move::Right) => self.bar_key = bar_step(items, self.bar_key, 1),
            Action::Move(Move::Down) => self.leave_top_bar(cx),
            // Up stays in the bar and Tab isn't the bar's, as on `main`; both are swallowed
            // rather than moving the page's cursor behind the bar's highlight.
            Action::Move(Move::Up | Move::NextSection | Move::PrevSection) => {}
            Action::Activate => {
                if let Some(item) = self.bar_key {
                    self.press_bar_item(item, cx);
                }
            }
            _ => {}
        }
    }

    /// Moves the keyboard into the top bar, from Up past the page's first row.
    fn enter_top_bar(&mut self) {
        self.bar_key = bar_step(bar_items(self.can_go_back()), self.bar_key, 0);
    }

    /// Back to the page from the bar or the search box, with its cursor shown where it was:
    /// a Down on the page only shows a hidden cursor, it doesn't move it.
    fn leave_top_bar(&mut self, cx: &mut PageCx) {
        self.bar_key = None;
        cx.ctx
            .memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
        self.top_mut().on_key(Action::Move(Move::Down), cx);
        cx.ctx.request_repaint();
    }

    /// Does what a bar item does when it's pressed.
    fn press_bar_item(&mut self, item: BarItem, cx: &mut PageCx) {
        match item {
            BarItem::Back => {
                self.bar_key = None;
                cx.nav.push(NavRequest::Back);
            }
            BarItem::Search => self.focus_search(cx),
            BarItem::SyncPlay => self.open_syncplay_panel(true, cx),
            BarItem::Settings => self.toggle_settings(cx),
            BarItem::Account => self.open_account_menu(cx.ctx),
        }
    }

    /// The SyncPlay panel's keys, which egui keeps while its popup is open (it closes on Esc
    /// itself). They're read before anything in the bar draws, as the account menu's are, so
    /// egui's focus pass can't hand the keyboard to a button or card behind the panel. Tab and
    /// the horizontal arrows stay in the panel, Backspace and Alt+Left close it like Esc,
    /// Ctrl/Cmd+F closes it and opens the search box, Up and Down move the highlight, Enter
    /// picks, and Alt+Up/Alt+Down move the highlighted queue entry with the highlight, Delete
    /// removes it.
    fn syncplay_panel_keys(&mut self, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, syncplay_panel_id()) {
            self.syncplay_highlight = None;
            return;
        }
        let ctx = cx.ctx;
        if keys::take_exact(ctx, Modifiers::NONE, Key::Tab) | keys::take_exact(ctx, Modifiers::SHIFT, Key::Tab) {
            cancel_focus(ctx);
        }
        take_horizontal_keys(ctx);
        if keys::take_exact(ctx, Modifiers::NONE, Key::Backspace)
            | keys::take_exact(ctx, Modifiers::ALT, Key::ArrowLeft)
        {
            self.close_syncplay_panel(ctx);
            return;
        }
        if keys::take_exact(ctx, Modifiers::COMMAND, Key::F) {
            self.close_syncplay_panel(ctx);
            self.focus_search(cx);
            return;
        }
        let entries = entries_of(&cx.services.syncplay);
        // The queue can change under the highlight (another member edits it, or a removal just
        // landed): keep it on an entry that's there, and off *Leave group* or a *Join* row when
        // its own entry has gone.
        resolve_highlight(&mut self.syncplay_highlight, &entries);
        let highlighted = self
            .syncplay_highlight
            .as_ref()
            .map(|highlight| highlight.entry.clone());
        let position = highlighted
            .as_ref()
            .and_then(|entry| entries.iter().position(|candidate| candidate == entry));
        let alt_up = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowUp);
        let alt_down = keys::take_exact(ctx, Modifiers::ALT, Key::ArrowDown);
        if alt_up | alt_down {
            let step = if alt_up { -1 } else { 1 };
            if let Some(index) = position
                && let Some(MenuEntry::QueueEntry(id)) = entries.get(index).cloned()
                && move_target(&entries, index, step).is_some()
            {
                cx.services.syncplay.move_entry(id, step);
                // The highlight is the entry, so it follows it to its new place once the queue
                // update lands; `main` moved an index instead (see §6).
                self.syncplay_scroll = true;
            }
            cancel_focus(ctx);
            return;
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Delete)
            && let Some(MenuEntry::QueueEntry(id)) = highlighted
        {
            cx.services.syncplay.remove_entry(id);
            // The highlight stays on the entry until the queue update takes it away; then it
            // falls back to a neighbouring queue entry, never to *Leave group*.
            self.syncplay_scroll = true;
            return;
        }
        let down = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowDown);
        let up = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowUp);
        if down | up {
            let step = if down { 1 } else { -1 };
            self.syncplay_highlight =
                highlight_step(position, step, entries.len()).and_then(|index| Highlight::at(&entries, index));
            self.syncplay_scroll = true;
            cancel_focus(ctx);
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Enter)
            && let Some(entry) = highlighted
        {
            self.choose_syncplay_entry(entry, true, cx);
        }
    }

    /// Opens the SyncPlay panel under its button, with fresh groups from the server. From the
    /// keyboard the first entry is highlighted; from a click none is, until an arrow picks
    /// one, as the account menu does.
    fn open_syncplay_panel(&mut self, from_keyboard: bool, cx: &mut PageCx) {
        let first = entries_of(&cx.services.syncplay).into_iter().next();
        self.syncplay_highlight = if from_keyboard {
            first.map(|entry| Highlight { entry, index: 0 })
        } else {
            None
        };
        self.syncplay_scroll = true;
        cx.services.syncplay.refresh_groups();
        egui::Popup::open_id(cx.ctx, syncplay_panel_id());
        cx.ctx.request_repaint();
    }

    fn close_syncplay_panel(&mut self, ctx: &egui::Context) {
        egui::Popup::close_id(ctx, syncplay_panel_id());
        self.syncplay_highlight = None;
        self.syncplay_scroll = false;
        ctx.request_repaint();
    }

    /// Opens the panel if it's closed and closes it if it's open, for the button's click.
    fn toggle_syncplay_panel(&mut self, cx: &mut PageCx) {
        if egui::Popup::is_id_open(cx.ctx, syncplay_panel_id()) {
            self.close_syncplay_panel(cx.ctx);
        } else {
            self.open_syncplay_panel(false, cx);
        }
    }

    /// Does what a SyncPlay entry says. Shuffle and repeat leave the panel open, to see the
    /// change; anything else closes it, and a choice made with the keyboard gives the keyboard
    /// back to the page, as `main`'s `leave_top_bar` did. A click only closes the panel: the
    /// page's cursor stays where it was (`main`'s clickable rows never went near it).
    fn choose_syncplay_entry(&mut self, entry: MenuEntry, from_keyboard: bool, cx: &mut PageCx) {
        match entry {
            MenuEntry::ToggleShuffle => return cx.services.syncplay.toggle_shuffle(),
            MenuEntry::CycleRepeat => return cx.services.syncplay.cycle_repeat(),
            _ => {}
        }
        self.close_syncplay_panel(cx.ctx);
        if from_keyboard {
            self.leave_top_bar(cx);
        }
        match entry {
            MenuEntry::JoinGroup(id) => cx.services.syncplay.join(id),
            MenuEntry::NewGroup => {
                let name = new_group_name(&cx.services.session);
                cx.services.syncplay.create(name);
            }
            MenuEntry::QueueEntry(id) => cx.services.syncplay.play_entry(id),
            MenuEntry::LeaveGroup => cx.services.syncplay.leave(),
            MenuEntry::ToggleShuffle | MenuEntry::CycleRepeat => {}
        }
    }

    /// Opens the account menu from the keyboard, its first entry highlighted, loading the saved
    /// accounts as `main` did when its menu opened.
    fn open_account_menu(&mut self, ctx: &egui::Context) {
        self.accounts = Config::load().accounts;
        PopupMenu::open(ctx, account_menu_id(), Opened::Keyboard);
        ctx.request_repaint();
    }

    /// Puts the keyboard in the search box, opening it if it's the button. The page's cursor
    /// goes, so Esc back to the page shows it where it was instead of moving it a row.
    fn focus_search(&mut self, cx: &mut PageCx) {
        // A box on its way back into the button turns round and opens again, as on `main`.
        let closing = self.search_motion.is_some_and(|motion| motion.closing);
        if !self.search_shown() || closing {
            self.start_search_motion(false, cx.ctx);
        }
        self.search_open = true;
        self.want_search_focus = true;
        self.bar_key = None;
        self.top_mut().hide_cursor();
        cx.ctx.request_repaint();
    }

    /// Turns the box back into the button, unless something's typed in it (it stays open with
    /// the results), and moves it back there. `main`'s `close_search`.
    fn close_search(&mut self, ctx: &egui::Context) {
        let was_open = std::mem::take(&mut self.search_open);
        if !was_open || !self.search_text.trim().is_empty() {
            return;
        }
        // The field is on its way out: a widget that stops drawing mustn't keep egui's
        // keyboard, or the app would stop taking its own keys (`keys::egui_wants_keys`).
        ctx.memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
        self.start_search_motion(true, ctx);
        ctx.request_repaint();
    }

    /// Starts the box opening out of the button or closing back into it. Without the button's
    /// place (the box opened before the bar ever drew) or with the animations setting off, the
    /// box just appears or goes, as `main`'s did without the button's position.
    fn start_search_motion(&mut self, closing: bool, ctx: &egui::Context) {
        let from = if closing { self.search_slot } else { self.search_button };
        let Some(from) = from.filter(|_| theme::animations(ctx)) else {
            self.search_motion = None;
            return;
        };
        self.search_motion = Some(SearchMotion {
            closing,
            started: ctx.input(|input| input.time),
            from,
        });
        ctx.request_repaint();
    }

    /// Ends the box's motion once it has run its time (or the animations setting went off), and
    /// asks for the next frame only while it runs.
    fn step_search_motion(&mut self, ctx: &egui::Context) {
        let Some(motion) = self.search_motion else {
            return;
        };
        let now = ctx.input(|input| input.time);
        if !theme::animations(ctx) || motion.progress(now) >= 1. {
            // Closed, the box is the button again; open, it stays where it has arrived.
            self.search_motion = None;
        } else {
            ctx.request_repaint();
        }
    }

    /// The Settings button: opens Settings, or closes it (back to the page before) when it's on
    /// top, as on `main`.
    fn toggle_settings(&mut self, cx: &mut PageCx) {
        self.bar_key = None;
        let on_top = self
            .stack
            .last()
            .is_some_and(|entry| matches!(entry.route, Route::Settings));
        let request = if on_top {
            NavRequest::Back
        } else {
            NavRequest::Push(Route::Settings)
        };
        cx.nav.push(request);
        cx.ctx.request_repaint();
    }

    fn can_go_back(&self) -> bool {
        self.stack.len() > 1
    }

    /// The results page's place on the stack, if it's there.
    fn search_page(&self) -> Option<usize> {
        self.stack
            .iter()
            .rposition(|entry| matches!(entry.route, Route::Search))
    }

    /// Whether the results page is the one on top.
    fn top_is_search(&self) -> bool {
        self.search_page() == Some(self.stack.len() - 1)
    }

    /// Whether the box is in the bar rather than the search button (it stays while it has
    /// text, as on `main`, and while it closes back into the button).
    fn search_shown(&self) -> bool {
        self.search_open
            || !self.search_text.trim().is_empty()
            || self.search_motion.is_some_and(|motion| motion.closing)
    }

    /// The box changed: the results open as soon as there's a term (or come back from a page
    /// opened from them), and close when it's empty.
    fn search_changed(&mut self, cx: &mut PageCx) {
        let term = self.search_text.trim().to_owned();
        if term.is_empty() {
            if let Some(index) = self.search_page() {
                self.stack.remove(index);
            }
            cx.ctx.request_repaint();
            return;
        }
        let index = match self.search_page() {
            Some(index) => {
                self.stack.truncate(index + 1);
                index
            }
            None => {
                self.push(Route::Search, cx);
                self.stack.len() - 1
            }
        };
        self.stack[index].page.set_search_term(&term, cx);
        cx.ctx.request_repaint();
    }

    /// The search box's keys, which egui's `TextEdit` would otherwise take: Esc gives the
    /// keyboard back, Enter and Down move into the results, and Tab stays in the box. Read
    /// before the box draws, so they don't also move its caret. Modifiers match exactly: Shift
    /// or Alt with a key is the widget's own (Alt+Down mustn't open the results).
    fn search_box_keys(&mut self, cx: &mut PageCx) {
        if !self.search_shown() || !self.search_focused {
            return;
        }
        let ctx = cx.ctx;
        if keys::take_exact(ctx, Modifiers::NONE, Key::Escape) {
            self.escape_search(cx);
            return;
        }
        // Tab is swallowed: handing it to egui would move the focus to a button behind the box
        // and the app would lose the keyboard (the box's filter keeps egui from taking it).
        keys::take_exact(ctx, Modifiers::NONE, Key::Tab);
        keys::take_exact(ctx, Modifiers::SHIFT, Key::Tab);
        let enter = keys::take_exact(ctx, Modifiers::NONE, Key::Enter);
        let down = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowDown);
        if enter || down {
            self.enter_search_results(cx);
        }
    }

    /// Esc in the search box: over the results, closes them and clears the box, back to the
    /// page they were opened from; over another page, back to that page, as on `main`.
    fn escape_search(&mut self, cx: &mut PageCx) {
        if self.top_is_search() {
            self.stack.pop();
            self.search_text.clear();
            self.close_search(cx.ctx);
            cx.ctx.request_repaint();
        } else {
            self.leave_top_bar(cx);
        }
    }

    /// Enter or Down in the search box. With the results on top it asks them to take the
    /// keyboard (`Page::enter_results`); if they have nothing to move onto yet (still
    /// searching, or nothing found) the box keeps it. With no results page on top (the box
    /// over another page) the keyboard goes back to that page instead, as on `main`.
    fn enter_search_results(&mut self, cx: &mut PageCx) {
        if self.top_is_search() {
            let took = self.stack.last_mut().is_some_and(|entry| entry.page.enter_results(cx));
            if took {
                cx.ctx
                    .memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
                cx.ctx.request_repaint();
            }
        } else {
            self.leave_top_bar(cx);
        }
    }

    /// Draws the bar, a fixed height of it: the icon, Back and the title on the left, the
    /// buttons on the right, and the open search box between them (opening out of the search
    /// button's place, and shrinking back into it as it closes).
    fn top_bar(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        self.step_search_motion(cx.ctx);
        self.search_box_keys(cx);
        self.syncplay_panel_keys(cx);
        let can_go_back = self.can_go_back();
        let keyboard = self.bar_key;
        let box_shown = self.search_shown();
        // The button shows while the box is closed, and fades out under the box as it opens (in
        // as it closes), so one turns into the other, as on `main`.
        let now = ui.input(|input| input.time);
        let (button_opacity, box_motion) = match self.search_motion {
            Some(motion) => {
                let t = motion.progress(now);
                if motion.closing {
                    (t, Some(t))
                } else {
                    (1. - t, Some(t))
                }
            }
            None => (1., None),
        };
        let button_shown = !box_shown || self.search_motion.is_some();
        let in_group = cx.services.in_group();
        let title = self.top().title().to_owned();
        let logo = self.logo.clone();
        let gap = ui.spacing().item_spacing.x;

        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
        // The right-hand side: the account, Settings, SyncPlay and the search button (whose
        // slot is kept while the box is open, so the other three keep their places as on
        // `main`).
        let buttons_w = 4. * BAR_BUTTON + 3. * gap;
        // With the box open both sides grow alike, which keeps the box in the middle of the bar.
        // It shrinks when the window or the zoom leaves no room for one next to the icon, Back
        // and a title on the left and the buttons on the right.
        let icon_and_back = LOGO + BAR_BUTTON + 2. * gap;
        let box_w = (bar.width() * 0.5)
            .min(SEARCH_BOX_W)
            .min(bar.width() - 2. * buttons_w - 4. * gap)
            .min(bar.width() - 2. * (icon_and_back + 60.) - 4. * gap)
            .max(120.);
        let side = if box_shown {
            ((bar.width() - box_w) * 0.5 - 2. * gap).max(0.)
        } else {
            bar.width() - buttons_w - 2. * gap
        };
        let right_w = side.max(buttons_w);

        let left = Rect::from_min_size(bar.min, vec2(side, bar.height()));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(left)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let (rect, _) = ui.allocate_exact_size(vec2(LOGO, LOGO), Sense::hover());
                if let Some(logo) = &logo {
                    ui.painter().image(
                        logo.id(),
                        rect,
                        Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
                if can_go_back {
                    let back = bar_button(ui, ICON_ARROW_BACK, "Back", keyboard == Some(BarItem::Back), false);
                    if back.clicked() {
                        // A mouse click leaves no keyboard in the bar, as on `main` going back
                        // focused the page.
                        self.bar_key = None;
                        cx.nav.push(NavRequest::Back);
                    }
                }
                // `main`'s bar title: `text_xl` semibold.
                ui.add(egui::Label::new(RichText::new(title).font(text::font(ui, "xl")).strong()).truncate());
            },
        );

        let right = Rect::from_min_size(
            egui::pos2(bar.right() - right_w, bar.top()),
            vec2(right_w, bar.height()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(right)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                self.account_button(ui, cx, keyboard == Some(BarItem::Account));
                let settings = bar_button(
                    ui,
                    ICON_SETTINGS,
                    "Settings",
                    keyboard == Some(BarItem::Settings),
                    false,
                );
                if settings.clicked() {
                    self.toggle_settings(cx);
                }
                // Pressed while in a group, as on `main`.
                let syncplay = bar_button(
                    ui,
                    ICON_GROUPS,
                    "Watch together (SyncPlay)",
                    keyboard == Some(BarItem::SyncPlay),
                    in_group,
                );
                if syncplay.clicked() {
                    // A click leaves no keyboard in the bar (the panel's keys are
                    // `syncplay_panel_keys`').
                    self.bar_key = None;
                    self.toggle_syncplay_panel(cx);
                }
                self.syncplay_panel(&syncplay, cx);
                // The search button, or its empty slot while the box has the middle of the bar.
                if button_shown {
                    let search = motion::faded(ui, button_opacity, |ui| {
                        bar_button(ui, ICON_SEARCH, "Search", keyboard == Some(BarItem::Search), false)
                    });
                    self.search_button = Some(search.rect);
                    if search.clicked() {
                        self.focus_search(cx);
                    }
                } else {
                    let (rect, _) = ui.allocate_exact_size(vec2(BAR_BUTTON, BAR_BUTTON), Sense::hover());
                    self.search_button = Some(rect);
                }
            },
        );

        if box_shown {
            let resting = Rect::from_center_size(bar.center(), vec2(box_w, BAR_BUTTON));
            // While it moves: out of the button into its place, with the button fading the other
            // way, or back into the button (where it was last drawn) from its place.
            let (rect, opacity) = match (self.search_motion, box_motion) {
                (Some(motion), Some(t)) => {
                    let to = if motion.closing {
                        self.search_button.unwrap_or(resting)
                    } else {
                        resting
                    };
                    let opacity = if motion.closing { 1. - t } else { t };
                    (lerp_rect(motion.from, to, t), opacity)
                }
                _ => (resting, 1.),
            };
            self.search_slot = Some(rect);
            let moving = box_motion.is_some();
            self.search_box(ui, rect, opacity, moving, keyboard == Some(BarItem::Search), cx);
        } else {
            self.search_focused = false;
            self.search_slot = None;
        }

        // A click on the page takes the keyboard out of the bar, as it hides the page's cursor.
        let pressed_outside = ui.input(|input| input.pointer.any_pressed())
            && !ui
                .input(|input| input.pointer.interact_pos())
                .is_some_and(|pos| bar.contains(pos));
        if pressed_outside {
            self.bar_key = None;
        }
    }

    /// The user's button and menu: the saved accounts to switch to (the current one checked),
    /// adding one, and signing out. The app carries the choice out after the frame.
    fn account_button(&mut self, ui: &mut egui::Ui, cx: &mut PageCx, highlighted: bool) {
        let name = cx.services.session.user.name.clone();
        let tooltip = match &cx.services.session.server.server_name {
            Some(server) => format!("{name} on {server}"),
            None => name.clone(),
        };
        let (rect, response) = ui.allocate_exact_size(vec2(BAR_BUTTON, BAR_BUTTON), Sense::click());
        // `main`'s avatar: `muted` behind the initial in the foreground colour.
        let tokens = theme::tokens_of(ui);
        let fill = if response.hovered() {
            tokens.accent
        } else {
            tokens.muted
        };
        let center = rect.center();
        ui.painter().circle_filled(center, 16., fill);
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            initial(&name),
            text::font(ui, "base"),
            tokens.foreground,
        );
        if highlighted {
            ring(ui, &response);
        }
        let response = response.on_hover_text(tooltip);
        let id = account_menu_id();
        if response.clicked() {
            // A click leaves no keyboard in the bar; the menu takes it from here.
            self.bar_key = None;
            if !PopupMenu::is_open(ui.ctx(), id) {
                self.accounts = Config::load().accounts;
            }
            PopupMenu::toggle(ui.ctx(), id, Opened::Pointer);
        }

        // The saved accounts, the current one checked and each with its server at the end, then
        // adding one and signing out.
        let current = cx.services.session.account_key();
        let mut entries: Vec<Entry> = self
            .accounts
            .iter()
            .map(|account| {
                let server = account.server_name.as_deref().unwrap_or(&account.url);
                Entry::new(account.username.as_str())
                    .detail(server)
                    .checked(account.credential_account() == current)
            })
            .collect();
        entries.push(Entry::new("Add account…").checked(false).separated());
        entries.push(Entry::new("Sign out").checked(false));
        let shown = PopupMenu::new(id)
            .align(RectAlign::BOTTOM_END)
            .width(ACCOUNT_MENU_W)
            .show(&response, &entries);
        if shown.search {
            self.focus_search(cx);
        }
        if let Some(index) = shown.chosen {
            self.choose_account(index, cx);
        }
    }

    /// Does what an account-menu entry says: switch to that account, add one, or sign out.
    /// The app carries the event out after the frame.
    fn choose_account(&mut self, index: usize, cx: &mut PageCx) {
        let actions = self.accounts.len();
        if index < actions {
            let account = self.accounts[index].clone();
            // Choosing the account already signed in just closes the menu, as on `main`:
            // switching to it would end the session and lose the page stack. The keyboard stays
            // in the bar, like a menu that closed without a choice.
            if account.credential_account() != cx.services.session.account_key() {
                self.bar_key = None;
                cx.services.events.push(AppEvent::SwitchAccount(account));
            }
        } else {
            // The screen changes: the bar's keyboard goes with it.
            self.bar_key = None;
            if index == actions {
                cx.services.events.push(AppEvent::AddAccount);
            } else {
                cx.services.events.push(AppEvent::SignOut);
            }
        }
    }

    /// The SyncPlay button's panel: the server's groups to join, or the group this session is
    /// in with its queue. Drawn from the button it hangs under, so it follows the bar. A click
    /// inside may step a toggle or press a queue row's button; the entries that move on close it
    /// themselves.
    fn syncplay_panel(&mut self, button: &Response, cx: &mut PageCx) {
        let chosen = popup::panel(
            syncplay_panel_id(),
            button,
            RectAlign::BOTTOM_END,
            SYNCPLAY_PANEL_W,
            |ui| self.syncplay_content(ui, cx),
        )
        .flatten();
        if let Some(entry) = chosen {
            // A click closes the panel where it is: the page's cursor stays put.
            self.choose_syncplay_entry(entry, false, cx);
        }
    }

    /// The panel's body, and the entry a click picked (the caller carries it out).
    fn syncplay_content(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) -> Option<MenuEntry> {
        let state = &cx.services.syncplay;
        let highlight = self.syncplay_highlight.as_ref().map(|highlight| &highlight.entry);
        let mut chosen = None;
        match &state.group {
            Some(group) => {
                group_header(ui, group);
                popup::separator(ui);
                let scroll = std::mem::take(&mut self.syncplay_scroll);
                if let Some(entry) = group_queue(ui, state, highlight, scroll) {
                    chosen = Some(entry);
                }
                let leave = Entry::new("Leave group").icon(ICON_LOGOUT).danger();
                if popup::row(ui, &leave, true, matches!(highlight, Some(MenuEntry::LeaveGroup))).clicked() {
                    chosen = Some(MenuEntry::LeaveGroup);
                }
            }
            None => {
                header_text(ui, |ui| {
                    ui.add(egui::Label::new(panel_title(ui, "Watch together")).truncate());
                    ui.add(egui::Label::new(panel_note(
                        ui,
                        "Play in sync with others on this server.",
                    )));
                });
                popup::separator(ui);
                if state.groups.is_empty() {
                    header_text(ui, |ui| {
                        ui.add(egui::Label::new(panel_note(
                            ui,
                            "No one is watching together right now.",
                        )));
                    });
                }
                for group in &state.groups {
                    let watching = format!("{} watching", group.participants.len());
                    let row = Entry::new(group.group_name.as_str()).icon(ICON_GROUPS).detail(watching);
                    let highlighted = matches!(highlight, Some(MenuEntry::JoinGroup(id)) if id == &group.group_id);
                    let response = popup::row(ui, &row, true, highlighted)
                        .on_hover_text(format!("Join: {}", people(&group.participants)));
                    if response.clicked() {
                        chosen = Some(MenuEntry::JoinGroup(group.group_id.clone()));
                    }
                }
                if !state.groups.is_empty() {
                    popup::separator(ui);
                }
                let new_group = Entry::new("New group").icon(ICON_ADD);
                if popup::row(ui, &new_group, true, matches!(highlight, Some(MenuEntry::NewGroup))).clicked() {
                    chosen = Some(MenuEntry::NewGroup);
                }
            }
        }
        chosen
    }

    /// The search box, centred in the bar while it's open; `rect` is where it is while it opens
    /// or closes, `opacity` its fade with the button, and `clip` holds the field to that
    /// rectangle while it moves (the field can't get as narrow as the button).
    fn search_box(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        opacity: f32,
        clip: bool,
        highlighted: bool,
        cx: &mut PageCx,
    ) {
        // `Memory::has_focus`, not `Response::has_focus`: the latter also wants the window to be
        // focused, which an Xvfb display without a window manager never is.
        let focused = ui.ctx().memory(|memory| memory.has_focus(Id::unique(SEARCH_ID)));
        // `main`'s field puts the search icon inside its left padding, a gap after it, and the
        // text after that (`gpui-kit`'s input prefix): 10 points, the icon at the text size, and
        // 6 points between them.
        let icon = text::icon(ui, ICON_SEARCH.font_family());
        let icon_size = icon.size;
        let text_inset = (10. + icon_size + 6.).round() as i8;
        let clear_inset = (10. + icon_size + 6.).round() as i8;
        let mut clear_clicked = false;
        let response = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    if clip {
                        // Only while it moves: otherwise the field's focus ring would be cut in
                        // half by its own rectangle.
                        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                    }
                    // The box locks the focus keys: egui must neither move the keyboard on
                    // Tab nor drop it on Esc before the box reads Esc (`search_box_keys`), or
                    // the app would stop getting keys until something was clicked.
                    motion::faded(ui, opacity, |ui| {
                        // `main`'s field was `cleanable`: a clear button at its right end while
                        // it had text, which leaves the text room as the prefix icon does.
                        let clear = !self.search_text.is_empty();
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.search_text)
                                .id(Id::unique(SEARCH_ID))
                                .hint_text("Search")
                                .desired_width(rect.width())
                                // `main`'s 40-point field: a `text_sm` line with 10 points above
                                // and below it, and the icon's room on the left.
                                .margin(egui::Margin {
                                    left: text_inset,
                                    right: if clear { clear_inset } else { 10 },
                                    top: 9,
                                    bottom: 9,
                                })
                                .event_filter(egui::EventFilter {
                                    tab: true,
                                    horizontal_arrows: true,
                                    vertical_arrows: true,
                                    escape: true,
                                }),
                        );
                        // Drawn over the field, or the field's own background would cover it.
                        ui.painter().text(
                            egui::pos2(rect.left() + 10. + icon_size / 2., rect.center().y),
                            Align2::CENTER_CENTER,
                            ICON_SEARCH.codepoint,
                            icon.clone(),
                            theme::tokens_of(ui).muted_foreground,
                        );
                        if clear {
                            let center = egui::pos2(rect.right() - 10. - icon_size / 2., rect.center().y);
                            // Not focusable (R6's rule for painted controls): a Tab that gave egui
                            // the keyboard would take the box's keys. Its AccessKit name says what
                            // it does.
                            let hit = Rect::from_center_size(center, vec2(icon_size + 8., icon_size + 8.));
                            let button = ui.interact(hit, Id::unique("top_bar_search_clear"), Sense::CLICK);
                            button.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), "Clear search"));
                            // gpui-kit's text button: the foreground at 90 % at rest, and the
                            // foreground under the pointer.
                            let tokens = theme::tokens_of(ui);
                            let color = if button.hovered() {
                                tokens.foreground
                            } else {
                                tokens.foreground.gamma_multiply(0.9)
                            };
                            ui.painter()
                                .text(center, Align2::CENTER_CENTER, ICON_CLOSE.codepoint, icon, color);
                            clear_clicked = button.clicked();
                        }
                        response
                    })
                },
            )
            .inner;
        // Clearing keeps the keyboard in the box, as `main`'s clear button did; the results
        // close through `search_changed` from the empty text, as clearing with the keyboard does.
        if clear_clicked {
            self.search_text.clear();
            self.want_search_focus = true;
        }
        if std::mem::take(&mut self.want_search_focus) {
            response.request_focus();
        }
        // A click into the box takes the keyboard as `focus_search` does: the bar's keyboard
        // and the page's cursor go, so leaving the box doesn't land on either.
        if response.gained_focus() {
            self.bar_key = None;
            self.top_mut().hide_cursor();
        }
        self.search_focused = ui.ctx().memory(|memory| memory.has_focus(Id::unique(SEARCH_ID)));
        if response.changed() || clear_clicked {
            self.search_changed(cx);
        }
        // Closed, the box goes once it has nothing left to show, as `main`'s did on blur. A
        // clear click is the one way to empty it without leaving it.
        if !clear_clicked && response.lost_focus() && self.search_text.trim().is_empty() {
            self.close_search(cx.ctx);
        }
        if highlighted && !focused {
            ring(ui, &response);
        }
    }

    fn navigate(&mut self, cx: &mut PageCx) {
        if cx.nav.is_empty() {
            return;
        }
        for request in std::mem::take(cx.nav) {
            match request {
                NavRequest::Push(route) => self.push(route, cx),
                NavRequest::Back => self.back(cx),
                NavRequest::OpenSearch => self.focus_search(cx),
            }
        }
        // The new top page draws on the next frame.
        cx.ctx.request_repaint();
    }

    fn push(&mut self, route: Route, cx: &mut PageCx) {
        let page = views::open(route.clone(), cx);
        self.stack.push(StackEntry {
            serial: next_serial(),
            route,
            page,
        });
    }

    /// Closes the top page, unless it's Home. Leaving the results clears the box they were
    /// for; coming back to them (from an item opened there) puts the keyboard in the box again,
    /// as `main` did.
    fn back(&mut self, cx: &mut PageCx) {
        if self.stack.len() > 1
            && let Some(entry) = self.stack.pop()
            && matches!(entry.route, Route::Search)
        {
            self.search_text.clear();
            self.close_search(cx.ctx);
        } else if self.top_is_search() {
            self.focus_search(cx);
        }
    }

    fn top(&self) -> &dyn Page {
        self.stack.last().expect("the page stack is never empty").page.as_ref()
    }

    fn top_mut(&mut self) -> &mut dyn Page {
        self.stack
            .last_mut()
            .expect("the page stack is never empty")
            .page
            .as_mut()
    }

    /// When the update check finds a newer release, says so with a link to it.
    fn notify_update(&mut self, cx: &mut PageCx) {
        let Some(result) = self.update_check.as_mut().and_then(Task::poll) else {
            return;
        };
        self.update_check = None;
        if let Some(update) = result {
            let text = format!(
                "Version {} is out (this is {}).",
                update.version,
                env!("CARGO_PKG_VERSION")
            );
            let toast = Toast::info(text)
                .title("Update available")
                .action("Download", ToastAction::OpenUrl(update.url));
            cx.services.toast(toast);
        }
    }

    /// Dev aid: `TABBYFIN_ROUTE` opens a page on top of Home (`debug::Start`).
    #[cfg(debug_assertions)]
    fn debug_start(&mut self, cx: &mut PageCx) {
        use crate::debug::Start;

        match Start::from_env() {
            Some(Start::Open(route)) => {
                tracing::info!("TABBYFIN_ROUTE: opening {route:?}");
                self.push(route, cx);
            }
            Some(Start::Search(term)) => {
                tracing::info!("TABBYFIN_ROUTE: typing {term:?} into the search box");
                self.search_text = term;
                self.search_open = true;
                self.want_search_focus = true;
                self.search_changed(cx);
            }
            Some(Start::Play {
                id,
                audio,
                subtitle,
                source,
            }) => {
                let options = playback::PlayOptions {
                    audio_stream_index: audio,
                    subtitle: match subtitle {
                        Some(-1) => playback::SubtitleChoice::Off,
                        Some(index) => playback::SubtitleChoice::Stream(index),
                        None => playback::SubtitleChoice::Auto,
                    },
                    media_source_id: source,
                    ..Default::default()
                };
                let client = cx.services.session.client.clone();
                let item = {
                    let id = id.clone();
                    Task::spawn(cx.ctx, async move { client.item(&id, &[]).await })
                };
                self.debug_play = Some((item, options));
                let name = "Item".to_owned();
                self.push(Route::Item { id, name }, cx);
            }
            None => {}
        }
    }
}

impl Shell {
    /// Dev aid: plays the `TABBYFIN_ROUTE=play:…` item once it's fetched, from its resume point
    /// with the tracks given. With `TABBYFIN_SYNCPLAY` too, it waits until the group is joined,
    /// so the item plays for the group.
    #[cfg(debug_assertions)]
    fn debug_play(&mut self, cx: &mut PageCx) {
        if std::env::var_os("TABBYFIN_SYNCPLAY").is_some() && !cx.services.in_group() {
            return;
        }
        let Some(result) = self.debug_play.as_mut().and_then(|(item, _)| item.poll()) else {
            return;
        };
        let Some((_, options)) = self.debug_play.take() else {
            return;
        };
        match result {
            Ok(item) => {
                let start = item
                    .user_data
                    .as_ref()
                    .map(|d| d.playback_position_ticks)
                    .filter(|t| t.0 > 0);
                cx.services.play(&item, playback::PlayOptions { start, ..options });
            }
            Err(err) => tracing::error!("TABBYFIN_ROUTE: couldn't load the item to play: {err}"),
        }
    }
}

/// A page on the stack and the number that scopes its ids.
struct StackEntry {
    /// Different for every page ever pushed, so two pages of the same kind don't share widget
    /// state (a shelf's scroll offset, a scroll area's position).
    serial: u64,
    /// What it was opened for, which the shell matches on for Settings and the search results.
    route: Route,
    page: Box<dyn Page>,
}

/// A top-bar icon button: every item takes the same slot, with the keyboard's ring around it
/// while the bar's keyboard is on it.
fn bar_button(ui: &mut Ui, icon: MaterialIcon, tooltip: &str, highlighted: bool, selected: bool) -> Response {
    // No frame at rest, as `main`'s ghost buttons had; the pointer and the keyboard's ring show
    // where the button is.
    let button = egui::Button::new(icon.rich_text().size(BAR_ICON))
        .min_size(vec2(BAR_BUTTON, BAR_BUTTON))
        .frame_when_inactive(false);
    let button = theme::selected(ui, selected, button);
    let response = ui.add(button).on_hover_text(tooltip);
    if highlighted {
        ring(ui, &response);
    }
    response
}

/// The SyncPlay panel's id in egui's popup memory, where `Popup::is_any_open` looks for it.
fn syncplay_panel_id() -> Id {
    Id::unique(SYNCPLAY_PANEL)
}

/// The name *New group* gives the group: `main`'s `"{user}'s group"`.
fn new_group_name(session: &Session) -> String {
    format!("{}'s group", session.user.name)
}

/// What the SyncPlay panel lists, in the order the keyboard walks it: with a group, its queue's
/// toggles and entries and *Leave group*; without one, the server's groups and *New group*.
fn syncplay_entries(group: Option<&GroupInfo>, groups: &[GroupInfo], queue: Option<&PlayQueue>) -> Vec<MenuEntry> {
    match group {
        Some(_) => {
            let mut entries = Vec::new();
            if let Some(queue) = queue.filter(|queue| !queue.playlist.is_empty()) {
                entries.push(MenuEntry::ToggleShuffle);
                entries.push(MenuEntry::CycleRepeat);
                entries.extend(
                    queue
                        .playlist
                        .iter()
                        .map(|entry| MenuEntry::QueueEntry(entry.playlist_item_id.clone())),
                );
            }
            entries.push(MenuEntry::LeaveGroup);
            entries
        }
        None => {
            let mut entries: Vec<MenuEntry> = groups
                .iter()
                .map(|group| MenuEntry::JoinGroup(group.group_id.clone()))
                .collect();
            entries.push(MenuEntry::NewGroup);
            entries
        }
    }
}

/// What [`syncplay_entries`] lists for the panel's current state.
fn entries_of(syncplay: &SyncPlay) -> Vec<MenuEntry> {
    syncplay_entries(syncplay.group.as_ref(), &syncplay.groups, syncplay.queue.as_ref())
}

/// Keeps the panel's highlight on an entry the list still has: the queue can change under it
/// (another member edits it, or a removal just landed). An entry that has gone falls back to a
/// neighbouring queue entry, the one that took its place (the next, else the previous), and
/// otherwise to no highlight at all: never to *Leave group*, *Join* or *New group*.
fn resolve_highlight(highlight: &mut Option<Highlight>, entries: &[MenuEntry]) {
    let Some(current) = highlight else {
        return;
    };
    if let Some(index) = entries.iter().position(|entry| entry == &current.entry) {
        current.index = index;
        return;
    }
    let from = current.index.min(entries.len());
    let next = entries[from..]
        .iter()
        .position(|entry| matches!(entry, MenuEntry::QueueEntry(_)))
        .map(|offset| from + offset);
    *highlight = next
        .or_else(|| {
            entries[..from]
                .iter()
                .rposition(|entry| matches!(entry, MenuEntry::QueueEntry(_)))
        })
        .and_then(|index| Highlight::at(entries, index));
}

/// One step of the panel's keyboard highlight: Down goes to the first entry when none is picked
/// and stops at the last, Up goes to the last and stops at the first, as `main`'s menu did.
fn highlight_step(current: Option<usize>, step: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, step) {
        (None, 1..) => 0,
        (None, _) => len - 1,
        (Some(index), 1..) => (index + 1).min(len - 1),
        (Some(index), _) => index.saturating_sub(1),
    })
}

/// Where Alt+Up or Alt+Down would move the entry at `from`: only a queue entry swaps with
/// another queue entry, so the toggles before the queue and *Leave group* after it stay put.
fn move_target(entries: &[MenuEntry], from: usize, by: isize) -> Option<usize> {
    if !matches!(entries.get(from), Some(MenuEntry::QueueEntry(_))) {
        return None;
    }
    let target = from.checked_add_signed(by)?;
    matches!(entries.get(target), Some(MenuEntry::QueueEntry(_))).then_some(target)
}

/// The panel's text above its rows, inset to line up with the rows' labels' left edge. Left
/// aligned: in the panel's justified layout a wrapped line would be stretched to both edges.
fn header_text(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(popup::INSET as i8, 8))
        .show(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.;
                content(ui);
            });
        });
}

fn panel_title(ui: &Ui, title: &str) -> RichText {
    RichText::new(title)
        .font(text::font(ui, "base"))
        .color(theme::tokens_of(ui).foreground)
        .strong()
}

fn panel_note(ui: &Ui, note: &str) -> RichText {
    RichText::new(note)
        .font(text::font(ui, "xs"))
        .color(theme::tokens_of(ui).muted_foreground)
}

/// Who's in a group, as a sentence: "dev", "dev and ana", "dev, ana and bo".
fn people(names: &[String]) -> String {
    match names {
        [] => "nobody yet".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The group's name with its state beside it as a coloured dot, who's in it, and what being in
/// it means.
fn group_header(ui: &mut Ui, group: &GroupInfo) {
    let tokens = theme::tokens_of(ui);
    let (status, dot) = match group.state {
        GroupState::Playing => ("Playing", tokens.success),
        GroupState::Waiting => ("Waiting for everyone", tokens.warning),
        GroupState::Paused => ("Paused", tokens.muted_foreground),
        GroupState::Idle => ("Nothing playing", tokens.muted_foreground),
    };
    header_text(ui, |ui| {
        // A row is wrapped in `ui.horizontal` so that it measures only itself: a `with_layout`
        // directly in a vertical `Ui` would take the height left in the panel.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                ui.label(panel_note(ui, status));
                let (rect, _) = ui.allocate_exact_size(vec2(8., 8.), Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, dot);
                // The name takes what's left, so a long one truncates rather than pushing the
                // state out of the panel.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(panel_title(ui, &group.group_name)).truncate());
                });
            });
        });
        ui.add(egui::Label::new(panel_note(ui, &format!("With {}", people(&group.participants)))).truncate());
        ui.add(egui::Label::new(panel_note(
            ui,
            "Anything you play now plays for everyone in the group.",
        )));
    });
}

/// The group's queue: its label with shuffle and repeat beside it, then the entries, the
/// playing one marked, and a line under them. `highlight` is the entry the keyboard is on, so a
/// reordered queue keeps it on the same row.
fn group_queue(ui: &mut Ui, state: &SyncPlay, highlight: Option<&MenuEntry>, scroll: bool) -> Option<MenuEntry> {
    let queue = state.queue.as_ref().filter(|queue| !queue.playlist.is_empty())?;
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.set_height(QUEUE_TOGGLE);
        ui.add_space(popup::INSET);
        ui.label(panel_note(ui, "Queue"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            // Room for the keyboard's ring inside the panel's edge.
            ui.add_space(popup::INSET - 4.);
            // Off, all, one: each press moves to the next, as on `main`.
            let (icon, tooltip, on) = match queue.repeat_mode {
                RepeatMode::RepeatNone => (ICON_REPEAT, "Repeat is off", false),
                RepeatMode::RepeatAll => (ICON_REPEAT, "Repeating the queue", true),
                RepeatMode::RepeatOne => (ICON_REPEAT_ONE, "Repeating one", true),
            };
            let repeat_ringed = matches!(highlight, Some(MenuEntry::CycleRepeat));
            if icon_toggle(ui, icon, tooltip, on, repeat_ringed).clicked() {
                chosen = Some(MenuEntry::CycleRepeat);
            }
            let shuffled = queue.shuffle_mode == ShuffleMode::Shuffle;
            let tooltip = if shuffled { "Shuffle is on" } else { "Shuffle is off" };
            let shuffle_ringed = matches!(highlight, Some(MenuEntry::ToggleShuffle));
            if icon_toggle(ui, ICON_SHUFFLE, tooltip, shuffled, shuffle_ringed).clicked() {
                chosen = Some(MenuEntry::ToggleShuffle);
            }
        });
    });
    let last = queue.playlist.len() - 1;
    egui::ScrollArea::vertical()
        .max_height(SYNCPLAY_QUEUE_H)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            for (index, entry) in queue.playlist.iter().enumerate() {
                let name = state.names.get(&entry.item_id).map_or("…", String::as_str);
                let id = &entry.playlist_item_id;
                let row = QueueRow {
                    name,
                    index,
                    playing: queue.playing_item_index == index as i32,
                    first: index == 0,
                    last: index == last,
                    highlighted: matches!(highlight, Some(MenuEntry::QueueEntry(on)) if on == id),
                };
                let (response, pressed) = queue_row(ui, &row);
                match pressed {
                    Some(QueueButton::Up) => state.move_entry(id.clone(), -1),
                    Some(QueueButton::Down) => state.move_entry(id.clone(), 1),
                    Some(QueueButton::Remove) => state.remove_entry(id.clone()),
                    None if response.clicked() => chosen = Some(MenuEntry::QueueEntry(id.clone())),
                    None => {}
                }
                if row.highlighted && scroll {
                    response.scroll_to_me(Some(Align::Center));
                }
            }
        });
    popup::separator(ui);
    chosen
}

/// A queue entry as [`queue_row`] draws it.
struct QueueRow<'a> {
    name: &'a str,
    index: usize,
    playing: bool,
    first: bool,
    last: bool,
    highlighted: bool,
}

#[derive(Clone, Copy)]
enum QueueButton {
    Up,
    Down,
    Remove,
}

/// One queue entry: a play mark on the playing one and the others' place in the queue, the
/// name, and (on the row the keyboard or the pointer is on) buttons to move and remove it.
/// Clicking the row plays it for the group. Returns the row and the button pressed.
fn queue_row(ui: &mut Ui, row: &QueueRow) -> (Response, Option<QueueButton>) {
    let tokens = theme::tokens_of(ui);
    let height = popup::row_height(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::CLICK);
    let label = if row.playing {
        format!("{}, playing", row.name)
    } else {
        row.name.to_owned()
    };
    response.widget_info(|| WidgetInfo::labeled(Role::MenuItem, true, &label));
    // The pointer over one of its buttons is over the row too, so the row asks for the pointer
    // rather than its response's hover (which the button takes).
    let active = row.highlighted || ui.rect_contains_pointer(rect);
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(tokens.radius_lg - 4), tokens.accent);
    }

    let slot = Rect::from_min_size(rect.min + vec2(popup::INSET, 0.), vec2(popup::SLOT_W, rect.height()));
    if row.playing {
        ui.painter().text(
            slot.center(),
            Align2::CENTER_CENTER,
            ICON_PLAY_ARROW.codepoint,
            text::icon(ui, ICON_PLAY_ARROW.font_family()),
            tokens.foreground,
        );
    } else {
        ui.painter().text(
            slot.center(),
            Align2::CENTER_CENTER,
            (row.index + 1).to_string(),
            text::font(ui, "xs"),
            tokens.muted_foreground,
        );
    }

    let mut pressed = None;
    let mut right = rect.right() - 4.;
    if active {
        let buttons = [
            (QueueButton::Remove, ICON_CLOSE, "Remove (Delete)", false),
            (QueueButton::Down, ICON_ARROW_DOWNWARD, "Move down (Alt+Down)", row.last),
            (QueueButton::Up, ICON_ARROW_UPWARD, "Move up (Alt+Up)", row.first),
        ];
        for (which, icon, tooltip, disabled) in buttons {
            let button = Rect::from_center_size(
                egui::pos2(right - QUEUE_BUTTON / 2., rect.center().y),
                vec2(QUEUE_BUTTON, QUEUE_BUTTON),
            );
            right -= QUEUE_BUTTON;
            if icon_button(ui, button, icon, tooltip, disabled).clicked() {
                pressed = Some(which);
            }
        }
        right -= 4.;
    } else {
        right -= popup::INSET - 4.;
    }

    let left = slot.right() + popup::INSET;
    let font = text::font(ui, "sm");
    let mut job = egui::text::LayoutJob::single_section(
        row.name.to_owned(),
        egui::TextFormat {
            font_id: font,
            color: tokens.foreground,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(0.));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(left, rect.center().y - galley.size().y / 2.),
        galley,
        tokens.foreground,
    );
    (response, pressed)
}

/// A queue row's small icon button at `rect`, dimmed and inert at the queue's ends.
fn icon_button(ui: &mut Ui, rect: Rect, icon: MaterialIcon, tooltip: &str, disabled: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let sense = if disabled { Sense::hover() } else { Sense::CLICK };
    let response = ui.interact(rect, ui.scope_id().with((tooltip, rect.min.y as i32)), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, !disabled, tooltip));
    if response.hovered() && !disabled {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(tokens.radius_lg - 6), tokens.button_hover);
    }
    let color = if disabled {
        tokens.muted_foreground.gamma_multiply(0.45)
    } else if response.hovered() {
        tokens.foreground
    } else {
        tokens.muted_foreground
    };
    let mut font = text::font(ui, "sm");
    font.family = icon.font_family();
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, icon.codepoint, font, color);
    if disabled {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Shuffle or repeat: an icon filled with the row highlight's colour while it's on, ringed while
/// the keyboard is on it, as the bar's buttons are.
fn icon_toggle(ui: &mut Ui, icon: MaterialIcon, tooltip: &str, on: bool, ringed: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(QUEUE_TOGGLE, QUEUE_TOGGLE), Sense::CLICK);
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, tooltip));
    let radius = CornerRadius::same(tokens.radius_lg - 6);
    if on {
        ui.painter().rect_filled(rect, radius, tokens.accent);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, tokens.button_hover);
    }
    let color = if on || response.hovered() {
        tokens.foreground
    } else {
        tokens.muted_foreground
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon.codepoint,
        text::icon(ui, icon.font_family()),
        color,
    );
    if ringed {
        ring(ui, &response);
    }
    response.on_hover_text(tooltip)
}

/// Cancels the focus move egui queued for a key a menu reads itself, before a widget behind the
/// menu can take the focus (Enter would then press that as well).
fn cancel_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
}

/// Takes the plain Left and Right arrows from egui while a top-bar menu is open. egui's
/// `MenuState::handle_keyboard` gives the focus to the menu's anchor button (the SyncPlay or
/// account button) on a Left when nothing in the menu is focused, and from then on egui keeps
/// the keyboard: Space or Backspace would go to that button rather than the menu. Neither menu
/// has anything horizontal to move, so they keep these keys too.
fn take_horizontal_keys(ctx: &egui::Context) -> bool {
    let left = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowLeft);
    let right = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowRight);
    if left | right {
        cancel_focus(ctx);
    }
    left | right
}

/// The keyboard's ring around a bar item (`main`'s selection ring).
fn ring(ui: &Ui, response: &Response) {
    ui.painter().rect_stroke(
        response.rect.expand(1.),
        CornerRadius::same(4),
        Stroke::new(1.5, theme::tokens_of(ui).foreground),
        StrokeKind::Outside,
    );
}

/// The account menu's id in egui's popup memory, where `Popup::is_any_open` looks for it.
fn account_menu_id() -> Id {
    Id::unique(ACCOUNT_MENU)
}

/// A round badge with the first letter of `name`, standing in for the user's picture.
fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// The app icon at the start of the bar: the 128 px copy rather than the 512 px one the
/// packages ship, as `main` used it, resized once to twice the 32 px it's drawn at. egui-wgpu
/// uploads one mip level (I1), so a 128 px texture drawn at 32 px would alias.
fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let pixels = image::load_from_memory_with_format(include_bytes!("../../assets/logo.png"), image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let pixels = image::imageops::resize(&pixels, 64, 64, image::imageops::FilterType::Lanczos3);
    let size = [pixels.width() as usize, pixels.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
    Some(ctx.load_texture("logo", image, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use jellyfin::{GroupInfo, GroupState, PlayQueue, QueueItem};

    use egui::{Event, Key, Modifiers, RawInput, Rect, pos2};

    use super::{
        BarItem, Highlight, MenuEntry, SearchMotion, bar_items, bar_step, highlight_step, lerp_rect, move_target,
        resolve_highlight, syncplay_entries, take_horizontal_keys,
    };

    #[test]
    fn the_bar_starts_on_back_or_search() {
        let items = bar_items(true);
        assert_eq!(items.first(), Some(&BarItem::Back));
        assert_eq!(bar_step(items, None, 0), Some(BarItem::Back));
        // With nowhere to go back to, Back isn't there.
        let items = bar_items(false);
        assert_eq!(bar_step(items, None, 0), Some(BarItem::Search));
    }

    #[test]
    fn left_and_right_stop_at_the_ends() {
        let items = bar_items(true);
        assert_eq!(bar_step(items, Some(BarItem::Back), -1), Some(BarItem::Back));
        assert_eq!(bar_step(items, Some(BarItem::Account), 1), Some(BarItem::Account));
        assert_eq!(bar_step(items, Some(BarItem::Search), 1), Some(BarItem::SyncPlay));
        assert_eq!(bar_step(items, Some(BarItem::SyncPlay), -1), Some(BarItem::Search));
    }

    #[test]
    fn a_missing_item_steps_from_its_nearest_place() {
        // Back left the bar with the first page; Search is where it was.
        let without_back = bar_items(false);
        assert_eq!(bar_step(without_back, Some(BarItem::Back), 0), Some(BarItem::Search));
        // A hole in the middle (not possible today, but the bar's order decides): Settings
        // gone, Account is the nearest place, not the bar's first item.
        let hole = [BarItem::Search, BarItem::Account];
        assert_eq!(bar_step(&hole, Some(BarItem::Settings), 0), Some(BarItem::Account));
        assert_eq!(bar_step(&[], Some(BarItem::Back), 0), None);
    }

    #[test]
    fn the_search_box_moves_out_of_the_button_into_its_place() {
        // `main`'s numbers at 1280 wide: the button on the right, the box's place in the middle.
        let button = Rect::from_min_size(pos2(1048., 16.), egui::vec2(40., 40.));
        let place = Rect::from_min_size(pos2(440., 16.), egui::vec2(400., 40.));
        assert_eq!(lerp_rect(button, place, 0.), button);
        assert_eq!(lerp_rect(button, place, 1.), place);
        // Halfway it is halfway between them, and as tall as either.
        let half = lerp_rect(button, place, 0.5);
        assert_eq!(half.left(), 744.);
        assert_eq!(half.width(), 220.);
        assert_eq!(half.height(), 40.);
    }

    #[test]
    fn the_search_motion_eases_out_and_ends_after_its_time() {
        let motion = SearchMotion {
            closing: false,
            started: 10.,
            from: Rect::from_min_size(pos2(0., 0.), egui::vec2(40., 40.)),
        };
        assert_eq!(motion.progress(10.), 0.);
        // A clock that went back leaves it at the start rather than behind it.
        assert_eq!(motion.progress(9.9), 0.);
        assert_eq!(motion.progress(10. + crate::motion::ENTER), 1.);
        assert_eq!(motion.progress(100.), 1.);
        // `main`'s `ease_out_quint`: nearly all the way there when half the time is up.
        let half = motion.progress(10. + crate::motion::ENTER / 2.);
        assert!(half > 0.95, "{half}");
        assert!(half < 1.);
    }

    fn group(id: &str) -> GroupInfo {
        GroupInfo {
            group_id: id.into(),
            group_name: format!("Group {id}"),
            state: GroupState::Idle,
            participants: Vec::new(),
        }
    }

    fn queue(items: &[&str], playing_index: i32) -> PlayQueue {
        PlayQueue {
            reason: "NewPlaylist".into(),
            playlist: items
                .iter()
                .map(|id| QueueItem {
                    item_id: (*id).into(),
                    playlist_item_id: (*id).into(),
                })
                .collect(),
            playing_item_index: playing_index,
            start_position_ticks: Default::default(),
            is_playing: false,
            shuffle_mode: Default::default(),
            repeat_mode: Default::default(),
        }
    }

    #[test]
    fn without_a_group_the_panel_lists_the_groups_then_new_group() {
        let groups = [group("a"), group("b")];
        let entries = syncplay_entries(None, &groups, None);
        assert_eq!(
            entries,
            vec![
                MenuEntry::JoinGroup("a".into()),
                MenuEntry::JoinGroup("b".into()),
                MenuEntry::NewGroup,
            ]
        );
        // `main` highlighted *New group* with `groups.len()`.
        assert_eq!(entries.len() - 1, groups.len());
        assert_eq!(syncplay_entries(None, &[], None), vec![MenuEntry::NewGroup]);
    }

    #[test]
    fn in_a_group_the_queue_sits_between_the_toggles_and_leave() {
        let queue = queue(&["x", "y"], 1);
        let entries = syncplay_entries(Some(&group("g")), &[group("other")], Some(&queue));
        assert_eq!(
            entries,
            vec![
                MenuEntry::ToggleShuffle,
                MenuEntry::CycleRepeat,
                MenuEntry::QueueEntry("x".into()),
                MenuEntry::QueueEntry("y".into()),
                MenuEntry::LeaveGroup,
            ]
        );
        assert_eq!(entries.last(), Some(&MenuEntry::LeaveGroup));
    }

    #[test]
    fn a_group_without_a_queue_lists_only_leave() {
        // `main` showed no queue at all (not even the toggles) until it had an entry.
        let empty = queue(&[], -1);
        assert_eq!(
            syncplay_entries(Some(&group("g")), &[], Some(&empty)),
            vec![MenuEntry::LeaveGroup]
        );
        assert_eq!(
            syncplay_entries(Some(&group("g")), &[], None),
            vec![MenuEntry::LeaveGroup]
        );
    }

    #[test]
    fn the_panel_highlight_stops_at_the_ends() {
        // Opened with a click, the first Down picks the first entry and the first Up the last.
        assert_eq!(highlight_step(None, 1, 3), Some(0));
        assert_eq!(highlight_step(None, -1, 3), Some(2));
        assert_eq!(highlight_step(Some(0), 1, 3), Some(1));
        assert_eq!(highlight_step(Some(1), 1, 3), Some(2));
        // Up and Down stop rather than wrapping round, as `main`'s menu did.
        assert_eq!(highlight_step(Some(2), 1, 3), Some(2));
        assert_eq!(highlight_step(Some(0), -1, 3), Some(0));
        assert_eq!(highlight_step(Some(0), 1, 0), None);
    }

    #[test]
    fn only_queue_entries_move_with_alt_up_and_down() {
        let entries = vec![
            MenuEntry::ToggleShuffle,
            MenuEntry::CycleRepeat,
            MenuEntry::QueueEntry("x".into()),
            MenuEntry::QueueEntry("y".into()),
            MenuEntry::LeaveGroup,
        ];
        assert_eq!(move_target(&entries, 2, 1), Some(3));
        assert_eq!(move_target(&entries, 3, -1), Some(2));
        // The toggles before the queue and *Leave group* after it aren't targets.
        assert_eq!(move_target(&entries, 2, -1), None);
        assert_eq!(move_target(&entries, 3, 1), None);
        assert_eq!(move_target(&entries, 1, 1), None);
    }

    /// The entries of a group with a queue of `ids` (the playing entry first).
    fn group_entries(ids: &[&str]) -> Vec<MenuEntry> {
        syncplay_entries(Some(&group("g")), &[], Some(&queue(ids, 0)))
    }

    fn highlighted(entry: MenuEntry, index: usize) -> Option<Highlight> {
        Some(Highlight { entry, index })
    }

    #[test]
    fn the_highlight_follows_its_entry_through_a_reorder() {
        // The keyboard is on "b"; the server moves it (the list is redrawn in the new order).
        let mut highlight = highlighted(MenuEntry::QueueEntry("b".into()), 3);
        let entries = vec![
            MenuEntry::ToggleShuffle,
            MenuEntry::CycleRepeat,
            MenuEntry::QueueEntry("b".into()),
            MenuEntry::QueueEntry("a".into()),
            MenuEntry::LeaveGroup,
        ];
        resolve_highlight(&mut highlight, &entries);
        assert_eq!(highlight, highlighted(MenuEntry::QueueEntry("b".into()), 2));
    }

    #[test]
    fn a_removed_entry_falls_back_to_its_neighbour_not_to_leave() {
        // "b" was at index 3; the queue update takes it out. "c" took its place: the next one.
        let mut highlight = highlighted(MenuEntry::QueueEntry("b".into()), 3);
        resolve_highlight(&mut highlight, &group_entries(&["a", "c"]));
        assert_eq!(highlight, highlighted(MenuEntry::QueueEntry("c".into()), 3));

        // The last entry went: the highlight goes back to the previous queue entry.
        let mut highlight = highlighted(MenuEntry::QueueEntry("c".into()), 4);
        resolve_highlight(&mut highlight, &group_entries(&["a", "b"]));
        assert_eq!(highlight, highlighted(MenuEntry::QueueEntry("b".into()), 3));

        // The only entry went: nothing to highlight (and *Leave group* is next).
        let mut highlight = highlighted(MenuEntry::QueueEntry("a".into()), 2);
        resolve_highlight(&mut highlight, &group_entries(&[]));
        assert_eq!(highlight, None);
    }

    #[test]
    fn a_highlight_never_lands_on_a_button() {
        // The group ended under the highlight: the list is the server's groups now, and none of
        // them is a queue entry.
        let mut highlight = highlighted(MenuEntry::LeaveGroup, 2);
        let groups = [group("a"), group("b")];
        resolve_highlight(&mut highlight, &syncplay_entries(None, &groups, None));
        assert_eq!(highlight, None);

        // *Shuffle*'s queue went: neither the toggles nor *Leave group* take the highlight.
        let mut highlight = highlighted(MenuEntry::ToggleShuffle, 0);
        resolve_highlight(&mut highlight, &group_entries(&[]));
        assert_eq!(highlight, None);
    }

    #[test]
    fn the_menus_keep_the_horizontal_arrows() {
        // Plain Left and Right are the menus' (egui would otherwise give the focus to the
        // button they hang under); Shift+Left is the page's and stays with egui.
        let ctx = egui::Context::default();
        let left = |modifiers| Event::Key {
            key: Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut taken = false;
        let mut remaining = Vec::new();
        let mut output = ctx.run_ui(
            RawInput {
                events: vec![left(Modifiers::NONE), left(Modifiers::SHIFT)],
                ..Default::default()
            },
            |ctx| {
                taken = take_horizontal_keys(ctx);
                remaining = ctx.input(|input| input.events.clone());
            },
        );
        // No painter here: the pass's (empty) texture updates would panic on drop otherwise.
        output.textures_delta.clear();
        assert!(taken);
        assert_eq!(remaining.len(), 1);
        assert!(
            matches!(&remaining[0], Event::Key { key: Key::ArrowLeft, modifiers, .. } if *modifiers == Modifiers::SHIFT)
        );
    }
}
