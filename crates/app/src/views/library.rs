//! A library's or person's items as a poster grid, with sort and filter: the toolbar's filters
//! on the left (progress, rating, genres and tags, year, resolution) and the sort on the right,
//! the values the filters offer, and where the jump bar's letters scroll to. Rows are
//! virtualised (`ScrollArea::show_rows`) and pages of [`PAGE_SIZE`] are fetched as their rows
//! scroll into view, with placeholders until they arrive.
//!
//! - The view is remembered per library in the config ([`LibraryView`], upgraded once), and
//!   the library's own genres, tags, ratings and years come from `/Items/Filters`; a remembered
//!   value the library no longer has stays listed, so what's applied is still shown. Opened
//!   from Home's *Latest* shelf title, the library starts sorted by date added, newest first.
//! - Changing the sort or filter bumps a generation: pages fetched before it are dropped.
//! - Excluded genres and tags, and requiring several of either at once, have no server parameter.
//!   While any are set, the page first lists
//!   the ids of every item the other filters leave and drops those with an excluded value
//!   ([`order`]); the grid's pages are then fetched by id from that list.
//! - The keyboard: the toolbar row, then the grid. Enter on a picker steps it to its next
//!   choice; on the order button it flips the order, on the genres and tags button it opens
//!   the popover, and on *Clear filters* it clears them. Typing a letter while sorted by name
//!   jumps like the letter bar (`Page::on_text`), highlighting the first item from that letter.
//! - Watch data changing elsewhere reloads the pages already loaded, keeping the items on
//!   screen until the new ones arrive.
//! - A page that fails is retried when its rows leave the screen and come back, rather than
//!   every frame while they stay there (`main` asked every frame).
//!
//! The popover is `views/value_filters.rs`; the cards and the item menu are `widgets/`.

use std::collections::{HashMap, HashSet};

use egui::{Align, Align2, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use egui_material_icons::icons::{ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_FILTER_LIST};
use jellyfin::models::{
    BaseItemDto, BaseItemKind, CollectionType, ItemField, ItemFilter, ItemFilterValues, ItemSortBy, QueryResult,
};
use jellyfin::{Client, ItemsQuery, SortOrder};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::config::{Config, LibraryView};
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::views::dialogs::AddToPlaylist;
use crate::views::value_filters::{self, ValueFilters};
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction};
use crate::widgets::text::{CARD_GAP, POSTER_H, POSTER_W};
use crate::widgets::{button, card, select, state};

/// Items per request: rows show placeholders until the page holding them arrives.
const PAGE_SIZE: usize = 100;
/// `main`'s `px_6` around the toolbar and the grid.
const GRID_PADDING: f32 = 24.;
/// Space around a toolbar control for the cursor's ring.
const RING_GAP: f32 = 3.;
/// The genres and tags button's width.
const VALUES_BUTTON_W: f32 = 200.;
/// The jump bar's letters; `#` is the top (names starting with digits or symbols).
const LETTERS: &str = "#ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LETTER_BAR_W: f32 = 28.;

/// The cursor's sections: the toolbar's controls, then the grid.
const TOOLBAR: usize = 0;
const GRID: usize = 1;
const GRID_START: Position = Position {
    section: GRID,
    index: 0,
};

/// Sort choices: config key, label, server sort, and whether it starts descending.
pub const SORTS: [(&str, &str, ItemSortBy, bool); 6] = [
    ("name", "Name", ItemSortBy::SortName, false),
    ("added", "Date added", ItemSortBy::DateCreated, true),
    ("released", "Release date", ItemSortBy::PremiereDate, true),
    ("rating", "Rating", ItemSortBy::CommunityRating, true),
    ("runtime", "Runtime", ItemSortBy::Runtime, false),
    ("random", "Random", ItemSortBy::Random, false),
];

/// Progress choices: config key, label, server filter.
pub const FILTERS: [(&str, &str, Option<ItemFilter>); 4] = [
    ("all", "Any progress", None),
    ("unwatched", "Unwatched", Some(ItemFilter::IsUnplayed)),
    ("watched", "Watched", Some(ItemFilter::IsPlayed)),
    ("in-progress", "In progress", Some(ItemFilter::IsResumable)),
];

/// Resolution choices: config key, label, and video width bounds (4K from 3800 wide, HD from
/// 1200, which 720p's 1280 clears). 12.2 goes by an item's widest version, 10.11 by its first.
pub const RESOLUTIONS: [(&str, &str, Option<u32>, Option<u32>); 4] = [
    ("any", "Any resolution", None, None),
    ("4k", "4K", Some(3800), None),
    ("hd", "HD", Some(1200), Some(3799)),
    ("sd", "SD", None, Some(1199)),
];

/// The value picked from an "any, then `values`" menu; `None` for "any".
pub fn chosen(values: &[String], index: i32) -> Option<String> {
    usize::try_from(index - 1).ok().and_then(|i| values.get(i)).cloned()
}

/// Year menu keys, newest first: each year, preceded by its decade ("2020s") when the library
/// has more than one year from it.
pub fn year_choices(years: &[i32]) -> Vec<String> {
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
pub fn years_of(key: &str) -> Vec<i32> {
    match key.strip_suffix('s').and_then(|decade| decade.parse::<i32>().ok()) {
        Some(decade) => (decade..decade + 10).collect(),
        None => key.parse().ok().into_iter().collect(),
    }
}

/// Where a letter of the jump bar scrolls to.
#[derive(Debug, PartialEq)]
pub enum Jump {
    Top,
    Bottom,
    /// Past the names that sort before this letter (counted by the server).
    After(char),
}

/// `#` is the first row ascending and the last descending. A letter starts where the names
/// before it end; descending, that's after every name from the next letter on.
pub fn jump_target(letter: char, descending: bool) -> Jump {
    match (letter, descending) {
        ('#', false) | ('Z', true) => Jump::Top,
        ('#', true) => Jump::Bottom,
        (letter, false) => Jump::After(letter),
        (letter, true) => Jump::After(char::from(letter as u8 + 1)),
    }
}

/// The library's genres, tags and official ratings, which the popover and the rating picker
/// offer.
#[derive(Clone, Default)]
pub struct FilterValues {
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub ratings: Vec<String>,
}

/// The kinds of values in the genres and tags popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Genre,
    Tag,
}

