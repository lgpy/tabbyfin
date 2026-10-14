//! Signed-in frame: top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, h_flex, v_flex};
use gpui_kit::*;

use crate::images::ImageCache;
use crate::router::{Navigator, Route};
use crate::session::Session;
use crate::views::home::HomePage;
use crate::views::item::ItemPage;
use crate::views::library::LibraryPage;

actions!(jelly, [Back]);

pub const KEY_CONTEXT: &str = "Shell";

pub enum ShellEvent {
    SignOut,
}

struct Page {
    title: SharedString,
    view: AnyView,
}

pub struct ShellView {
    session: Session,
    stack: Vec<Page>,
    focus_handle: FocusHandle,
}

impl EventEmitter<ShellEvent> for ShellView {}

impl ShellView {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ImageCache::init(session.client.clone(), cx);
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let mut this = Self {
            session,
            stack: Vec::new(),
            focus_handle,
        };
        this.push(Route::Home, window, cx);
        #[cfg(debug_assertions)]
        if let Some(route) = debug_start_route() {
            this.push(route, window, cx);
        }
        this
    }

    pub fn push(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let nav = Navigator::new(cx.entity().downgrade());
        let client = self.session.client.clone();
        let page = match route {
            Route::Home => Page {
                title: "Home".into(),
                view: cx.new(|cx| HomePage::new(client, nav, cx)).into(),
            },
            Route::Library {
                id,
                name,
                collection_type,
            } => Page {
                title: name.into(),
                view: cx
                    .new(|cx| LibraryPage::new(client, nav, id, collection_type, cx))
                    .into(),
            },
            Route::Item { id, name } => Page {
                title: name.into(),
                view: cx.new(|cx| ItemPage::new(client, nav, id, cx)).into(),
            },
        };
        self.stack.push(page);
        // Keep keyboard shortcuts working after clicks moved focus elsewhere.
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn back(&mut self, _: &Back, _window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.len() > 1 {
            self.stack.pop();
            cx.notify();
        }
    }

    fn home(&mut self, cx: &mut Context<Self>) {
        self.stack.truncate(1);
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
                    .on_click(cx.listener(|this, _, _, cx| this.home(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
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

impl Focusable for ShellView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.stack.last().map(|p| p.view.clone());
        v_flex()
            .size_full()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::back))
            .bg(cx.theme().background)
            .child(self.render_top_bar(cx))
            .child(div().flex_1().min_h_0().children(page))
    }
}

/// Dev aid: open a page on top of Home at startup, for screenshots and quick iteration.
/// `JELLY_GPUI_ROUTE=library:<id>:<movies|tvshows>` or `item:<id>`.
#[cfg(debug_assertions)]
fn debug_start_route() -> Option<Route> {
    use jellyfin::models::CollectionType;

    let value = std::env::var("JELLY_GPUI_ROUTE").ok()?;
    let mut parts = value.split(':');
    match (parts.next()?, parts.next()?) {
        ("item", id) => Some(Route::Item {
            id: id.into(),
            name: "Item".into(),
        }),
        ("library", id) => Some(Route::Library {
            id: id.into(),
            name: "Library".into(),
            collection_type: match parts.next() {
                Some("movies") => Some(CollectionType::Movies),
                Some("tvshows") => Some(CollectionType::TvShows),
                _ => None,
            },
        }),
        _ => None,
    }
}
