//! Home: libraries, Continue Watching, Next Up, Favorites, and Latest per library, in the order
//! and with the libraries the user set in the web client's Settings → Home.
//!
//! The page is the exemplar the other pages copy:
//!
//! - Data loads through a [`Task`] polled in `show`, never awaited on the UI thread, and the
//!   page shows what it has until new data arrives.
//! - It loads again when `services.user_data` moves on (watch data changed elsewhere), checked
//!   in `show`, so a page under others catches up when it's back on top.
//! - Clicks and Enter open pages through `cx.nav`; Esc going back is the shell's.
//! - The browse cursor: [`Browse`] for the keys, `Cursor::take_reveal` and
//!   [`browse::hide_on_click`] while drawing.
//! - Cards and shelves are `widgets/`, the item menu `widgets/menu.rs`; what a menu asks for is
//!   carried out here, after the frame's drawing. *Add to playlist…* opens U8's dialog.
//!
//! `main` staggered the shelves in as the page appeared; egui draws them all at once until R2
//! brings the animations back.

use std::collections::HashMap;

use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType, ItemField, ItemFilter, ItemSortBy};
use jellyfin::{Client, ItemsQuery};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::images::ImageCache;
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::views::dialogs::AddToPlaylist;
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction};
use crate::widgets::text::Load;
use crate::widgets::{card, shelf, state};

/// How many items each shelf asks for.
const SHELF_LIMIT: u32 = 24;
const FIELDS: &[ItemField] = &[ItemField::PrimaryImageAspectRatio];
/// Home's cards are larger than the other pages', with more room between them.
const CARD_SCALE: f32 = 1.2;
const CARD_GAP: f32 = 24.;
/// Home's cards start closer under their shelf's title than the other pages' (`main`'s
/// `shelf_with_rail` argument of 8 px).
const TITLE_GAP: f32 = 8.;
/// The page's top and bottom padding, and the space between its shelves (`main`'s `py_6` and
/// `gap_8`).
const PAGE_PADDING: f32 = 24.;
const SHELF_GAP: f32 = 32.;

/// The Home sections of the web client's Settings → Home that this app has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSection {
    MyMedia,
    Resume,
    NextUp,
    Latest,
}

/// The web client's section for each of its ten Home slots when the user hasn't picked one.
pub const DEFAULT_SECTIONS: [&str; 10] = [
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
/// preferences. Sections this app doesn't have (music, books, live TV) are skipped, and a
/// section listed twice keeps its first place.
pub fn home_sections(prefs: &HashMap<String, String>) -> Vec<HomeSection> {
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
    // One request after another, so a server with many libraries isn't asked for all at once.
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

/// One row of cards on the page, in display order.
struct Shelf<'a> {
    id: ShelfId,
    title: String,
    items: &'a [BaseItemDto],
    landscape: bool,
}

/// A shelf's identity, so each row keeps its own scroll position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ShelfId {
    Views,
    Resume,
    NextUp,
    Favorites,
    /// The shelf of the library at this index in `HomeData::latest`.
    Latest(usize),
}

/// The shelves in the user's section order. Favorites, which the web client has no section
/// for, comes just before the Latest shelves, or last without them. Empty shelves are left out,
/// so the cursor never stops on a row with nothing in it.
fn shelves(data: &HomeData) -> Vec<Shelf<'_>> {
    let favorites = || row(ShelfId::Favorites, "Favorites".to_owned(), &data.favorites, false);
    let mut shelves = Vec::new();
    for section in &data.sections {
        match section {
            HomeSection::MyMedia => shelves.push(row(ShelfId::Views, "My Media".to_owned(), &data.views, true)),
            HomeSection::Resume => {
                shelves.push(row(ShelfId::Resume, "Continue Watching".to_owned(), &data.resume, true))
            }
            HomeSection::NextUp => shelves.push(row(ShelfId::NextUp, "Next Up".to_owned(), &data.next_up, true)),
            HomeSection::Latest => {
                shelves.push(favorites());
                shelves.extend(
                    data.latest.iter().enumerate().map(|(i, (view, items))| {
                        row(ShelfId::Latest(i), format!("Latest {}", view.name), items, false)
                    }),
                );
            }
        }
    }
    if !data.sections.contains(&HomeSection::Latest) {
        shelves.push(favorites());
    }
    shelves.retain(|shelf| !shelf.items.is_empty());
    shelves
}

