//! Signed-in frame: top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::{Button, ButtonVariants, Toggle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::{GroupState, RepeatMode, ShuffleMode};

use crate::browse::{MoveDown, MoveUp};
use crate::components::{menu_left_focus, menu_popover, selection_ring};
use crate::config::{Config, SavedServer};
use crate::images::ImageCache;
use crate::motion;
use crate::playback::{PlaybackEvent, PlaybackManager};
use crate::remote::{Remote, RemoteEvent};
use crate::router::{Navigator, Route};
use crate::session::Session;
use crate::syncplay::{SyncPlay, SyncPlayEvent};
use crate::views::home::HomePage;
use crate::views::item::ItemPage;
use crate::views::library::{LibraryPage, Source};
use crate::views::player::PlayerPage;
use crate::views::search::SearchPage;
use crate::views::settings::SettingsPage;
use crate::zoom::{dp, dp_px};

actions!(jelly, [Back, OpenSearch]);

pub const KEY_CONTEXT: &str = "Shell";

const TOP_BAR_H: f32 = 72.;
const TOP_BAR_ICON: f32 = 24.;
/// The top bar's buttons' height.
const TOP_BAR_BUTTON: f32 = 40.;
/// The open search box's width, when the window has room for it.
const SEARCH_BOX_W: f32 = 400.;

/// A top-bar button's icon, a size up from a button's own. (A button sizes the icon it's
/// given with `icon`, so these go in as children.)
fn bar_icon(icon: impl Into<Icon>) -> Icon {
    icon.into().size(dp(TOP_BAR_ICON))
}

/// The app icon at the start of the bar. A 128 px copy rather than the 512 px one the
/// packages ship: GPUI scales images without mipmaps, so a big one would shimmer this small.
static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/logo.png").to_vec(),
    ))
});

/// The top bar's controls, in order, for the keyboard: Up past a page's first row reaches
/// them, Left and Right move along, Enter presses, Down or Esc returns to the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TopBarItem {
    Back,
    Search,
    SyncPlay,
    Settings,
    Account,
}

const TOP_BAR: [TopBarItem; 5] = [
    TopBarItem::Back,
    TopBarItem::Search,
    TopBarItem::SyncPlay,
    TopBarItem::Settings,
    TopBarItem::Account,
];

/// The top bar's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    SyncPlay,
    Account,
}

/// A choice in the SyncPlay panel, in the order it lists them (the account menu is a
/// [`PopupMenu`], which has its own keyboard handling). The keyboard highlight is an
/// index into these.
enum MenuEntry {
    JoinGroup(String),
    NewGroup,
    ToggleShuffle,
    CycleRepeat,
    /// An entry of the group's queue, by playlist item id: Enter plays it, Delete removes
    /// it, Alt+Up and Alt+Down move it.
    QueueEntry(String),
    LeaveGroup,
}

pub enum ShellEvent {
    SignOut,
    /// The server rejected the session token.
    SessionExpired,
    /// Switch to another saved account.
    SwitchAccount(SavedServer),
    /// Sign in to another server or user, keeping this session to come back to.
    AddAccount,
}

struct Page {
    title: SharedString,
    view: AnyView,
    /// What takes keyboard focus when the page is on top; the shell itself if `None`.
    focus: Option<Focuser>,
    /// The embedded player: full window, no top bar; leaving it stops playback.
    is_player: bool,
}

pub struct ShellView {
    session: Session,
    stack: Vec<Page>,
    focus_handle: FocusHandle,
    _playback: Subscription,
    _syncplay: Vec<Subscription>,
    _remote: Subscription,
    /// The saved accounts, for the account menu (read when it opens).
    accounts: Vec<SavedServer>,
    /// The session websocket; dropped (disconnecting) with the shell on sign-out.
    _live: Task<()>,
    /// The top-bar button the keyboard is on (shown while the shell itself has focus).
    top_bar: Option<TopBarItem>,
    /// The open top-bar menu, and the entry the keyboard highlights (none when opened with a
    /// click).
    menu: Option<(Menu, Option<usize>)>,
    /// What the open SyncPlay panel focuses. It renders in an overlay outside the shell, so its
    /// content forwards keys to `on_key_down` (see `menu_keys`).
    menu_focus: FocusHandle,
    /// The account menu, built fresh each time it opens, and its dismissal.
    account_menu: Option<(Entity<PopupMenu>, Subscription)>,
    /// The search box. Typing in it opens the results page, clearing it closes the page.
    search: Entity<InputState>,
    _search: Subscription,
    /// The search box is open in the middle of the bar (it also stays open while it has text).
    /// Closed, it's a button with the others on the right.
    search_open: bool,
    /// How far the search button was from the window's right edge when last drawn (the
    /// buttons are right-aligned, so it stays there): where the box opens from and closes to.
    search_button: Rc<Cell<Option<Pixels>>>,
    /// Where the open box was last drawn.
    search_slot: Rc<Cell<Bounds<Pixels>>>,
    /// The box opening out of the button or closing back into it.
    search_motion: Option<SearchMotion>,
    /// Ends the running animation.
    _search_motion_end: Option<Task<()>>,
}

#[derive(Clone, Copy)]
struct SearchMotion {
    /// Counts the animations, for their ids.
    epoch: usize,
    closing: bool,
    /// How far right of the box's place the button is.
    dx: Pixels,
    /// Still moving: the button is drawn too, fading out as the box opens, or in as it closes,
    /// so one turns into the other.
    running: bool,
}

impl EventEmitter<ShellEvent> for ShellView {}

