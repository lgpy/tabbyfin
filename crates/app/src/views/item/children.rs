//! What's under an item on its page: a series' seasons, a collection's items, a season's
//! episodes and a playlist's entries, and the open season's episodes under a series' seasons.
//!
//! A series' seasons are inset rows (`media_row::season`) in as many columns as fit; the season
//! chosen lists its episodes under them, as a season's own page lists them: one inset
//! `MediaRow` each, with its Watched toggle. A playlist's rows also move and remove their entry,
//! their handles showing on the row under the pointer or the cursor. A collection keeps its
//! poster cards. Picked by C11's mockup over posters with the episodes in an inset panel.

use egui::{Layout, Rect, RichText, Ui, pos2, vec2};
use jellyfin::models::{BaseItemDto, BaseItemKind};

use super::{Asked, BIG_CARD_GAP, BIG_CARD_SCALE, CHILDREN, EPISODES, EntryAction, ItemData, OpenSeason, SECTION_GAP};
use crate::browse::Cursor;
use crate::images::ImageCache;
use crate::item_menu::is_played;
use crate::nav::Route;
use crate::widgets::media_row::{self, Handle, Handles, MediaRow};
use crate::widgets::text::{
    LANDSCAPE_H, Load, POSTER_W, card_text, episode_label, is_placeholder_episode_name, landscape_image, poster_image,
    runtime_text,
};
use crate::widgets::{card, menu, state};

/// Between a list's heading and its first row.
pub(super) const HEADING_GAP: f32 = 12.;

/// The heading over the item's children: a series' seasons, a collection's items, a season's
/// episodes or a playlist's entries, as on `main`. Nothing when there are none.
pub(super) fn children_heading(data: &ItemData) -> Option<&'static str> {
    if data.children.is_empty() {
        return None;
    }
    Some(match data.item.kind {
        BaseItemKind::Series => "Seasons",
        BaseItemKind::BoxSet => "In this collection",
        BaseItemKind::Playlist => "Playlist",
        _ => "Episodes",
    })
}

/// A list's heading, the shelves' size and weight.
pub(super) fn heading(ui: &mut Ui, title: &str) -> Rect {
    ui.label(RichText::new(title).heading().strong()).rect
}

/// How many children a row of `available` points holds: a series' season rows, or a
/// collection's poster cards at [`BIG_CARD_SCALE`], [`BIG_CARD_GAP`] apart (`main`'s count).
pub(super) fn grid_columns(kind: BaseItemKind, available: f32) -> usize {
    if kind == BaseItemKind::Series {
        return media_row::season_columns(available);
    }
    let card = POSTER_W * BIG_CARD_SCALE;
    (((available + BIG_CARD_GAP) / (card + BIG_CARD_GAP)).floor() as usize).max(1)
}

/// `rect` with `gap` points added above it, for a scroll that leaves the section's gap.
pub(super) fn with_gap_above(rect: Rect, gap: f32) -> Rect {
    Rect::from_min_max(pos2(rect.left(), rect.top() - gap), rect.max)
}

/// What a season the keyboard just opened scrolls to: the union of the season's row and the
/// top of its episodes (their heading and first row), so the least scroll shows which season
/// opened and where its episodes start (`None` rather than `TOP`, as the children's heading is
/// revealed). A window too short to hold both shows the episodes' top alone, with the section's
/// gap above it, as a move onto them does.
pub(super) fn band_reveal(row: Option<Rect>, band: Rect, visible: f32) -> Rect {
    match row.map(|row| row.union(band)) {
        Some(union) if union.height() <= visible => union,
        _ => with_gap_above(band, SECTION_GAP),
    }
}

/// A series' seasons as inset rows in rows of `columns`, the open one marked. A row opens or
/// closes its episodes under the seasons; right-clicking one gives the item menu, as on `main`.
/// Returns the row holding the cursor, for the page to scroll into view with its heading.
pub(super) fn seasons(
    ui: &mut Ui,
    data: &ItemData,
    columns: usize,
    open: Option<&str>,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Option<Rect> {
    let columns = columns.max(1);
    let gap = media_row::GAP;
    let width = (ui.available_width() - gap * (columns - 1) as f32) / columns as f32;
    let mut selected_rect = None;
    for (row, children) in data.children.chunks(columns).enumerate() {
        if row > 0 {
            ui.add_space(gap);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (column, season) in children.iter().enumerate() {
                let index = row * columns + column;
                let selected = cursor.is_selected(CHILDREN, index);
                let response = media_row::season(
                    ui,
                    images,
                    season,
                    poster_image(season),
                    &season_line(season),
                    width,
                    open == Some(season.id.as_str()),
                    selected,
                );
                if selected {
                    selected_rect = Some(response.rect);
                }
                if response.clicked() {
                    asked.season = Some(season.clone());
                }
                if let Some(action) = menu::item_menu(&response, season) {
                    asked.menu = Some((action, season.clone()));
                }
            }
        });
    }
    selected_rect
}

/// The line under a season's name: how many episodes it has and how many are left to watch.
fn season_line(season: &BaseItemDto) -> String {
    let mut parts = Vec::new();
    if let Some(count) = season.child_count.filter(|&count| count > 0) {
        parts.push(format!("{count} episode{}", if count == 1 { "" } else { "s" }));
    }
    match season.user_data.as_ref().and_then(|data| data.unplayed_item_count) {
        Some(0) => parts.push("Watched".to_owned()),
        Some(left) => parts.push(format!("{left} to watch")),
        None => {}
    }
    parts.join(" · ")
}

