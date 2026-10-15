//! A library (or folder, or a person's items) as a poster grid, with sort and filter. Rows are
//! virtualized and pages are fetched as they scroll into view.

use std::collections::HashSet;
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, IndexPath, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType, ItemField, ItemFilter, ItemSortBy};
use jellyfin::{Client, ItemsQuery, SortOrder};

use crate::browse::{Browse, Cursor, Position, Section, on_browse_actions};
use crate::changes::{Changes, UserDataChanged};
use crate::components::{
    CARD_GAP, CARD_TEXT_H, POSTER_H, POSTER_W, centered_message, centered_spinner, error_state, placeholder_card,
    poster_card, selection_ring,
};
use crate::config::{Config, LibraryView};
use crate::images::ImageCache;
use crate::item_menu::with_item_menu;
use crate::router::Navigator;
use crate::runtime;
use crate::smooth_scroll::smooth_wheel;
use crate::tracks::{Choice, ChoiceSelect, cycle_choice};
use crate::zoom::{dp, dp_px};

const PAGE_SIZE: usize = 100;
const GRID_PADDING: f32 = 24.;
const ROW_H: f32 = POSTER_H + CARD_TEXT_H + CARD_GAP;
/// The jump bar's letters; `#` is the top (names starting with digits or symbols).
const LETTERS: &str = "#ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LETTER_BAR_W: f32 = 28.;

/// Cursor sections: the toolbar's controls, then the grid.
const TOOLBAR: usize = 0;
const GRID: usize = 1;
const GRID_START: Position = Position {
    section: GRID,
    index: 0,
};
/// Space around a toolbar control for the cursor's ring.
const RING_GAP: f32 = 3.;

/// The toolbar's controls, left to right, as the keyboard cursor sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Sort,
    Order,
    Show,
    /// Opens the genres, tags and ratings dialog.
    Values,
    Year,
    Resolution,
    Clear,
}

/// The library's genres, tags and official ratings, which the dialog lists to choose from.
#[derive(Clone, Default)]
pub struct FilterValues {
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub ratings: Vec<String>,
}

impl FilterValues {
    fn is_empty(&self) -> bool {
        self.genres.is_empty() && self.tags.is_empty() && self.ratings.is_empty()
    }
}

/// The kinds of values in the genres, tags and ratings dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Genre,
    Tag,
    Rating,
}

/// What the grid lists.
pub enum Source {
    /// A library or folder. Movie and show libraries list their top-level items across all
    /// folders; anything else is browsed folder by folder.
    Folder {
        id: String,
        collection_type: Option<CollectionType>,
    },
    /// Movies and shows a person appears in.
    Person { id: String },
}

/// Sort choices: config key, label, server sort, and whether it starts descending.
const SORTS: [(&str, &str, ItemSortBy, bool); 6] = [
    ("name", "Name", ItemSortBy::SortName, false),
    ("added", "Date added", ItemSortBy::DateCreated, true),
    ("released", "Release date", ItemSortBy::PremiereDate, true),
    ("rating", "Rating", ItemSortBy::CommunityRating, true),
    ("runtime", "Runtime", ItemSortBy::Runtime, false),
    ("random", "Random", ItemSortBy::Random, false),
];

/// Filter choices: config key, label, server filter.
const FILTERS: [(&str, &str, Option<ItemFilter>); 5] = [
    ("all", "All", None),
    ("unwatched", "Unwatched", Some(ItemFilter::IsUnplayed)),
    ("watched", "Watched", Some(ItemFilter::IsPlayed)),
    ("favorites", "Favorites", Some(ItemFilter::IsFavorite)),
    ("in-progress", "In progress", Some(ItemFilter::IsResumable)),
];

