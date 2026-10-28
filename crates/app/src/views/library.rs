//! A library's or person's items as a poster grid, with sort and filter: the toolbar's sort,
//! filter and resolution choices, the years it can filter by, and where the jump bar's letters
//! scroll to. Rows are virtualised (`ScrollArea::show_rows`) and pages of [`PAGE_SIZE`] are
//! fetched as their rows scroll into view, with placeholders until they arrive.
//!
//! - The view is remembered per library in the config ([`LibraryView`], upgraded once), and
//!   the library's own genres, tags, ratings and years come from `/Items/Filters`; a remembered
//!   value the library no longer has stays listed, so what's applied is still shown.
//! - Changing the sort or filter bumps a generation: pages fetched before it are dropped.
//! - The keyboard: the toolbar row, then the grid. Enter on a picker steps it to its next
//!   choice; on the order button it flips the order, on the filters button it opens the
//!   dialog, and on *Clear filters* it clears them. Typing a letter while sorted by name jumps
//!   like the letter bar (`Page::on_text`), highlighting the first item from that letter.
//! - Watch data changing elsewhere reloads the pages already loaded, keeping the items on
//!   screen until the new ones arrive.
//! - A page that fails is retried when its rows leave the screen and come back, rather than
//!   every frame while they stay there (`main` asked every frame).
//!
//! The dialog is `views/value_filters.rs`; the cards and the item menu are `widgets/`.

use std::collections::HashSet;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, StrokeKind, TextFormat, TextStyle,
    Ui, WidgetText, vec2,
};
use egui_material_icons::MaterialIcon;
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
use crate::views::value_filters::{Outcome, ValueFilters};
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction};
use crate::widgets::text::{CARD_GAP, POSTER_H, POSTER_W};
use crate::widgets::{card, state};

/// Items per request: rows show placeholders until the page holding them arrives.
const PAGE_SIZE: usize = 100;
/// Space around the grid and the toolbar (`main`'s 24 px, less the central panel's own margin).
const GRID_PADDING: f32 = 16.;
/// Space around a toolbar control for the cursor's ring.
const RING_GAP: f32 = 3.;
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

/// Filter choices: config key, label, server filter.
pub const FILTERS: [(&str, &str, Option<ItemFilter>); 5] = [
    ("all", "All", None),
    ("unwatched", "Unwatched", Some(ItemFilter::IsUnplayed)),
    ("watched", "Watched", Some(ItemFilter::IsPlayed)),
    ("favorites", "Favorites", Some(ItemFilter::IsFavorite)),
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

/// The library's genres, tags and official ratings, which the dialog lists to choose from.
#[derive(Clone, Default)]
pub struct FilterValues {
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub ratings: Vec<String>,
}

impl FilterValues {
    pub fn is_empty(&self) -> bool {
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
    Sort,
    Order,
    Show,
    /// Opens the genres, tags and ratings dialog.
    Values,
    Year,
    Resolution,
    Clear,
}

/// What a toolbar control asked for, carried out after the toolbar is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolAction {
    Sort(usize),
    Show(usize),
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
    result: jellyfin::Result<QueryResult<BaseItemDto>>,
}

/// A fetched page's generation, page number and items.
type PageResult = (u64, usize, jellyfin::Result<QueryResult<BaseItemDto>>);

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
    year_index: usize,
    resolution_index: usize,
    values: FilterValues,
    /// The library's year keys, offered after "Any year".
    years: Vec<String>,
    dialog: Option<ValueFilters>,
    /// *Add to playlist…* from a card's menu, while it's open.
    add_to_playlist: Option<AddToPlaylist>,
}

impl LibraryPage {
    pub fn new(cx: &mut PageCx, source: Source, title: String) -> Self {
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
        let sort_index = SORTS.iter().position(|(key, ..)| *key == view.sort).unwrap_or(0);
        let filter_index = FILTERS.iter().position(|(key, ..)| *key == view.filter).unwrap_or(0);
        let resolution_index = view
            .resolution
            .as_ref()
            .and_then(|resolution| RESOLUTIONS.iter().position(|(key, ..)| key == resolution))
            .unwrap_or(0);
        // Until the library's own values arrive, the remembered ones are the only choices.
        let values = FilterValues {
            genres: view.genres.clone(),
            tags: view.tags.clone(),
            ratings: view.ratings.clone(),
        };
        let years: Vec<String> = view.year.iter().cloned().collect();
        let year_index = usize::from(view.year.is_some());
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
            values_task: None,
            jump: None,
            scroll_to: None,
            loaded_at: Some(cx.services.user_data),
            cursor: Cursor::starting_at(GRID_START),
            columns: 1,
            offset: 0.,
            sort_index,
            filter_index,
            year_index,
            resolution_index,
            values,
            years,
            dialog: None,
            add_to_playlist: None,
        };
        this.apply_view();
        this.request_page(0, cx);
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
                Ok(result) => {
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
        self.values = FilterValues {
            genres: with_chosen(values.genres, &self.view.genres),
            tags: with_chosen(values.tags, &self.view.tags),
            ratings: with_chosen(values.official_ratings, &self.view.ratings),
        };
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
        self.base_query.genres = self.view.genres.clone();
        self.base_query.tags = self.view.tags.clone();
        self.base_query.official_ratings = self.view.ratings.clone();
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
            || !self.view.ratings.is_empty()
            || self.view.year.is_some()
            || self.view.resolution.is_some()
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
        self.request_page(0, cx);
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
        let query = ItemsQuery {
            name_less_than: Some(bound.to_string()),
            start_index: None,
            limit: Some(0),
            ..self.base_query.clone()
        };
        let client = self.client.clone();
        let generation = self.generation;
        self.jump = Some(Task::spawn(cx.ctx, async move {
            JumpResult {
                generation,
                letter,
                highlight,
                total,
                descending,
                result: client.items(&query).await,
            }
        }));
    }