/// A collection's items as poster cards in rows of `columns`, opening their pages, with the
/// item menu. Returns the row holding the cursor, for the page to scroll into view with its
/// heading.
pub(super) fn posters(
    ui: &mut Ui,
    data: &ItemData,
    columns: usize,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Option<Rect> {
    let columns = columns.max(1);
    let selected_row = cursor
        .selected()
        .filter(|position| position.section == CHILDREN)
        .map(|position| position.index / columns);
    let mut row_rect = None;
    for (row, children) in data.children.chunks(columns).enumerate() {
        let drawn = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BIG_CARD_GAP;
            for (column, child) in children.iter().enumerate() {
                let selected = cursor.is_selected(CHILDREN, row * columns + column);
                let response = card::poster_scaled(ui, images, child, BIG_CARD_SCALE, selected);
                if response.clicked() {
                    asked.open = Some(Route::for_item(child));
                }
                if let Some(action) = menu::item_menu(&response, child) {
                    asked.menu = Some((action, child.clone()));
                }
            }
        });
        if selected_row == Some(row) {
            row_rect = Some(drawn.response.rect);
        }
        ui.add_space(BIG_CARD_GAP);
    }
    row_rect
}

/// `items` as inset rows, one per line, the cursor's `section`: a season's episodes or a
/// playlist's entries (with their handles when `playlist`). A click (or Enter) opens the item;
/// the toggle and the handles don't, as on `main`. Returns the rows of the first item and of the
/// cursor's, for the page to scroll into view.
pub(super) fn rows(
    ui: &mut Ui,
    items: &[BaseItemDto],
    section: usize,
    playlist: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rows {
    let len = items.len();
    let mut drawn = Rows::default();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            ui.add_space(media_row::GAP);
        }
        let selected = cursor.is_selected(section, index);
        let (heading, detail) = row_text(item);
        let mut row = MediaRow::new(item, landscape_image(item), &heading, &detail)
            .overview(item.overview.as_deref())
            .played(is_played(item))
            .highlighted(selected);
        if playlist {
            row = row.handles(Handles {
                up: index > 0,
                down: index + 1 < len,
            });
        }
        let response = row.show(ui, images);
        if index == 0 {
            drawn.first = Some(response.row.rect);
        }
        if selected {
            drawn.selected = Some(response.row.rect);
        }
        if let Some(handle) = response.handle {
            let action = match handle {
                Handle::Up => EntryAction::Up,
                Handle::Down => EntryAction::Down,
                Handle::Remove => EntryAction::Remove,
            };
            asked.entry = Some((index, action));
        } else if response.watched.clicked() {
            asked.played = Some((item.id.clone(), !is_played(item)));
        } else if response.row.clicked() {
            asked.open = Some(Route::for_item(item));
        }
        if let Some(action) = menu::item_menu(&response.row, item) {
            asked.menu = Some((action, item.clone()));
        }
    }
    drawn
}

/// Where [`rows`] drew the first row and the cursor's.
#[derive(Default)]
pub(super) struct Rows {
    pub first: Option<Rect>,
    pub selected: Option<Rect>,
}

/// A row's heading and the line under it: episodes read "S1:E2 · Name" over their runtime (just
/// "S1:E3" when Jellyfin only named it "Episode 3"); a movie in a playlist its name over its
/// year and runtime.
fn row_text(item: &BaseItemDto) -> (String, String) {
    let (name, episode_line) = card_text(item);
    let is_episode = item.kind == BaseItemKind::Episode;
    let label = episode_label(item);
    let heading = match is_episode {
        true if is_placeholder_episode_name(&item.name) && !label.is_empty() => label,
        true => episode_line,
        false => name,
    };
    let detail = [
        item.production_year
            .filter(|_| !is_episode)
            .map(|year| year.to_string()),
        item.run_time_ticks.map(runtime_text),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    (heading, detail)
}

/// Where the open season's episodes went: their top (the heading and the first row, which
/// opening a season scrolls to) and the cursor's row.
pub(super) struct Band {
    pub top: Rect,
    pub selected: Option<Rect>,
}

/// The open season's episodes under the seasons: the season's name, then its episodes as rows,
/// the mascot while they load or a line when there are none.
pub(super) fn episodes_band(
    ui: &mut Ui,
    open: &OpenSeason,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Band {
    let top = ui.cursor().min;
    heading(ui, &open.name);
    ui.add_space(HEADING_GAP);
    let (first, selected) = match &open.episodes {
        Load::Loaded(episodes) if !episodes.is_empty() => {
            let drawn = rows(ui, episodes, EPISODES, false, cursor, images, asked);
            (drawn.first, drawn.selected)
        }
        Load::Loaded(_) => (Some(band_note(ui, "No episodes in this season.", false)), None),
        Load::Loading => (Some(band_note(ui, "", true)), None),
        Load::Failed(message) => (Some(band_note(ui, message, false)), None),
    };
    let right = ui.max_rect().right();
    let first_bottom = first.map_or(ui.cursor().min.y, |first| first.bottom());
    Band {
        top: Rect::from_min_max(top, pos2(right, first_bottom)),
        selected,
    }
}

/// A note where the episodes go, at least a row's height: the page's loader (the mascot in its
/// glow) while they load, or a message (C8's `widgets::state`).
fn band_note(ui: &mut Ui, message: &str, loading: bool) -> Rect {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), LANDSCAPE_H),
        Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_min_height(LANDSCAPE_H);
            if loading {
                state::spinner(ui);
            } else {
                state::message(ui, message);
            }
        },
    )
    .response
    .rect
}