/// Resolution choices: config key, label, and video width bounds (4K from 3800 wide, HD from
/// 1200, which 720p's 1280 clears). 12.2 goes by an item's widest version, 10.11 by its first.
const RESOLUTIONS: [(&str, &str, Option<u32>, Option<u32>); 4] = [
    ("any", "Any resolution", None, None),
    ("4k", "4K", Some(3800), None),
    ("hd", "HD", Some(1200), Some(3799)),
    ("sd", "SD", None, Some(1199)),
];

/// Where the genre, year and resolution filters apply: movie and show libraries, whose items
/// the server can list those values for.
struct FilterScope {
    parent_id: String,
    item_types: Vec<BaseItemKind>,
    /// Shows have no resolution of their own, so only movie libraries filter by it.
    resolution: bool,
}

pub struct LibraryPage {
    client: Client,
    nav: Navigator,
    /// Key for the remembered sort and filter.
    view_key: String,
    view: LibraryView,
    base_query: ItemsQuery,
    /// `None` until the first page arrives.
    total: Option<usize>,
    items: Vec<Option<BaseItemDto>>,
    pages_requested: HashSet<usize>,
    /// Bumped when the sort or filter changes, so pages fetched before are dropped.
    generation: u64,
    error: Option<SharedString>,
    scroll: UniformListScrollHandle,
    cursor: Cursor,
    focus_handle: FocusHandle,
    sort: ChoiceSelect,
    filter: ChoiceSelect,
    filter_scope: Option<FilterScope>,
    /// The library's genres, tags and ratings, for the dialog.
    values: FilterValues,
    /// The library's year keys, offered after the "any" choice at index 0.
    years: Vec<String>,
    year: ChoiceSelect,
    resolution: ChoiceSelect,
    _subscriptions: Vec<Subscription>,
}