/// What the grid lists.
pub enum Source {
    /// A library or folder. Movie and show libraries list their top-level items across all
    /// folders; anything else is browsed folder by folder.
    Folder {
        id: String,
        collection_type: Option<CollectionType>,
        /// Start sorted by date added, newest first, rather than the remembered sort.
        newest_first: bool,
    },
    /// Movies and shows a person appears in.
    Person { id: String },
}

/// Where the genre, year and resolution filters apply: movie and show libraries, whose items
/// the server can list those values for.
struct FilterScope {
    parent_id: String,
    item_types: Vec<BaseItemKind>,
    /// Shows have no resolution of their own, so only movie libraries filter by it.
    resolution: bool,
}

/// The toolbar's controls, left to right, as the keyboard cursor sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Show,
    Rating,
    /// Opens the genres and tags popover.
    Values,
    Year,
    Resolution,
    Clear,
    Sort,
    Order,
}

/// What a toolbar control asked for, carried out after the toolbar is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolAction {
    Sort(usize),
    Show(usize),
    Rating(usize),
    Year(usize),
    Resolution(usize),
    ToggleOrder,
    OpenValues,
    Clear,
}

/// A letter jump's counted answer, for [`LibraryPage::land_from_jump`].
struct JumpResult {
    generation: u64,
    letter: char,
    highlight: bool,
    total: usize,
    descending: bool,
    /// How many of the grid's items sort before the letter.
    result: jellyfin::Result<usize>,
}

/// A fetched page's generation, page number and items.
type PageResult = (u64, usize, jellyfin::Result<QueryResult<BaseItemDto>>);
/// The ids of the grid's items in order, while genres or tags are excluded, and the generation
/// they're for.
type OrderResult = (u64, jellyfin::Result<Vec<String>>);

pub struct LibraryPage {
    title: String,
    client: Client,
    /// Key for the remembered sort and filter.
    view_key: String,
    view: LibraryView,
    base_query: ItemsQuery,
    filter_scope: Option<FilterScope>,
    /// `None` until the first page arrives.
    total: Option<usize>,
    items: Vec<Option<BaseItemDto>>,
    /// Pages asked for, including those still in flight: a row only asks once.
    pages_requested: HashSet<usize>,
    /// Pages whose request failed, waiting for their rows to leave the screen and come back.
    failed_pages: HashSet<usize>,
    /// The pages on screen in the last frame, for the retry above.
    visible_pages: HashSet<usize>,
    /// Bumped when the sort or filter changes, so pages fetched before are dropped.
    generation: u64,
    error: Option<String>,
    /// The pages being fetched.
    requests: Vec<Task<PageResult>>,
    /// While genres or tags are excluded: the ids of the grid's items in order, which pages
    /// are fetched by. `None` until they arrive, and whenever nothing is excluded.
    order: Option<Vec<String>>,
    order_task: Option<Task<OrderResult>>,
    /// The library's genres, tags, ratings and years, while they load.
    values_task: Option<Task<jellyfin::Result<ItemFilterValues>>>,
    /// The count a letter jump needs, while it runs.
    jump: Option<Task<JumpResult>>,
    /// The item a letter jump scrolls to, when the grid draws next.
    scroll_to: Option<usize>,
    /// The `services.user_data` the loaded pages were fetched at.
    loaded_at: Option<u64>,
    cursor: Cursor,
    /// Cards per row, from the last frame: the keys use the layout the page last showed.
    columns: usize,
    /// Where the scroll area is, from the last frame. `show_rows` draws only what's on screen,
    /// so a card that isn't can't scroll itself into view; the page moves the offset itself for
    /// the cursor and the letter jumps.
    offset: f32,
    sort_index: usize,
    filter_index: usize,
    rating_index: usize,
    year_index: usize,
    resolution_index: usize,
    values: FilterValues,
    /// The library's year keys, offered after "Any year".
    years: Vec<String>,
    value_filters: ValueFilters,
    /// *Add to playlist…* from a card's menu, while it's open.
    add_to_playlist: Option<AddToPlaylist>,
}

