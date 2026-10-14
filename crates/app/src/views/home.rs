//! Home: libraries, Continue Watching, Next Up, and Latest per library.

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;
use jellyfin::Client;
use jellyfin::models::{BaseItemDto, CollectionType, ItemField};

use crate::changes::{Changes, UserDataChanged};
use crate::components::{Load, centered_message, centered_spinner, error_state, landscape_card, poster_card, shelf};
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
        if matches!(view.collection_type, Some(CollectionType::Movies | CollectionType::TvShows)) {
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

impl HomePage {
    fn cards(&self, prefix: &'static str, items: &[BaseItemDto], landscape: bool, cx: &mut App) -> Vec<AnyElement> {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let nav = self.nav.clone();
                let target = item.clone();
                let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| nav.open(&target, window, cx);
                let id = (SharedString::from(prefix), i);
                if landscape {
                    landscape_card(id, item, on_click, cx).into_any_element()
                } else {
                    poster_card(id, item, on_click, cx).into_any_element()
                }
            })
            .collect()
    }
}

impl Render for HomePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        let mut sections: Vec<AnyElement> = Vec::new();
        if !data.views.is_empty() {
            let cards = self.cards("view", &data.views, true, cx);
            sections.push(shelf("views", "My Media", cards, cx).into_any_element());
        }
        if !data.resume.is_empty() {
            let cards = self.cards("resume", &data.resume, true, cx);
            sections.push(shelf("resume", "Continue Watching", cards, cx).into_any_element());
        }
        if !data.next_up.is_empty() {
            let cards = self.cards("next-up", &data.next_up, true, cx);
            sections.push(shelf("next-up", "Next Up", cards, cx).into_any_element());
        }
        for (i, (view, items)) in data.latest.iter().enumerate() {
            let cards = self.cards("latest", items, false, cx);
            sections.push(shelf(("latest", i), format!("Latest {}", view.name), cards, cx).into_any_element());
        }
        if sections.is_empty() {
            return centered_message("Your server has no libraries yet.", cx).into_any_element();
        }

        div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(v_flex().py_6().gap_8().text_color(cx.theme().foreground).children(sections))
            .into_any_element()
    }
}
