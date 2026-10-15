//! Signed-in frame: top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.

use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::GroupState;

use crate::browse::{MoveDown, MoveUp};
use crate::components::selection_ring;
use crate::config::{Config, SavedServer};
use crate::images::ImageCache;
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
use crate::zoom::dp;

actions!(jelly, [Back, OpenSearch]);

pub const KEY_CONTEXT: &str = "Shell";

/// The top bar's buttons, in order, for the keyboard: Up past a page's first row reaches
/// them, Left and Right move along, Enter presses, Down or Esc returns to the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TopBarItem {
    Back,
    Home,
    Search,
    SyncPlay,
    Settings,
    Account,
}

const TOP_BAR: [TopBarItem; 6] = [
    TopBarItem::Back,
    TopBarItem::Home,
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

/// A choice in a top-bar menu, in the order the menu lists them. The keyboard highlight is an
/// index into these.
enum MenuEntry {
    SwitchAccount(SavedServer),
    AddAccount,
    SignOut,
    JoinGroup(String),
    NewGroup,
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
    /// What an open menu focuses. Menus render in an overlay outside the shell, so their
    /// content forwards keys to `on_key_down` (see `menu_keys`).
    menu_focus: FocusHandle,
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
        };
        this.push(Route::Home, window, cx);
        #[cfg(debug_assertions)]
        if let Some((route, autoplay)) = debug_start_route() {
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
                    let page = cx.new(|cx| SearchPage::new(client, nav, window, cx));
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
        } else {
            self.go_back(window, cx);
        }
    }

    /// Up past a page's first row (the page lets the action through): into the top bar.
    fn enter_top_bar(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_top_bar(window, cx);
    }

    /// Moves the keyboard into the top bar (also for Up in the search box).
    pub fn focus_top_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.last().is_some_and(|page| page.is_player) {
            return;
        }
        let first = if self.stack.len() > 1 {
            TopBarItem::Back
        } else {
            TopBarItem::Home
        };
        self.top_bar = Some(self.top_bar.unwrap_or(first));
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
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        if let Some((menu, Some(highlight))) = self.menu {
            let count = self.menu_entries(menu, cx).len();
            match key {
                "down" if highlight + 1 < count => self.menu = Some((menu, Some(highlight + 1))),
                "up" if highlight > 0 => self.menu = Some((menu, Some(highlight - 1))),
                "enter" => self.choose_menu_entry(menu, highlight, window, cx),
                "up" | "down" => {}
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
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
            TopBarItem::Home => {
                self.top_bar = None;
                self.home(window, cx);
            }
            TopBarItem::Search => {
                self.top_bar = None;
                self.push(Route::Search, window, cx);
            }
            TopBarItem::Settings => {
                self.top_bar = None;
                self.push(Route::Settings, window, cx);
            }
            TopBarItem::SyncPlay => self.open_menu(Menu::SyncPlay, Some(0), cx),
            TopBarItem::Account => self.open_menu(Menu::Account, Some(0), cx),
        }
    }

    /// Opens a top-bar menu, with fresh contents.
    fn open_menu(&mut self, menu: Menu, highlight: Option<usize>, cx: &mut Context<Self>) {
        match menu {
            Menu::Account => self.accounts = Config::load().accounts,
            Menu::SyncPlay => SyncPlay::global(cx).update(cx, |syncplay, cx| syncplay.refresh_groups(cx)),
        }
        self.menu = Some((menu, highlight));
        cx.notify();
    }

    fn menu_entries(&self, menu: Menu, cx: &App) -> Vec<MenuEntry> {
        match menu {
            Menu::Account => {
                let current = self.session.account_key();
                let mut entries: Vec<MenuEntry> = self
                    .accounts
                    .iter()
                    .filter(|account| account.credential_account() != current)
                    .cloned()
                    .map(MenuEntry::SwitchAccount)
                    .collect();
                entries.extend([MenuEntry::AddAccount, MenuEntry::SignOut]);
                entries
            }
            Menu::SyncPlay => {
                let syncplay = SyncPlay::global(cx).read(cx);
                if syncplay.group.is_some() {
                    return vec![MenuEntry::LeaveGroup];
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

    /// Does what a menu entry's button does, and closes the menu.
    fn choose_menu_entry(&mut self, menu: Menu, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.menu_entries(menu, cx).into_iter().nth(index) else {
            return;
        };
        self.leave_top_bar(window, cx);
        let syncplay = SyncPlay::global(cx);
        match entry {
            MenuEntry::SwitchAccount(account) => cx.emit(ShellEvent::SwitchAccount(account)),
            MenuEntry::AddAccount => cx.emit(ShellEvent::AddAccount),
            MenuEntry::SignOut => cx.emit(ShellEvent::SignOut),
            MenuEntry::JoinGroup(id) => syncplay.update(cx, |syncplay, cx| syncplay.join(id, cx)),
            MenuEntry::NewGroup => {
                let name = new_group_name(&self.session);
                syncplay.update(cx, |syncplay, cx| syncplay.create(name, cx));
            }
            MenuEntry::LeaveGroup => syncplay.update(cx, |syncplay, cx| syncplay.leave(cx)),
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
            self.focus_top(window, cx);
            cx.notify();
        }
    }

    fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.push(Route::Search, window, cx);
    }

    fn home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stack.truncate(1);
        self.focus_top(window, cx);
        cx.notify();
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
                .rounded_md()
                .child(element)
                .when(keyboard == Some(item), |el| el.child(selection_ring(cx)))
        };
        let title = self.stack.last().map(|p| p.title.clone()).unwrap_or_default();
        h_flex()
            .h(dp(52.))
            .flex_none()
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(ring(
                TopBarItem::Back,
                Button::new("back")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .disabled(!can_go_back)
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx)))
                    .into_any_element(),
            ))
            .child(ring(
                TopBarItem::Home,
                Button::new("home")
                    .ghost()
                    .icon(Lucide::House)
                    .tooltip("Home")
                    .on_click(cx.listener(|this, _, window, cx| this.home(window, cx)))
                    .into_any_element(),
            ))
            .child(div().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(ring(
                TopBarItem::Search,
                Button::new("search")
                    .ghost()
                    .icon(IconName::Search)
                    .tooltip("Search")
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(&OpenSearch, window, cx)))
                    .into_any_element(),
            ))
            .child(ring(TopBarItem::SyncPlay, syncplay))
            .child(ring(
                TopBarItem::Settings,
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _, window, cx| this.push(Route::Settings, window, cx)))
                    .into_any_element(),
            ))
            .child(ring(TopBarItem::Account, account))
    }

    /// The user button and its menu: the saved accounts to switch to, adding one, signing out.
    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let current = self.session.account_key();
        Popover::new("account")
            .anchor(Anchor::TopRight)
            .open(matches!(self.menu, Some((Menu::Account, _))))
            .track_focus(&self.menu_focus)
            .on_open_change({
                let shell = shell.clone();
                move |open, window, cx| shell.update(cx, |this, cx| this.menu_toggled(Menu::Account, *open, window, cx))
            })
            .trigger(
                Button::new("account-button")
                    .ghost()
                    .label(self.session.user.name.clone())
                    .icon(IconName::ChevronDown)
                    .tooltip(self.session.server.server_name.clone().unwrap_or_default()),
            )
            .content({
                let shell = shell.clone();
                move |_, window, cx| {
                    let panel = account_panel(&shell, &current, window, cx);
                    menu_keys(&shell, panel, cx)
                }
            })
    }

    /// A menu opened with the mouse, or closed (a click outside, Esc, or a choice in it).
    fn menu_toggled(&mut self, menu: Menu, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open {
            self.open_menu(menu, None, cx);
        } else if self.menu.is_some_and(|(m, _)| m == menu) {
            self.menu = None;
            // Back to the top bar, if the keyboard came from there.
            if self.top_bar.is_some() {
                self.focus_handle.focus(window, cx);
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

fn new_group_name(session: &Session) -> String {
    format!("{}'s group", session.user.name)
}

/// The account menu: other saved accounts first, then adding one and signing out. Each choice
/// closes the menu and goes to the app view through a [`ShellEvent`].
fn account_panel(
    shell: &Entity<ShellView>,
    current: &str,
    _window: &mut Window,
    cx: &mut Context<PopoverState>,
) -> AnyElement {
    let popover = cx.entity();
    let choose = move |event: fn() -> ShellEvent| {
        let (shell, popover) = (shell.clone(), popover.clone());
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            popover.update(cx, |popover, cx| popover.dismiss(window, cx));
            shell.update(cx, |_, cx| cx.emit(event()));
        }
    };
    let highlight = shell.read(cx).menu_highlight(Menu::Account);
    let accounts = &shell.read(cx).accounts;
    let current_label = accounts
        .iter()
        .find(|account| account.credential_account() == current)
        .map(SavedServer::label);
    let others: Vec<AnyElement> = accounts
        .iter()
        .filter(|account| account.credential_account() != current)
        .enumerate()
        .map(|(i, account)| {
            let (shell, popover, account) = (shell.clone(), cx.entity(), account.clone());
            Button::new(("switch-account", i))
                .ghost()
                .w_full()
                .justify_start()
                .label(account.label())
                .selected(highlight == Some(i))
                .on_click(move |_, window, cx| {
                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    let account = account.clone();
                    shell.update(cx, |_, cx| cx.emit(ShellEvent::SwitchAccount(account)));
                })
                .into_any_element()
        })
        .collect();
    let others_count = others.len();
    let muted = cx.theme().muted_foreground;
    v_flex()
        .w(dp(280.))
        .gap_1()
        .children(current_label.map(|label| div().px_2().pb_1().text_sm().text_color(muted).child(label)))
        .when(!others.is_empty(), |el| {
            el.child(div().px_2().pt_1().text_xs().text_color(muted).child("Switch to"))
                .children(others)
        })
        .child(
            Button::new("add-account")
                .ghost()
                .w_full()
                .justify_start()
                .icon(IconName::Plus)
                .label("Add account…")
                .selected(highlight == Some(others_count))
                .on_click(choose(|| ShellEvent::AddAccount)),
        )
        .child(
            Button::new("sign-out")
                .ghost()
                .w_full()
                .justify_start()
                .label("Sign out")
                .selected(highlight == Some(others_count + 1))
                .on_click(choose(|| ShellEvent::SignOut)),
        )
        .into_any_element()
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
                    .icon(Lucide::Users)
                    .when(in_group, |b| b.primary())
                    .tooltip("Watch together (SyncPlay)"),
            )
            .content(move |_, _, cx| {
                let panel = syncplay_panel(&syncplay, &new_name, highlight, cx);
                menu_keys(&keys, panel, cx)
            })
    }
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
                .child(
                    Button::new("syncplay-leave")
                        .outline()
                        .label("Leave group")
                        .selected(highlight == Some(0))
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
        let page = self.stack.last().map(|p| p.view.clone());
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
                        .primary()
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

/// Dev aid: open a page on top of Home at startup, for screenshots and quick iteration.
/// `JELLY_GPUI_ROUTE=library:<id>:<movies|tvshows>`, `item:<id>`, `search`, `settings`, or
/// `play:<id>[:<audio index>[:<subtitle index or -1>[:<media source id>]]]` (opens the item and
/// plays it from its resume point; empty fields keep the defaults). Returns the route and, for `play`, the track choices.
#[cfg(debug_assertions)]
#[allow(clippy::type_complexity)]
fn debug_start_route() -> Option<(Route, Option<(Option<i32>, Option<i32>, Option<String>)>)> {
    use jellyfin::models::CollectionType;

    let value = std::env::var("JELLY_GPUI_ROUTE").ok()?;
    let mut parts = value.split(':');
    match (parts.next()?, parts.next()) {
        ("search", _) => Some((Route::Search, None)),
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