impl LibraryPage {
    pub fn new(cx: &mut PageCx, source: Source, title: String) -> Self {
        let mut newest = false;
        let (view_key, base_query, filter_scope) = match source {
            Source::Folder {
                id,
                collection_type,
                newest_first,
            } => {
                newest = newest_first;
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
        if newest {
            // Not saved: the remembered sort is back next time, unless the view changes here.
            view.sort = SORTS[1].0.into();
            view.descending = SORTS[1].3;
        }
        let sort_index = SORTS.iter().position(|(key, ..)| *key == view.sort).unwrap_or(0);
        let filter_index = FILTERS.iter().position(|(key, ..)| *key == view.filter).unwrap_or(0);
        view.filter = FILTERS[filter_index].0.into();
        let resolution_index = view
            .resolution
            .as_ref()
            .and_then(|resolution| RESOLUTIONS.iter().position(|(key, ..)| key == resolution))
            .unwrap_or(0);
        // Until the library's own values arrive, the remembered ones are the only choices.
        let values = FilterValues {
            genres: view.genres.iter().chain(&view.exclude_genres).cloned().collect(),
            tags: view.tags.iter().chain(&view.exclude_tags).cloned().collect(),
            ratings: view.rating.iter().cloned().collect(),
        };
        let years: Vec<String> = view.year.iter().cloned().collect();
        let year_index = usize::from(view.year.is_some());
        let rating_index = usize::from(view.rating.is_some());
        let mut this = Self {
            title,
            client: cx.services.session.client.clone(),
            view_key,
            view,
            base_query,
            filter_scope,
            total: None,
            items: Vec::new(),
            pages_requested: HashSet::new(),
            failed_pages: HashSet::new(),
            visible_pages: HashSet::new(),
            generation: 0,
            error: None,
            requests: Vec::new(),
            order: None,
            order_task: None,
            values_task: None,
            jump: None,
            scroll_to: None,
            loaded_at: Some(cx.services.user_data),
            cursor: Cursor::starting_at(GRID_START),
            columns: 1,
            offset: 0.,
            sort_index,
            filter_index,
            rating_index,
            year_index,
            resolution_index,
            values,
            years,
            value_filters: ValueFilters::default(),
            add_to_playlist: None,
        };
        this.apply_view();
        this.load(cx);
        this.load_filter_values(cx);
        this
    }

    /// The results of the page's requests, as of this frame.
    fn poll(&mut self, cx: &mut PageCx) {
        // A finished task is dropped (which is how it ends); the ones still running stay.
        let mut done = Vec::new();
        let mut pending = Vec::new();
        for mut request in std::mem::take(&mut self.requests) {
            match request.poll() {
                Some(result) => done.push(result),
                None => pending.push(request),
            }
        }
        self.requests = pending;
        for (generation, page, result) in done {
            if generation != self.generation {
                continue;
            }
            match result {
                Ok(result) => match &self.order {
                    // Fetched by id: the server sorts the page by itself, so each item goes
                    // where the list has it.
                    Some(order) => {
                        let start = page * PAGE_SIZE;
                        let end = (start + PAGE_SIZE).min(order.len());
                        let at: HashMap<&str, usize> = order[start.min(end)..end]
                            .iter()
                            .enumerate()
                            .map(|(offset, id)| (id.as_str(), start + offset))
                            .collect();
                        for item in result.items {
                            if let Some(&index) = at.get(item.id.as_str()) {
                                self.items[index] = Some(item);
                            }
                        }
                    }
                    None => {
                        let total = result.total_record_count as usize;
                        self.total = Some(total);
                        self.items.resize(total, None);
                        let start = page * PAGE_SIZE;
                        for (offset, item) in result.items.into_iter().enumerate() {
                            if let Some(slot) = self.items.get_mut(start + offset) {
                                *slot = Some(item);
                            }
                        }
                    }
                },
                Err(err) => {
                    // Asked for again when the rows scroll back into view (`grid`), rather
                    // than every frame while they stay on screen.
                    self.pages_requested.remove(&page);
                    self.failed_pages.insert(page);
                    let message = cx.services.handle_error(&err);
                    tracing::warn!(page, "{message}");
                    self.error = Some(message);
                }
            }
        }
        if let Some((generation, result)) = self.order_task.as_mut().and_then(Task::poll) {
            self.order_task = None;
            if generation == self.generation {
                match result {
                    Ok(order) => self.apply_order(order, cx),
                    Err(err) => {
                        let message = cx.services.handle_error(&err);
                        tracing::warn!("couldn't list the library's items: {message}");
                        self.error = Some(message);
                    }
                }
            }
        }
        if let Some(result) = self.values_task.as_mut().and_then(Task::poll) {
            self.values_task = None;
            match result {
                Ok(values) => self.apply_filter_values(values),
                Err(err) => tracing::warn!("couldn't load the library's genres and years: {err}"),
            }
        }
        if let Some(jump) = self.jump.as_mut().and_then(Task::poll) {
            self.jump = None;
            self.land_from_jump(jump);
        }
    }

    /// Fills the genres, tags and ratings and the year menu with the values found in the
    /// library. A remembered choice the library no longer has stays listed, so what's applied
    /// is still shown.
    fn apply_filter_values(&mut self, values: ItemFilterValues) {
        let with_chosen = |mut found: Vec<String>, chosen: &[String]| {
            let missing: Vec<String> = chosen.iter().filter(|c| !found.contains(c)).cloned().collect();
            if !missing.is_empty() {
                found.extend(missing);
                found.sort_by_key(|value| value.to_lowercase());
            }
            found
        };
        let chosen_genres: Vec<String> = self
            .view
            .genres
            .iter()
            .chain(&self.view.exclude_genres)
            .cloned()
            .collect();
        let chosen_tags: Vec<String> = self.view.tags.iter().chain(&self.view.exclude_tags).cloned().collect();
        let chosen_rating: Vec<String> = self.view.rating.iter().cloned().collect();
        self.values = FilterValues {
            genres: with_chosen(values.genres, &chosen_genres),
            tags: with_chosen(values.tags, &chosen_tags),
            ratings: with_chosen(values.official_ratings, &chosen_rating),
        };
        self.rating_index = self
            .view
            .rating
            .as_ref()
            .and_then(|rating| self.values.ratings.iter().position(|value| value == rating))
            .map_or(0, |index| index + 1);
        let mut years = year_choices(&values.years);
        if let Some(year) = &self.view.year
            && !years.contains(year)
        {
            years.push(year.clone());
        }
        self.year_index = self
            .view
            .year
            .as_ref()
            .and_then(|year| years.iter().position(|value| value == year))
            .map_or(0, |index| index + 1);
        self.years = years;
    }

    fn load_filter_values(&mut self, cx: &mut PageCx) {
        let Some(scope) = &self.filter_scope else {
            return;
        };
        let (client, parent_id, item_types) = (self.client.clone(), scope.parent_id.clone(), scope.item_types.clone());
        self.values_task = Some(Task::spawn(cx.ctx, async move {
            client.item_filter_values(&parent_id, &item_types).await
        }));
    }

    /// Puts the sort and filter into the query.
    fn apply_view(&mut self) {
        let sort = SORTS
            .iter()
            .find(|(key, ..)| *key == self.view.sort)
            .unwrap_or(&SORTS[0]);
        // Ties (same date, rating…) fall back to the name.
        self.base_query.sort_by = vec![sort.2, ItemSortBy::SortName];
        self.base_query.sort_order = Some(if self.view.descending {
            SortOrder::Descending
        } else {
            SortOrder::Ascending
        });
        let filter = FILTERS
            .iter()
            .find(|(key, ..)| *key == self.view.filter)
            .unwrap_or(&FILTERS[0]);
        self.base_query.filters = filter.2.into_iter().collect();
        let Some(scope) = &self.filter_scope else {
            return;
        };
        // The server matches any of the genres given; the rest are intersected in `order`.
        self.base_query.genres = self.view.genres.iter().take(1).cloned().collect();
        self.base_query.tags = self.view.tags.iter().take(1).cloned().collect();
        self.base_query.official_ratings = self.view.rating.iter().cloned().collect();
        self.base_query.years = self.view.year.as_deref().map(years_of).unwrap_or_default();
        let resolution = RESOLUTIONS
            .iter()
            .find(|(key, ..)| scope.resolution && self.view.resolution.as_deref() == Some(key))
            .unwrap_or(&RESOLUTIONS[0]);
        (self.base_query.min_width, self.base_query.max_width) = (resolution.2, resolution.3);
    }

    /// Whether anything is filtered out (the sort aside).
    fn filtered(&self) -> bool {
        self.view.filter != FILTERS[0].0
            || !self.view.genres.is_empty()
            || !self.view.tags.is_empty()
            || !self.view.exclude_genres.is_empty()
            || !self.view.exclude_tags.is_empty()
            || self.view.rating.is_some()
            || self.view.year.is_some()
            || self.view.resolution.is_some()
    }

    /// Whether genres or tags are excluded, or several genres or tags required (the server matches any
    /// of them, we need all), so the grid goes by [`Self::order`].
    fn ordered(&self) -> bool {
        self.filter_scope.is_some()
            && (self.view.genres.len() > 1
                || self.view.tags.len() > 1
                || !(self.view.exclude_genres.is_empty() && self.view.exclude_tags.is_empty()))
    }

    /// Loads the grid from the top: its first page, or first the order it's fetched by.
    fn load(&mut self, cx: &mut PageCx) {
        if self.ordered() {
            self.request_order(cx);
        } else {
            self.request_page(0, cx);
        }
    }

    /// Lists the ids of the grid's items while genres or tags are excluded: every item the
    /// other filters leave, in the grid's order, less those with an excluded value.
    fn request_order(&mut self, cx: &mut PageCx) {
        let client = self.client.clone();
        let query = ItemsQuery {
            fields: Vec::new(),
            start_index: None,
            limit: None,
            lean: true,
            ..self.base_query.clone()
        };
        let (genres, tags) = (self.view.exclude_genres.clone(), self.view.exclude_tags.clone());
        let required: Vec<(Vec<String>, Vec<String>)> = (self.view.genres.iter().skip(1))
            .map(|genre| (vec![genre.clone()], Vec::new()))
            .chain(self.view.tags.iter().skip(1).map(|tag| (Vec::new(), vec![tag.clone()])))
            .collect();
        let generation = self.generation;
        self.order_task = Some(Task::spawn(cx.ctx, async move {
            (generation, order(&client, query, required, genres, tags).await)
        }));
    }

    /// The order arrived: the grid is that long, and the pages already shown are fetched again
    /// from it (all of them after a watch-data refresh, else the first).
    fn apply_order(&mut self, order: Vec<String>, cx: &mut PageCx) {
        self.total = Some(order.len());
        self.items.resize(order.len(), None);
        self.order = Some(order);
        let mut pages: Vec<usize> = self.pages_requested.drain().collect();
        if pages.is_empty() {
            pages.push(0);
        }
        self.failed_pages.clear();
        self.requests.clear();
        for page in pages {
            self.request_page(page, cx);
        }
    }

    /// The sort or filter changed: remember it and load the grid again from the top.
    fn change_view(&mut self, cx: &mut PageCx) {
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
        self.failed_pages.clear();
        self.visible_pages.clear();
        self.requests.clear();
        self.order = None;
        self.order_task = None;
        self.jump = None;
        self.scroll_to = None;
        self.error = None;
        // Changed from the toolbar, the cursor stays there; otherwise it starts on the grid.
        if self
            .cursor
            .selected()
            .is_none_or(|position| position.section != TOOLBAR)
        {
            self.cursor = Cursor::starting_at(GRID_START);
        }
        self.offset = 0.;
        self.load(cx);
    }

    /// The letter bar shows while sorted by name, where names starting with a letter are
    /// together.
    fn letters_shown(&self) -> bool {
        self.view.sort == SORTS[0].0 && self.total.is_some_and(|total| total > 0)
    }

    /// Scrolls to the first name starting with `letter`. Its index is the number of names
    /// before it (after it, descending), which the server counts with the grid's filters.
    /// From the keyboard, the highlight moves there too.
    fn jump_to(&mut self, letter: char, highlight: bool, cx: &mut PageCx) {
        let Some(total) = self.total else {
            return;
        };
        let descending = self.view.descending;
        let bound = match jump_target(letter, descending) {
            Jump::Top => return self.land(0, highlight),
            Jump::Bottom => return self.land(total.saturating_sub(1), highlight),
            Jump::After(bound) => bound,
        };
        // Excluding, the server's count would include the excluded items: the names before
        // the letter are listed and those in the grid counted.
        let kept: Option<HashSet<String>> = self.order.as_ref().map(|order| order.iter().cloned().collect());
        let query = ItemsQuery {
            name_less_than: Some(bound.to_string()),
            start_index: None,
            limit: if kept.is_some() { None } else { Some(0) },
            fields: Vec::new(),
            lean: true,
            ..self.base_query.clone()
        };
        let client = self.client.clone();
        let generation = self.generation;
        self.jump = Some(Task::spawn(cx.ctx, async move {
            let result = client.items(&query).await.map(|result| match &kept {
                Some(kept) => result.items.iter().filter(|item| kept.contains(&item.id)).count(),
                None => result.total_record_count as usize,
            });
            JumpResult {
                generation,
                letter,
                highlight,
                total,
                descending,
                result,
            }
        }));
    }

    fn land_from_jump(&mut self, jump: JumpResult) {
        if jump.generation != self.generation {
            return;
        }
        match jump.result {
            Ok(before) => {
                let index = if jump.descending {
                    jump.total.saturating_sub(before)
                } else {
                    before
                };
                self.land(index.min(jump.total.saturating_sub(1)), jump.highlight);
            }
            Err(err) => tracing::warn!("couldn't find where {} starts: {err}", jump.letter),
        }
    }

    fn land(&mut self, index: usize, highlight: bool) {
        self.scroll_to = Some(index);
        if highlight {
            self.cursor.show_at(Position { section: GRID, index });
        }
    }

    /// A control's action, after the toolbar row is drawn: the view's change, or what the
    /// buttons do.
    fn apply_tool(&mut self, action: ToolAction, cx: &mut PageCx) {
        match action {
            ToolAction::Sort(index) => {
                let Some(sort) = SORTS.get(index) else {
                    return;
                };
                if self.view.sort == sort.0 {
                    return;
                }
                self.view.sort = sort.0.into();
                self.view.descending = sort.3;
                self.sort_index = index;
                self.change_view(cx);
            }
            ToolAction::Show(index) => {
                let Some(filter) = FILTERS.get(index) else {
                    return;
                };
                if self.view.filter == filter.0 {
                    return;
                }
                self.view.filter = filter.0.into();
                self.filter_index = index;
                self.change_view(cx);
            }
            ToolAction::Rating(index) => {
                let rating = chosen(&self.values.ratings, index as i32);
                if self.view.rating == rating {
                    return;
                }
                self.view.rating = rating;
                self.rating_index = index;
                self.change_view(cx);
            }
            ToolAction::Year(index) => {
                let year = chosen(&self.years, index as i32);
                if self.view.year == year {
                    return;
                }
                self.view.year = year;
                self.year_index = index;
                self.change_view(cx);
            }
            ToolAction::Resolution(index) => {
                let Some(resolution) = RESOLUTIONS.get(index) else {
                    return;
                };
                let chosen = (index > 0).then(|| resolution.0.to_owned());
                if self.view.resolution == chosen {
                    return;
                }
                self.view.resolution = chosen;
                self.resolution_index = index;
                self.change_view(cx);
            }
            ToolAction::ToggleOrder => {
                self.view.descending = !self.view.descending;
                self.change_view(cx);
            }
            ToolAction::OpenValues => self.value_filters.toggle(cx.ctx),
            ToolAction::Clear => self.clear_filters(cx),
        }
    }

    /// Enter on a toolbar control: a picker steps to its next choice, wrapping round.
    fn step(&mut self, tool: Tool, cx: &mut PageCx) {
        let action = match tool {
            Tool::Sort => ToolAction::Sort((self.sort_index + 1) % SORTS.len()),
            Tool::Show => ToolAction::Show((self.filter_index + 1) % FILTERS.len()),
            Tool::Rating => ToolAction::Rating((self.rating_index + 1) % (self.values.ratings.len() + 1)),
            Tool::Year => ToolAction::Year((self.year_index + 1) % (self.years.len() + 1)),
            Tool::Resolution => ToolAction::Resolution((self.resolution_index + 1) % RESOLUTIONS.len()),
            Tool::Order => ToolAction::ToggleOrder,
            Tool::Values => {
                self.value_filters.open(cx.ctx);
                return;
            }
            Tool::Clear if !self.filtered() => return,
            Tool::Clear => ToolAction::Clear,
        };
        self.apply_tool(action, cx);
    }

    fn clear_filters(&mut self, cx: &mut PageCx) {
        self.view.filter = FILTERS[0].0.into();
        self.filter_index = 0;
        (self.view.year, self.year_index) = (None, 0);
        (self.view.rating, self.rating_index) = (None, 0);
        (self.view.resolution, self.resolution_index) = (None, 0);
        self.view.genres.clear();
        self.view.tags.clear();
        self.view.exclude_genres.clear();
        self.view.exclude_tags.clear();
        self.change_view(cx);
        // The Clear button is gone now; from the keyboard, land on the control before it.
        if self
            .cursor
            .selected()
            .is_some_and(|position| position.section == TOOLBAR)
        {
            let before = self.tools().iter().position(|tool| *tool == Tool::Sort).unwrap_or(1);
            self.cursor.show_at(Position {
                section: TOOLBAR,
                index: before.saturating_sub(1),
            });
        }
    }

    /// Re-fetches the pages already loaded (e.g. for new watched badges), keeping the current
    /// items on screen until the new ones arrive.
    fn refresh(&mut self, cx: &mut PageCx) {
        // Ordered, the order may have changed too (a watched filter): it comes first, and
        // brings the pages with it (`apply_order`).
        if self.order.is_some() {
            self.requests.clear();
            self.request_order(cx);
            return;
        }
        let pages: Vec<usize> = self.pages_requested.drain().collect();
        self.failed_pages.clear();
        // Requests still running were fetched with the old watch data: let the new ones
        // replace them rather than race them.
        self.requests.clear();
        for page in pages {
            self.request_page(page, cx);
        }
    }

    fn retry(&mut self, cx: &mut PageCx) {
        self.error = None;
        self.pages_requested.clear();
        self.failed_pages.clear();
        self.requests.clear();
        if self.order.is_some() {
            self.request_page(0, cx);
        } else {
            self.load(cx);
        }
    }

    fn request_page(&mut self, page: usize, cx: &mut PageCx) {
        if !self.pages_requested.insert(page) {
            return;
        }
        let query = match &self.order {
            Some(order) => {
                let start = (page * PAGE_SIZE).min(order.len());
                let end = (start + PAGE_SIZE).min(order.len());
                // No ids would ask for every item.
                if start == end {
                    return;
                }
                ItemsQuery {
                    ids: order[start..end].to_vec(),
                    start_index: None,
                    limit: None,
                    ..self.base_query.clone()
                }
            }
            // Ordered, pages wait for the order.
            None if self.ordered() => {
                self.pages_requested.remove(&page);
                return;
            }
            None => ItemsQuery {
                start_index: Some((page * PAGE_SIZE) as u32),
                ..self.base_query.clone()
            },
        };
        let client = self.client.clone();
        let generation = self.generation;
        self.requests.push(Task::spawn(cx.ctx, async move {
            (generation, page, client.items(&query).await)
        }));
    }

    /// The toolbar's controls shown now, left to right: the filters, then the sort.
    fn tools(&self) -> Vec<Tool> {
        let mut tools = vec![Tool::Show];
        if let Some(scope) = &self.filter_scope {
            if !self.values.ratings.is_empty() {
                tools.push(Tool::Rating);
            }
            if !(self.values.genres.is_empty() && self.values.tags.is_empty()) {
                tools.push(Tool::Values);
            }
            if !self.years.is_empty() {
                tools.push(Tool::Year);
            }
            if scope.resolution {
                tools.push(Tool::Resolution);
            }
        }
        tools.push(Tool::Clear);
        tools.extend([Tool::Sort, Tool::Order]);
        tools
    }

    fn toolbar(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let tools = self.tools();
        let split = tools.iter().position(|tool| *tool == Tool::Sort).unwrap_or(tools.len());
        let muted = crate::theme::tokens_of(ui).muted_foreground;
        let mut action = None;
        let mut values_button = None;
        ui.horizontal(|ui| {
            ui.add_space(GRID_PADDING - RING_GAP);
            for (index, tool) in tools[..split].iter().copied().enumerate() {
                let (response, control) = self.tool(ui, tool);
                action = action.or(control);
                if self.cursor.is_selected(TOOLBAR, index) {
                    ring(ui, &response);
                }
                if tool == Tool::Values {
                    values_button = Some(response);
                }
            }
            if let Some(total) = self.total {
                ui.add_space(12.);
                ui.label(
                    RichText::new(item_count(total))
                        .font(crate::theme::text::font(ui, "sm"))
                        .color(muted),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // `main`'s `px_6` on the right too: the sort sits clear of the window's edge
                // now that the page owns the whole width.
                ui.add_space(GRID_PADDING - RING_GAP);
                // Right to left: the order button, the sort picker, then its label.
                for index in (split..tools.len()).rev() {
                    let (response, control) = self.tool(ui, tools[index]);
                    action = action.or(control);
                    if self.cursor.is_selected(TOOLBAR, index) {
                        ring(ui, &response);
                    }
                }
                ui.label(
                    RichText::new("Sort")
                        .font(crate::theme::text::font(ui, "sm"))
                        .color(muted),
                );
            });
        });
        if let Some(anchor) = values_button
            && self.value_filters.show(&anchor, &self.values, &mut self.view)
        {
            self.change_view(cx);
        }
        if let Some(action) = action {
            self.apply_tool(action, cx);
        }
    }

    /// One toolbar control: its response and what it asked for when clicked.
    fn tool(&self, ui: &mut Ui, tool: Tool) -> (Response, Option<ToolAction>) {
        let view = &self.view;
        match tool {
            Tool::Sort => {
                let labels = SORTS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(ui, "library_sort", "Sort", 150., labels, self.sort_index);
                (response, index.map(ToolAction::Sort))
            }
            Tool::Show => {
                let labels = FILTERS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(ui, "library_show", "Progress", 140., labels, self.filter_index);
                (response, index.map(ToolAction::Show))
            }
            Tool::Rating => {
                let mut labels = vec!["Any rating"];
                labels.extend(self.values.ratings.iter().map(String::as_str));
                let (response, index) = picker(ui, "library_rating", "Rating", 120., labels, self.rating_index);
                (response, index.map(ToolAction::Rating))
            }
            Tool::Year => {
                let mut labels = vec!["Any year"];
                labels.extend(self.years.iter().map(String::as_str));
                let (response, index) = picker(ui, "library_year", "Year", 120., labels, self.year_index);
                (response, index.map(ToolAction::Year))
            }
            Tool::Resolution => {
                let labels = RESOLUTIONS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(
                    ui,
                    "library_resolution",
                    "Resolution",
                    150.,
                    labels,
                    self.resolution_index,
                );
                (response, index.map(ToolAction::Resolution))
            }
            Tool::Order => {
                // The icon shows the direction even for Random, as on `main`; only what a click
                // does changes there.
                let icon = if view.descending {
                    ICON_ARROW_DOWNWARD
                } else {
                    ICON_ARROW_UPWARD
                };
                let tooltip = if view.sort == "random" {
                    "Shuffle again"
                } else if view.descending {
                    "Sort ascending"
                } else {
                    "Sort descending"
                };
                let button = egui::Button::new(icon.outlined().rich_text())
                    .frame(false)
                    .min_size(vec2(24., ui.spacing().interact_size.y));
                let response = ui.add(button).on_hover_text(tooltip);
                let action = response.clicked().then_some(ToolAction::ToggleOrder);
                (response, action)
            }
            Tool::Values => {
                let label = value_filters::summary(view);
                // A set width, so the controls after it don't shift as the choice changes.
                let button = button::with_icon(ui, ICON_FILTER_LIST, &label).truncate();
                let response = ui.add_sized(vec2(VALUES_BUTTON_W, ui.spacing().interact_size.y), button);
                let action = response.clicked().then_some(ToolAction::OpenValues);
                (response, action)
            }
            Tool::Clear => {
                let response = ui.add_enabled(self.filtered(), egui::Button::new("Clear filters"));
                let action = response.clicked().then_some(ToolAction::Clear);
                (response, action)
            }
        }
    }

    fn letters(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        // The letters spread down the whole grid, as `main`'s did, at 12 px each.
        let count = LETTERS.chars().count() as f32;
        let line = 12.;
        let gap = ((ui.available_height() - line * count) / (count - 1.)).clamp(0., 16.);
        let size = vec2(LETTER_BAR_W, ui.available_height());
        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = gap;
            for letter in LETTERS.chars() {
                let (rect, response) = ui.allocate_exact_size(vec2(LETTER_BAR_W, line), Sense::click());
                let tokens = crate::theme::tokens_of(ui);
                if response.hovered() {
                    let fill = tokens.accent;
                    ui.painter()
                        .rect_filled(rect.shrink2(vec2(4., 0.)), CornerRadius::same(2), fill);
                }
                let color = if response.hovered() {
                    tokens.accent_foreground
                } else {
                    tokens.muted_foreground
                };
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    letter,
                    FontId::new(11., egui::FontFamily::Proportional),
                    color,
                );
                if response.clicked() {
                    self.jump_to(letter, false, cx);
                }
            }
        });
    }

    fn grid(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let total = self.total.unwrap_or(0);
        let letters = self.letters_shown();
        let bar = if letters {
            LETTER_BAR_W + ui.spacing().item_spacing.x
        } else {
            0.
        };
        let grid_width = (ui.available_width() - bar).max(1.);
        let columns = columns(grid_width);
        self.columns = columns;
        let rows = total.div_ceil(columns);
        let card_height = POSTER_H + card::text_height(ui);
        let pitch = card_height + CARD_GAP;
        // The grid sits `main`'s `pt_4` under the toolbar, so it has that much less room.
        let viewport = (ui.available_height() - 16.).max(0.);
        // `show_rows` leaves out the last row's gap below it.
        let max_offset = (rows as f32 * pitch - CARD_GAP - viewport).max(0.);
        if let Some(index) = self.scroll_to.take() {
            self.offset = (index / columns) as f32 * pitch;
        }
        if self.cursor.take_reveal()
            && let Some(position) = self.cursor.selected()
            && position.section == GRID
        {
            scroll_to_row(
                position.index,
                &mut self.offset,
                columns,
                viewport,
                card_height,
                pitch,
                max_offset,
            );
        }
        self.offset = self.offset.clamp(0., max_offset);
        let previous_pages = std::mem::take(&mut self.visible_pages);
        let mut visible_pages = HashSet::new();
        let mut missing = Vec::new();
        let mut opened = None;
        let mut chosen = None;
        // `main`'s `pt_4` between the toolbar and the grid.
        ui.add_space(16.);
        ui.horizontal_top(|ui| {
            let size = vec2(grid_width, viewport);
            ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                // `show_rows` spaces its rows by the ui's item spacing: it must be the grid's
                // gap before it's called, and not only inside its contents.
                ui.spacing_mut().item_spacing.y = CARD_GAP;
                let out = egui::ScrollArea::vertical()
                    .id_salt("library_grid")
                    .auto_shrink([false, false])
                    .vertical_scroll_offset(self.offset)
                    .show_rows(ui, card_height, rows, |ui, visible| {
                        for row in visible {
                            let start = row * columns;
                            let end = (start + columns).min(total);
                            visible_pages.insert(start / PAGE_SIZE);
                            visible_pages.insert((end - 1) / PAGE_SIZE);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = CARD_GAP;
                                ui.add_space(GRID_PADDING);
                                for (column, slot) in self.items[start..end].iter().enumerate() {
                                    let index = start + column;
                                    let selected = self.cursor.is_selected(GRID, index);
                                    match slot {
                                        Some(item) => {
                                            let response = card::poster(ui, &mut cx.services.images, item, selected);
                                            if response.clicked() {
                                                opened = Some(Route::for_item(item));
                                            }
                                            if let Some(action) = menu::item_menu(&response, item) {
                                                chosen = Some((action, item.clone()));
                                            }
                                        }
                                        None => {
                                            let page = index / PAGE_SIZE;
                                            if !missing.contains(&page) {
                                                missing.push(page);
                                            }
                                            card::placeholder(ui);
                                        }
                                    }
                                }
                            });
                        }
                    });
                self.offset = out.state.clamped_offset().y;
            });
            if letters {
                self.letters(ui, cx);
            }
        });
        self.visible_pages = visible_pages;
        for page in missing {
            if self.failed_pages.contains(&page) && previous_pages.contains(&page) {
                continue;
            }
            self.failed_pages.remove(&page);
            self.request_page(page, cx);
        }
        if let Some(route) = opened {
            cx.nav.push(NavRequest::Push(route));
        }
        if let Some((action, item)) = chosen {
            self.carry_out(action, &item, cx);
        }
    }

    /// What a card's menu asked for: the services play it or mark it, and *Add to playlist…*
    /// opens the dialog, which this page then keeps until it's done.
    fn carry_out(&mut self, action: MenuAction, item: &BaseItemDto, cx: &mut PageCx) {
        if let Some(MenuAction::AddToPlaylist) = cx.services.carry_out(action, item) {
            self.add_to_playlist = Some(AddToPlaylist::new(cx, item));
        }
    }

    fn dialogs(&mut self, cx: &mut PageCx) {
        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
    }
}

