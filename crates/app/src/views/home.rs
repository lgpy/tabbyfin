//! Home. For now it shows the user's libraries and the newest items as shelves of cards: the
//! exemplar page that shows the patterns every page follows. U3 turns it into the real Home
//! (its sections and order), drawing them in the order [`home_sections`] gives.
//!
//! - Data loads through a [`Task`] polled in `show`, never awaited on the UI thread, and the
//!   page shows what it has until new data arrives.
//! - It loads again when `services.user_data` moves on (watch data changed elsewhere), checked
//!   in `show`, so a page under others catches up when it's back on top.
//! - Clicks and Enter open pages through `cx.nav`; Esc going back is the shell's.
//! - The browse cursor: [`Browse`] for the keys, [`browse::reveal`] and
//!   [`browse::hide_on_click`] while drawing.
//! - Cards and shelves are `widgets/`, the item menu `widgets/menu.rs`; what a menu asks for is
//!   carried out here, after the frame's drawing. *Add to playlist…* opens U8's dialog.

use std::collections::HashMap;

use jellyfin::ItemsQuery;
use jellyfin::SortOrder;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, ItemSortBy};

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

/// How many of the newest items the second shelf shows.
const LATEST_LIMIT: u32 = 16;

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

/// What the exemplar page shows.
struct HomeData {
    libraries: Vec<BaseItemDto>,
    /// The newest movies and shows, so the page has item cards (and their menus) to show until
    /// U3 brings Home's real shelves.
    latest: Vec<BaseItemDto>,
}

impl HomeData {
    /// The page's shelves in drawing order: their items. The cursor's sections follow it.
    fn rows(&self) -> [&[BaseItemDto]; 2] {
        [&self.libraries, &self.latest]
    }

    fn section(&self, section: usize) -> Option<&[BaseItemDto]> {
        self.rows().get(section).copied()
    }
}

#[derive(Default)]
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

impl HomePage {
    /// Fetches everything again. What's on screen stays up until it arrives; a load still
    /// running is replaced, which cancels it.
    fn reload(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        self.loading = Some(Task::spawn(cx.ctx, async move {
            let libraries = client.user_views().await?.items;
            let latest = client
                .items(&ItemsQuery {
                    include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series],
                    recursive: true,
                    sort_by: vec![ItemSortBy::DateCreated],
                    sort_order: Some(SortOrder::Descending),
                    fields: vec![ItemField::PrimaryImageAspectRatio],
                    limit: Some(LATEST_LIMIT),
                    ..Default::default()
                })
                .await?
                .items;
            Ok(HomeData { libraries, latest })
        }));
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
            Load::Loaded(data) if data.libraries.is_empty() => {
                state::empty(ui, "Your server has no libraries yet.");
            }
            Load::Loaded(data) => shelves(ui, data, &mut self.cursor, &mut self.add_to_playlist, cx),
        }

        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
    }
}

/// The page's two shelves, then what they asked for.
fn shelves(
    ui: &mut egui::Ui,
    data: &HomeData,
    cursor: &mut Cursor,
    dialog: &mut Option<AddToPlaylist>,
    cx: &mut PageCx,
) {
    let mut opened = None;
    let mut chosen = None;
    let images = &mut cx.services.images;
    egui::ScrollArea::vertical().show(ui, |ui| {
        shelf::shelf(ui, "libraries", "Libraries", |ui| {
            cards(ui, &data.libraries, 0, false, cursor, images, &mut opened, &mut chosen);
        });
        ui.add_space(16.);
        shelf::shelf(ui, "latest", "Recently added", |ui| {
            cards(ui, &data.latest, 1, true, cursor, images, &mut opened, &mut chosen);
        });
    });
    if let Some(route) = opened {
        cx.nav.push(NavRequest::Push(route));
    }
    if let Some((action, item)) = chosen {
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
/// right-click its menu.
#[allow(clippy::too_many_arguments)]
fn cards(
    ui: &mut egui::Ui,
    items: &[BaseItemDto],
    section: usize,
    posters: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    opened: &mut Option<Route>,
    chosen: &mut Option<(MenuAction, BaseItemDto)>,
) {
    for (index, item) in items.iter().enumerate() {
        let selected = cursor.is_selected(section, index);
        let response = if posters {
            card::poster(ui, images, item, selected)
        } else {
            card::landscape(ui, images, item, selected)
        };
        if selected {
            browse::reveal(cursor, &response);
        }
        if response.clicked() {
            *opened = Some(Route::for_item(item));
        }
        if let Some(action) = menu::item_menu(&response, item) {
            *chosen = Some((action, item.clone()));
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
            Load::Loaded(data) => data.rows().iter().map(|items| Section::row(items.len())).collect(),
            _ => Vec::new(),
        }
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Load::Loaded(data) = &self.data
            && let Some(item) = data
                .section(position.section)
                .and_then(|items| items.get(position.index))
        {
            cx.nav.push(NavRequest::Push(Route::for_item(item)));
        }
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
