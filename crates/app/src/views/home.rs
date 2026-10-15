//! Home: libraries, Continue Watching, Next Up, Favorites, and Latest per library, in the order
//! and with the libraries the user set in the web client's Settings → Home.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType, ItemField, ItemFilter, ItemSortBy};
use jellyfin::{Client, ItemsQuery};

use crate::browse::{self, Browse, Cursor, Position, Section, on_browse_actions};
use crate::changes::{Changes, UserDataChanged};
use crate::components::{
    Load, centered_message, centered_spinner, error_state, landscape_card, poster_card, shelf_with_rail,
};
use crate::images::ImageCache;
use crate::motion;
use crate::router::Navigator;
use crate::runtime;
use crate::smooth_scroll::smooth_wheel;

const SHELF_LIMIT: u32 = 24;
const FIELDS: &[ItemField] = &[ItemField::PrimaryImageAspectRatio];
/// The delay between one shelf appearing and the next.
const SHELF_STAGGER: Duration = Duration::from_millis(60);
/// How far the mouse moves with the button down before a press on a shelf becomes a drag.
const DRAG_THRESHOLD: Pixels = px(4.);

/// The Home sections of the web client's Settings → Home that this app has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeSection {
    MyMedia,
    Resume,
    NextUp,
    Latest,
}

/// The web client's section for each of its ten Home slots when the user hasn't picked one.
const DEFAULT_SECTIONS: [&str; 10] = [
    "smalllibrarytiles",
    "resume",
    "resumeaudio",
    "resumebook",
    "livetv",
    "nextup",
    "latestmedia",
    "none",
    "none",
    "none",
];

/// The sections in the user's order, from the web client's `homesection0`…`homesection9`
/// preferences. Sections this app doesn't have (music, books, live TV) are skipped.
fn home_sections(prefs: &HashMap<String, String>) -> Vec<HomeSection> {
    let mut sections = Vec::new();
    for (slot, default) in DEFAULT_SECTIONS.iter().enumerate() {
        let value = prefs
            .get(&format!("homesection{slot}"))
            .map_or(*default, String::as_str);
        let section = match value {
            "smalllibrarytiles" | "librarybuttons" => HomeSection::MyMedia,
            "resume" => HomeSection::Resume,
            "nextup" => HomeSection::NextUp,
            "latestmedia" => HomeSection::Latest,
            _ => continue,
        };
        if !sections.contains(&section) {
            sections.push(section);
        }
    }
    sections
}

struct HomeData {
    sections: Vec<HomeSection>,
    views: Vec<BaseItemDto>,
    resume: Vec<BaseItemDto>,
    next_up: Vec<BaseItemDto>,
    favorites: Vec<BaseItemDto>,
    /// Latest items per library.
    latest: Vec<(BaseItemDto, Vec<BaseItemDto>)>,
}