impl Page for LibraryPage {
    fn title(&self) -> &str {
        &self.title
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Browse)
    }

    fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        browse::on_key(self, action, cx)
    }

    /// A letter typed on the grid jumps like the letter bar, while sorted by name.
    fn on_text(&mut self, text: &str, cx: &mut PageCx) -> bool {
        let mut chars = text.chars();
        let (Some(letter), None) = (chars.next(), chars.next()) else {
            return false;
        };
        if !letter.is_ascii_alphabetic() || !self.letters_shown() {
            return false;
        }
        // Plain letters only, as on `main`: Ctrl, Alt or Shift with a letter is something else
        // (the zoom and the shortcuts take theirs first).
        if !cx
            .ctx
            .input(|input| input.modifiers.matches_exact(egui::Modifiers::NONE))
        {
            return false;
        }
        self.jump_to(letter.to_ascii_uppercase(), true, cx);
        true
    }

    fn hide_cursor(&mut self) {
        self.cursor.hide();
    }

    fn show(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        self.poll(cx);
        if self.loaded_at != Some(cx.services.user_data) {
            self.loaded_at = Some(cx.services.user_data);
            self.refresh(cx);
        }
        browse::hide_on_click(&mut self.cursor, ui);
        // `main`'s `pt_4` over the toolbar.
        ui.add_space(16.);
        self.toolbar(ui, cx);

        if self.total.is_none() {
            match &self.error {
                Some(message) => {
                    let message = message.clone();
                    if state::error(ui, message) {
                        self.retry(cx);
                    }
                }
                None => state::spinner(ui),
            }
        } else if self.total == Some(0) {
            let message = if self.filtered() {
                "Nothing matches these filters."
            } else {
                "Nothing here yet."
            };
            state::empty(ui, message);
        } else {
            self.grid(ui, cx);
        }

        self.dialogs(cx);
    }
}

