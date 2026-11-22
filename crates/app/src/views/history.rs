//! Watch history: what the user has played, most recent first, under a heading for each day it
//! was last played, from the account menu.
//!
//! The days are the server's UTC days (the standard library knows no time zone), so a late
//! evening's film can sit under the next day. The page loads again when the watch data changes
//! elsewhere (`services.user_data`): marking something unwatched takes it off the list.

use jellyfin::ItemsQuery;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, ItemFilter, ItemSortBy};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::i18n::tr;
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::views::dialogs::AddToPlaylist;
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction};
use crate::widgets::text::{CARD_GAP, LANDSCAPE_W, Load};
use crate::widgets::{backdrop, card, label, state};

/// How many played items to ask for.
const LIMIT: u32 = 120;
/// The page's side padding, and the gaps round the day headings (as the search results').
const PADDING: f32 = 24.;
const TITLE_GAP: f32 = 12.;
const GROUP_GAP: f32 = 24.;

pub struct HistoryPage {
    /// The `services.user_data` the list was fetched at; a different one loads it again.
    loaded_at: Option<u64>,
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    items: Load<Vec<BaseItemDto>>,
    cursor: Cursor,
    /// The page's width from the last frame, which the columns come from.
    width: f32,
    add_to_playlist: Option<AddToPlaylist>,
}

impl HistoryPage {
    pub fn new() -> Self {
        Self {
            loaded_at: None,
            loading: None,
            items: Load::Loading,
            cursor: Cursor::default(),
            width: 0.,
            add_to_playlist: None,
        }
    }

    fn load(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        let query = ItemsQuery {
            include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Episode],
            recursive: true,
            filters: vec![ItemFilter::IsPlayed],
            sort_by: vec![ItemSortBy::DatePlayed],
            sort_order: Some(jellyfin::SortOrder::Descending),
            fields: vec![ItemField::PrimaryImageAspectRatio, ItemField::CanDownload],
            limit: Some(LIMIT),
            ..Default::default()
        };
        self.loading = Some(Task::spawn(cx.ctx, async move {
            client.items(&query).await.map(|items| items.items)
        }));
    }

    fn days(&self) -> Vec<Day<'_>> {
        match &self.items {
            Load::Loaded(items) => days(items, self.width),
            _ => Vec::new(),
        }
    }
}

impl Default for HistoryPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Page for HistoryPage {
    fn title(&self) -> String {
        tr!("history-title")
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
            self.load(cx);
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.items = match result {
                Ok(items) => Load::Loaded(items),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        self.width = ui.available_width();
        browse::hide_on_click(&mut self.cursor, ui);
        match &self.items {
            Load::Loading => state::spinner(ui),
            Load::Failed(message) => {
                if state::error(ui, message.clone()) {
                    self.items = Load::Loading;
                    self.load(cx);
                }
            }
            Load::Loaded(items) if items.is_empty() => state::empty(ui, tr!("history-empty")),
            Load::Loaded(items) => show_days(ui, items, self.width, &mut self.cursor, &mut self.add_to_playlist, cx),
        }
        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
    }
}

impl Browse for HistoryPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// One section per day, in the order they're drawn.
    fn sections(&self) -> Vec<Section> {
        self.days()
            .iter()
            .map(|day| Section::grid(day.items.len(), day.columns))
            .collect()
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Some(item) = self
            .days()
            .get(position.section)
            .and_then(|day| day.items.get(position.index))
        {
            cx.nav.push(NavRequest::Push(Route::for_item(item)));
        }
    }
}

/// The items last played on one day.
struct Day<'a> {
    /// `2026-10-14`, or empty for items the server has no date for.
    date: String,
    items: Vec<&'a BaseItemDto>,
    columns: usize,
}

/// The day an item was last played, as the server's UTC `YYYY-MM-DD`.
fn played_on(item: &BaseItemDto) -> String {
    item.user_data
        .as_ref()
        .and_then(|data| data.last_played_date.as_deref())
        .and_then(|date| date.get(..10))
        .unwrap_or_default()
        .to_owned()
}

/// `items` (already newest first) cut into days, for `available` px of page width.
fn days(items: &[BaseItemDto], available: f32) -> Vec<Day<'_>> {
    let available = (available - 2. * PADDING).max(0.);
    let columns = (((available + CARD_GAP) / (LANDSCAPE_W + CARD_GAP)).floor() as usize).max(1);
    let mut days: Vec<Day> = Vec::new();
    for item in items {
        let date = played_on(item);
        match days.last_mut() {
            Some(day) if day.date == date => day.items.push(item),
            _ => days.push(Day {
                date,
                items: vec![item],
                columns,
            }),
        }
    }
    days
}

fn show_days(
    ui: &mut egui::Ui,
    items: &[BaseItemDto],
    width: f32,
    cursor: &mut Cursor,
    dialog: &mut Option<AddToPlaylist>,
    cx: &mut PageCx,
) {
    let rows = days(items, width);
    let moved = cursor.take_reveal();
    let mut open = None;
    let mut chosen: Option<(MenuAction, BaseItemDto)> = None;
    let area = egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_space(PADDING);
        for (section, day) in rows.iter().enumerate() {
            if section > 0 {
                ui.add_space(GROUP_GAP);
            }
            ui.horizontal(|ui| {
                ui.add_space(PADDING);
                let heading = if day.date.is_empty() {
                    tr!("history-undated")
                } else {
                    day.date.clone()
                };
                ui.label(label::heading(ui, &heading));
            });
            ui.add_space(TITLE_GAP);
            for (row, cards) in day.items.chunks(day.columns).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(PADDING);
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for (column, item) in cards.iter().enumerate() {
                        let selected = cursor.is_selected(section, row * day.columns + column);
                        let response = card::landscape(ui, &mut cx.services.images, item, selected);
                        if selected && moved {
                            response.scroll_to_me(None);
                        }
                        if response.hovered() {
                            cx.services.prefetch.hover(Route::for_item(item));
                        }
                        if response.clicked() {
                            open = Some(Route::for_item(item));
                        }
                        if let Some(action) = menu::item_menu(&response, item, &cx.services.downloads) {
                            chosen = Some((action, (*item).clone()));
                        }
                    }
                });
                ui.add_space(CARD_GAP);
            }
        }
        ui.add_space(PADDING);
    });
    backdrop::scroll_edge(ui, &area);
    if let Some(route) = open {
        cx.nav.push(NavRequest::Push(route));
    }
    if let Some((action, item)) = chosen
        && let Some(MenuAction::AddToPlaylist) = cx.services.carry_out(action, &item)
    {
        *dialog = Some(AddToPlaylist::new(cx, &item));
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::days;

    fn item(id: &str, played: Option<&str>) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": id,
            "Type": "Movie",
            "UserData": { "Played": true, "PlaybackPositionTicks": 0, "LastPlayedDate": played },
        }))
        .expect("an item")
    }

    #[test]
    fn items_are_cut_into_the_days_they_were_played() {
        let items = [
            item("a", Some("2026-10-14T20:27:46.0298539Z")),
            item("b", Some("2026-10-14T09:00:00Z")),
            item("c", Some("2026-10-13T23:59:59Z")),
            item("d", None),
        ];
        let by_day = days(&items, 1000.);
        let shape: Vec<(&str, usize)> = by_day.iter().map(|day| (day.date.as_str(), day.items.len())).collect();
        assert_eq!(shape, [("2026-10-14", 2), ("2026-10-13", 1), ("", 1)]);
    }
}