impl LibraryPage {
    pub fn new(client: Client, nav: Navigator, source: Source, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (view_key, base_query, filter_scope) = match source {
            Source::Folder { id, collection_type } => {
                let (include_item_types, recursive) = match collection_type {
                    Some(CollectionType::Movies) => (vec![BaseItemKind::Movie], true),
                    Some(CollectionType::TvShows) => (vec![BaseItemKind::Series], true),
                    _ => (Vec::new(), false),
                };
                let scope = recursive.then(|| FilterScope {
                    parent_id: id.clone(),
                    item_types: include_item_types.clone(),
                    resolution: collection_type == Some(CollectionType::Movies),
                });
                let query = ItemsQuery {
                    parent_id: Some(id.clone()),
                    include_item_types,
                    recursive,
                    ..Default::default()
                };
                (id, query, scope)
            }
            Source::Person { id } => {
                let query = ItemsQuery {
                    person_ids: vec![id],
                    include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series],
                    recursive: true,
                    ..Default::default()
                };
                ("person".to_owned(), query, None)
            }
        };
        let base_query = ItemsQuery {
            fields: vec![ItemField::PrimaryImageAspectRatio],
            limit: Some(PAGE_SIZE as u32),
            ..base_query
        };
        let mut view = Config::load()
            .library_views
            .get(&view_key)
            .cloned()
            .unwrap_or_else(|| LibraryView {
                sort: SORTS[0].0.into(),
                descending: SORTS[0].3,
                filter: FILTERS[0].0.into(),
                ..Default::default()
            });
        view.upgrade();
        let select = |choices: Vec<(&str, &str)>, current: &str, window: &mut Window, cx: &mut Context<Self>| {
            let selected = choices.iter().position(|(key, _)| *key == current).unwrap_or(0);
            choice_select(choices.iter().map(|(_, label)| *label), selected, window, cx)
        };
        let sort = select(SORTS.iter().map(|s| (s.0, s.1)).collect(), &view.sort, window, cx);
        let filter = select(FILTERS.iter().map(|f| (f.0, f.1)).collect(), &view.filter, window, cx);
        let resolution = select(
            RESOLUTIONS.iter().map(|r| (r.0, r.1)).collect(),
            view.resolution.as_deref().unwrap_or_default(),
            window,
            cx,
        );
        // Until the library's own values arrive, the remembered ones are the only choices.
        let values = FilterValues {
            genres: view.genres.clone(),
            tags: view.tags.clone(),
            ratings: view.ratings.clone(),
        };
        let years: Vec<String> = view.year.iter().cloned().collect();
        let year = any_select("Any year", &years, view.year.as_ref(), window, cx);
        let mut this = Self {
            client,
            nav,
            view_key,
            view,
            base_query,
            total: None,
            items: Vec::new(),
            pages_requested: HashSet::new(),
            generation: 0,
            error: None,
            scroll: UniformListScrollHandle::new(),
            cursor: Cursor::starting_at(GRID_START),
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![
                cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
                cx.subscribe(&Changes::global(cx), |this, _, _: &UserDataChanged, cx| {
                    this.refresh(cx)
                }),
                cx.subscribe(&sort, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(i)) = event
                        && let Some(sort) = SORTS.get(*i as usize)
                        && this.view.sort != sort.0
                    {
                        this.view.sort = sort.0.into();
                        this.view.descending = sort.3;
                        this.change_view(cx);
                    }
                }),
                cx.subscribe(&filter, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(i)) = event
                        && let Some(filter) = FILTERS.get(*i as usize)
                        && this.view.filter != filter.0
                    {
                        this.view.filter = filter.0.into();
                        this.change_view(cx);
                    }
                }),
                cx.subscribe(&year, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(i)) = event {
                        let year = chosen(&this.years, *i);
                        if this.view.year != year {
                            this.view.year = year;
                            this.change_view(cx);
                        }
                    }
                }),
                cx.subscribe(&resolution, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(i)) = event {
                        let resolution = (*i > 0).then(|| RESOLUTIONS.get(*i as usize)).flatten();
                        let resolution = resolution.map(|r| r.0.to_owned());
                        if this.view.resolution != resolution {
                            this.view.resolution = resolution;
                            this.change_view(cx);
                        }
                    }
                }),
            ],
            sort,
            filter,
            filter_scope,
            values,
            years,
            year,
            resolution,
        };
        this.apply_view();
        this.request_page(0, cx);
        this.load_filter_values(window, cx);
        this
    }

    /// Fills the genres, tags and ratings and the year menu with the values found in the
    /// library. A remembered choice the library no longer has stays listed, so what's applied
    /// is still shown.
    fn load_filter_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = &self.filter_scope else {
            return;
        };
        let (client, parent_id, item_types) = (self.client.clone(), scope.parent_id.clone(), scope.item_types.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime::spawn(async move { client.item_filter_values(&parent_id, &item_types).await }).await;
            let values = match result {
                Ok(values) => values,
                Err(err) => return tracing::warn!("couldn't load the library's genres and years: {err}"),
            };
            this.update_in(cx, |this, window, cx| {
                let with_chosen = |mut found: Vec<String>, chosen: &[String]| {
                    let missing: Vec<String> = chosen.iter().filter(|c| !found.contains(c)).cloned().collect();
                    if !missing.is_empty() {
                        found.extend(missing);
                        found.sort_by_key(|value| value.to_lowercase());
                    }
                    found
                };
                this.values = FilterValues {
                    genres: with_chosen(values.genres, &this.view.genres),
                    tags: with_chosen(values.tags, &this.view.tags),
                    ratings: with_chosen(values.official_ratings, &this.view.ratings),
                };
                let mut years = year_choices(&values.years);
                if let Some(year) = &this.view.year
                    && !years.contains(year)
                {
                    years.push(year.clone());
                }
                let labels = |any: &'static str, values: &[String]| {
                    choices(std::iter::once(any.into()).chain(values.iter().map(|v| v.clone().into())))
                };
                let selected = |values: &[String], current: Option<&String>| {
                    let index = current.and_then(|c| values.iter().position(|v| v == c));
                    Some(IndexPath::default().row(index.map_or(0, |i| i + 1)))
                };
                let (year_choices, year_index) =
                    (labels("Any year", &years), selected(&years, this.view.year.as_ref()));
                this.year.update(cx, |select, cx| {
                    select.set_items(year_choices, window, cx);
                    select.set_selected_index(year_index, window, cx);
                });
                this.years = years;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Puts the sort and filter into the query.
    fn apply_view(&mut self) {
        let sort = SORTS.iter().find(|s| s.0 == self.view.sort).unwrap_or(&SORTS[0]);
        // Ties (same date, rating…) fall back to the name.
        self.base_query.sort_by = vec![sort.2, ItemSortBy::SortName];
        self.base_query.sort_order = Some(if self.view.descending {
            SortOrder::Descending
        } else {
            SortOrder::Ascending
        });
        let filter = FILTERS.iter().find(|f| f.0 == self.view.filter).unwrap_or(&FILTERS[0]);
        self.base_query.filters = filter.2.into_iter().collect();
        let Some(scope) = &self.filter_scope else {
            return;
        };
        self.base_query.genres = self.view.genres.clone();
        self.base_query.tags = self.view.tags.clone();
        self.base_query.official_ratings = self.view.ratings.clone();
        self.base_query.years = self.view.year.as_deref().map(years_of).unwrap_or_default();
        let resolution = RESOLUTIONS
            .iter()
            .find(|r| scope.resolution && self.view.resolution.as_deref() == Some(r.0))
            .unwrap_or(&RESOLUTIONS[0]);
        (self.base_query.min_width, self.base_query.max_width) = (resolution.2, resolution.3);
    }

    /// The library's genres, tags and ratings, for the dialog.
    pub fn filter_values(&self) -> &FilterValues {
        &self.values
    }

    /// The chosen values of a kind.
    pub fn chosen_values(&self, kind: ValueKind) -> &[String] {
        match kind {
            ValueKind::Genre => &self.view.genres,
            ValueKind::Tag => &self.view.tags,
            ValueKind::Rating => &self.view.ratings,
        }
    }

    /// Ticks or unticks a genre, tag or rating, and reloads the grid.
    pub fn toggle_value(&mut self, kind: ValueKind, value: &str, cx: &mut Context<Self>) {
        let chosen = match kind {
            ValueKind::Genre => &mut self.view.genres,
            ValueKind::Tag => &mut self.view.tags,
            ValueKind::Rating => &mut self.view.ratings,
        };
        match chosen.iter().position(|v| v == value) {
            Some(i) => {
                chosen.remove(i);
            }
            None => chosen.push(value.to_owned()),
        }
        self.change_view(cx);
    }

    fn open_value_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::views::value_filters::open(cx.entity(), window, cx);
    }

    /// The button's label: what's chosen, or what it's for.
    fn values_summary(&self) -> SharedString {
        let chosen: Vec<&String> = self
            .view
            .genres
            .iter()
            .chain(&self.view.tags)
            .chain(&self.view.ratings)
            .collect();
        match chosen.as_slice() {
            [] => "Genres, tags…".into(),
            [one] => (*one).clone().into(),
            [one, two] => format!("{one}, {two}").into(),
            many => format!("{} filters", many.len()).into(),
        }
    }

    /// Whether anything is filtered out (the sort aside).
    fn filtered(&self) -> bool {
        self.view.filter != FILTERS[0].0
            || !self.view.genres.is_empty()
            || !self.view.tags.is_empty()
            || !self.view.ratings.is_empty()
            || self.view.year.is_some()
            || self.view.resolution.is_some()
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view.filter = FILTERS[0].0.into();
        (self.view.year, self.view.resolution) = (None, None);
        self.view.genres.clear();
        self.view.tags.clear();
        self.view.ratings.clear();
        for select in [&self.filter, &self.year, &self.resolution] {
            select.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::default()), window, cx)
            });
        }
        self.change_view(cx);
        // The Clear button is gone now; from the keyboard, land on the control before it.
        if self
            .cursor
            .selected()
            .is_some_and(|position| position.section == TOOLBAR)
        {
            let last = self.tools().len() - 1;
            self.cursor.show_at(Position {
                section: TOOLBAR,
                index: last,
            });
        }
    }

    /// The sort or filter changed: remember it and load the grid again from the top.
    fn change_view(&mut self, cx: &mut Context<Self>) {
        let mut config = Config::load();
        config.library_views.insert(self.view_key.clone(), self.view.clone());
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
        self.apply_view();
        self.generation += 1;
        self.total = None;
        self.items.clear();
        self.pages_requested.clear();
        self.error = None;
        // Changed from the toolbar, the cursor stays there; otherwise it starts on the grid.
        if self
            .cursor
            .selected()
            .is_none_or(|position| position.section != TOOLBAR)
        {
            self.cursor = Cursor::starting_at(GRID_START);
        }
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.request_page(0, cx);
        cx.notify();
    }

    /// The letter bar shows while sorted by name, where names starting with a letter are
    /// together.
    fn letters_shown(&self) -> bool {
        self.view.sort == SORTS[0].0 && self.total.is_some_and(|total| total > 0)
    }

    /// Cards per row at the window's width, next to the letter bar when it shows.
    fn columns(&self, window: &Window) -> usize {
        let bar = if self.letters_shown() { LETTER_BAR_W } else { 0. };
        let available = window.viewport_size().width - dp_px(GRID_PADDING * 2. + bar, window);
        ((available + dp_px(CARD_GAP, window)) / dp_px(POSTER_W + CARD_GAP, window))
            .floor()
            .max(1.) as usize
    }

    /// Scrolls to the first name starting with `letter`. Its index is the number of names
    /// before it (after it, descending), which the server counts with the grid's filters.
    /// From the keyboard, the highlight moves there too.
    fn jump_to(&mut self, letter: char, highlight: bool, window: &mut Window, cx: &mut Context<Self>) {
        let columns = self.columns(window);
        let (descending, total) = (self.view.descending, self.total.unwrap_or(0));
        let bound = match jump_target(letter, descending) {
            Jump::Top => return self.land(0, highlight, columns, cx),
            Jump::Bottom => return self.land(total.saturating_sub(1), highlight, columns, cx),
            Jump::After(bound) => bound,
        };
        let query = ItemsQuery {
            name_less_than: Some(bound.to_string()),
            start_index: None,
            limit: Some(0),
            ..self.base_query.clone()
        };
        let (client, generation) = (self.client.clone(), self.generation);
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(result) => {
                        let before = result.total_record_count as usize;
                        let index = if descending {
                            total.saturating_sub(before)
                        } else {
                            before
                        };
                        this.land(index.min(total.saturating_sub(1)), highlight, columns, cx);
                    }
                    Err(err) => tracing::warn!("couldn't find where {letter} starts: {err}"),
                }
            })
            .ok();
        })
        .detach();
    }

    fn land(&mut self, index: usize, highlight: bool, columns: usize, cx: &mut Context<Self>) {
        self.scroll.scroll_to_item(index / columns, ScrollStrategy::Top);
        if highlight {
            self.cursor.show_at(Position { section: GRID, index });
        }
        cx.notify();
    }

    /// A letter typed on the grid jumps like the letter bar, while sorted by name.
    fn type_letter(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mut chars = keystroke.key.chars();
        if let (Some(letter), None) = (chars.next(), chars.next())
            && letter.is_ascii_alphabetic()
            && !keystroke.modifiers.modified()
            && self.letters_shown()
        {
            self.jump_to(letter.to_ascii_uppercase(), true, window, cx);
            cx.stop_propagation();
        }
    }

    fn render_letters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_none()
            .w(dp(LETTER_BAR_W))
            .h_full()
            .py_2()
            .justify_between()
            .items_center()
            // A fixed size, so all 27 fit the window at any text size (typing a letter does
            // the same).
            .text_size(px(11.))
            .text_color(cx.theme().muted_foreground)
            .children(LETTERS.chars().map(|letter| {
                div()
                    .id(("letter", letter as usize))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent).text_color(cx.theme().accent_foreground))
                    .on_click(cx.listener(move |this, _, window, cx| this.jump_to(letter, false, window, cx)))
                    .child(letter.to_string())
            }))
    }

    fn toggle_order(&mut self, cx: &mut Context<Self>) {
        self.view.descending = !self.view.descending;
        self.change_view(cx);
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
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
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
                        let card = poster_card(
                            ("item", index),
                            item,
                            self.cursor.is_selected(GRID, index),
                            move |_, window, cx| nav.open(&target, window, cx),
                            cx,
                        );
                        with_item_menu(card, item, &self.client, &self.nav)
                    }
                    None => placeholder_card(cx).into_any_element(),
                })
                .collect();
            h_flex()
                .id(("row", row))
                .h(dp(ROW_H))
                .px(dp(GRID_PADDING))
                .gap(dp(CARD_GAP))
                .items_start()
                .children(cards)
        })
        .collect()
    }
}