impl Browse for LibraryPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self) -> Vec<Section> {
        vec![
            Section::row(self.tools().len()),
            Section::grid(self.total.unwrap_or(0), self.columns),
        ]
    }

    /// Enter on a toolbar control steps or presses it; on a card it opens the item.
    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if position.section == GRID {
            if let Some(Some(item)) = self.items.get(position.index) {
                cx.nav.push(NavRequest::Push(Route::for_item(item)));
            }
            return;
        }
        let Some(tool) = self.tools().get(position.index).copied() else {
            return;
        };
        self.step(tool, cx);
    }
}

/// A select over `labels`, with `selected` shown: its response and the line picked. It takes no
/// focus: the toolbar is on the page's cursor, and Enter there steps it instead.
fn picker(
    ui: &mut Ui,
    id: &'static str,
    name: &str,
    width: f32,
    labels: Vec<&str>,
    selected: usize,
) -> (Response, Option<usize>) {
    let id = ui.make_persistent_id(id);
    select::select(ui, id, width, name, &labels, selected, false)
}

/// The keyboard's ring around a toolbar control (`main`'s selection ring).
fn ring(ui: &Ui, response: &Response) {
    ui.painter().rect_stroke(
        response.rect.expand(RING_GAP),
        CornerRadius::same(4),
        Stroke::new(1.5, crate::theme::tokens_of(ui).brand),
        StrokeKind::Outside,
    );
}

