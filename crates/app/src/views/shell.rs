//! Signed-in frame: top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.

use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::GroupState;

use crate::images::ImageCache;
use crate::playback::{PlaybackEvent, PlaybackManager};
use crate::router::{Navigator, Route};
use crate::session::Session;
use crate::syncplay::{SyncPlay, SyncPlayEvent};
use crate::views::home::HomePage;
use crate::views::item::ItemPage;
use crate::views::library::{LibraryPage, Source};
use crate::views::player::PlayerPage;
use crate::views::search::SearchPage;
use crate::views::settings::SettingsPage;

actions!(jelly, [Back, OpenSearch]);

pub const KEY_CONTEXT: &str = "Shell";

pub enum ShellEvent {
    SignOut,
    /// The server rejected the session token.
    SessionExpired,
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
    /// The session websocket; dropped (disconnecting) with the shell on sign-out.
    _live: Task<()>,
}

impl EventEmitter<ShellEvent> for ShellView {}

impl ShellView {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ImageCache::init(session.client.clone(), cx);
        PlaybackManager::init(session.client.clone(), cx);
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
        let _live = crate::live::connect(session.client.clone(), cx);
        let mut this = Self {
            session,
            stack: Vec::new(),
            focus_handle: cx.focus_handle(),
            _playback,
            _syncplay,
            _live,
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
        #[cfg(debug_assertions)]
        replay_debug_keys(window, cx);
        this
    }

    pub fn push(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let nav = Navigator::new(cx.entity().downgrade());
        let client = self.session.client.clone();
        let mut focus = None;
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
                view: cx.new(|cx| SettingsPage::new(self.session.clone(), window, cx)).into(),
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

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
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

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_go_back = self.stack.len() > 1;
        let title = self.stack.last().map(|p| p.title.clone()).unwrap_or_default();
        h_flex()
            .h(px(52.))
            .flex_none()
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("back")
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .disabled(!can_go_back)
                    .on_click(cx.listener(|this, _, window, cx| this.back(&Back, window, cx))),
            )
            .child(
                Button::new("home")
                    .ghost()
                    .label("Home")
                    .on_click(cx.listener(|this, _, window, cx| this.home(window, cx))),
            )
            .child(div().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                Button::new("search")
                    .ghost()
                    .icon(IconName::Search)
                    .tooltip("Search")
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(&OpenSearch, window, cx))),
            )
            .child(self.render_syncplay(cx))
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _, window, cx| this.push(Route::Settings, window, cx))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.session.user.name.clone()),
            )
            .child(
                Button::new("sign-out")
                    .ghost()
                    .label("Sign out")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::SignOut))),
            )
    }
}

impl ShellView {
    /// The SyncPlay button and its popover: join or create a group, or see and leave it.
    fn render_syncplay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let syncplay = SyncPlay::global(cx);
        let in_group = syncplay.read(cx).group.is_some();
        let new_name = format!("{}'s group", self.session.user.name);
        Popover::new("syncplay")
            .anchor(Anchor::TopRight)
            .on_open_change({
                let syncplay = syncplay.clone();
                move |open, _, cx| {
                    if *open {
                        syncplay.update(cx, |syncplay, cx| syncplay.refresh_groups(cx));
                    }
                }
            })
            .trigger(
                Button::new("syncplay-button")
                    .ghost()
                    .icon(Lucide::Users)
                    .when(in_group, |b| b.primary())
                    .tooltip("Watch together (SyncPlay)"),
            )
            .content(move |_, _, cx| syncplay_panel(&syncplay, &new_name, cx))
    }
}

fn syncplay_panel(syncplay: &Entity<SyncPlay>, new_name: &str, cx: &mut App) -> AnyElement {
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
                .child(Button::new("syncplay-leave").outline().label("Leave group").on_click({
                    let syncplay = syncplay.clone();
                    move |_, _, cx| syncplay.update(cx, |syncplay, cx| syncplay.leave(cx))
                }))
                .into_any_element()
        }
        None => {
            let groups = state.groups.iter().map(|group| {
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
                .child(Button::new("syncplay-new").primary().label("New group").on_click({
                    let syncplay = syncplay.clone();
                    move |_, _, cx| syncplay.update(cx, |syncplay, cx| syncplay.create(new_name.clone(), cx))
                }))
                .into_any_element()
        }
    };
    div().w(px(300.)).child(body).into_any_element()
}

impl Focusable for ShellView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.stack.last().map(|p| p.view.clone());
        let show_top_bar = !self.stack.last().is_some_and(|p| p.is_player);
        v_flex()
            .size_full()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::open_search))
            .bg(cx.theme().background)
            .when(show_top_bar, |el| el.child(self.render_top_bar(cx)))
            .child(div().flex_1().min_h_0().children(page))
    }
}

/// Looks up a page's focus handle when it comes back on top (it can change, e.g. search).
type Focuser = Rc<dyn Fn(&App) -> FocusHandle>;

fn focuser<T: Focusable>(page: &Entity<T>) -> Focuser {
    let page = page.clone();
    Rc::new(move |cx| page.read(cx).focus_handle(cx))
}

/// Dev aid: `JELLY_GPUI_KEYS="down right enter wait escape"` presses keys after startup, for
/// testing keyboard navigation without touching the desktop. `wait` pauses for a second.
#[cfg(debug_assertions)]
fn replay_debug_keys(window: &mut Window, cx: &mut Context<ShellView>) {
    use std::time::Duration;

    let Ok(keys) = std::env::var("JELLY_GPUI_KEYS") else {
        return;
    };
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        for key in keys.split_whitespace() {
            if key == "wait" {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                continue;
            }
            match Keystroke::parse(key) {
                Ok(keystroke) => {
                    tracing::info!("debug key: {key}");
                    cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx)).ok();
                }
                Err(err) => tracing::warn!("bad debug key {key:?}: {err}"),
            }
            cx.background_executor().timer(Duration::from_millis(250)).await;
        }
    })
    .detach();
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