fn choices(labels: impl IntoIterator<Item = SharedString>) -> Vec<Choice> {
    labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| Choice { label, value: i as i32 })
        .collect()
}

fn choice_select(
    labels: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: usize,
    window: &mut Window,
    cx: &mut App,
) -> ChoiceSelect {
    let choices = choices(labels.into_iter().map(Into::into));
    cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx))
}

/// A menu of "any" followed by `values`, with `current` selected.
fn any_select(
    any: &'static str,
    values: &[String],
    current: Option<&String>,
    window: &mut Window,
    cx: &mut App,
) -> ChoiceSelect {
    let selected = current
        .and_then(|c| values.iter().position(|v| v == c))
        .map_or(0, |i| i + 1);
    let labels = std::iter::once(SharedString::from(any)).chain(values.iter().map(|v| v.clone().into()));
    choice_select(labels, selected, window, cx)
}

/// The value picked from an "any, then `values`" menu; `None` for "any".
fn chosen(values: &[String], index: i32) -> Option<String> {
    usize::try_from(index - 1).ok().and_then(|i| values.get(i)).cloned()
}

/// Year menu keys, newest first: each year, preceded by its decade ("2020s") when the library
/// has more than one year from it.
fn year_choices(years: &[i32]) -> Vec<String> {
    let mut years = years.to_vec();
    years.sort_unstable_by(|a, b| b.cmp(a));
    years.dedup();
    let decade = |year: i32| year - year.rem_euclid(10);
    let mut keys = Vec::new();
    for (i, &year) in years.iter().enumerate() {
        let starts_decade = i == 0 || decade(years[i - 1]) != decade(year);
        if starts_decade && years.iter().filter(|&&y| decade(y) == decade(year)).count() > 1 {
            keys.push(format!("{}s", decade(year)));
        }
        keys.push(year.to_string());
    }
    keys
}