    fn land_from_jump(&mut self, jump: JumpResult) {
        if jump.generation != self.generation {
            return;
        }
        match jump.result {
            Ok(result) => {
                let before = result.total_record_count as usize;
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
            ToolAction::OpenValues => {
                self.dialog = Some(ValueFilters::new());
                cx.ctx.request_repaint();
            }
            ToolAction::Clear => self.clear_filters(cx),
        }
    }

    /// Enter on a toolbar control: a picker steps to its next choice, wrapping round.
    fn step(&mut self, tool: Tool, cx: &mut PageCx) {
        let action = match tool {
            Tool::Sort => ToolAction::Sort((self.sort_index + 1) % SORTS.len()),
            Tool::Show => ToolAction::Show((self.filter_index + 1) % FILTERS.len()),
            Tool::Year => ToolAction::Year((self.year_index + 1) % (self.years.len() + 1)),
            Tool::Resolution => ToolAction::Resolution((self.resolution_index + 1) % RESOLUTIONS.len()),
            Tool::Order => ToolAction::ToggleOrder,
            Tool::Values => ToolAction::OpenValues,
            Tool::Clear => ToolAction::Clear,
        };
        self.apply_tool(action, cx);
    }

    fn clear_filters(&mut self, cx: &mut PageCx) {
        self.view.filter = FILTERS[0].0.into();
        self.filter_index = 0;
        (self.view.year, self.year_index) = (None, 0);
        (self.view.resolution, self.resolution_index) = (None, 0);
        self.view.genres.clear();
        self.view.tags.clear();
        self.view.ratings.clear();
        self.change_view(cx);
        // The Clear button is gone now; from the keyboard, land on the control before it.
        if self
            .cursor
            .selected()
            .is_some_and(|position| position.section == TOOLBAR)
        {
            let last = self.tools().len().saturating_sub(1);
            self.cursor.show_at(Position {
                section: TOOLBAR,
                index: last,
            });
        }
    }

    /// Re-fetches the pages already loaded (e.g. for new watched badges), keeping the current
    /// items on screen until the new ones arrive.
    fn refresh(&mut self, cx: &mut PageCx) {
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
        self.request_page(0, cx);
    }

    fn request_page(&mut self, page: usize, cx: &mut PageCx) {
        if !self.pages_requested.insert(page) {
            return;
        }
        let client = self.client.clone();
        let query = ItemsQuery {
            start_index: Some((page * PAGE_SIZE) as u32),
            ..self.base_query.clone()
        };
        let generation = self.generation;
        self.requests.push(Task::spawn(cx.ctx, async move {
            (generation, page, client.items(&query).await)
        }));
    }

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

    fn toolbar(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let tools = self.tools();
        let muted = ui.visuals().weak_text_color();
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(GRID_PADDING - RING_GAP);
            for (index, tool) in tools.iter().copied().enumerate() {
                if tool == Tool::Show {
                    ui.add_space(4.);
                }
                if let Some(label) = match tool {
                    Tool::Sort => Some("Sort"),
                    Tool::Show => Some("Show"),
                    _ => None,
                } {
                    ui.label(RichText::new(label).small().color(muted));
                }
                let (response, control) = Self::tool(ui, tool, &self.view, &self.years, self.tool_index(tool));
                if let Some(control) = control {
                    action = Some(control);
                }
                if self.cursor.is_selected(TOOLBAR, index) {
                    ring(ui, &response);
                }
            }
            let total = self.total;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(total) = total {
                    ui.label(RichText::new(item_count(total)).small().color(muted));
                }
            });
        });
        if let Some(action) = action {
            self.apply_tool(action, cx);
        }
    }

    /// The selected index of a picker, for its [`Self::tool`].
    fn tool_index(&self, tool: Tool) -> usize {
        match tool {
            Tool::Sort => self.sort_index,
            Tool::Show => self.filter_index,
            Tool::Year => self.year_index,
            Tool::Resolution => self.resolution_index,
            Tool::Order | Tool::Values | Tool::Clear => 0,
        }
    }

    /// One toolbar control: its response and what it asked for when clicked. A free function
    /// for the borrow checker: the toolbar draws with `&self` and acts with `&mut self` after.
    fn tool(
        ui: &mut Ui,
        tool: Tool,
        view: &LibraryView,
        years: &[String],
        selected: usize,
    ) -> (Response, Option<ToolAction>) {
        match tool {
            Tool::Sort => {
                let labels = SORTS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(ui, "library_sort", 160., labels, selected);
                (response, index.map(ToolAction::Sort))
            }
            Tool::Show => {
                let labels = FILTERS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(ui, "library_show", 140., labels, selected);
                (response, index.map(ToolAction::Show))
            }
            Tool::Year => {
                let mut labels = vec!["Any year"];
                labels.extend(years.iter().map(String::as_str));
                let (response, index) = picker(ui, "library_year", 120., labels, selected);
                (response, index.map(ToolAction::Year))
            }
            Tool::Resolution => {
                let labels = RESOLUTIONS.iter().map(|(_, label, ..)| *label).collect();
                let (response, index) = picker(ui, "library_resolution", 150., labels, selected);
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
                let label = values_summary(view);
                let response = ui.add(icon_button(ui, ICON_FILTER_LIST, &label));
                let action = response.clicked().then_some(ToolAction::OpenValues);
                (response, action)
            }
            Tool::Clear => {
                let response = ui.button("Clear filters");
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
                if response.hovered() {
                    let fill = ui.visuals().selection.bg_fill;
                    ui.painter()
                        .rect_filled(rect.shrink2(vec2(4., 0.)), CornerRadius::same(2), fill);
                }
                let color = if response.hovered() {
                    ui.visuals().selection.stroke.color
                } else {
                    ui.visuals().weak_text_color()
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
        let viewport = ui.available_height();
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
        if self.dialog.is_some() {
            let outcome = {
                let dialog = self.dialog.as_mut().expect("checked");
                dialog.show(cx.ctx, &self.values, &mut self.view)
            };
            match outcome {
                Outcome::Stay => {}
                // Each tick reloads the grid behind the dialog.
                Outcome::Changed => self.change_view(cx),
                Outcome::Close => self.dialog = None,
            }
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

/// A menu of `labels`, with `selected` shown: its response and the line clicked.
fn picker(ui: &mut Ui, id: &'static str, width: f32, labels: Vec<&str>, selected: usize) -> (Response, Option<usize>) {
    let selected_text = labels.get(selected).copied().unwrap_or_default();
    let chosen = egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            let mut chosen = None;
            for (index, label) in labels.iter().enumerate() {
                if ui.selectable_label(index == selected, *label).clicked() {
                    chosen = Some(index);
                }
            }
            chosen
        });
    (chosen.response, chosen.inner.flatten())
}

/// What the values button says: what's chosen, or what it's for.
fn values_summary(view: &LibraryView) -> String {
    let chosen: Vec<&String> = view.genres.iter().chain(&view.tags).chain(&view.ratings).collect();
    match chosen.as_slice() {
        [] => "Genres, tags…".to_owned(),
        [one] => (*one).clone(),
        [one, two] => format!("{one}, {two}"),
        many => format!("{} filters", many.len()),
    }
}

/// A button with a material icon before its label, as `main`'s toolbar buttons had.
fn icon_button(ui: &Ui, icon: MaterialIcon, label: &str) -> egui::Button<'static> {
    let body = TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.append(
        icon.codepoint,
        0.,
        TextFormat {
            font_id: FontId::new(body.size + 2., icon.font_family()),
            color,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {label}"),
        0.,
        TextFormat {
            font_id: body,
            color,
            ..Default::default()
        },
    );
    egui::Button::new(WidgetText::LayoutJob(job.into()))
}

/// The keyboard's ring around a toolbar control (`main`'s selection ring).
fn ring(ui: &Ui, response: &Response) {
    ui.painter().rect_stroke(
        response.rect.expand(RING_GAP),
        CornerRadius::same(4),
        Stroke::new(1.5, ui.visuals().selection.stroke.color),
        StrokeKind::Outside,
    );
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
    use super::{Jump, chosen, columns, item_count, jump_target, year_choices, years_of};

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
        // Two posters and the gap between them, plus the padding on both sides.
        assert_eq!(columns(2. * 150. + 16. + 2. * 16.), 2);
        assert_eq!(columns(3. * 150. + 2. * 16. + 2. * 16.), 3);
        // A poster and a bit doesn't fit a second.
        assert_eq!(columns(2. * 150. + 16. + 2. * 16. - 1.), 1);
    }

    #[test]
    fn the_count_reads_naturally() {
        assert_eq!(item_count(0), "0 items");
        assert_eq!(item_count(1), "1 item");
        assert_eq!(item_count(2), "2 items");
    }
}
