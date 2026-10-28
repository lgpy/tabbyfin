//! Search results: Movies, Shows and Episodes, each kind in its own grid as wide as the window,
//! for the term the top bar's search box holds.
//!
//! The box is the shell's: typing in it opens this page, and clearing it closes the page again.
//! The page searches the term it was given 300 ms after the last change (`main`'s debounce), so a
//! word typed at speed costs one request; a spinner stands in while the search is out. Enter or
//! Down in the box moves the cursor into the first result (the shell asks with
//! [`Page::enter_results`]); Up or Shift+Tab from the first row leaves the key unused, and the
//! shell puts the keyboard back in the box.
//!
//! Results aren't reloaded when watch data changes elsewhere: they're what the user asked for at
//! the time, and a new search is a keystroke away (`main` didn't reload them either).

use std::time::Duration;

use jellyfin::ItemsQuery;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, ItemSortBy};

use crate::browse::{self, Browse, Cursor, Move, Position, Section};
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::{Deadline, Task};
use crate::views::dialogs::AddToPlaylist;
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction};
use crate::widgets::text::{CARD_GAP, LANDSCAPE_W, Load, POSTER_W};
use crate::widgets::{card, state};

/// How long the box has to be still before the search goes out (`main`'s debounce).
const DEBOUNCE: Duration = Duration::from_millis(300);
/// How many results to ask for, all kinds together.
const LIMIT: u32 = 60;
/// The page's side padding, at the left of every row and the right of the width the grids are
/// laid out in.
const PADDING: f32 = 24.;
/// Between a group's title and its first row, and above every group but the first.
const TITLE_GAP: f32 = 12.;
const GROUP_GAP: f32 = 24.;

pub struct SearchPage {
    /// What the results are for, or will be: the box's term, trimmed.
    term: String,
    /// Due 300 ms after the last change to the box; the search starts when it fires.
    debounce: Deadline,
    /// The search in flight. Replacing it cancels the request, so an older term's results can
    /// never land on a newer one's.
    search: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    results: Load<Vec<BaseItemDto>>,
    cursor: Cursor,
    /// The page's width from the last frame, which the groups' column counts come from: the
    /// keys use the layout the page last showed.
    width: f32,
    /// *Add to playlist…* from a card's menu, while it's open.
    add_to_playlist: Option<AddToPlaylist>,
}

impl Default for SearchPage {
    fn default() -> Self {
        Self {
            term: String::new(),
            debounce: Deadline::default(),
            search: None,
            results: Load::Loading,
            cursor: Cursor::default(),
            width: 0.,
            add_to_playlist: None,
        }
    }
}

impl SearchPage {
    /// The results loaded so far; nothing while searching or after a failure.
    fn items(&self) -> &[BaseItemDto] {
        match &self.results {
            Load::Loaded(items) => items,
            _ => &[],
        }
    }

    /// The groups as they are laid out at the page's width from the last frame.
    fn groups(&self) -> Vec<Group<'_>> {
        groups(self.items(), self.width)
    }

    /// Starts the search for the page's term, now that the box has been still for
    /// [`DEBOUNCE`].
    fn search_now(&mut self, cx: &mut PageCx) {
        if self.term.is_empty() {
            return;
        }
        let client = cx.services.session.client.clone();
        let query = ItemsQuery {
            search_term: Some(self.term.clone()),
            include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series, BaseItemKind::Episode],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            fields: vec![ItemField::PrimaryImageAspectRatio],
            limit: Some(LIMIT),
            ..Default::default()
        };
        self.search = Some(Task::spawn(cx.ctx, async move {
            client.items(&query).await.map(|items| items.items)
        }));
    }
}