/// The years a year menu key covers.
fn years_of(key: &str) -> Vec<i32> {
    match key.strip_suffix('s').and_then(|decade| decade.parse::<i32>().ok()) {
        Some(decade) => (decade..decade + 10).collect(),
        None => key.parse().ok().into_iter().collect(),
    }
}

/// Where a letter of the jump bar scrolls to.
#[derive(Debug, PartialEq)]
enum Jump {
    Top,
    Bottom,
    /// Past the names that sort before this letter (counted by the server).
    After(char),
}

/// `#` is the first row ascending and the last descending. A letter starts where the names
/// before it end; descending, that's after every name from the next letter on.
fn jump_target(letter: char, descending: bool) -> Jump {
    match (letter, descending) {
        ('#', false) | ('Z', true) => Jump::Top,
        ('#', true) => Jump::Bottom,
        (letter, false) => Jump::After(letter),
        (letter, true) => Jump::After(char::from(letter as u8 + 1)),
    }
}

impl Browse for LibraryPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, window: &Window, _cx: &App) -> Vec<Section> {
        vec![
            Section::row(self.tools().len()),
            Section::grid(self.total.unwrap_or(0), self.columns(window)),
        ]
    }

    /// The toolbar stays put above the grid; only the grid scrolls.
    fn reveal(&mut self, position: Position, window: &mut Window, _cx: &mut Context<Self>) {
        if position.section == GRID {
            self.scroll
                .scroll_to_item(position.index / self.columns(window), ScrollStrategy::Nearest);
        }
    }

    /// Enter on a toolbar picker moves it to the next choice (Left and Right move along the
    /// toolbar); on a button it presses it; on a card it opens it.
    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        if position.section == GRID {
            if let Some(Some(item)) = self.items.get(position.index) {
                self.nav.open(item, window, cx);
            }
            return;
        }
        let Some(tool) = self.tools().get(position.index).copied() else {
            return;
        };
        match tool {
            Tool::Order => self.toggle_order(cx),
            Tool::Values => self.open_value_filters(window, cx),
            Tool::Clear => self.clear_filters(window, cx),
            tool => {
                if let Some(select) = self.tool_select(tool) {
                    cycle_choice(&select.clone(), window, cx);
                }
            }
        }
    }
}