impl ShellView {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ImageCache::init(session.client.clone(), cx);
        PlaybackManager::init(session.client.clone(), session.account_key(), cx);
        crate::playback::report_unfinished(&session.client, &session.account_key());
        SyncPlay::init(session.client.clone(), cx);
        let syncplay = SyncPlay::global(cx);
        let _syncplay = vec![
            cx.observe(&syncplay, |_, _, cx| cx.notify()),
            cx.subscribe_in(&syncplay, window, |_, _, event, window, cx| match event {
                SyncPlayEvent::Notice(text) => window.push_notification(Notification::info(text.clone()), cx),
            }),
        ];
        let _playback = cx.subscribe_in(
            &PlaybackManager::global(cx),
            window,
            |this, _, event, window, cx| match event {
                PlaybackEvent::Started(now_playing) => this.push_player(now_playing.clone(), window, cx),
                PlaybackEvent::Ended => this.close_player(window, cx),
                PlaybackEvent::Failed(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
                }
                PlaybackEvent::Notice(message) => {
                    window.push_notification(Notification::info(message.clone()), cx);
                }
            },
        );
        Remote::init(session.client.clone(), cx);
        notify_update(window, cx);
        let _remote = cx.subscribe_in(&Remote::global(cx), window, |_, _, event, window, cx| match event {
            RemoteEvent::Message(text) => window.push_notification(Notification::info(text.clone()), cx),
            RemoteEvent::ToggleFullscreen => window.toggle_fullscreen(),
        });
        let _live = crate::live::connect(session.client.clone(), cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let _search = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.search_changed(window, cx),
            InputEvent::PressEnter { .. } => this.enter_search_results(window, cx),
            // Back to a button once nothing's typed in it.
            InputEvent::Blur => this.close_search(window, cx),
            _ => {}
        });
        let mut this = Self {
            session,
            stack: Vec::new(),
            focus_handle: cx.focus_handle(),
            _playback,
            _syncplay,
            _remote,
            accounts: Config::load().accounts,
            _live,
            top_bar: None,
            menu: None,
            menu_focus: cx.focus_handle(),
            account_menu: None,
            search,
            _search,
            search_open: false,
            search_button: Rc::default(),
            search_slot: Rc::default(),
            search_motion: None,
            _search_motion_end: None,
        };
        this.push(Route::Home, window, cx);
        #[cfg(debug_assertions)]
        if let Some(term) = debug_search_term() {
            this.search.update(cx, |search, cx| search.set_value(term, window, cx));
            this.search_changed(window, cx);
        } else if let Some((route, autoplay)) = debug_start_route() {
            if let (Some(autoplay_tracks), Route::Item { id, .. }) = (autoplay, &route) {
                let (client, id) = (this.session.client.clone(), id.clone());
                cx.spawn(async move |_, cx| {
                    let item = crate::runtime::spawn(async move { client.item(&id, &[]).await }).await;
                    match item {
                        Ok(item) => cx.update(|cx| {
                            let start = item
                                .user_data
                                .as_ref()
                                .map(|d| d.playback_position_ticks)
                                .filter(|t| t.0 > 0);
                            let options = crate::playback::PlayOptions {
                                start,
                                audio_stream_index: autoplay_tracks.0,
                                subtitle: match autoplay_tracks.1 {
                                    Some(-1) => crate::playback::SubtitleChoice::Off,
                                    Some(index) => crate::playback::SubtitleChoice::Stream(index),
                                    None => crate::playback::SubtitleChoice::Auto,
                                },
                                media_source_id: autoplay_tracks.2,
                                ..Default::default()
                            };
                            PlaybackManager::global(cx).update(cx, |p, cx| p.play(&item, options, cx));
                        }),
                        Err(err) => tracing::error!("autoplay failed: {err}"),
                    }
                })
                .detach();
            }
            this.push(route, window, cx);
        }
        this
    }

    pub fn push(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let nav = Navigator::new(cx.entity().downgrade());
        let client = self.session.client.clone();
        let focus;
        let page = match route {
            Route::Home => Page {
                title: "Home".into(),
                view: {
                    let page = cx.new(|cx| HomePage::new(client, nav, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
            Route::Library {
                id,
                name,
                collection_type,
            } => Page {
                title: name.into(),
                view: {
                    let source = Source::Folder { id, collection_type };
                    let page = cx.new(|cx| LibraryPage::new(client, nav, source, window, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
            Route::Item { id, name } => Page {
                title: name.into(),
                view: {
                    let page = cx.new(|cx| ItemPage::new(client, nav, id, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
            Route::Person { id, name } => Page {
                title: name.into(),
                view: {
                    let source = Source::Person { id };
                    let page = cx.new(|cx| LibraryPage::new(client, nav, source, window, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
            Route::Search => Page {
                title: "Search".into(),
                view: {
                    let search = self.search.clone();
                    let page = cx.new(|cx| SearchPage::new(client, nav, search, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
            Route::Settings => Page {
                title: "Settings".into(),
                view: {
                    let page = cx.new(|cx| SettingsPage::new(self.session.clone(), window, cx));
                    focus = Some(focuser(&page));
                    page.into()
                },
                focus: None,
                is_player: false,
            },
        };
        self.stack.push(Page { focus, ..page });
        self.focus_top(window, cx);
        cx.notify();
    }

    /// Gives keyboard focus to the page on top, so its keys work after clicks moved focus
    /// elsewhere.
    fn focus_top(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.stack.last().and_then(|page| page.focus.clone()) {
            Some(focus) => focus(cx).focus(window, cx),
            None => self.focus_handle.focus(window, cx),
        }
    }

    fn push_player(
        &mut self,
        now_playing: Entity<crate::playback::NowPlaying>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A new session replaces any player already open.
        self.stack.retain(|page| !page.is_player);
        let title = now_playing.read(cx).item.name.clone().into();
        let view = cx.new(|cx| PlayerPage::new(now_playing, window, cx)).into();
        self.stack.push(Page {
            title,
            view,
            focus: None,
            is_player: true,
        });
        cx.notify();
    }

    fn close_player(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.iter().any(|page| page.is_player) {
            self.stack.retain(|page| !page.is_player);
            if window.is_fullscreen() {
                window.toggle_fullscreen();
            }
            self.focus_top(window, cx);
            cx.notify();
        }
    }

    /// Esc and Backspace close a top-bar menu, then leave the top bar, before going back.
    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            // Back to the top bar, if the keyboard came from there.
            if self.top_bar.is_some() {
                self.focus_handle.focus(window, cx);
            }
            cx.notify();
        } else if self.top_bar.is_some() && self.focus_handle.is_focused(window) {
            self.leave_top_bar(window, cx);
        } else if self.search.focus_handle(cx).is_focused(window)
            && self.search_page().is_none_or(|i| i + 1 < self.stack.len())
        {
            // Esc in the box over another page: back to that page (over the results it closes them).
            self.leave_top_bar(window, cx);
        } else {
            self.go_back(window, cx);
        }
    }

    /// Up past a page's first row (the page lets the action through): into the top bar.
    fn enter_top_bar(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_top_bar(window, cx);
    }

    /// Moves the keyboard into the top bar.
    pub fn focus_top_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.last().is_some_and(|page| page.is_player) {
            return;
        }
        let can_go_back = self.stack.len() > 1;
        let first = if can_go_back {
            TopBarItem::Back
        } else {
            TopBarItem::Search
        };
        // Back is hidden on the first page.
        let kept = self.top_bar.filter(|item| *item != TopBarItem::Back || can_go_back);
        self.top_bar = Some(kept.unwrap_or(first));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn leave_top_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.top_bar = None;
        self.menu = None;
        self.focus_top(window, cx);
        // The page's cursor hid when Up left it; a move while it's hidden shows it in place.
        window.dispatch_action(MoveDown.boxed_clone(), cx);
        cx.notify();
    }

    /// Keys in the top bar and its menus. (Esc arrives as `Back`, bound before this runs.)
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if let Some((menu, Some(highlight))) = self.menu {
            let entries = self.menu_entries(menu, cx);
            let queue_entry = match entries.get(highlight) {
                Some(MenuEntry::QueueEntry(id)) => Some(id.clone()),
                _ => None,
            };
            let syncplay = SyncPlay::global(cx);
            match (key, queue_entry) {
                // Moving a queue entry: the highlight goes with it, if there's room.
                ("up" | "down", Some(id)) if modifiers.alt && !modifiers.control && !modifiers.shift => {
                    let by = if key == "up" { -1 } else { 1 };
                    let target = highlight.checked_add_signed(by);
                    if let Some(target) = target.filter(|t| matches!(entries.get(*t), Some(MenuEntry::QueueEntry(_)))) {
                        syncplay.update(cx, |syncplay, cx| syncplay.move_entry(id, by, cx));
                        self.menu = Some((menu, Some(target)));
                    }
                }
                ("delete", Some(id)) if !modifiers.modified() => {
                    syncplay.update(cx, |syncplay, cx| syncplay.remove_entry(id, cx));
                }
                _ if modifiers.modified() => return,
                ("down", _) if highlight + 1 < entries.len() => self.menu = Some((menu, Some(highlight + 1))),
                ("up", _) if highlight > 0 => self.menu = Some((menu, Some(highlight - 1))),
                ("enter", _) => self.choose_menu_entry(menu, highlight, window, cx),
                ("up" | "down", _) => {}
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if modifiers.modified() {
            return;
        }
        let Some(item) = self.top_bar.filter(|_| self.focus_handle.is_focused(window)) else {
            return;
        };
        let position = TOP_BAR.iter().position(|i| *i == item).unwrap_or(0);
        let can_go_back = self.stack.len() > 1;
        match key {
            "left" => {
                let min = if can_go_back { 0 } else { 1 };
                self.top_bar = Some(TOP_BAR[position.saturating_sub(1).max(min)]);
            }
            "right" => self.top_bar = Some(TOP_BAR[(position + 1).min(TOP_BAR.len() - 1)]),
            "down" => self.leave_top_bar(window, cx),
            "enter" => self.press_top_bar(item, window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn press_top_bar(&mut self, item: TopBarItem, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            TopBarItem::Back => {
                self.top_bar = None;
                self.go_back(window, cx);
            }
            TopBarItem::Search => self.focus_search(window, cx),
            TopBarItem::Settings => {
                self.top_bar = None;
                self.toggle_settings(window, cx);
            }
            TopBarItem::SyncPlay => self.open_menu(Menu::SyncPlay, Some(0), window, cx),
            TopBarItem::Account => self.open_menu(Menu::Account, None, window, cx),
        }
    }

    /// Opens a top-bar menu, with fresh contents.
    fn open_menu(&mut self, menu: Menu, highlight: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        match menu {
            Menu::Account => {
                self.accounts = Config::load().accounts;
                let account_menu = self.build_account_menu(window, cx);
                account_menu.focus_handle(cx).focus(window, cx);
                let dismissed = cx.subscribe_in(&account_menu, window, |this, _, _: &DismissEvent, window, cx| {
                    this.menu_toggled(Menu::Account, false, window, cx)
                });
                self.account_menu = Some((account_menu, dismissed));
            }
            Menu::SyncPlay => SyncPlay::global(cx).update(cx, |syncplay, cx| syncplay.refresh_groups(cx)),
        }
        self.menu = Some((menu, highlight));
        cx.notify();
    }

    fn menu_entries(&self, menu: Menu, cx: &App) -> Vec<MenuEntry> {
        match menu {
            Menu::Account => Vec::new(),
            Menu::SyncPlay => {
                let syncplay = SyncPlay::global(cx).read(cx);
                if syncplay.group.is_some() {
                    let mut entries = Vec::new();
                    if let Some(queue) = syncplay.queue.as_ref().filter(|q| !q.playlist.is_empty()) {
                        entries.extend([MenuEntry::ToggleShuffle, MenuEntry::CycleRepeat]);
                        entries.extend(
                            queue
                                .playlist
                                .iter()
                                .map(|entry| MenuEntry::QueueEntry(entry.playlist_item_id.clone())),
                        );
                    }
                    entries.push(MenuEntry::LeaveGroup);
                    return entries;
                }
                let mut entries: Vec<MenuEntry> = syncplay
                    .groups
                    .iter()
                    .map(|group| MenuEntry::JoinGroup(group.group_id.clone()))
                    .collect();
                entries.push(MenuEntry::NewGroup);
                entries
            }
        }
    }

    /// Does what a menu entry's button does, and closes the menu (shuffle and repeat leave it
    /// open, to see the change).
    fn choose_menu_entry(&mut self, menu: Menu, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.menu_entries(menu, cx).into_iter().nth(index) else {
            return;
        };
        let syncplay = SyncPlay::global(cx);
        match entry {
            MenuEntry::ToggleShuffle => return syncplay.update(cx, |syncplay, cx| syncplay.toggle_shuffle(cx)),
            MenuEntry::CycleRepeat => return syncplay.update(cx, |syncplay, cx| syncplay.cycle_repeat(cx)),
            _ => {}
        }
        self.leave_top_bar(window, cx);
        match entry {
            MenuEntry::JoinGroup(id) => syncplay.update(cx, |syncplay, cx| syncplay.join(id, cx)),
            MenuEntry::NewGroup => {
                let name = new_group_name(&self.session);
                syncplay.update(cx, |syncplay, cx| syncplay.create(name, cx));
            }
            MenuEntry::QueueEntry(id) => syncplay.update(cx, |syncplay, cx| syncplay.play_entry(id, cx)),
            MenuEntry::LeaveGroup => syncplay.update(cx, |syncplay, cx| syncplay.leave(cx)),
            MenuEntry::ToggleShuffle | MenuEntry::CycleRepeat => {}
        }
    }

    /// The Settings button: opens Settings, or closes it (back to the page before) when it's on
    /// top.
    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on_top = self
            .stack
            .last()
            .is_some_and(|page| page.view.clone().downcast::<SettingsPage>().is_ok());
        if on_top {
            self.go_back(window, cx);
        } else {
            self.push(Route::Settings, window, cx);
        }
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Leaving the player stops playback; the page closes when the session ends.
        if self.stack.last().is_some_and(|page| page.is_player) {
            PlaybackManager::global(cx).update(cx, |playback, cx| playback.stop(cx));
            return;
        }
        if self.stack.len() > 1 {
            self.stack.pop();
            // Leaving the results clears the box they were for.
            if self.search_page().is_none() && !self.search.read(cx).value().is_empty() {
                self.search.update(cx, |search, cx| search.set_value("", window, cx));
                self.close_search(window, cx);
            }
            self.focus_top(window, cx);
            cx.notify();
        }
    }

    fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
    }

    /// Opens the search box, if it's a button, and puts the keyboard in it (not over the
    /// player, which hides the top bar).
    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.last().is_some_and(|page| page.is_player) {
            return;
        }
        let closing = self.search_motion.is_some_and(|motion| motion.closing);
        if !self.search_shown(cx) || closing {
            // The box goes in the middle of the bar, which (with as much on either side) is the
            // middle of the window.
            let width = dp_px(SEARCH_BOX_W, window);
            let place = (window.viewport_size().width - width) / 2.;
            self.start_search_motion(false, place, window, cx);
        }
        self.search_open = true;
        self.top_bar = None;
        self.menu = None;
        self.search.update(cx, |search, cx| search.focus(window, cx));
        cx.notify();
    }

    /// Turns the box back into the button, unless something's typed in it (it stays open
    /// with the results).
    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = std::mem::take(&mut self.search_open);
        if !was_open || !self.search.read(cx).value().is_empty() {
            cx.notify();
            return;
        }
        self.start_search_motion(true, self.search_slot.get().origin.x, window, cx);
        cx.notify();
    }

    /// Starts the box opening from, or closing into, the button, with the box's place at
    /// `place` (from the window's left). Without the button's position, the box just appears.
    fn start_search_motion(&mut self, closing: bool, place: Pixels, window: &Window, cx: &mut Context<Self>) {
        let epoch = self.search_motion.map_or(0, |motion| motion.epoch + 1);
        self.search_motion = self.search_button.get().map(|from_right| SearchMotion {
            epoch,
            closing,
            dx: window.viewport_size().width - from_right - place,
            running: true,
        });
        let duration = motion::morph_duration(cx);
        self._search_motion_end = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |this, cx| {
                // Closed, the box goes; open, the button does.
                match this.search_motion {
                    Some(SearchMotion { closing: true, .. }) => this.search_motion = None,
                    Some(ref mut motion) => motion.running = false,
                    None => {}
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Whether the search box is in the bar rather than the button (also while it closes).
    fn search_shown(&self, cx: &App) -> bool {
        self.search_open
            || !self.search.read(cx).value().is_empty()
            || self.search_motion.is_some_and(|motion| motion.closing)
    }

    /// Where the results page is on the stack.
    fn search_page(&self) -> Option<usize> {
        self.stack
            .iter()
            .rposition(|page| page.view.clone().downcast::<SearchPage>().is_ok())
    }

    /// The box changed: search, opening the results (or going back to them, from a page
    /// opened from them), or close the results when it's empty.
    fn search_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let term = self.search.read(cx).value().trim().to_string();
        let index = self.search_page();
        if term.is_empty() {
            // The keyboard stays in the box; Down or Esc gives it to the page below.
            if let Some(index) = index {
                self.stack.remove(index);
                cx.notify();
            }
            return;
        }
        match index {
            Some(index) => self.stack.truncate(index + 1),
            None => self.push(Route::Search, window, cx),
        }
        if let Some(page) = self
            .stack
            .last()
            .and_then(|page| page.view.clone().downcast::<SearchPage>().ok())
        {
            page.update(cx, |page, cx| page.set_term(term, cx));
        }
        cx.notify();
    }

    /// Enter or Down in the search box: into the results, or back to the page below.
    fn enter_search_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let results = self
            .stack
            .last()
            .and_then(|page| page.view.clone().downcast::<SearchPage>().ok());
        match results {
            Some(page) => page.update(cx, |page, cx| page.enter_results(window, cx)),
            None => self.leave_top_bar(window, cx),
        }
    }

    /// Down in the search box (which would otherwise ignore it).
    fn search_box_keys(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "down" && !event.keystroke.modifiers.modified() {
            cx.stop_propagation();
            self.enter_search_results(window, cx);
        }
    }

    fn render_top_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_go_back = self.stack.len() > 1;
        // The keyboard's ring, while the shell itself has focus (a click elsewhere takes it).
        let keyboard = self.top_bar.filter(|_| self.focus_handle.is_focused(window));
        let syncplay = self.render_syncplay(cx).into_any_element();
        let account = self.render_account(cx).into_any_element();
        let ring = |item: TopBarItem, element: AnyElement| {
            div()
                .relative()
                .rounded_lg()
                .child(element)
                .when(keyboard == Some(item), |el| el.child(selection_ring(cx)))
        };
        let title = self.stack.last().map(|p| p.title.clone()).unwrap_or_default();
        let search_shown = self.search_shown(cx);
        // Logo, Back and the title on the left, the buttons on the right, the open search box
        // between them. The two sides grow alike, which keeps the box in the middle.
        let start = h_flex()
            .flex_1()
            .min_w_0()
            .gap_4()
            .child(img(LOGO.clone()).flex_none().size(dp(TOP_BAR_BUTTON)))
            // Only when there's a page to go back to.
            .when(can_go_back, |el| {
                el.child(ring(
                    TopBarItem::Back,
                    Button::new("back")
                        .ghost()
                        .h(dp(TOP_BAR_BUTTON))
                        .child(bar_icon(IconName::ArrowLeft))
                        // No tooltip: the button goes away under the pointer when it reaches the
                        // first page, and gpui-kit's tooltip only hides when the pointer leaves
                        // its button, so it stayed on screen.
                        .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx)))
                        .into_any_element(),
                ))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            );
        let running = self.search_motion.filter(|motion| motion.running);
        let search_button = (!search_shown || running.is_some()).then(|| {
            let bounds = self.search_button.clone();
            let button = ring(
                TopBarItem::Search,
                Button::new("search")
                    .ghost()
                    .h(dp(TOP_BAR_BUTTON))
                    .child(bar_icon(IconName::Search))
                    .tooltip("Search")
                    .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
                    .child(
                        canvas(
                            move |b, window, _| bounds.set(Some(window.viewport_size().width - b.origin.x)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .into_any_element(),
            );
            match running {
                Some(SearchMotion { epoch, closing, .. }) => {
                    let (from, to) = if closing { (0., 1.) } else { (1., 0.) };
                    motion::fade(button, ("search-button-fade", epoch), from, to).into_any_element()
                }
                None => button.into_any_element(),
            }
        });
        let search_box = search_shown.then(|| {
            // While it moves it clips the input, which won't get as narrow as the button. (Not
            // otherwise: that would cut off the focus ring.)
            let input = div().w_full().when(running.is_some(), |el| el.overflow_hidden()).child(
                Input::new(&self.search)
                    .h(dp(TOP_BAR_BUTTON))
                    .prefix(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
                    .cleanable(true),
            );
            // Grows out of the button into its place, or back.
            let input = match self.search_motion {
                Some(SearchMotion { epoch, closing, dx, .. }) => {
                    let button = (dx, dp_px(TOP_BAR_BUTTON, window));
                    // Opening, its slot may not have been laid out yet; `morph` keeps it within.
                    let width = match closing {
                        true => self.search_slot.get().size.width,
                        false => dp_px(SEARCH_BOX_W, window),
                    };
                    let (button, place) = ((button.0, button.1, 0.), (px(0.), width.max(button.1), 1.));
                    let (from, to) = if closing { (place, button) } else { (button, place) };
                    motion::morph(input, ("search-motion", epoch), from, to).into_any_element()
                }
                None => input.into_any_element(),
            };
            let slot = self.search_slot.clone();
            div()
                .relative()
                .min_w(dp(TOP_BAR_BUTTON * 4.))
                .w(dp(SEARCH_BOX_W))
                .text_base()
                .capture_key_down(cx.listener(Self::search_box_keys))
                .child(
                    canvas(move |b, _, _| slot.set(b), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(
                    div()
                        .relative()
                        .w_full()
                        .rounded_lg()
                        .child(input)
                        .when(keyboard == Some(TopBarItem::Search), |el| el.child(selection_ring(cx))),
                )
        });
        let end = h_flex()
            .flex_1()
            .justify_end()
            .gap_4()
            .children(search_button)
            .child(ring(TopBarItem::SyncPlay, syncplay))
            .child(ring(
                TopBarItem::Settings,
                Button::new("settings")
                    .ghost()
                    .h(dp(TOP_BAR_BUTTON))
                    .child(bar_icon(IconName::Settings))
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx)))
                    .into_any_element(),
            ))
            .child(ring(TopBarItem::Account, account));
        h_flex()
            .h(dp(TOP_BAR_H))
            .flex_none()
            .px_6()
            .gap_4()
            // The account name; the title sets its own.
            .text_lg()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(start)
            .children(search_box)
            .child(end)
    }

    /// The user button and its menu: the saved accounts to switch to, adding one, signing out.
    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let account_menu = self
            .account_menu
            .as_ref()
            .filter(|_| matches!(self.menu, Some((Menu::Account, _))))
            .map(|(menu, _)| menu.clone());
        menu_popover(
            "account",
            Anchor::TopRight,
            account_menu,
            Button::new("account-button")
                .ghost()
                .h(dp(TOP_BAR_BUTTON))
                .child(avatar(&self.session.user.name, cx))
                .tooltip(match &self.session.server.server_name {
                    Some(server) => format!("{} on {server}", self.session.user.name),
                    None => self.session.user.name.clone(),
                }),
            move |open, window, cx| shell.update(cx, |this, cx| this.menu_toggled(Menu::Account, *open, window, cx)),
            cx,
        )
    }

    /// The account menu: the saved accounts to switch between, then adding one and signing
    /// out. Each choice goes to the app view through a [`ShellEvent`].
    fn build_account_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<PopupMenu> {
        let shell = cx.entity().downgrade();
        let current = self.session.account_key();
        let accounts = self.accounts.clone();
        PopupMenu::build(window, cx, move |mut menu, _, _| {
            let emit = |event: Box<dyn Fn() -> ShellEvent>| {
                let shell = shell.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    shell.update(cx, |_, cx| cx.emit(event())).ok();
                }
            };
            // Every saved account, the current one checked (choosing it just closes the menu).
            for account in &accounts {
                let label = account.label();
                let is_current = account.credential_account() == current;
                let account = account.clone();
                menu = menu.item(PopupMenuItem::new(label).checked(is_current).when(!is_current, |item| {
                    item.on_click(emit(Box::new(move || ShellEvent::SwitchAccount(account.clone()))))
                }));
            }
            menu.separator()
                .item(
                    PopupMenuItem::new("Add account…")
                        .icon(IconName::Plus)
                        .on_click(emit(Box::new(|| ShellEvent::AddAccount))),
                )
                .item(PopupMenuItem::new("Sign out").on_click(emit(Box::new(|| ShellEvent::SignOut))))
        })
    }

    /// A menu opened with the mouse, or closed (a click outside, Esc, or a choice in it).
    fn menu_toggled(&mut self, menu: Menu, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open {
            self.open_menu(menu, None, window, cx);
        } else if self.menu.is_some_and(|(m, _)| m == menu) {
            self.menu = None;
            let account_menu = self.account_menu.take();
            // A choice that moved focus (adding an account) keeps it.
            if account_menu.is_some_and(|(menu, _)| !menu_left_focus(&menu, window, cx)) {
                cx.notify();
                return;
            }
            // Back to the top bar if the keyboard came from there, else to the page.
            if self.top_bar.is_some() {
                self.focus_handle.focus(window, cx);
            } else {
                self.focus_top(window, cx);
            }
            cx.notify();
        }
    }

    /// The keyboard's highlight in `menu`, if it's open from the keyboard.
    fn menu_highlight(&self, menu: Menu) -> Option<usize> {
        self.menu
            .filter(|(m, _)| *m == menu)
            .and_then(|(_, highlight)| highlight)
    }
}

/// A menu's content, passing its keys to the shell's top-bar handling. It takes the focus the
/// popover moves to on opening, so it's where keys start.
fn menu_keys(shell: &Entity<ShellView>, content: AnyElement, cx: &App) -> AnyElement {
    let focus = shell.read(cx).menu_focus.clone();
    let shell = shell.clone();
    div()
        .track_focus(&focus)
        // The popover binds Enter to `Confirm` (which dismisses it); bindings come before key
        // listeners, so the highlighted entry is chosen here.
        .on_action({
            let shell = shell.clone();
            move |_: &Confirm, window, cx| {
                shell.update(cx, |this, cx| {
                    if let Some((menu, Some(highlight))) = this.menu {
                        cx.stop_propagation();
                        this.choose_menu_entry(menu, highlight, window, cx);
                    }
                })
            }
        })
        .on_key_down(move |event, window, cx| shell.update(cx, |this, cx| this.on_key_down(event, window, cx)))
        .child(content)
        .into_any_element()
}

/// A round badge with the first letter of `name`, standing in for the user's picture.
fn avatar(name: &str, cx: &App) -> impl IntoElement {
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".into());
    div()
        .size(dp(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(cx.theme().muted)
        .text_color(cx.theme().foreground)
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
}

fn new_group_name(session: &Session) -> String {
    format!("{}'s group", session.user.name)
}

impl ShellView {
    /// The SyncPlay button and its popover: join or create a group, or see and leave it.
    fn render_syncplay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let syncplay = SyncPlay::global(cx);
        let in_group = syncplay.read(cx).group.is_some();
        let new_name = new_group_name(&self.session);
        let shell = cx.entity();
        let highlight = self.menu_highlight(Menu::SyncPlay);
        let keys = shell.clone();
        Popover::new("syncplay")
            .anchor(Anchor::TopRight)
            .open(matches!(self.menu, Some((Menu::SyncPlay, _))))
            .track_focus(&self.menu_focus)
            .on_open_change(move |open, window, cx| {
                shell.update(cx, |this, cx| this.menu_toggled(Menu::SyncPlay, *open, window, cx))
            })
            .trigger(
                Button::new("syncplay-button")
                    .ghost()
                    .h(dp(TOP_BAR_BUTTON))
                    .child(bar_icon(Lucide::Users))
                    // Pressed while in a group.
                    .selected(in_group)
                    .tooltip("Watch together (SyncPlay)"),
            )
            .content(move |_, _, cx| {
                let panel = syncplay_panel(&syncplay, &new_name, highlight, cx);
                menu_keys(&keys, panel, cx)
            })
    }
}

/// The Leave button's place in the keyboard's entries: after shuffle, repeat and the queue.
fn leave_index(state: &SyncPlay) -> usize {
    state
        .queue
        .as_ref()
        .filter(|q| !q.playlist.is_empty())
        .map_or(0, |q| 2 + q.playlist.len())
}

/// The group's queue: shuffle and repeat, then the entries (the playing one highlighted), each
/// with buttons to play it, move it and remove it. Indexes match `ShellView::menu_entries`.
fn group_queue(syncplay: &Entity<SyncPlay>, highlight: Option<usize>, cx: &App) -> Option<AnyElement> {
    let state = syncplay.read(cx);
    let queue = state.queue.as_ref().filter(|q| !q.playlist.is_empty())?;
    let muted = cx.theme().muted_foreground;
    let shuffled = queue.shuffle_mode == ShuffleMode::Shuffle;
    let repeat_label = match queue.repeat_mode {
        RepeatMode::RepeatNone => "Repeat is off",
        RepeatMode::RepeatAll => "Repeating the queue",
        RepeatMode::RepeatOne => "Repeating one",
    };
    // Toggles, pressed while on; the keyboard's highlight is a ring around them.
    let ringed = |index: usize, toggle: Toggle| {
        div()
            .relative()
            .rounded_md()
            .child(toggle)
            .when(highlight == Some(index), |el| el.child(selection_ring(cx)))
    };
    let toggle_action = |syncplay: &Entity<SyncPlay>, run: fn(&mut SyncPlay, &mut Context<SyncPlay>)| {
        let syncplay = syncplay.clone();
        move |_: &bool, _: &mut Window, cx: &mut App| syncplay.update(cx, run)
    };
    let modes = h_flex()
        .gap_1()
        .child(ringed(
            0,
            Toggle::new("syncplay-shuffle")
                .small()
                .gap_1()
                .icon(Lucide::Shuffle)
                .label("Shuffle")
                .checked(shuffled)
                .on_click(toggle_action(syncplay, |s, cx| s.toggle_shuffle(cx))),
        ))
        .child(ringed(
            1,
            // Off, all, one: each click moves to the next.
            Toggle::new("syncplay-repeat")
                .small()
                .gap_1()
                .icon(if queue.repeat_mode == RepeatMode::RepeatOne {
                    Lucide::Repeat1
                } else {
                    Lucide::Repeat
                })
                .label("Repeat")
                .checked(queue.repeat_mode != RepeatMode::RepeatNone)
                .tooltip(repeat_label)
                .on_click(toggle_action(syncplay, |s, cx| s.cycle_repeat(cx))),
        ));
    let last = queue.playlist.len() - 1;
    let rows = queue.playlist.iter().enumerate().map(|(i, entry)| {
        let name = state.names.get(&entry.item_id).cloned().unwrap_or_else(|| "…".into());
        let playing = queue.playing_item_index == i as i32;
        let id = entry.playlist_item_id.clone();
        let button = |key: &'static str, icon: Lucide, tip: &'static str, disabled: bool| {
            Button::new(SharedString::from(format!("{key}-{id}")))
                .xsmall()
                .ghost()
                .icon(icon)
                .tooltip(tip)
                .disabled(disabled)
        };
        let (play, up, down, remove) = (id.clone(), id.clone(), id.clone(), id.clone());
        let syncplay = syncplay.clone();
        h_flex()
            .id(SharedString::from(format!("queue-{id}")))
            .gap_1()
            .px_1()
            .rounded_md()
            .when(highlight == Some(2 + i), |el| el.bg(cx.theme().accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(playing, |el| el.font_weight(FontWeight::SEMIBOLD))
                    .when(!playing, |el| el.text_color(muted))
                    .child(name)
                    .on_mouse_down(MouseButton::Left, {
                        let syncplay = syncplay.clone();
                        move |_, _, cx| syncplay.update(cx, |s, cx| s.play_entry(play.clone(), cx))
                    }),
            )
            .child(
                button("queue-up", Lucide::ArrowUp, "Move up (Alt+Up)", i == 0).on_click({
                    let syncplay = syncplay.clone();
                    move |_, _, cx| syncplay.update(cx, |s, cx| s.move_entry(up.clone(), -1, cx))
                }),
            )
            .child(
                button("queue-down", Lucide::ArrowDown, "Move down (Alt+Down)", i == last).on_click({
                    let syncplay = syncplay.clone();
                    move |_, _, cx| syncplay.update(cx, |s, cx| s.move_entry(down.clone(), 1, cx))
                }),
            )
            .child(
                button("queue-remove", Lucide::X, "Remove (Delete)", false)
                    .on_click(move |_, _, cx| syncplay.update(cx, |s, cx| s.remove_entry(remove.clone(), cx))),
            )
            .into_any_element()
    });
    Some(
        v_flex()
            .gap_1()
            .child(div().pt_1().text_sm().text_color(muted).child("Queue"))
            .child(modes)
            .children(rows)
            .into_any_element(),
    )
}

/// `highlight` is the keyboard's entry, as `ShellView::menu_entries` lists them.
fn syncplay_panel(syncplay: &Entity<SyncPlay>, new_name: &str, highlight: Option<usize>, cx: &mut App) -> AnyElement {
    let state = syncplay.read(cx);
    let muted = cx.theme().muted_foreground;
    let body = match &state.group {
        Some(group) => {
            let status = match group.state {
                GroupState::Idle => "Nothing playing",
                GroupState::Waiting => "Waiting for everyone",
                GroupState::Paused => "Paused",
                GroupState::Playing => "Playing",
            };
            v_flex()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(group.group_name.clone()))
                .child(div().text_sm().text_color(muted).child(status))
                .child(
                    div()
                        .text_sm()
                        .child(format!("Watching: {}", group.participants.join(", "))),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Anything you play now plays for everyone in the group."),
                )
                .children(group_queue(syncplay, highlight, cx))
                .child(
                    Button::new("syncplay-leave")
                        .outline()
                        .label("Leave group")
                        .selected(highlight == Some(leave_index(state)))
                        .on_click({
                            let syncplay = syncplay.clone();
                            move |_, _, cx| syncplay.update(cx, |syncplay, cx| syncplay.leave(cx))
                        }),
                )
                .into_any_element()
        }
        None => {
            let groups = state.groups.iter().enumerate().map(|(i, group)| {
                let id = group.group_id.clone();
                h_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(group.group_name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .truncate()
                                    .child(group.participants.join(", ")),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("join-{id}")))
                            .small()
                            .label("Join")
                            .selected(highlight == Some(i))
                            .on_click({
                                let syncplay = syncplay.clone();
                                move |_, _, cx| syncplay.update(cx, |syncplay, cx| syncplay.join(id.clone(), cx))
                            }),
                    )
            });
            let new_name = new_name.to_owned();
            v_flex()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Watch together"))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Play in sync with others on this server."),
                )
                .when(state.groups.is_empty(), |el| {
                    el.child(div().text_sm().text_color(muted).child("No groups yet."))
                })
                .children(groups)
                .child(
                    Button::new("syncplay-new")
                        .primary()
                        .label("New group")
                        .selected(highlight == Some(state.groups.len()))
                        .on_click({
                            let syncplay = syncplay.clone();
                            move |_, _, cx| syncplay.update(cx, |syncplay, cx| syncplay.create(new_name.clone(), cx))
                        }),
                )
                .into_any_element()
        }
    };
    div().w(dp(300.)).child(body).into_any_element()
}

impl Focusable for ShellView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.stack.last().map(|p| {
            let view = p.view.clone();
            let id = ("page", view.entity_id().as_u64() as usize);
            let page = div().size_full().child(view);
            if p.is_player {
                page.into_any_element()
            } else {
                motion::enter(page, id, Duration::ZERO).into_any_element()
            }
        });
        let show_top_bar = !self.stack.last().is_some_and(|p| p.is_player);
        v_flex()
            .size_full()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::open_search))
            .on_action(cx.listener(Self::enter_top_bar))
            .on_key_down(cx.listener(Self::on_key_down))
            .bg(cx.theme().background)
            .when(show_top_bar, |el| el.child(self.render_top_bar(window, cx)))
            .child(div().flex_1().min_h_0().children(page))
    }
}

/// Looks up a page's focus handle when it comes back on top (it can change, e.g. search).
type Focuser = Rc<dyn Fn(&App) -> FocusHandle>;

/// Runs the update check and, if there's a newer release, says so with a link to it.
fn notify_update(window: &mut Window, cx: &mut Context<ShellView>) {
    cx.spawn_in(window, async move |_, cx| {
        let Some(update) = crate::runtime::spawn(crate::update::check()).await else {
            return;
        };
        cx.update(|window, cx| {
            let message = format!(
                "Version {} is out (this is {}).",
                update.version,
                env!("CARGO_PKG_VERSION")
            );
            let url = update.url;
            let notification = Notification::info(message)
                .title("Update available")
                .action(move |_, _, _| {
                    let url = url.clone();
                    Button::new("download-update")
                        .label("Download")
                        .on_click(move |_, _, cx| cx.open_url(&url))
                });
            window.push_notification(notification, cx);
        })
        .ok();
    })
    .detach();
}

fn focuser<T: Focusable>(page: &Entity<T>) -> Focuser {
    let page = page.clone();
    Rc::new(move |cx| page.read(cx).focus_handle(cx))
}

/// Dev aid: `JELLY_GPUI_ROUTE=search:<term>` types `term` into the search box at startup.
#[cfg(debug_assertions)]
fn debug_search_term() -> Option<String> {
    let value = std::env::var("JELLY_GPUI_ROUTE").ok()?;
    value.strip_prefix("search:").map(str::to_string)
}

/// Dev aid: open a page on top of Home at startup, for screenshots and quick iteration.
/// `JELLY_GPUI_ROUTE=library:<id>:<movies|tvshows>`, `item:<id>`, `settings`, or
/// `play:<id>[:<audio index>[:<subtitle index or -1>[:<media source id>]]]` (opens the item and
/// plays it from its resume point; empty fields keep the defaults). Returns the route and, for `play`, the track choices.
#[cfg(debug_assertions)]
#[allow(clippy::type_complexity)]
fn debug_start_route() -> Option<(Route, Option<(Option<i32>, Option<i32>, Option<String>)>)> {
    use jellyfin::models::CollectionType;

    let value = std::env::var("JELLY_GPUI_ROUTE").ok()?;
    let mut parts = value.split(':');
    match (parts.next()?, parts.next()) {
        ("settings", _) => Some((Route::Settings, None)),
        (kind @ ("item" | "play"), Some(id)) => {
            let rest: Vec<&str> = parts.collect();
            let index = |i: usize| rest.get(i).and_then(|p| p.parse().ok());
            let source = rest.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string());
            let tracks = (kind == "play").then(|| (index(0), index(1), source));
            Some((
                Route::Item {
                    id: id.into(),
                    name: "Item".into(),
                },
                tracks,
            ))
        }
        ("library", Some(id)) => Some((
            Route::Library {
                id: id.into(),
                name: "Library".into(),
                collection_type: match parts.next() {
                    Some("movies") => Some(CollectionType::Movies),
                    Some("tvshows") => Some(CollectionType::TvShows),
                    _ => None,
                },
            },
            None,
        )),
        _ => None,
    }
}