/// The ids of what `query` lists, in order, less the items with any of the excluded `genres` or
/// `tags`, and only those that also have every one of the `required` genres and tags, each given
/// as the genres and tags to ask the server for (`query` already asks for one of each kind). The server can't do either itself, so it lists the items of each of those
/// values too, with the same filters bar the included ones, and they're taken off or kept.
async fn order(
    client: &Client,
    query: ItemsQuery,
    required: Vec<(Vec<String>, Vec<String>)>,
    genres: Vec<String>,
    tags: Vec<String>,
) -> jellyfin::Result<Vec<String>> {
    let ids = |query: ItemsQuery| async move {
        client
            .items(&query)
            .await
            .map(|result| result.items.into_iter().map(|item| item.id).collect::<Vec<_>>())
    };
    let apart = |genres: Vec<String>, tags: Vec<String>| ItemsQuery {
        genres,
        tags,
        sort_by: Vec::new(),
        sort_order: None,
        ..query.clone()
    };
    let (all, with_genres, with_tags) = tokio::try_join!(
        ids(query.clone()),
        async {
            if genres.is_empty() {
                Ok(Vec::new())
            } else {
                ids(apart(genres.clone(), Vec::new())).await
            }
        },
        async {
            if tags.is_empty() {
                Ok(Vec::new())
            } else {
                ids(apart(Vec::new(), tags.clone())).await
            }
        },
    )?;
    let having =
        futures_util::future::try_join_all(required.into_iter().map(|(genres, tags)| ids(apart(genres, tags)))).await?;
    let having: Vec<HashSet<String>> = having.into_iter().map(|ids| ids.into_iter().collect()).collect();
    let excluded: HashSet<String> = with_genres.into_iter().chain(with_tags).collect();
    Ok(all
        .into_iter()
        .filter(|id| !excluded.contains(id) && having.iter().all(|set| set.contains(id)))
        .collect())
}