impl Page for SearchPage {
    fn title(&self) -> &str {
        "Search"
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Browse)
    }

    fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        browse::on_key(self, action, cx)
    }

    /// A new term: the old results go, the search waits for the box to be still, and the cursor
    /// goes with them (the box has the keyboard). An unchanged term is only Enter reporting a
    /// change, as on `main`: searching again would drop the cursor for nothing.
    fn set_search_term(&mut self, term: &str, cx: &mut PageCx) {
        if term == self.term {
            return;
        }
        self.term = term.to_owned();
        self.results = Load::Loading;
        // The term that request was for is gone; the box's next pause starts a new one.
        self.search = None;
        self.cursor.hide();
        self.debounce.set(DEBOUNCE);
        // The deadline is only checked in `show`, so ask for the frame that fires it.
        cx.ctx.request_repaint_after(DEBOUNCE);
    }

    fn enter_results(&mut self, _cx: &mut PageCx) -> bool {
        if self.groups().is_empty() {
            return false;
        }
        self.cursor.show_at(Position::default());
        true
    }

    fn hide_cursor(&mut self) {
        self.cursor.hide();
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        if self.debounce.fire(cx.ctx) {
            self.search_now(cx);
        }
        if let Some(result) = self.search.as_mut().and_then(Task::poll) {
            self.search = None;
            self.results = match result {
                Ok(items) => Load::Loaded(items),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        self.width = ui.available_width();
        browse::hide_on_click(&mut self.cursor, ui);

        // Only reachable if the page were opened without the box saying anything.
        if self.term.is_empty() {
            state::message(ui, "Type in the search box to search.");
        } else {
            match &self.results {
                Load::Loading => state::spinner(ui),
                // `main` had no Retry here: the box is how a search is repeated.
                Load::Failed(message) => state::message(ui, message.clone()),
                Load::Loaded(items) if items.is_empty() => state::empty(ui, "No results."),
                Load::Loaded(items) => {
                    show_results(ui, items, self.width, &mut self.cursor, &mut self.add_to_playlist, cx)
                }
            }
        }

        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
    }
}

impl Browse for SearchPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// One group per kind of result, in the order they're drawn.
    fn sections(&self) -> Vec<Section> {
        self.groups()
            .iter()
            .map(|group| Section::grid(group.items.len(), group.columns))
            .collect()
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Some(item) = self
            .groups()
            .get(position.section)
            .and_then(|group| group.items.get(position.index))
        {
            cx.nav.push(NavRequest::Push(Route::for_item(item)));
        }
    }

    /// Up from the first row goes back to the search box: the shell focuses it when the page
    /// leaves the key unused, which it does with the cursor out of the way.
    fn at_edge(&mut self, mv: Move, _cx: &mut PageCx) -> bool {
        if matches!(mv, Move::Up | Move::PrevSection) {
            self.cursor.hide();
            return false;
        }
        true
    }
}

/// What a card asked for this frame, carried out once the page is drawn.
#[derive(Default)]
struct Asked {
    open: Option<Route>,
    menu: Option<(MenuAction, BaseItemDto)>,
}

/// One kind of result: its title, its items and how they're laid out.
struct Group<'a> {
    title: &'static str,
    items: Vec<&'a BaseItemDto>,
    /// Episodes are wide stills; movies and shows are posters.
    landscape: bool,
    columns: usize,
}

/// The groups for `available` px of page width, empty ones left out so the cursor never stops
/// on a heading with nothing under it.
fn groups(items: &[BaseItemDto], available: f32) -> Vec<Group<'_>> {
    let available = (available - 2. * PADDING).max(0.);
    [
        ("Movies", BaseItemKind::Movie, false),
        ("Shows", BaseItemKind::Series, false),
        ("Episodes", BaseItemKind::Episode, true),
    ]
    .into_iter()
    .map(|(title, kind, landscape)| {
        let card = if landscape { LANDSCAPE_W } else { POSTER_W };
        Group {
            title,
            items: items.iter().filter(|item| item.kind == kind).collect(),
            landscape,
            columns: columns(available, card),
        }
    })
    .filter(|group| !group.items.is_empty())
    .collect()
}

/// Cards of `card` width that fit `available` px in a row, with [`CARD_GAP`] between them.
fn columns(available: f32, card: f32) -> usize {
    (((available + CARD_GAP) / (card + CARD_GAP)).floor() as usize).max(1)
}

/// The groups in a scrolling page: each heading over its rows of cards, a click (or Enter) on a
/// card opening the item and a right-click its menu.
fn show_results(
    ui: &mut egui::Ui,
    items: &[BaseItemDto],
    width: f32,
    cursor: &mut Cursor,
    dialog: &mut Option<AddToPlaylist>,
    cx: &mut PageCx,
) {
    let rows = groups(items, width);
    // A key moved the cursor this frame: its card scrolls itself into view. The page is the
    // only scroll area here (a row is a plain line of cards), so the card's target is the one it
    // takes.
    let moved = cursor.take_reveal();
    let mut asked = Asked::default();
    // The page takes the whole width, so the scroll bar sits at its right edge rather than at
    // the end of the widest row of cards.
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_space(PADDING);
        for (section, group) in rows.iter().enumerate() {
            if section > 0 {
                ui.add_space(GROUP_GAP);
            }
            ui.horizontal(|ui| {
                ui.add_space(PADDING);
                ui.label(egui::RichText::new(group.title).heading().strong());
            });
            ui.add_space(TITLE_GAP);
            for (row, cards) in group.items.chunks(group.columns).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(PADDING);
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for (column, item) in cards.iter().enumerate() {
                        let selected = cursor.is_selected(section, row * group.columns + column);
                        let response = if group.landscape {
                            card::landscape(ui, &mut cx.services.images, item, selected)
                        } else {
                            card::poster(ui, &mut cx.services.images, item, selected)
                        };
                        if selected && moved {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            asked.open = Some(Route::for_item(item));
                        }
                        if let Some(action) = menu::item_menu(&response, item) {
                            asked.menu = Some((action, (*item).clone()));
                        }
                    }
                });
                ui.add_space(CARD_GAP);
            }
        }
        ui.add_space(PADDING);
    });
    if let Some(route) = asked.open {
        cx.nav.push(NavRequest::Push(route));
    }
    if let Some((action, item)) = asked.menu
        && let Some(MenuAction::AddToPlaylist) = cx.services.carry_out(action, &item)
    {
        *dialog = Some(AddToPlaylist::new(cx, &item));
    }
}