fn row<'a>(id: ShelfId, title: String, items: &'a [BaseItemDto], landscape: bool) -> Shelf<'a> {
    Shelf {
        id,
        title,
        items,
        landscape,
    }
}

pub struct HomePage {
    data: Load<HomeData>,
    loading: Option<Task<jellyfin::Result<HomeData>>>,
    /// The `services.user_data` the data was loaded at; `None` before the first load or after
    /// Retry.
    loaded_at: Option<u64>,
    cursor: Cursor,
    /// *Add to playlist…* from a card's menu, while it's open.
    add_to_playlist: Option<AddToPlaylist>,
}

impl Default for HomePage {
    fn default() -> Self {
        Self {
            data: Load::Loading,
            loading: None,
            loaded_at: None,
            cursor: Cursor::default(),
            add_to_playlist: None,
        }
    }
}

impl HomePage {
    /// Fetches everything again. What's on screen stays up until it arrives; a load still
    /// running is replaced, which cancels it.
    fn reload(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        self.loading = Some(Task::spawn(cx.ctx, load(client)));
    }
}

impl Page for HomePage {
    fn title(&self) -> &str {
        "Home"
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Browse)
    }

    fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        browse::on_key(self, action, cx)
    }

    fn hide_cursor(&mut self) {
        self.cursor.hide();
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        if self.loaded_at != Some(cx.services.user_data) {
            self.reload(cx);
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.data = match result {
                Ok(data) => Load::Loaded(data),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        browse::hide_on_click(&mut self.cursor, ui);

        match &self.data {
            Load::Loading => state::spinner(ui),
            Load::Failed(message) => {
                let retry = state::error(ui, message.clone());
                if retry {
                    self.data = Load::Loading;
                    self.loaded_at = None;
                }
            }
            // `main` showed the same line whenever there was no shelf to draw, whether the
            // server has no libraries or the user hid every section in the web client.
            Load::Loaded(data) if shelves(data).is_empty() => {
                state::empty(ui, "Your server has no libraries yet.");
            }
            Load::Loaded(data) => show_shelves(ui, data, &mut self.cursor, &mut self.add_to_playlist, cx),
        }

        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
    }
}

/// What the cards asked for this frame, carried out once the page is drawn: a page to open and
/// a menu action to take.
#[derive(Default)]
struct Asked {
    open: Option<Route>,
    menu: Option<(MenuAction, BaseItemDto)>,
}

/// The page's shelves, then what they asked for.
fn show_shelves(
    ui: &mut egui::Ui,
    data: &HomeData,
    cursor: &mut Cursor,
    dialog: &mut Option<AddToPlaylist>,
    cx: &mut PageCx,
) {
    let rows = shelves(data);
    let mut asked = Asked::default();
    // A key moved the cursor this frame: the card scrolls itself into view inside its shelf, and
    // the page scrolls to the whole shelf. The page's target has to wait until every shelf is
    // drawn, because the next scroll area to finish takes it — which would otherwise be another
    // shelf's own row (a horizontal area, which ignores the vertical part).
    let moved = cursor.take_reveal();
    let selected = cursor.selected();
    let mut target = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(PAGE_PADDING);
        let images = &mut cx.services.images;
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                ui.add_space(SHELF_GAP);
            }
            let shelf = shelf::shelf_with_gaps(ui, row.id, &row.title, CARD_GAP, TITLE_GAP, |ui| {
                cards(
                    ui,
                    row.items,
                    index,
                    row.landscape,
                    moved,
                    &mut *cursor,
                    &mut *images,
                    &mut asked,
                );
            });
            if moved && selected.is_some_and(|position| position.section == index) {
                target = Some(shelf);
            }
        }
        ui.add_space(PAGE_PADDING);
        if let Some(rect) = target {
            ui.scroll_to_rect(rect, None);
        }
    });
    if let Some(route) = asked.open {
        cx.nav.push(NavRequest::Push(route));
    }
    if let Some((action, item)) = asked.menu {
        act(action, &item, dialog, cx);
    }
}

