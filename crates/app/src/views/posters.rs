//! I1's check page (debug builds only): a library's items as posters, for seeing the image
//! cache at work — posters with blurhash placeholders, and with `dev/fake-library.py` and
//! `TABBYFIN_KEYS="… down*400"` the queue dropping what scrolled past and the memory budget
//! holding. U5's library page replaces it; X3 deletes it (docs/egui-migration.md §6).
//!
//! Its cards are W1's (`widgets/card.rs`), so what it shows is what pages get.

use jellyfin::SortOrder;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemSortBy};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::views::{Page, PageCx};
use crate::widgets::text::{Load, POSTER_H, POSTER_W};
use crate::widgets::{card, state};

/// Items asked for at once: enough to scroll well past [`crate::images`]' budget (a 400×600
/// poster is about 1 MB decoded).
const LIMIT: u32 = 1000;
const GAP: f32 = 16.;

pub struct PostersPage {
    library_id: String,
    items: Load<Vec<BaseItemDto>>,
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    /// The `services.user_data` the items were loaded at; `None` before the first load.
    loaded_at: Option<u64>,
    cursor: Cursor,
    /// Cards per row, from the last frame: the keys use the layout the page last showed.
    columns: usize,
    /// Where the scroll area is, from the last frame. The page scrolls to the cursor itself:
    /// `show_rows` only draws the rows on screen, so a card that's off screen can't scroll
    /// itself into view (U5's grid needs the real thing).
    offset: f32,
}

impl PostersPage {
    pub fn new(library_id: String) -> Self {
        Self {
            library_id,
            items: Load::Loading,
            loading: None,
            loaded_at: None,
            cursor: Cursor::default(),
            columns: 1,
            offset: 0.,
        }
    }

    fn reload(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        let query = jellyfin::ItemsQuery {
            parent_id: Some(self.library_id.clone()),
            include_item_types: vec![BaseItemKind::Movie],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            sort_order: Some(SortOrder::Ascending),
            limit: Some(LIMIT),
            ..Default::default()
        };
        self.loading = Some(Task::spawn(cx.ctx, async move {
            client.items(&query).await.map(|items| items.items)
        }));
    }

    /// Scrolls the selected row into view, keeping the scroll where the mouse left it otherwise.
    #[allow(clippy::too_many_arguments)]
    fn scroll_to_cursor(
        cursor: &Cursor,
        offset: &mut f32,
        columns: usize,
        viewport: f32,
        card_height: f32,
        pitch: f32,
        rows: usize,
    ) {
        let Some(position) = cursor.selected() else {
            return;
        };
        let Some(row) = position.index.checked_div(columns) else {
            return;
        };
        let top = row as f32 * pitch;
        if top < *offset {
            *offset = top;
        } else if top + card_height > *offset + viewport {
            *offset = top + card_height - viewport;
        }
        let total = rows as f32 * pitch;
        *offset = offset.clamp(0., (total - viewport).max(0.));
    }
}

impl Page for PostersPage {
    fn title(&self) -> &str {
        "Posters"
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
            self.items = match result {
                Ok(items) => Load::Loaded(items),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        browse::hide_on_click(&mut self.cursor, ui);

        match &self.items {
            Load::Loading => state::spinner(ui),
            Load::Failed(message) => {
                if state::error(ui, message.clone()) {
                    self.items = Load::Loading;
                    self.loaded_at = None;
                }
            }
            Load::Loaded(items) if items.is_empty() => state::empty(ui, "This library has no movies."),
            Load::Loaded(items) => {
                let columns = columns(ui.available_width());
                self.columns = columns;
                let rows = items.len().div_ceil(columns);
                // Only the rows on screen ask for their posters, as U5's grid will: asking for
                // everything would queue the whole library and load what nobody sees.
                let card_height = POSTER_H + card::text_height(ui);
                // Rows are `card_height` apart plus the item spacing below; `show_rows` adds
                // that spacing itself, so it takes the height without it.
                let pitch = card_height + GAP;
                // Only a key moves the page to the cursor; a wheel scroll stays where it is.
                // (The selected card can't scroll itself into view: `show_rows` doesn't draw the
                // rows that are off screen.)
                if self.cursor.take_reveal() {
                    Self::scroll_to_cursor(
                        &self.cursor,
                        &mut self.offset,
                        columns,
                        ui.available_height(),
                        card_height,
                        pitch,
                        rows,
                    );
                }
                ui.spacing_mut().item_spacing.y = GAP;
                let mut open = None;
                let out = egui::ScrollArea::vertical()
                    .vertical_scroll_offset(self.offset)
                    .show_rows(ui, card_height, rows, |ui, visible| {
                        for row in visible {
                            let start = row * columns;
                            let cards = &items[start..(start + columns).min(items.len())];
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = GAP;
                                for (column, item) in cards.iter().enumerate() {
                                    let index = start + column;
                                    let selected = self.cursor.is_selected(0, index);
                                    // The cursor's row was scrolled into view above; a card
                                    // that `show_rows` doesn't draw can't scroll itself.
                                    let response = card::poster(ui, &mut cx.services.images, item, selected);
                                    if response.clicked() {
                                        open = Some(Route::for_item(item));
                                    }
                                }
                            });
                        }
                    });
                self.offset = out.state.clamped_offset().y;
                if let Some(route) = open {
                    cx.nav.push(NavRequest::Push(route));
                }
            }
        }
    }
}

impl Browse for PostersPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self) -> Vec<Section> {
        match &self.items {
            Load::Loaded(items) => vec![Section::grid(items.len(), self.columns)],
            _ => Vec::new(),
        }
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Load::Loaded(items) = &self.items
            && let Some(item) = items.get(position.index)
        {
            let route = Route::for_item(item);
            cx.nav.push(NavRequest::Push(route));
        }
    }
}

/// Cards per row that fit `width`.
fn columns(width: f32) -> usize {
    (((width + GAP) / (POSTER_W + GAP)).floor() as usize).max(1)
}