/// Cards per row that fit `width`, with the toolbar's padding on both sides.
fn columns(width: f32) -> usize {
    let inner = (width - 2. * GRID_PADDING).max(0.);
    (((inner + CARD_GAP) / (POSTER_W + CARD_GAP)).floor() as usize).max(1)
}

/// Scrolls the cursor's row into view when a key moved it, keeping the scroll where the mouse
/// left it otherwise.
#[allow(clippy::too_many_arguments)]
fn scroll_to_row(
    index: usize,
    offset: &mut f32,
    columns: usize,
    viewport: f32,
    card_height: f32,
    pitch: f32,
    max_offset: f32,
) {
    let row = index / columns;
    let top = row as f32 * pitch;
    if top < *offset {
        *offset = top;
    } else if top + card_height > *offset + viewport {
        *offset = top + card_height - viewport;
    }
    *offset = offset.clamp(0., max_offset);
}

fn item_count(total: usize) -> String {
    match total {
        1 => "1 item".into(),
        n => format!("{n} items"),
    }
}

#[cfg(test)]
mod tests {
    use super::{GRID_PADDING, Jump, chosen, columns, item_count, jump_target, year_choices, years_of};

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

    #[test]
    fn the_grid_fits_as_many_posters_as_the_width_has_room_for() {
        // No room at all is still one column.
        assert_eq!(columns(0.), 1);
        assert_eq!(columns(150.), 1);
        // Two posters and the gap between them, plus `main`'s padding on both sides.
        assert_eq!(columns(2. * 150. + 16. + 2. * GRID_PADDING), 2);
        assert_eq!(columns(3. * 150. + 2. * 16. + 2. * GRID_PADDING), 3);
        // A poster and a bit doesn't fit a second.
        assert_eq!(columns(2. * 150. + 16. + 2. * GRID_PADDING - 1.), 1);
    }

    #[test]
    fn the_count_reads_naturally() {
        assert_eq!(item_count(0), "0 items");
        assert_eq!(item_count(1), "1 item");
        assert_eq!(item_count(2), "2 items");
    }
}