/// What a card's menu asked for: the services play it or mark it, and *Add to playlist…* opens
/// the dialog, which this page then keeps until it's done.
fn act(action: MenuAction, item: &BaseItemDto, dialog: &mut Option<AddToPlaylist>, cx: &mut PageCx) {
    if let Some(MenuAction::AddToPlaylist) = cx.services.carry_out(action, item) {
        *dialog = Some(AddToPlaylist::new(cx, item));
    }
}

/// One shelf's cards, in the cursor's section: a click (or Enter) opens the item's page and a
/// right-click its menu. Home's cards are [`CARD_SCALE`] times the other pages' size. `moved`
/// says a key moved the cursor this frame, so the selected card scrolls into view inside its
/// shelf.
#[allow(clippy::too_many_arguments)]
fn cards(
    ui: &mut egui::Ui,
    items: &[BaseItemDto],
    section: usize,
    landscape: bool,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) {
    for (index, item) in items.iter().enumerate() {
        let selected = cursor.is_selected(section, index);
        let response = if landscape {
            card::landscape_scaled(ui, images, item, CARD_SCALE, selected)
        } else {
            card::poster_scaled(ui, images, item, CARD_SCALE, selected)
        };
        if selected && moved {
            response.scroll_to_me(None);
        }
        if response.clicked() {
            asked.open = Some(Route::for_item(item));
        }
        if let Some(action) = menu::item_menu(&response, item) {
            asked.menu = Some((action, item.clone()));
        }
    }
}

impl Browse for HomePage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// One row per shelf.
    fn sections(&self) -> Vec<Section> {
        match &self.data {
            Load::Loaded(data) => shelves(data)
                .iter()
                .map(|shelf| Section::row(shelf.items.len()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Load::Loaded(data) = &self.data
            && let Some(item) = shelves(data)
                .get(position.section)
                .and_then(|shelf| shelf.items.get(position.index))
        {
            cx.nav.push(NavRequest::Push(Route::for_item(item)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use jellyfin::models::BaseItemDto;

    use super::{HomeData, HomeSection, ShelfId, home_sections, shelves};

    fn item(id: &str) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({ "Id": id, "Name": id, "Type": "Movie" })).expect("item")
    }

    fn data(sections: Vec<HomeSection>) -> HomeData {
        HomeData {
            sections,
            views: vec![item("library")],
            resume: vec![item("resume")],
            next_up: vec![item("next")],
            favorites: vec![item("favorite")],
            latest: vec![(item("movies"), vec![item("latest")])],
        }
    }

    fn shown(data: &HomeData) -> Vec<(ShelfId, String)> {
        shelves(data).into_iter().map(|shelf| (shelf.id, shelf.title)).collect()
    }

    fn name(id: ShelfId, title: &str) -> (ShelfId, String) {
        (id, title.to_owned())
    }

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

    #[test]
    fn favorites_go_just_before_the_latest_shelves() {
        let data = data(home_sections(&HashMap::new()));
        assert_eq!(
            shown(&data),
            [
                name(ShelfId::Views, "My Media"),
                name(ShelfId::Resume, "Continue Watching"),
                name(ShelfId::NextUp, "Next Up"),
                name(ShelfId::Favorites, "Favorites"),
                name(ShelfId::Latest(0), "Latest movies"),
            ]
        );
    }

    #[test]
    fn favorites_go_last_without_latest() {
        let data = data(vec![HomeSection::NextUp, HomeSection::MyMedia]);
        assert_eq!(
            shown(&data),
            [
                name(ShelfId::NextUp, "Next Up"),
                name(ShelfId::Views, "My Media"),
                name(ShelfId::Favorites, "Favorites"),
            ]
        );
    }

    #[test]
    fn empty_shelves_are_left_out() {
        let mut data = data(vec![HomeSection::Resume]);
        data.resume.clear();
        data.favorites.clear();
        data.latest.clear();
        assert!(shown(&data).is_empty());
    }
}
