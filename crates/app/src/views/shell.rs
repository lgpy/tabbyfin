//! The signed-in frame: a top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.
//!
//! The bar has the app's icon, Back when there's a page to go back to, the page's title, and
//! on the right the search box, SyncPlay (U2 fills its panel), Settings and the account menu.
//! The keyboard walks it: Up past a page's first row moves into the bar, Left and Right move
//! along, Enter presses, Down or Esc goes back to the page with its cursor where it was. The
//! account menu is drawn and driven here too: its keys are read before egui's focus pass can
//! hand them to a button or card behind it.
//!
//! The search box is the bar's only text field: typing opens the results page (or comes back
//! to it from a page opened from them), clearing it closes them, and Enter or Down moves into
//! the results, which hand Up and Shift+Tab back to the box; Esc gives the keyboard back to the
//! page.
//!
//! While a playback session shows, the player takes the window: no top bar, the video filling
//! the window with a bar over it saying what's playing, and the player's keys working, until
//! U10's player page replaces it. Going back stops playback; the player closing leaves
//! fullscreen.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, Rect, RectAlign, Response, RichText,
    Sense, Stroke, StrokeKind, TextFormat, TextStyle, Ui, UiBuilder, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_ARROW_BACK, ICON_CHECK, ICON_GROUPS, ICON_SEARCH, ICON_SETTINGS};

use crate::browse::Move;
use crate::config::{Config, SavedServer};
use crate::keys::{self, Action, Press, Scope};
use crate::nav::{NavRequest, Route};
use crate::playback;
use crate::services::AppEvent;
use crate::task::Task;
use crate::theme::over_video;
use crate::toasts::{Toast, ToastAction};
use crate::update::{self, Update};
use crate::views::{self, Page, PageCx, player};
use crate::widgets::text::timestamp;
use crate::widgets::video::Video;

/// The bar's height, inside the panel's own margin.
const BAR_H: f32 = 52.;
/// The size of the bar's buttons, and of the slot each takes (so the right-hand side lines up
/// whatever the icon is).
const BAR_BUTTON: f32 = 36.;
const BAR_ICON: f32 = 20.;
/// The app icon's size in the bar.
const LOGO: f32 = 32.;
/// The open search box's width, when the window has room for it.
const SEARCH_BOX_W: f32 = 400.;
/// The search box's id: it gives its focus back when the keyboard leaves it.
const SEARCH_ID: &str = "top_bar_search";
/// The account menu's width, and the id it lives under in egui's popup memory: the app reads
/// keys for it itself, but egui keeps those keys while a popup is open only if it is in that
/// memory (`Popup::is_any_open`).
const ACCOUNT_MENU_W: f32 = 240.;
const ACCOUNT_MENU: &str = "account_menu";

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

/// Up and Down in the account menu wrap round, as a menu's highlight does.
fn menu_step(current: Option<usize>, step: i32, len: usize) -> Option<usize> {
    let len = len as i32;
    if len == 0 {
        return None;
    }
    Some(match current {
        Some(index) => (index as i32 + step).rem_euclid(len) as usize,
        None if step < 0 => (len - 1) as usize,
        None => 0,
    })
}