impl LibraryPage {
    /// The toolbar's controls shown now, left to right.
    fn tools(&self) -> Vec<Tool> {
        let mut tools = vec![Tool::Sort, Tool::Order, Tool::Show];
        if let Some(scope) = &self.filter_scope {
            if !self.values.is_empty() {
                tools.push(Tool::Values);
            }
            if !self.years.is_empty() {
                tools.push(Tool::Year);
            }
            if scope.resolution {
                tools.push(Tool::Resolution);
            }
        }
        if self.filtered() {
            tools.push(Tool::Clear);
        }
        tools
    }

    fn tool_select(&self, tool: Tool) -> Option<&ChoiceSelect> {
        match tool {
            Tool::Sort => Some(&self.sort),
            Tool::Show => Some(&self.filter),
            Tool::Year => Some(&self.year),
            Tool::Resolution => Some(&self.resolution),
            Tool::Order | Tool::Values | Tool::Clear => None,
        }
    }

    fn render_tool(&self, tool: Tool, cx: &mut Context<Self>) -> AnyElement {
        let random = self.view.sort == "random";
        match tool {
            Tool::Order => Button::new("sort-order")
                .ghost()
                .small()
                .icon(if self.view.descending {
                    Lucide::ArrowDownWideNarrow
                } else {
                    Lucide::ArrowUpNarrowWide
                })
                .tooltip(if random {
                    "Shuffle again"
                } else if self.view.descending {
                    "Descending"
                } else {
                    "Ascending"
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_order(cx)))
                .into_any_element(),
            Tool::Values => Button::new("value-filters")
                .outline()
                .small()
                .icon(Lucide::ListFilter)
                .label(self.values_summary())
                .on_click(cx.listener(|this, _, window, cx| this.open_value_filters(window, cx)))
                .into_any_element(),
            Tool::Clear => Button::new("clear-filters")
                .ghost()
                .small()
                .label("Clear filters")
                .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx)))
                .into_any_element(),
            tool => {
                let width = match tool {
                    Tool::Sort => 160.,
                    Tool::Show => 140.,
                    Tool::Year => 120.,
                    _ => 150.,
                };
                let select = self.tool_select(tool).expect("a picker");
                // In rems, so the menus grow with the text size.
                div()
                    .w(rems(width / 16.))
                    .child(Select::new(select).small())
                    .into_any_element()
            }
        }
    }
}