pub struct HomePage {
    client: Client,
    nav: Navigator,
    data: Load<HomeData>,
    scroll: ScrollHandle,
    /// One per shelf, in display order.
    shelf_scrolls: Vec<ScrollHandle>,
    /// The shelf being dragged sideways with the mouse, while the button is down.
    drag: Option<ShelfDrag>,
    /// Whether the last press on a shelf became a drag, so its release doesn't open a card.
    dragged: Rc<Cell<bool>>,
    cursor: Cursor,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl HomePage {
    pub fn new(client: Client, nav: Navigator, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            // Resume points and Next Up change after playback or marking items watched.
            cx.subscribe(&Changes::global(cx), |this, _, _: &UserDataChanged, cx| this.reload(cx)),
        ];
        let mut this = Self {
            client,
            nav,
            data: Load::Loading,
            scroll: ScrollHandle::new(),
            shelf_scrolls: Vec::new(),
            drag: None,
            dragged: Rc::default(),
            cursor: Cursor::default(),
            focus_handle: cx.focus_handle(),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Fetches everything again. The current content stays up until the new data arrives.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(load(client)).await;
            this.update(cx, |this, cx| {
                this.data = match result {
                    Ok(data) => Load::Loaded(data),
                    Err(err) => Load::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

async fn load(client: Client) -> jellyfin::Result<HomeData> {
    let favorites = ItemsQuery {
        include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series, BaseItemKind::Episode],
        recursive: true,
        filters: vec![ItemFilter::IsFavorite],
        sort_by: vec![ItemSortBy::SortName],
        fields: FIELDS.to_vec(),
        limit: Some(SHELF_LIMIT),
        ..Default::default()
    };
    // The user's Home settings are a nicety: without them, Home uses the defaults.
    let settings = async {
        let (user, prefs) = tokio::join!(
            client.current_user(),
            client.display_preferences("usersettings", "emby")
        );
        let latest_excludes = user
            .inspect_err(|err| tracing::warn!("couldn't read the user's settings: {err}"))
            .ok()
            .and_then(|user| user.configuration)
            .unwrap_or_default()
            .latest_items_excludes;
        let prefs = prefs
            .inspect_err(|err| tracing::warn!("couldn't read the Home settings: {err}"))
            .unwrap_or_default()
            .custom_prefs;
        Ok((home_sections(&prefs), latest_excludes))
    };
    let (views, resume, next_up, favorites, (sections, latest_excludes)) = tokio::try_join!(
        client.user_views(),
        client.resume(SHELF_LIMIT, FIELDS),
        client.next_up(SHELF_LIMIT, FIELDS),
        client.items(&favorites),
        settings,
    )?;
    let mut latest = Vec::new();
    for view in &views.items {
        if matches!(
            view.collection_type,
            Some(CollectionType::Movies | CollectionType::TvShows)
        ) && !latest_excludes.contains(&view.id)
        {
            let items = client.latest(&view.id, SHELF_LIMIT, FIELDS).await?;
            if !items.is_empty() {
                latest.push((view.clone(), items));
            }
        }
    }
    Ok(HomeData {
        sections,
        views: views.items,
        resume: resume.items,
        next_up: next_up.items,
        favorites: favorites.items,
        latest,
    })
}

/// A press on a shelf: which shelf, and where the mouse and the shelf's scroll were then.
struct ShelfDrag {
    shelf: usize,
    start_x: Pixels,
    start_offset: Pixels,
}

/// One row of cards on the page, in display order.
struct Shelf<'a> {
    id: ElementId,
    title: SharedString,
    items: &'a [BaseItemDto],
    landscape: bool,
}

/// The shelves in the user's section order. Favorites, which the web client has no section
/// for, comes just before the Latest shelves, or last without them.
fn shelves(data: &HomeData) -> Vec<Shelf<'_>> {
    let row = |id: &'static str, title: &'static str, items, landscape| Shelf {
        id: id.into(),
        title: title.into(),
        items,
        landscape,
    };
    let favorites = || row("favorites", "Favorites", &data.favorites, false);
    let mut shelves = Vec::new();
    for section in &data.sections {
        match section {
            HomeSection::MyMedia => shelves.push(row("views", "My Media", &data.views, true)),
            HomeSection::Resume => shelves.push(row("resume", "Continue Watching", &data.resume, true)),
            HomeSection::NextUp => shelves.push(row("next-up", "Next Up", &data.next_up, true)),
            HomeSection::Latest => {
                shelves.push(favorites());
                shelves.extend(data.latest.iter().enumerate().map(|(i, (view, items))| Shelf {
                    id: ("latest", i).into(),
                    title: format!("Latest {}", view.name).into(),
                    items,
                    landscape: false,
                }));
            }
        }
    }
    if !data.sections.contains(&HomeSection::Latest) {
        shelves.push(favorites());
    }
    shelves.retain(|shelf| !shelf.items.is_empty());
    shelves
}

impl HomePage {
    fn cards(&self, section: usize, shelf: &Shelf, cx: &mut App) -> Vec<AnyElement> {
        shelf
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let nav = self.nav.clone();
                let target = item.clone();
                let dragged = self.dragged.clone();
                let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    if !dragged.get() {
                        nav.open(&target, window, cx);
                    }
                };
                let selected = self.cursor.is_selected(section, i);
                if shelf.landscape {
                    landscape_card(i, item, selected, on_click, cx).into_any_element()
                } else {
                    poster_card(i, item, selected, on_click, cx).into_any_element()
                }
            })
            .collect()
    }

    fn render_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let count = match &self.data {
            Load::Loaded(data) => shelves(data).len(),
            _ => 0,
        };
        if self.shelf_scrolls.len() < count {
            self.shelf_scrolls.resize_with(count, ScrollHandle::new);
        }
        let data = match &self.data {
            Load::Loading => return centered_spinner().into_any_element(),
            Load::Failed(message) => {
                let retry = cx.listener(|this, _, _, cx| {
                    this.data = Load::Loading;
                    this.reload(cx);
                });
                return error_state(message.clone(), retry, cx).into_any_element();
            }
            Load::Loaded(data) => data,
        };