pub struct Shell {
    /// Never empty: Home is at the bottom.
    stack: Vec<StackEntry>,
    update_check: Option<Task<Option<Update>>>,
    /// The player is open: from a session's start until playback ends for good (it stays
    /// through autoplay and reconnects). Where U10's player page goes on the stack; leaving it
    /// stops playback.
    player: bool,
    /// The playing session's video, with the session's number: a new session (the next
    /// episode, a reconnect) brings a new video output. Kept between sessions, so the last
    /// frame stays up while the next one loads.
    video: Option<(u64, Video)>,
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
    /// The box had the keyboard when the last frame ended: what this frame's key routing saw
    /// (`keys::take` runs before egui changes the focus). The box's own keys are read on this
    /// rather than `Memory::has_focus`, which later events in the frame could have changed.
    search_focused: bool,
    /// The entry the account menu's keyboard highlights (`None` when a click opened it, until
    /// an arrow key picks one). Whether the menu is open is egui's popup memory's, so egui keeps
    /// the keyboard while it is (`Popup::is_any_open`).
    account_highlight: Option<usize>,
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
            player: false,
            video: None,
            #[cfg(debug_assertions)]
            debug_play: None,
            accounts: Vec::new(),
            bar_key: None,
            search_text: String::new(),
            search_open: false,
            want_search_focus: false,
            search_focused: false,
            account_highlight: None,
            logo: load_logo(cx.ctx),
        };
        // A new session starts with no menu left over from the last one.
        egui::Popup::close_id(cx.ctx, account_menu_id());
        #[cfg(debug_assertions)]
        shell.debug_start(cx);
        shell
    }

    /// The scopes whose keys the app takes from egui (`keys.rs`): the top page's, then the
    /// shell's.
    pub fn scopes(&self) -> Vec<Scope> {
        if self.player {
            return vec![Scope::Player, Scope::Shell];
        }
        self.top().scope().into_iter().chain([Scope::Shell]).collect()
    }

    /// A playback session started (`AppEvent::PlaybackStarted`): the player opens, or a new
    /// session takes over the open one.
    pub fn player_started(&mut self, ctx: &egui::Context) {
        self.player = true;
        // The player takes the window, and the bar's keyboard and menus leave with the bar. The
        // search box would otherwise keep egui's keyboard (and the player's keys) while it's
        // hidden.
        self.bar_key = None;
        self.want_search_focus = false;
        self.search_focused = false;
        self.account_highlight = None;
        egui::Popup::close_id(ctx, account_menu_id());
        ctx.memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
        ctx.request_repaint();
    }

    /// Playback ended for good (`AppEvent::PlaybackEnded`): the player closes, leaving
    /// fullscreen.
    pub fn player_ended(&mut self, ctx: &egui::Context) {
        self.video = None;
        if std::mem::take(&mut self.player) {
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
        self.notify_update(cx);
        #[cfg(debug_assertions)]
        self.debug_play(cx);
        if self.player {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(over_video::BACKDROP))
                .show(ui, |ui| self.show_player(ui, cx));
        } else {
            egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui, cx));
            egui::CentralPanel::default().show(ui, |ui| {
                // Each page's widgets and scroll areas live under its own id scope, so stacked
                // pages can use the same names for theirs (two `"cast"` shelves, two
                // `ScrollArea`s).
                let entry = self.stack.last_mut().expect("the page stack is never empty");
                ui.push_id(entry.serial, |ui| entry.page.show(ui, cx));
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
        if self.player {
            if let Some(action) = press.action(Scope::Player)
                && player::on_key(action, cx.services, cx.ctx)
            {
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
            BarItem::SyncPlay => self.syncplay_notice(cx),
            BarItem::Settings => self.toggle_settings(cx),
            BarItem::Account => self.open_account_menu(cx.ctx),
        }
    }

    /// The account menu's keys, which egui keeps while its popup is open: Tab stays in the
    /// menu, Backspace and Alt+Left close it like Esc, Ctrl/Cmd+F closes it and opens the box,
    /// Up and Down move its highlight, Enter picks. Read before anything in the bar draws, so
    /// egui's focus pass can't hand the keyboard to a button or card behind the menu (Enter
    /// would then press that as well); the popup itself only draws its rows.
    fn account_menu_keys(&mut self, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, account_menu_id()) {
            return;
        }
        let ctx = cx.ctx;
        // Tab and the arrows queued an egui focus move in its focus pass; cancel it before any
        // widget behind the menu can take the focus (Enter would then press that as well).
        fn cancel_focus(ctx: &egui::Context) {
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Tab) | keys::take_exact(ctx, Modifiers::SHIFT, Key::Tab) {
            cancel_focus(ctx);
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Backspace)
            | keys::take_exact(ctx, Modifiers::ALT, Key::ArrowLeft)
        {
            egui::Popup::close_id(ctx, account_menu_id());
            self.account_highlight = None;
            return;
        }
        if keys::take_exact(ctx, Modifiers::COMMAND, Key::F) {
            egui::Popup::close_id(ctx, account_menu_id());
            self.account_highlight = None;
            self.focus_search(cx);
            return;
        }
        let count = self.accounts.len() + 2;
        let down = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowDown);
        let up = keys::take_exact(ctx, Modifiers::NONE, Key::ArrowUp);
        if down | up {
            let step = if down { 1 } else { -1 };
            self.account_highlight = menu_step(self.account_highlight, step, count);
            cancel_focus(ctx);
        }
        if keys::take_exact(ctx, Modifiers::NONE, Key::Enter)
            && let Some(index) = self.account_highlight
        {
            egui::Popup::close_id(ctx, account_menu_id());
            self.account_highlight = None;
            self.choose_account(index, cx);
        }
    }

    /// Opens the account menu, loading the saved accounts as `main` did when its menu opened.
    /// From the keyboard the first entry is highlighted; from a click the first arrow key
    /// picks one.
    fn open_account_menu(&mut self, ctx: &egui::Context) {
        self.accounts = Config::load().accounts;
        self.account_highlight = Some(0);
        egui::Popup::open_id(ctx, account_menu_id());
        ctx.request_repaint();
    }

    /// Puts the keyboard in the search box, opening it if it's the button. The page's cursor
    /// goes, so Esc back to the page shows it where it was instead of moving it a row.
    fn focus_search(&mut self, cx: &mut PageCx) {
        self.search_open = true;
        self.want_search_focus = true;
        self.bar_key = None;
        self.top_mut().hide_cursor();
        cx.ctx.request_repaint();
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

    /// U2 brings the SyncPlay panel; until then the button says so, like the item menu's
    /// *Add to playlist…* until U8.
    fn syncplay_notice(&mut self, cx: &mut PageCx) {
        cx.services.toast(Toast::info("SyncPlay arrives with U2."));
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
    /// text, as on `main`).
    fn search_shown(&self) -> bool {
        self.search_open || !self.search_text.trim().is_empty()
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
            self.search_open = false;
            cx.ctx
                .memory_mut(|memory| memory.surrender_focus(Id::unique(SEARCH_ID)));
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
    /// buttons on the right, and the open search box between them.
    fn top_bar(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        self.search_box_keys(cx);
        self.account_menu_keys(cx);
        let can_go_back = self.can_go_back();
        let keyboard = self.bar_key;
        let box_shown = self.search_shown();
        let in_group = cx.services.in_group();
        let title = self.top().title().to_owned();
        let logo = self.logo.clone();
        let gap = ui.spacing().item_spacing.x;

        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
        // The right-hand side: the account, Settings and SyncPlay, and the search button
        // while the box is closed.
        let slots = if box_shown { 3 } else { 4 };
        let buttons_w = slots as f32 * BAR_BUTTON + (slots - 1) as f32 * gap;
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
                ui.add(egui::Label::new(RichText::new(title).heading().strong()).truncate());
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
                    self.bar_key = None;
                    self.syncplay_notice(cx);
                }
                if !box_shown {
                    let search = bar_button(ui, ICON_SEARCH, "Search", keyboard == Some(BarItem::Search), false);
                    if search.clicked() {
                        self.focus_search(cx);
                    }
                }
            },
        );

        if box_shown {
            let rect = Rect::from_center_size(bar.center(), vec2(box_w, BAR_BUTTON));
            self.search_box(ui, rect, keyboard == Some(BarItem::Search), cx);
        } else {
            self.search_focused = false;
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
        let fill = if response.hovered() {
            ui.visuals().widgets.hovered.weak_bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        };
        let center = rect.center();
        ui.painter().circle_filled(center, 14., fill);
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            initial(&name),
            FontId::proportional(16.),
            ui.visuals().strong_text_color(),
        );
        if highlighted {
            ring(ui, &response);
        }
        let response = response.on_hover_text(tooltip);
        let id = account_menu_id();
        let was_open = egui::Popup::is_id_open(ui.ctx(), id);
        if response.clicked() {
            // A click leaves no keyboard in the bar (the menu's keys are `account_menu_keys`').
            self.bar_key = None;
            if was_open {
                egui::Popup::close_id(ui.ctx(), id);
                self.account_highlight = None;
            } else {
                self.accounts = Config::load().accounts;
                self.account_highlight = None;
                egui::Popup::open_id(ui.ctx(), id);
            }
        }

        // The rows only answer the pointer; the keyboard comes through `account_menu_keys`
        // (egui's focus pass would otherwise hand it to a button or card behind the menu).
        let mut chosen = None;
        let current = cx.services.session.account_key();
        if egui::Popup::is_id_open(ui.ctx(), id) {
            egui::Popup::menu(&response)
                .id(id)
                .open_memory(None::<egui::SetOpenCommand>)
                .align(RectAlign::BOTTOM_END)
                .width(ACCOUNT_MENU_W)
                .show(|ui| {
                    for (index, account) in self.accounts.iter().enumerate() {
                        let checked = account.credential_account() == current;
                        let entry = account_entry(ui, checked, &account.label(), self.account_highlight == Some(index));
                        if entry.clicked() {
                            chosen = Some(index);
                        }
                    }
                    ui.separator();
                    let actions = self.accounts.len();
                    if account_entry(ui, false, "Add account…", self.account_highlight == Some(actions)).clicked() {
                        chosen = Some(actions);
                    }
                    if account_entry(ui, false, "Sign out", self.account_highlight == Some(actions + 1)).clicked() {
                        chosen = Some(actions + 1);
                    }
                });
        }
        if let Some(index) = chosen {
            egui::Popup::close_id(ui.ctx(), id);
            self.account_highlight = None;
            self.choose_account(index, cx);
        } else if was_open && !egui::Popup::is_id_open(ui.ctx(), id) {
            // Closed without a choice (Esc or a click outside): back where the keyboard came
            // from. Opened from the bar, its item is still set, so its ring comes back.
            self.account_highlight = None;
        }
    }

    /// Does what an account-menu entry says: switch to that account, add one, or sign out.
    /// The app carries the event out after the frame.
    fn choose_account(&mut self, index: usize, cx: &mut PageCx) {
        let actions = self.accounts.len();
        self.account_highlight = None;
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

    /// The search box, centred in the bar while it's open.
    fn search_box(&mut self, ui: &mut egui::Ui, rect: Rect, highlighted: bool, cx: &mut PageCx) {
        // `Memory::has_focus`, not `Response::has_focus`: the latter also wants the window to be
        // focused, which an Xvfb display without a window manager never is.
        let focused = ui.ctx().memory(|memory| memory.has_focus(Id::unique(SEARCH_ID)));
        let response = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    // The box locks the focus keys: egui must neither move the keyboard on
                    // Tab nor drop it on Esc before the box reads Esc (`search_box_keys`), or
                    // the app would stop getting keys until something was clicked.
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .id(Id::unique(SEARCH_ID))
                            .hint_text("Search")
                            .desired_width(rect.width())
                            .margin(egui::Margin::symmetric(8, 6))
                            .event_filter(egui::EventFilter {
                                tab: true,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: true,
                            }),
                    )
                },
            )
            .inner;
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
        if response.changed() {
            self.search_changed(cx);
        }
        // Closed, the box goes once it has nothing left to show, as `main`'s did on blur.
        if response.lost_focus() && self.search_text.trim().is_empty() {
            self.search_open = false;
        }
        if highlighted && !focused {
            ring(ui, &response);
        }
    }

    /// The player until U10: the video filling the window, a spinner until it shows, and the
    /// bar over it.
    fn show_player(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let state = cx.services.playback.now_playing();
        if let (Some(state), Some(render_state)) = (state, cx.render_state)
            && self.video.as_ref().map(|(session, _)| *session) != Some(state.session)
        {
            self.video = Some((state.session, Video::new(cx.ctx, state.video.clone(), render_state)));
        }
        let rect = ui.max_rect();
        let (shown, import_failed) = match &mut self.video {
            Some((_, video)) => {
                video.show(ui);
                (video.has_frame(), video.import_failed())
            }
            None => (false, false),
        };
        if !shown || state.is_none_or(|state| state.loading) {
            let spinner = egui::Rect::from_center_size(rect.center(), egui::vec2(48., 48.));
            ui.put(
                spinner,
                egui::Spinner::new().size(48.).color(over_video::MUTED_FOREGROUND),
            );
        }
        if import_failed {
            cx.services.fall_back_to_pixels();
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| now_playing_bar(ui, cx));
    }

    fn navigate(&mut self, cx: &mut PageCx) {
        if cx.nav.is_empty() {
            return;
        }
        for request in std::mem::take(cx.nav) {
            match request {
                NavRequest::Push(route) => self.push(route, cx),
                NavRequest::Back => self.back(cx),
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
            self.search_open = false;
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
    let button = egui::Button::new(icon.rich_text().size(BAR_ICON))
        .min_size(vec2(BAR_BUTTON, BAR_BUTTON))
        .selected(selected);
    let response = ui.add(button).on_hover_text(tooltip);
    if highlighted {
        ring(ui, &response);
    }
    response
}

/// The keyboard's ring around a bar item (`main`'s selection ring).
fn ring(ui: &Ui, response: &Response) {
    ui.painter().rect_stroke(
        response.rect.expand(1.),
        CornerRadius::same(4),
        Stroke::new(1.5, ui.visuals().selection.stroke.color),
        StrokeKind::Outside,
    );
}

/// A row in the account menu: a check in front when it's the current account, and the
/// keyboard's highlight around it while the keyboard is on it.
fn account_entry(ui: &mut Ui, checked: bool, label: &str, highlighted: bool) -> Response {
    let body = TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    // An unchecked row keeps the check's room, so the labels line up.
    job.append(
        ICON_CHECK.codepoint,
        0.,
        TextFormat {
            font_id: FontId::new(body.size + 2., ICON_CHECK.font_family()),
            color: if checked { color } else { Color32::TRANSPARENT },
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
    let response = ui.add(egui::Button::new(egui::WidgetText::LayoutJob(job.into())).frame(false));
    if highlighted {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(4),
            Stroke::new(1.5, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    response
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

/// Over the top of the video: Back, what's playing and where, and Stop. All the controls there
/// are until U10. Dark in either theme, as the player is: the over-video colours, with egui's
/// dark buttons.
fn now_playing_bar(ui: &mut egui::Ui, cx: &mut PageCx) {
    ui.style_mut().visuals = egui::Visuals::dark();
    ui.style_mut().visuals.override_text_color = Some(over_video::FOREGROUND);
    egui::Frame::NONE
        .fill(over_video::SCRIM)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Leaving the player stops playback; it closes when the session ends.
                if ui.button(ICON_ARROW_BACK).on_hover_text("Back").clicked() {
                    cx.services.stop_playback();
                }
                let text = match cx.services.playback.now_playing() {
                    Some(state) => {
                        let mut text = format!("{}  {}", playback::title(&state.item), timestamp(state.position));
                        if let Some(duration) = state.duration {
                            text += &format!(" / {}", timestamp(duration));
                        }
                        if state.paused {
                            text += "  (paused)";
                        }
                        if state.is_transcoding() {
                            text += "  · Transcoding";
                        }
                        text
                    }
                    None => "Starting playback…".to_owned(),
                };
                ui.label(text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Stop").clicked() {
                        cx.services.control(playback::UiCommand::Stop);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::{BarItem, bar_items, bar_step, menu_step};

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
    fn the_account_menu_wraps_round() {
        assert_eq!(menu_step(Some(0), 1, 4), Some(1));
        assert_eq!(menu_step(Some(3), 1, 4), Some(0));
        assert_eq!(menu_step(Some(0), -1, 4), Some(3));
        // A menu opened with a click shows the first entry going down, the last going up.
        assert_eq!(menu_step(None, 1, 4), Some(0));
        assert_eq!(menu_step(None, -1, 4), Some(3));
        assert_eq!(menu_step(Some(0), 1, 0), None);
    }
}