impl Focusable for LibraryPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LibraryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_content(window, cx);
        let tools: Vec<(Tool, AnyElement)> = self
            .tools()
            .into_iter()
            .map(|tool| (tool, self.render_tool(tool, cx)))
            .collect();
        let muted = cx.theme().muted_foreground;
        let mut toolbar = h_flex()
            .flex_none()
            .px(dp(GRID_PADDING - RING_GAP))
            .pt_4()
            .gap(dp(8. - RING_GAP * 2.))
            .flex_wrap()
            .text_sm();
        for (i, (tool, element)) in tools.into_iter().enumerate() {
            let label = match tool {
                Tool::Sort => Some("Sort"),
                Tool::Show => Some("Show"),
                _ => None,
            };
            if tool == Tool::Show {
                toolbar = toolbar.child(div().w_4());
            }
            toolbar = toolbar
                .children(label.map(|label| div().px(dp(RING_GAP)).text_color(muted).child(label)))
                .child(
                    div()
                        .relative()
                        .p(dp(RING_GAP))
                        .rounded_xl()
                        .child(element)
                        .when(self.cursor.is_selected(TOOLBAR, i), |el| {
                            el.child(selection_ring(cx).rounded_xl())
                        }),
                );
        }
        let toolbar = toolbar.child(div().flex_1()).children(
            self.total
                .map(|total| div().text_color(cx.theme().muted_foreground).child(item_count(total))),
        );
        on_browse_actions(v_flex().size_full().track_focus(&self.focus_handle), cx)
            .on_key_down(cx.listener(Self::type_letter))
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(content))
    }
}

