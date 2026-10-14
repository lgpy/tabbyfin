//! A library (or folder) as a poster grid. Rows are virtualized and pages are fetched as they
//! scroll into view.

use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::component::h_flex;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery, SortOrder};

use crate::components::{
    CARD_GAP, CARD_TEXT_H, POSTER_H, POSTER_W, centered_message, centered_spinner, error_state,
    placeholder_card, poster_card,
};
use crate::changes::{Changes, UserDataChanged};
use crate::images::ImageCache;
use crate::router::Navigator;
use crate::runtime;

const PAGE_SIZE: usize = 100;
const GRID_PADDING: f32 = 24.;
const ROW_H: f32 = POSTER_H + CARD_TEXT_H + CARD_GAP;

pub struct LibraryPage {
    client: Client,
    nav: Navigator,
    base_query: ItemsQuery,
    /// `None` until the first page arrives.
    total: Option<usize>,
    items: Vec<Option<BaseItemDto>>,
    pages_requested: HashSet<usize>,
    error: Option<SharedString>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl LibraryPage {
    pub fn new(
        client: Client,
        nav: Navigator,
        parent_id: String,
        collection_type: Option<CollectionType>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Movie and show libraries list their top-level items across all folders; anything
        // else is browsed folder by folder.
        let (include_item_types, recursive) = match collection_type {
            Some(CollectionType::Movies) => (vec![BaseItemKind::Movie], true),
            Some(CollectionType::TvShows) => (vec![BaseItemKind::Series], true),
            _ => (Vec::new(), false),
        };
        let base_query = ItemsQuery {
            parent_id: Some(parent_id),
            include_item_types,
            recursive,
            sort_by: vec![ItemSortBy::SortName],
            sort_order: Some(SortOrder::Ascending),
            fields: vec![ItemField::PrimaryImageAspectRatio],
            limit: Some(PAGE_SIZE as u32),
            ..Default::default()
        };
        let mut this = Self {
            client,
            nav,
            base_query,
            total: None,
            items: Vec::new(),
            pages_requested: HashSet::new(),
            error: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: vec![
                cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
                cx.subscribe(&Changes::global(cx), |this, _, _: &UserDataChanged, cx| this.refresh(cx)),
            ],
        };
        this.request_page(0, cx);
        this
    }

    /// Re-fetches the pages already loaded (e.g. for new watched badges), keeping the current
    /// items on screen until the new ones arrive.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let pages: Vec<usize> = self.pages_requested.drain().collect();
        for page in pages {
            self.request_page(page, cx);
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.pages_requested.clear();
        self.request_page(0, cx);
        cx.notify();
    }

    fn request_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if !self.pages_requested.insert(page) {
            return;
        }
        let client = self.client.clone();
        let query = ItemsQuery {
            start_index: Some((page * PAGE_SIZE) as u32),
            ..self.base_query.clone()
        };
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(result) => {
                        let total = result.total_record_count as usize;
                        this.total = Some(total);
                        this.items.resize(total, None);
                        let start = page * PAGE_SIZE;
                        for (i, item) in result.items.into_iter().enumerate() {
                            if let Some(slot) = this.items.get_mut(start + i) {
                                *slot = Some(item);
                            }
                        }
                    }
                    Err(err) => {
                        // Allow a retry when the page scrolls into view again.
                        this.pages_requested.remove(&page);
                        this.error = Some(this.nav.handle_error(&err, cx));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_rows(&mut self, rows: Range<usize>, columns: usize, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let total = self.total.unwrap_or(0);
        rows.map(|row| {
            let indices = row * columns..((row + 1) * columns).min(total);
            for index in indices.clone() {
                if self.items[index].is_none() {
                    self.request_page(index / PAGE_SIZE, cx);
                }
            }
            let cards: Vec<AnyElement> = indices
                .map(|index| match &self.items[index] {
                    Some(item) => {
                        let nav = self.nav.clone();
                        let target = item.clone();
                        poster_card(("item", index), item, move |_, window, cx| nav.open(&target, window, cx), cx)
                            .into_any_element()
                    }
                    None => placeholder_card(cx).into_any_element(),
                })
                .collect();
            h_flex()
                .id(("row", row))
                .h(px(ROW_H))
                .px(px(GRID_PADDING))
                .gap(px(CARD_GAP))
                .items_start()
                .children(cards)
        })
        .collect()
    }
}

impl Render for LibraryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(total) = self.total else {
            return match &self.error {
                Some(error) => {
                    error_state(error.clone(), cx.listener(|this, _, _, cx| this.retry(cx)), cx).into_any_element()
                }
                None => centered_spinner().into_any_element(),
            };
        };
        if total == 0 {
            return centered_message("Nothing here yet.", cx).into_any_element();
        }

        let available = window.viewport_size().width - px(GRID_PADDING * 2.);
        let columns = ((available + px(CARD_GAP)) / px(POSTER_W + CARD_GAP)).floor().max(1.) as usize;
        let rows = total.div_ceil(columns);

        uniform_list(
            "library-grid",
            rows,
            cx.processor(move |this, rows: Range<usize>, _window, cx| this.render_rows(rows, columns, cx)),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .pt(px(GRID_PADDING))
        .into_any_element()
    }
}
