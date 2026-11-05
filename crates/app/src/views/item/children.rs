//! What's under an item on its page: a series' seasons, a collection's items, a season's
//! episodes and a playlist's entries, and the open season's episodes under a series' seasons.
//!
//! A series' seasons are inset rows (`media_row::season`) in as many columns as fit; the season
//! chosen lists its episodes under them, as a season's own page lists them: one compact inset
//! `MediaRow` each, with its Watched toggle, in as many columns as fit (`media_row::
//! episode_columns`), so a long season isn't a long scroll. A playlist keeps full rows, one per
//! line, which also move and remove their entry,
//! their handles showing on the row under the pointer or the cursor. A collection keeps its
//! poster cards.

use egui::accesskit::Role;
use egui::{Layout, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};
use jellyfin::models::{BaseItemDto, BaseItemKind};

use super::{
    Asked, BIG_CARD_GAP, BIG_CARD_SCALE, CHILDREN, COMPACT_CARD_GAP, COMPACT_SEASON_SCALE, EPISODES, EntryAction,
    ItemData, OpenSeason, SECTION_GAP,
};
use crate::browse::Cursor;
use crate::images::ImageCache;
use crate::item_menu::is_played;
use crate::nav::Route;
use crate::theme::text;
use crate::widgets::focus::{self, Place};
use crate::widgets::list_row::{Handle, Handles};
use crate::widgets::media_row::{self, MediaRow};
use crate::widgets::text::{
    LANDSCAPE_H, Load, POSTER_H, POSTER_W, card_text, episode_label, is_placeholder_episode_name, landscape_image,
    poster_image, runtime_text,
};
use crate::widgets::{card, image, label, menu, shelf, state};
use crate::{motion, theme};

/// Between a list's heading and its first row.
pub(super) const HEADING_GAP: f32 = 12.;

/// The heading over the item's children: a series' seasons, a collection's items, a season's
/// episodes or a playlist's entries. Nothing when there are none.
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
    ui.label(label::heading(ui, title)).rect
}

/// How many children a row of `available` points holds: a series' season rows, or a
/// collection's poster cards at [`BIG_CARD_SCALE`], [`BIG_CARD_GAP`] apart.
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
/// closes its episodes under the seasons; right-clicking one gives the item menu.
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
                if response.hovered() {
                    asked.hovered = Some(Route::for_item(child));
                }
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

/// `items` as inset rows, the cursor's `section`: a season's episodes as compact rows in rows of
/// `columns`, or a playlist's entries as full rows one per line, with their handles. A click (or
/// Enter) opens the item; the toggle and the handles don't. Returns the rows of the
/// first item and of the cursor's, for the page to scroll into view.
#[allow(clippy::too_many_arguments)]
pub(super) fn rows(
    ui: &mut Ui,
    items: &[BaseItemDto],
    section: usize,
    playlist: bool,
    columns: usize,
    stagger: Option<f64>,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rows {
    let columns = if playlist { 1 } else { columns.max(1) };
    // A phone's rows have no fill, so they sit further apart.
    let phone = !playlist && crate::layout::compact(ui.ctx());
    let gap = if phone { media_row::PHONE_GAP } else { media_row::GAP };
    let width = (ui.available_width() - gap * (columns - 1) as f32) / columns as f32;
    let mut drawn = Rows::default();
    let now = ui.input(|input| input.time);
    let mut fading = false;
    for (line, chunk) in items.chunks(columns).enumerate() {
        if line > 0 {
            ui.add_space(gap);
        }
        let alpha = stagger.map_or(1., |started| {
            motion::progress(started, now, motion::row_delay(line), motion::ENTER)
        });
        fading |= alpha < 1.;
        motion::faded(ui, alpha, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (column, item) in chunk.iter().enumerate() {
                    let index = line * columns + column;
                    ui.allocate_ui(vec2(width, 0.), |ui| {
                        ui.set_width(width);
                        row(
                            ui, items, index, item, section, playlist, cursor, images, asked, &mut drawn,
                        );
                    });
                }
            });
        });
    }
    if fading {
        ui.ctx().request_repaint();
    }
    drawn
}