fn item_count(total: usize) -> String {
    match total {
        1 => "1 item".into(),
        n => format!("{n} items"),
    }
}

impl LibraryPage {
    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(total) = self.total else {
            return match &self.error {
                Some(error) => {
                    error_state(error.clone(), cx.listener(|this, _, _, cx| this.retry(cx)), cx).into_any_element()
                }
                None => centered_spinner().into_any_element(),
            };
        };
        if total == 0 {
            let message = if self.filtered() {
                "Nothing matches these filters."
            } else {
                "Nothing here yet."
            };
            return centered_message(message, cx).into_any_element();
        }

        let columns = self.columns(window);
        let rows = total.div_ceil(columns);

        let grid = uniform_list(
            "library-grid",
            rows,
            cx.processor(move |this, rows: Range<usize>, _window, cx| this.render_rows(rows, columns, cx)),
        )
        .track_scroll(&self.scroll)
        .on_scroll_wheel(smooth_wheel("library-grid", &self.scroll.0.borrow().base_handle))
        .flex_1()
        .h_full()
        .pt_4();
        h_flex()
            .size_full()
            .child(grid)
            .when(self.letters_shown(), |el| el.child(self.render_letters(cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Jump, chosen, jump_target, year_choices, years_of};

    #[test]
    fn letters_jump_past_the_names_before_them() {
        assert_eq!(jump_target('#', false), Jump::Top);
        assert_eq!(jump_target('M', false), Jump::After('M'));
        assert_eq!(jump_target('#', true), Jump::Bottom);
        assert_eq!(jump_target('M', true), Jump::After('N'));
        assert_eq!(jump_target('Z', true), Jump::Top);
    }

    #[test]
    fn years_are_grouped_under_decades_with_more_than_one() {
        assert_eq!(
            year_choices(&[2024, 1999, 2025, 2024]),
            ["2020s", "2025", "2024", "1999"]
        );
        assert_eq!(year_choices(&[]), Vec::<String>::new());
        assert_eq!(years_of("2020s"), (2020..2030).collect::<Vec<_>>());
        assert_eq!(years_of("1999"), [1999]);
        assert_eq!(years_of("nonsense"), Vec::<i32>::new());
    }

    #[test]
    fn the_first_choice_is_any() {
        let values = ["Comedy".to_owned(), "Drama".to_owned()];
        assert_eq!(chosen(&values, 0), None);
        assert_eq!(chosen(&values, 2).as_deref(), Some("Drama"));
        assert_eq!(chosen(&values, 3), None);
    }
}
