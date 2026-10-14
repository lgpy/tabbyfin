//! Search across movies, shows and episodes. Searches as you type, after a short pause.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery};

use crate::components::{CARD_GAP, centered_message, centered_spinner, landscape_card, poster_card};
use crate::images::ImageCache;
use crate::router::Navigator;
use crate::runtime;

const DEBOUNCE: Duration = Duration::from_millis(300);
const LIMIT: u32 = 60;

enum Results {
    Idle,
    Searching,
    Loaded(Vec<BaseItemDto>),
    Failed(SharedString),
}

pub struct SearchPage {
    client: Client,
    nav: Navigator,
    input: Entity<InputState>,
    results: Results,
    /// The pending search. Replacing it cancels the previous one.
    search: Option<Task<()>>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl SearchPage {
    pub fn new(client: Client, nav: Navigator, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search movies, shows and episodes"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_search(cx);
                }
            }),
        ];
        Self {
            client,
            nav,
            input,
            results: Results::Idle,
            search: None,
            scroll: ScrollHandle::new(),
            _subscriptions,
        }
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        let term = self.input.read(cx).value().trim().to_string();
        if term.is_empty() {
            self.search = None;
            self.results = Results::Idle;
            cx.notify();
            return;
        }
        self.results = Results::Searching;
        cx.notify();

        let client = self.client.clone();
        self.search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let query = ItemsQuery {
                search_term: Some(term),
                include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series, BaseItemKind::Episode],
                recursive: true,
                sort_by: vec![ItemSortBy::SortName],
                fields: vec![ItemField::PrimaryImageAspectRatio],
                limit: Some(LIMIT),
                ..Default::default()
            };
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                this.results = match result {
                    Ok(result) => Results::Loaded(result.items),
                    Err(err) => Results::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn render_group(
        &self,
        title: &'static str,
        kind: BaseItemKind,
        items: &[BaseItemDto],
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let cards: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == kind)
            .map(|(i, item)| {
                let nav = self.nav.clone();
                let target = item.clone();
                let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| nav.open(&target, window, cx);
                if kind == BaseItemKind::Episode {
                    landscape_card(("result", i), item, on_click, cx).into_any_element()
                } else {
                    poster_card(("result", i), item, on_click, cx).into_any_element()
                }
            })
            .collect();
        (!cards.is_empty()).then(|| {
            v_flex()
                .gap_3()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(h_flex().flex_wrap().gap(px(CARD_GAP)).children(cards))
        })
    }
}

impl Render for SearchPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let results = match &self.results {
            Results::Idle => centered_message("Type to search your libraries.", cx).into_any_element(),
            Results::Searching => centered_spinner().into_any_element(),
            Results::Failed(message) => centered_message(message.clone(), cx).into_any_element(),
            Results::Loaded(items) if items.is_empty() => centered_message("No results.", cx).into_any_element(),
            Results::Loaded(items) => div()
                .id("search-results")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(
                    v_flex()
                        .px_6()
                        .pb_8()
                        .gap_8()
                        .text_color(cx.theme().foreground)
                        .children(self.render_group("Movies", BaseItemKind::Movie, items, cx))
                        .children(self.render_group("Shows", BaseItemKind::Series, items, cx))
                        .children(self.render_group("Episodes", BaseItemKind::Episode, items, cx)),
                )
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .gap_4()
            .child(div().px_6().pt_6().max_w(px(640.)).child(Input::new(&self.input)))
            .child(div().flex_1().min_h_0().child(results))
    }
}