/// One of [`rows`]' rows, `item` at `index` in `items`.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    items: &[BaseItemDto],
    index: usize,
    item: &BaseItemDto,
    section: usize,
    playlist: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
    drawn: &mut Rows,
) {
    let len = items.len();
    let selected = cursor.is_selected(section, index);
    let phone = !playlist && crate::layout::compact(ui.ctx());
    let (heading, detail) = if phone { phone_row_text(item) } else { row_text(item) };
    let mut row = MediaRow::new(item, landscape_image(item), &heading, &detail)
        .overview(item.overview.as_deref())
        .played(is_played(item))
        .highlighted(selected)
        .compact(!playlist)
        .phone(phone);
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
    if response.row.hovered() {
        asked.hovered = Some(Route::for_item(item));
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

/// A phone row's heading and detail: an episode's number and name ("2. The Long Tide", or just
/// "Episode 3" when Jellyfin only numbered it) over its runtime, as the season around it already
/// says which season it is.
fn phone_row_text(item: &BaseItemDto) -> (String, String) {
    if item.kind != BaseItemKind::Episode {
        return row_text(item);
    }
    let heading = match item.index_number {
        Some(number) if !is_placeholder_episode_name(&item.name) => format!("{number}. {}", item.name),
        _ => item.name.clone(),
    };
    (heading, item.run_time_ticks.map(runtime_text).unwrap_or_default())
}

/// A phone's seasons: a shelf of their posters, the open one ringed in `brand` with its name in
/// `brand`, each with how many episodes are left under its name. A tap opens a season's
/// episodes under the shelf; a long press gives the item menu. Returns the shelf's rectangle.
#[allow(clippy::too_many_arguments)]
pub(super) fn season_shelf(
    ui: &mut Ui,
    data: &ItemData,
    open: Option<&str>,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let size = vec2(POSTER_W, POSTER_H) * COMPACT_SEASON_SCALE;
    shelf::shelf_with_gaps(ui, "seasons", "Seasons", COMPACT_CARD_GAP, HEADING_GAP, 0., |ui| {
        for (index, season) in data.children.iter().enumerate() {
            let selected = cursor.is_selected(CHILDREN, index);
            let response = season_card(ui, images, season, size, open == Some(season.id.as_str()), selected);
            if selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.season = Some(season.clone());
            }
            if let Some(action) = menu::item_menu(&response, season) {
                asked.menu = Some((action, season.clone()));
            }
        }
    })
}

/// Room round a season's poster for its ring, which the shelf would otherwise clip.
const SEASON_RING_ROOM: f32 = 4.;

/// One of [`season_shelf`]'s cards: the poster `size`, then the name and the line under it,
/// centred.
fn season_card(
    ui: &mut Ui,
    images: &mut ImageCache,
    season: &BaseItemDto,
    size: egui::Vec2,
    open: bool,
    selected: bool,
) -> egui::Response {
    let tokens = theme::tokens_of(ui);
    let room = SEASON_RING_ROOM;
    // One short line under a narrow card: what's left, or how many there are.
    let line = match season.user_data.as_ref().and_then(|data| data.unplayed_item_count) {
        Some(0) => "Watched".to_owned(),
        Some(left) => format!("{left} to watch"),
        None => season
            .child_count
            .filter(|&count| count > 0)
            .map(|count| format!("{count} episode{}", if count == 1 { "" } else { "s" }))
            .unwrap_or_default(),
    };
    let cell = vec2(size.x + 2. * room, size.y + 2. * room + 44.);
    let (rect, response) = ui.allocate_exact_size(cell, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, open, &season.name));
    let poster = Rect::from_min_size(rect.min + vec2(room, room), size);
    image::show(
        ui,
        images,
        poster,
        poster_image(season).as_ref(),
        Some(season),
        image::RADIUS,
    );
    if response.is_pointer_button_down_on() {
        image::dim(ui, poster, image::RADIUS);
    }
    if open {
        ui.painter().rect_stroke(
            poster.expand(2.),
            image::RADIUS + 2,
            Stroke::new(2., tokens.brand),
            StrokeKind::Outside,
        );
    } else if selected {
        focus::ring(ui, poster, image::RADIUS as f32, Place::Inside);
    }
    let center = rect.center().x;
    let name_color = if open { tokens.brand } else { tokens.ink };
    let name = ui
        .painter()
        .layout_no_wrap(season.name.clone(), text::font(ui, "sm"), name_color);
    let y = poster.bottom() + room + 6.;
    let name_h = name.size().y;
    ui.painter()
        .galley(pos2(center - name.size().x / 2., y), name, name_color);
    if !line.is_empty() {
        ui.painter().text(
            pos2(center, y + name_h + 2.),
            egui::Align2::CENTER_TOP,
            line,
            text::font(ui, "xs"),
            tokens.ink_muted,
        );
    }
    response
}

/// Where the open season's episodes went: their top (the heading and the first row, which
/// opening a season scrolls to) and the cursor's row.
pub(super) struct Band {
    pub top: Rect,
    pub selected: Option<Rect>,
}

/// The open season's episodes under the seasons: the season's name, then its episodes as rows
/// in rows of `columns`,
/// the mascot while they load or a line when there are none.
pub(super) fn episodes_band(
    ui: &mut Ui,
    open: &OpenSeason,
    columns: usize,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Band {
    let top = ui.cursor().min;
    let ctx = ui.ctx().clone();
    // The season's name fades in as it opens (another season opening in its place included),
    // then what's under it, its rows one line after another as they arrive.
    let animations = theme::animations(&ctx);
    let now = ctx.input(|input| input.time);
    let opened = motion::appear(&ctx, ("season-band", open.id.as_str()));
    let name_alpha = if animations {
        motion::progress(opened, now, 0., motion::FADE)
    } else {
        1.
    };
    motion::faded(ui, name_alpha, |ui| heading(ui, &open.name));
    ui.add_space(HEADING_GAP);
    let (first, selected) = match &open.episodes {
        Load::Loaded(episodes) if !episodes.is_empty() => {
            let stagger = animations.then(|| motion::appear(&ctx, ("season-rows", open.id.as_str())));
            let drawn = rows(ui, episodes, EPISODES, false, columns, stagger, cursor, images, asked);
            (drawn.first, drawn.selected)
        }
        Load::Loaded(_) => (Some(band_note(ui, "No episodes in this season.", false)), None),
        Load::Loading => (Some(band_note(ui, "", true)), None),
        Load::Failed(message) => (Some(band_note(ui, message, false)), None),
    };
    if name_alpha < 1. {
        ctx.request_repaint();
    }
    let right = ui.max_rect().right();
    let first_bottom = first.map_or(ui.cursor().min.y, |first| first.bottom());
    Band {
        top: Rect::from_min_max(top, pos2(right, first_bottom)),
        selected,
    }
}

/// A note where the episodes go, at least a row's height: the page's loader (the mascot in its
/// glow) while they load, or a message (`widgets::state`).
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