        let sections: Vec<AnyElement> = shelves(data)
            .iter()
            .enumerate()
            .map(|(s, shelf)| {
                let cards = self.cards(s, shelf, cx);
                let scroll = self.shelf_scrolls[s].clone();
                let rail = |rail: Stateful<Div>| {
                    rail.on_scroll_wheel(move |event, window, _| undo_wheel_scroll(&scroll, event, window))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, _| {
                                this.dragged.set(false);
                                this.drag = Some(ShelfDrag {
                                    shelf: s,
                                    start_x: event.position.x,
                                    start_offset: this.shelf_scrolls[s].offset().x,
                                });
                            }),
                        )
                };
                let shelf = shelf_with_rail(
                    shelf.id.clone(),
                    shelf.title.clone(),
                    cards,
                    &self.shelf_scrolls[s],
                    rail,
                    cx,
                );
                // One after another, down the page.
                motion::enter(div().child(shelf), ("shelf", s), SHELF_STAGGER * s as u32).into_any_element()
            })
            .collect();
        if sections.is_empty() {
            return centered_message("Your server has no libraries yet.", cx).into_any_element();
        }

        // The shelves are the scroll container's direct children, for `browse::reveal`.
        v_flex()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(smooth_wheel("home", &self.scroll))
            .py_6()
            .gap_8()
            .text_color(cx.theme().foreground)
            .when(self.dragged.get() && self.drag.is_some(), |el| {
                el.cursor(CursorStyle::ClosedHand)
            })
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.drag_shelf(event, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_drag(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_drag(cx)))
            .children(sections)
            .into_any_element()
    }

    /// Scrolls the pressed shelf with the mouse once it has moved far enough to be a drag.
    fn drag_shelf(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_drag(cx);
            return;
        }
        let dx = event.position.x - drag.start_x;
        if !self.dragged.get() && dx.abs() < DRAG_THRESHOLD {
            return;
        }
        self.dragged.set(true);
        let Some(scroll) = self.shelf_scrolls.get(drag.shelf) else {
            return;
        };
        let mut offset = scroll.offset();
        offset.x = (drag.start_offset + dx).clamp(-scroll.max_offset().x, px(0.));
        scroll.set_offset(offset);
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }
}

/// Takes back the sideways scroll GPUI's own wheel handler just gave `scroll`: shelves move
/// only by dragging, so the wheel keeps scrolling the page. GPUI runs the element's own
/// handler first (bubble listeners run last-registered first, and an element's scroll handler
/// is registered after its `on_scroll_wheel` listeners), and it turns a vertical wheel into a
/// sideways scroll on a row that only scrolls sideways.
fn undo_wheel_scroll(scroll: &ScrollHandle, event: &ScrollWheelEvent, window: &Window) {
    let delta = event.delta.pixel_delta(window.line_height());
    let applied = if delta.x.is_zero() { delta.y } else { delta.x };
    let mut offset = scroll.offset();
    offset.x -= applied;
    scroll.set_offset(offset);
}

impl Browse for HomePage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, _window: &Window, _cx: &App) -> Vec<Section> {
        match &self.data {
            Load::Loaded(data) => shelves(data).iter().map(|s| Section::row(s.items.len())).collect(),
            _ => Vec::new(),
        }
    }

    fn reveal(&mut self, position: Position, _window: &mut Window, _cx: &mut Context<Self>) {
        browse::reveal(&self.scroll, position.section, px(24.));
        if let Some(scroll) = self.shelf_scrolls.get(position.section) {
            browse::reveal(scroll, position.index, px(24.));
        }
    }

    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        if let Some(item) = shelves(data)
            .get(position.section)
            .and_then(|shelf| shelf.items.get(position.index))
        {
            self.nav.open(item, window, cx);
        }
    }
}

impl Focusable for HomePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HomePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_content(cx);
        on_browse_actions(div().size_full().track_focus(&self.focus_handle), cx).child(content)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{HomeSection, home_sections};

    #[test]
    fn default_home_sections() {
        let sections = home_sections(&HashMap::new());
        assert_eq!(
            sections,
            [
                HomeSection::MyMedia,
                HomeSection::Resume,
                HomeSection::NextUp,
                HomeSection::Latest
            ]
        );
    }

    #[test]
    fn the_users_order_with_hidden_and_unknown_sections() {
        let prefs: HashMap<String, String> = [
            ("homesection0", "nextup"),
            ("homesection1", "livetv"),
            ("homesection2", "librarybuttons"),
            ("homesection3", "none"),
            ("homesection4", "nextup"),
            ("homesection5", "none"),
            ("homesection6", "none"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        // Slots 7–9 default to none: no Continue Watching and no Latest.
        assert_eq!(home_sections(&prefs), [HomeSection::NextUp, HomeSection::MyMedia]);
    }
}
