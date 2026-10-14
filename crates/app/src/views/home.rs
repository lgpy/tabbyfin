//! Home: libraries, Continue Watching, Next Up, and Latest per library.

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;
use jellyfin::Client;
use jellyfin::models::{BaseItemDto, CollectionType, ItemField};

use crate::browse::{self, Browse, Cursor, Position, Section, on_browse_actions};
use crate::changes::{Changes, UserDataChanged};
use crate::components::{Load, centered_message, centered_spinner, error_state, landscape_card, poster_card};
use crate::images::ImageCache;
use crate::router::Navigator;
use crate::runtime;

const SHELF_LIMIT: u32 = 24;
const FIELDS: &[ItemField] = &[ItemField::PrimaryImageAspectRatio];

struct HomeData {
    views: Vec<BaseItemDto>,
    resume: Vec<BaseItemDto>,
    next_up: Vec<BaseItemDto>,
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
    let (views, resume, next_up) = tokio::try_join!(
        client.user_views(),
        client.resume(SHELF_LIMIT, FIELDS),
        client.next_up(SHELF_LIMIT, FIELDS),
    )?;
    let mut latest = Vec::new();
    for view in &views.items {
        if matches!(
            view.collection_type,
            Some(CollectionType::Movies | CollectionType::TvShows)
        ) {
            let items = client.latest(&view.id, SHELF_LIMIT, FIELDS).await?;
            if !items.is_empty() {
                latest.push((view.clone(), items));
            }
        }
    }
    Ok(HomeData {
        views: views.items,
        resume: resume.items,
        next_up: next_up.items,
        latest,
    })
}

/// One row of cards on the page, in display order.
struct Shelf<'a> {
    id: ElementId,
    title: SharedString,
    items: &'a [BaseItemDto],
    landscape: bool,
}

fn shelves(data: &HomeData) -> Vec<Shelf<'_>> {
    let fixed = [
        ("views", "My Media", &data.views),
        ("resume", "Continue Watching", &data.resume),
        ("next-up", "Next Up", &data.next_up),
    ];
    let fixed = fixed.into_iter().map(|(id, title, items)| Shelf {
        id: id.into(),
        title: title.into(),
        items,
        landscape: true,
    });
    let latest = data.latest.iter().enumerate().map(|(i, (view, items))| Shelf {
        id: ("latest", i).into(),
        title: format!("Latest {}", view.name).into(),
        items,
        landscape: false,
    });
    fixed.chain(latest).filter(|shelf| !shelf.items.is_empty()).collect()
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
                let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| nav.open(&target, window, cx);
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
                crate::components::shelf(shelf.id.clone(), shelf.title.clone(), cards, &self.shelf_scrolls[s], cx)
                    .into_any_element()
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
            .py_6()
            .gap_8()
            .text_color(cx.theme().foreground)
            .children(sections)
            .into_any_element()
    }
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
