//! Detail page for a movie, series, season or episode, and for collections and playlists
//! (whose entries can be reordered and removed here). The page, its header, actions, pickers and
//! shelves are U6's; its seasons, episodes and playlist entries are U7's, which fills the two
//! empty cursor sections left for them here.
//!
//! The page follows `views/home.rs`: data loads through a [`Task`] polled in `show` and what's on
//! screen stays until new data arrives, the page loads again when `services.user_data` moves on,
//! the browse cursor moves over its sections, and what the drawing asked for is carried out
//! after the frame.
//!
//! Watched and Favorite show at once and go back if the server refuses: the services remember
//! the refusal in a numbered log (`Services::refused_marks`) and the page takes the change back
//! whenever it next draws, since a reload would fail too while the server is down. `main`'s pop on the icons is an animation
//! (R2) and waits for it.
//!
//! The More menu is drawn by the page rather than egui's menu buttons, so the keyboard can walk
//! it (`more_menu_keys`), as it could on `main`; egui keeps the keys while the popup is open.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Label, Layout, Margin, Rect, RectAlign, Response, RichText,
    Sense, Stroke, StrokeKind, TextFormat, TextStyle, Ui, UiBuilder, WidgetText, pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ARROW_DROP_DOWN, ICON_DELETE, ICON_EDIT, ICON_MORE_HORIZ, ICON_MOVIE, ICON_OPEN_IN_NEW, ICON_PLAY_ARROW,
    ICON_PLAYLIST_ADD, ICON_REPLAY, ICON_SHUFFLE,
};
use jellyfin::models::{BaseItemDto, BaseItemKind, ChapterInfo, ItemField, ItemSortBy, MediaSourceInfo};
use jellyfin::{Client, ItemsQuery, Ticks};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::images::ImageCache;
use crate::item_menu::{
    Mark, can_add_to_playlist, episodes_of, first_unwatched, is_favorite, is_playable, is_played, resume_point,
};
use crate::keys::{self, Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::playback::PlayOptions;
use crate::task::Task;
use crate::toasts::Toast;
use crate::views::dialogs::{AddToPlaylist, DeletePlaylist, EditPlaylist};
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction, RefusedMarks};
use crate::widgets::text::{CARD_GAP, Load, episode_label, runtime_text, timestamp};
use crate::widgets::{card, image, shelf, state};

/// Fisher-Yates with a random seed (uuid's v4 is the randomness the app already has).
pub fn shuffle<T>(items: &mut [T]) {
    let mut state = uuid::Uuid::new_v4().as_u128() as u64 | 1;
    for i in (1..items.len()).rev() {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(i, (state % (i as u64 + 1)) as usize);
    }
}

/// A collection's movies (and shows) by release date, as its page lists them. Playback's
/// previous and next movie follow the same order.
pub async fn collection_items(
    client: &Client,
    collection_id: &str,
    fields: &[ItemField],
) -> jellyfin::Result<Vec<BaseItemDto>> {
    let query = ItemsQuery {
        parent_id: Some(collection_id.to_owned()),
        sort_by: vec![ItemSortBy::PremiereDate, ItemSortBy::SortName],
        fields: fields.to_vec(),
        ..Default::default()
    };
    Ok(client.items(&query).await?.items)
}

/// `2024 · 1h 42m · PG-13 · ★ 7.8`
pub fn meta_line(item: &BaseItemDto) -> String {
    let mut parts = Vec::new();
    if item.kind == BaseItemKind::Episode {
        let label = episode_label(item);
        if !label.is_empty() {
            parts.push(label);
        }
    }
    if let Some(year) = item.production_year {
        parts.push(year.to_string());
    }
    if let Some(ticks) = item.run_time_ticks.filter(|t| t.0 > 0) {
        parts.push(runtime_text(ticks));
    }
    if let Some(rating) = &item.official_rating {
        parts.push(rating.clone());
    }
    if let Some(score) = item.community_rating {
        parts.push(format!("★ {score:.1}"));
    }
    parts.join(" · ")
}

/// The chapters worth a shelf: a playable item's, when it has more than one.
pub fn chapters(item: &BaseItemDto) -> &[ChapterInfo] {
    let playable = matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
    );
    if playable && item.chapters.len() > 1 {
        &item.chapters
    } else {
        &[]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemAction {
    Resume(Ticks),
    PlayFromStart,
    Play,
    Trailer,
    TogglePlayed,
    ToggleFavorite,
    /// Collections and playlists: everything in order, or shuffled.
    PlayAll,
    Shuffle,
    AddToPlaylist,
    /// Rename or share. Offered on every playlist; the server refuses other users' ones.
    EditPlaylist,
    /// Only offered when the server says the user may delete it.
    DeletePlaylist,
    /// Opens a menu of the item's less frequent actions ([`more_actions`]).
    More,
}

/// The buttons under an item's title, in order. A series or season plays `up_next`, its next
/// episode.
pub fn item_actions(item: &BaseItemDto, up_next: Option<&BaseItemDto>) -> Vec<ItemAction> {
    let (playable, resume_at) = match up_next {
        Some(episode) => (true, resume_point(episode)),
        None => (is_playable(item), resume_point(item)),
    };
    let mut actions = match (playable, resume_at) {
        _ if matches!(item.kind, BaseItemKind::BoxSet | BaseItemKind::Playlist) => {
            vec![ItemAction::PlayAll, ItemAction::Shuffle]
        }
        (false, _) => Vec::new(),
        (true, Some(position)) => vec![ItemAction::Resume(position), ItemAction::PlayFromStart],
        (true, None) => vec![ItemAction::Play],
    };
    if up_next.is_some() {
        actions.push(ItemAction::Shuffle);
    }
    if item.local_trailer_count.unwrap_or(0) > 0 || !item.remote_trailers.is_empty() {
        actions.push(ItemAction::Trailer);
    }
    actions.push(ItemAction::TogglePlayed);
    actions.push(ItemAction::ToggleFavorite);
    if !more_actions(item).is_empty() {
        actions.push(ItemAction::More);
    }
    actions
}

/// What the More menu offers, in order.
pub fn more_actions(item: &BaseItemDto) -> Vec<ItemAction> {
    let mut actions = Vec::new();
    if can_add_to_playlist(item) {
        actions.push(ItemAction::AddToPlaylist);
    }
    if item.kind == BaseItemKind::Playlist {
        actions.push(ItemAction::EditPlaylist);
    }
    if item.kind == BaseItemKind::Playlist && item.can_delete == Some(true) {
        actions.push(ItemAction::DeletePlaylist);
    }
    actions
}

/// What the page loads for an item: the item itself, what's under it, and the shelves below.
struct ItemData {
    item: BaseItemDto,
    /// Seasons of a series, episodes of a season, a collection's or playlist's entries.
    children: Vec<BaseItemDto>,
    /// Local trailers, then featurettes and other special features.
    extras: Vec<BaseItemDto>,
    /// "More like this".
    similar: Vec<BaseItemDto>,
    /// The episode a series' or season's Play starts ([`item_menu::first_unwatched`]).
    up_next: Option<BaseItemDto>,
}

/// How many items "More like this" shows.
const SIMILAR_LIMIT: u32 = 12;
/// The backdrop's height: the header's content starts [`BACKDROP_H`] − 160 down, so the poster
/// reaches under it.
const BACKDROP_H: f32 = 340.;
/// The width the backdrop is fetched at, as `main` did. A fixed width keeps the image cache key
/// (and so the download and its cache entry) the same while the window is resized, maximised or
/// zoomed; it is scaled to the band it fills.
const BACKDROP_FILL_W: u32 = 1280;
/// The narrowest the title gets beside the actions before they move under it.
const TITLE_MIN_W: f32 = 280.;
const DETAIL_POSTER_W: f32 = 200.;
const DETAIL_POSTER_H: f32 = 300.;
const PICKER_W: f32 = 260.;
const PICKER_H: f32 = 28.;
/// The version label's column, as on `main`.
const PICKER_LABEL_W: f32 = 90.;
/// Between the page's sections.
const SECTION_GAP: f32 = 24.;
const PAGE_PADDING: f32 = 24.;
const BOTTOM_PADDING: f32 = 32.;
/// Cast & crew and "More like this": larger cards, further apart, like Home's.
const BIG_CARD_SCALE: f32 = 1.2;
const BIG_CARD_GAP: f32 = 24.;
/// Between a shelf's title and its cards.
const SHELF_TITLE_GAP: f32 = 16.;
const ACTION_SIZE: f32 = 48.;
const ACTION_ICON: f32 = 24.;
/// Between the action buttons, whose cursor rings take 3 px on each side.
const ACTION_GAP: f32 = 16.;
const ACTION_RING_GAP: f32 = 3.;
/// The More menu's width.
const MENU_W: f32 = 220.;

/// Cursor sections, top to bottom, as on `main`. The children rows and the open season's
/// episodes are U7's; until then their sections are empty, so the cursor skips them.
const ACTIONS: usize = 0;
/// The version picker; Left and Right change it.
const PICKERS: usize = 1;
const CHAPTERS: usize = 2;
const CHILDREN: usize = 3;
const EPISODES: usize = 4;
const PEOPLE: usize = 5;
const EXTRAS: usize = 6;
const SIMILAR: usize = 7;

pub struct ItemPage {
    /// The top bar's title: the route's name until the item loads.
    name: String,
    item_id: String,
    data: Load<ItemData>,
    loading: Option<Task<jellyfin::Result<ItemData>>>,
    /// The `services.user_data` the data was loaded at; `None` before the first load or after
    /// Retry.
    loaded_at: Option<u64>,
    /// The chosen media source, an index into the item's versions. Used only when there is
    /// more than one (`selected_version`).
    version: usize,
    cursor: Cursor,
    /// The More menu's highlight while the keyboard is on it (`None` when a click opened it,
    /// until an arrow key picks one).
    more_highlight: Option<usize>,
    /// The More menu was asked for this frame (the button, or Enter on the cursor): its popup
    /// opens before the frame draws it.
    more_requested: bool,
    /// The last [`RefusedMarks`] sequence this page has applied. Refusals after it are taken
    /// back as the page draws, even ones that came while it was off screen; a page made after a
    /// refusal starts at the current sequence.
    marks_seen: u64,
    add_to_playlist: Option<AddToPlaylist>,
    edit_playlist: Option<EditPlaylist>,
    delete_playlist: Option<DeletePlaylist>,
}

impl ItemPage {
    /// `marks_seen` is the services' refusal sequence when the page is made
    /// (`RefusedMarks::sequence`): a page opened after a refusal shows the server's state and
    /// has nothing to take back.
    pub fn new(item_id: String, name: String, marks_seen: u64) -> Self {
        Self {
            name,
            item_id,
            data: Load::Loading,
            loading: None,
            loaded_at: None,
            version: 0,
            cursor: Cursor::default(),
            more_highlight: None,
            more_requested: false,
            marks_seen,
            add_to_playlist: None,
            edit_playlist: None,
            delete_playlist: None,
        }
    }

    /// Fetches the item again. What's on screen stays up until the new data arrives; a load
    /// still running is replaced, which cancels it.
    fn reload(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        let item_id = self.item_id.clone();
        self.loading = Some(Task::spawn(cx.ctx, load(client, item_id)));
    }

    /// Plays the item, a series' or season's next episode, or from `start` (a chapter) with the
    /// chosen version's tracks left to the server's defaults.
    fn play(&mut self, start: Option<Ticks>, cx: &mut PageCx) {
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let item = data.up_next.clone().unwrap_or_else(|| data.item.clone());
        let options = PlayOptions {
            start,
            media_source_id: self.selected_version(&data.item).map(str::to_owned),
            ..Default::default()
        };
        cx.services.play(&item, options);
    }

    /// The chosen version's media source id, when the item has more than one.
    fn selected_version<'a>(&self, item: &'a BaseItemDto) -> Option<&'a str> {
        let count = item.media_sources.len();
        (count > 1).then(|| item.media_sources[self.version.min(count - 1)].id.as_str())
    }

    fn set_played(&mut self, item_id: String, played: bool, cx: &mut PageCx) {
        self.mark(item_id, Mark::Played(played), cx);
    }

    /// Marks an item watched or favorite: on screen right away, then on the server. A refusal
    /// comes back through `Services::refused_marks` and [`ItemPage::show_mark`] takes it back.
    fn mark(&mut self, item_id: String, change: Mark, cx: &mut PageCx) {
        self.show_mark(&item_id, change, false);
        cx.services.mark(item_id, change);
    }

    /// Shows `change` on each copy of the item on the page, or with `undo`, takes it back from
    /// those still showing it.
    fn show_mark(&mut self, item_id: &str, change: Mark, undo: bool) {
        let Load::Loaded(data) = &mut self.data else {
            return;
        };
        for item in std::iter::once(&mut data.item)
            .chain(data.children.iter_mut())
            .filter(|item| item.id == item_id)
        {
            if !undo {
                change.apply(item);
            } else if change.shown_on(item) {
                change.undone().apply(item);
            }
        }
    }

    /// Takes back what this page showed ahead of the server for every mark it was refused since
    /// it last looked, then catches up with the log. The page may have been off screen when the
    /// server refused (under another page, behind the player, or with the window hidden), so the
    /// refusal waits for it here instead of being lost.
    fn take_back_refused_marks(&mut self, refused: &RefusedMarks) {
        let refusals: Vec<(String, Mark)> = refused
            .after(self.marks_seen)
            .map(|entry| (entry.item_id.clone(), entry.mark))
            .collect();
        self.marks_seen = refused.sequence();
        // Newest first, so two quick toggles both refused unwind to where they started.
        for (item_id, mark) in refusals.into_iter().rev() {
            self.show_mark(&item_id, mark, true);
        }
    }

    /// Does what a button, a card or a menu entry asked for.
    fn run_action(&mut self, action: ItemAction, cx: &mut PageCx) {
        match action {
            ItemAction::More => {
                self.more_requested = true;
                return;
            }
            ItemAction::Resume(position) => {
                self.play(Some(position), cx);
                return;
            }
            ItemAction::PlayFromStart | ItemAction::Play => {
                self.play(None, cx);
                return;
            }
            _ => {}
        }
        let Load::Loaded(data) = &self.data else {
            return;
        };
        match action {
            ItemAction::TogglePlayed => {
                let (item_id, played) = (data.item.id.clone(), is_played(&data.item));
                self.set_played(item_id, !played, cx);
            }
            ItemAction::ToggleFavorite => {
                let (item_id, favorite) = (data.item.id.clone(), is_favorite(&data.item));
                self.mark(item_id, Mark::Favorite(!favorite), cx);
            }
            // A show plays all its episodes, shuffled, which takes a request; the services run
            // it so leaving the page doesn't drop it, as `main`'s detached task didn't.
            ItemAction::Shuffle if matches!(data.item.kind, BaseItemKind::Series | BaseItemKind::Season) => {
                cx.services.play_shuffled(&data.item);
            }
            ItemAction::PlayAll | ItemAction::Shuffle => {
                let mut items: Vec<BaseItemDto> = data
                    .children
                    .iter()
                    .filter(|child| is_playable(child))
                    .cloned()
                    .collect();
                if matches!(action, ItemAction::Shuffle) {
                    shuffle(&mut items);
                }
                if items.is_empty() {
                    cx.services.toast(Toast::info("Nothing here to play."));
                } else {
                    cx.services.play_all(items);
                }
            }
            // A trailer file plays here; a trailer on the web opens in the browser.
            ItemAction::Trailer => {
                if let Some(trailer) = data
                    .extras
                    .iter()
                    .find(|extra| extra.kind == BaseItemKind::Trailer)
                    .cloned()
                {
                    cx.services.play(&trailer, PlayOptions::default());
                } else if let Some(trailer) = data.item.remote_trailers.first() {
                    cx.ctx.open_url(egui::OpenUrl::new_tab(&trailer.url));
                }
            }
            ItemAction::AddToPlaylist => {
                self.add_to_playlist = Some(AddToPlaylist::new(cx, &data.item));
            }
            ItemAction::EditPlaylist => {
                self.edit_playlist = Some(EditPlaylist::new(cx, &data.item));
            }
            ItemAction::DeletePlaylist => {
                self.delete_playlist = Some(DeletePlaylist::new(&data.item));
            }
            // Handled above.
            ItemAction::More | ItemAction::Resume(_) | ItemAction::PlayFromStart | ItemAction::Play => {}
        }
    }

    /// Backspace and Alt+Left close an open menu, as they close the account menu and as Esc
    /// closes any popup. egui keeps the keys while a popup is open, so the shell's Back would
    /// otherwise never see them.
    fn menu_back_keys(&mut self, ctx: &egui::Context, menus: [Id; 2]) {
        if !menus.iter().any(|&id| egui::Popup::is_id_open(ctx, id)) {
            return;
        }
        let back = keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::Backspace)
            | keys::take_exact(ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft);
        if !back {
            return;
        }
        for id in menus {
            egui::Popup::close_id(ctx, id);
        }
        self.more_highlight = None;
    }

    /// The More menu's keys, which egui keeps while its popup is open: Tab stays in the menu,
    /// Up and Down move its highlight, Enter picks. Read before anything draws, so egui's focus
    /// pass can't hand the keyboard to a button behind the menu (Enter would then press that as
    /// well). Esc and clicks close the popup as egui's popups do.
    fn more_menu_keys(&mut self, id: Id, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, id) {
            return;
        }
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let ctx = cx.ctx;
        if menu_search_key(ctx, id, cx.nav) {
            self.more_highlight = None;
            return;
        }
        // Tab and the arrows queued an egui focus move in its focus pass; cancel it.
        if keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::Tab)
            | keys::take_exact(ctx, egui::Modifiers::SHIFT, egui::Key::Tab)
        {
            cancel_focus(ctx);
        }
        let count = more_actions(&data.item).len();
        let down = keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::ArrowDown);
        let up = keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::ArrowUp);
        if down | up {
            let step = if down { 1 } else { -1 };
            self.more_highlight = step_highlight(self.more_highlight, step, count);
            cancel_focus(ctx);
        }
        let chosen = if keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::Enter) {
            self.more_highlight
                .and_then(|index| more_actions(&data.item).get(index).copied())
        } else {
            None
        };
        if let Some(action) = chosen {
            egui::Popup::close_id(ctx, id);
            self.more_highlight = None;
            self.run_action(action, cx);
        }
    }

    /// The More menu while it's open: its entries, the keyboard's highlight, and the one chosen
    /// with the pointer. Returns what was chosen, for the caller to carry out once the menu is
    /// closed.
    fn more_menu(&mut self, response: &Response, id: Id, cx: &mut PageCx) -> Option<ItemAction> {
        if !egui::Popup::is_id_open(cx.ctx, id) {
            return None;
        }
        let Load::Loaded(data) = &self.data else {
            return None;
        };
        let actions = more_actions(&data.item);
        let highlight = self.more_highlight;
        let mut chosen = None;
        egui::Popup::menu(response)
            .id(id)
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::BOTTOM_END)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(MENU_W)
            .show(|ui| {
                for (index, &action) in actions.iter().enumerate() {
                    if action == ItemAction::DeletePlaylist {
                        ui.separator();
                    }
                    let (icon, label) = more_entry(action);
                    if menu_row(ui, Some(icon), label, highlight == Some(index)).clicked() {
                        chosen = Some(action);
                    }
                }
            });
        if chosen.is_some() {
            egui::Popup::close_id(cx.ctx, id);
            self.more_highlight = None;
        }
        chosen
    }

    /// The version list's keys, which egui keeps while its popup is open: Tab and Shift+Tab
    /// stay in it (egui's focus pass would otherwise hand the keyboard to a button behind it),
    /// and Ctrl/Cmd+F closes it and opens the search box, as it does for the More menu.
    fn versions_menu_keys(ctx: &egui::Context, id: Id, nav: &mut Vec<NavRequest>) {
        if !egui::Popup::is_id_open(ctx, id) {
            return;
        }
        if keys::take_exact(ctx, egui::Modifiers::NONE, egui::Key::Tab)
            | keys::take_exact(ctx, egui::Modifiers::SHIFT, egui::Key::Tab)
        {
            cancel_focus(ctx);
        }
        menu_search_key(ctx, id, nav);
    }

    /// The version picker's menu while it's open: an item's versions, the chosen one checked.
    /// Clicking a row picks it; the keyboard changes the choice through the cursor instead
    /// (Left and Right step, Enter cycles).
    fn versions_menu(&mut self, response: &Response, id: Id, cx: &mut PageCx) {
        if !egui::Popup::is_id_open(cx.ctx, id) {
            return;
        }
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let sources = &data.item.media_sources;
        let current = self.version.min(sources.len().saturating_sub(1));
        let mut chosen = None;
        egui::Popup::menu(response)
            .id(id)
            .open_memory(None::<egui::SetOpenCommand>)
            .align(RectAlign::BOTTOM_START)
            .align_alternatives(&[])
            .gap(4.)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(PICKER_W)
            .show(|ui| {
                for (index, source) in sources.iter().enumerate() {
                    if menu_row(ui, None, &version_label(source, index), index == current).clicked() {
                        chosen = Some(index);
                    }
                }
            });
        if let Some(index) = chosen {
            egui::Popup::close_id(cx.ctx, id);
            self.version = index;
            cx.ctx.request_repaint();
        }
    }

    fn show_content(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let more_id = more_menu_id(ui);
        let versions_id = versions_menu_id(ui);
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let actions = item_actions(&data.item, data.up_next.as_ref());
        let version = self.version;
        let moved = self.cursor.take_reveal();
        let selected = self.cursor.selected();
        let images = &mut cx.services.images;
        let cursor = &mut self.cursor;
        let mut asked = Asked::default();
        // The page's own scroll target, set once every shelf is drawn: the next scroll area to
        // finish takes a target, and a shelf's row would be it (U3).
        let mut target = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: PAGE_PADDING as i8,
                    right: PAGE_PADDING as i8,
                    top: 0,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let header = header(ui, data, &actions, selected, images, &mut asked);
                    let mut pickers = None;
                    if data.item.media_sources.len() > 1 {
                        ui.add_space(SECTION_GAP);
                        pickers = Some(pickers_row(ui, data, selected, version, &mut asked));
                    }
                    let mut chapters_row = None;
                    if !chapters(&data.item).is_empty() {
                        ui.add_space(SECTION_GAP);
                        chapters_row = Some(chapters_shelf(ui, data, moved, cursor, images, &mut asked));
                    }
                    let mut people_row = None;
                    if !data.item.people.is_empty() {
                        ui.add_space(SECTION_GAP);
                        people_row = Some(people_shelf(ui, data, moved, cursor, images, &mut asked));
                    }
                    let mut extras_row = None;
                    if !data.extras.is_empty() {
                        ui.add_space(SECTION_GAP);
                        extras_row = Some(extras_shelf(ui, data, moved, cursor, images, &mut asked));
                    }
                    let mut similar_row = None;
                    if !data.similar.is_empty() {
                        ui.add_space(SECTION_GAP);
                        similar_row = Some(similar_shelf(ui, data, moved, cursor, images, &mut asked));
                    }
                    ui.add_space(BOTTOM_PADDING);

                    if moved {
                        target = match selected {
                            Some(position) if position.section == ACTIONS => Some((header, Some(Align::TOP))),
                            Some(position) if position.section == PICKERS => pickers.map(|rect| (rect, None)),
                            Some(position) if position.section == CHAPTERS => chapters_row.map(|rect| (rect, None)),
                            Some(position) if position.section == PEOPLE => people_row.map(|rect| (rect, None)),
                            Some(position) if position.section == EXTRAS => extras_row.map(|rect| (rect, None)),
                            Some(position) if position.section == SIMILAR => similar_row.map(|rect| (rect, None)),
                            _ => None,
                        };
                    }
                    if let Some((rect, align)) = target {
                        ui.scroll_to_rect(rect, align);
                    }
                });
        });

        let more_was_open = egui::Popup::is_id_open(cx.ctx, more_id);
        if let Some(action) = asked.action {
            // A second click on More closes its menu, as its trigger did on `main`.
            if action == ItemAction::More && egui::Popup::is_id_open(cx.ctx, more_id) {
                egui::Popup::close_id(cx.ctx, more_id);
                self.more_highlight = None;
            } else {
                if action == ItemAction::More {
                    self.more_highlight = None;
                }
                self.run_action(action, cx);
            }
        }
        if let Some((action, item)) = asked.menu
            && let Some(MenuAction::AddToPlaylist) = cx.services.carry_out(action, &item)
        {
            self.add_to_playlist = Some(AddToPlaylist::new(cx, &item));
        }
        if let Some(route) = asked.open {
            cx.nav.push(NavRequest::Push(route));
        }
        if let Some(start) = asked.play_from {
            self.play(Some(start), cx);
        }
        if let Some(extra) = asked.extra {
            cx.services.play(&extra, PlayOptions::default());
        }
        if let Some(response) = &asked.version_picker {
            if response.clicked() {
                toggle_popup(cx.ctx, versions_id);
            }
            self.versions_menu(response, versions_id, cx);
        } else {
            egui::Popup::close_id(cx.ctx, versions_id);
        }

        if let Some(response) = &asked.more {
            if std::mem::take(&mut self.more_requested) {
                egui::Popup::open_id(cx.ctx, more_id);
            }
            if let Some(action) = self.more_menu(response, more_id, cx) {
                self.run_action(action, cx);
            }
        } else {
            egui::Popup::close_id(cx.ctx, more_id);
            self.more_highlight = None;
            self.more_requested = false;
        }
        if more_was_open && !egui::Popup::is_id_open(cx.ctx, more_id) {
            self.more_highlight = None;
        }
    }
}

/// The chapters shelf: their image, name and start. Chosen, the item plays from there with the
/// version picked below its title.
fn chapters_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let chapters = chapters(&data.item);
    shelf::shelf_with_gaps(ui, "chapters", "Chapters", CARD_GAP, SHELF_TITLE_GAP, |ui| {
        for (index, chapter) in chapters.iter().enumerate() {
            let is_selected = cursor.is_selected(CHAPTERS, index);
            let response = card::chapter(ui, images, &data.item.id, index, chapter, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.play_from = Some(chapter.start_position_ticks);
            }
        }
    })
}

/// What the page's drawing asked for this frame, carried out once the page is drawn.
#[derive(Default)]
struct Asked {
    /// A button of the actions row was clicked.
    action: Option<ItemAction>,
    /// A chapter was chosen: play from its start.
    play_from: Option<Ticks>,
    /// An extra was chosen: play it from the start.
    extra: Option<BaseItemDto>,
    /// A card opens a page.
    open: Option<Route>,
    /// A card's context menu was used.
    menu: Option<(MenuAction, BaseItemDto)>,
    /// The More button's response, which its menu hangs from.
    more: Option<Response>,
    /// The version picker's response, which its menu hangs from.
    version_picker: Option<Response>,
}

/// The header: the backdrop with its fade, the poster, and the item's title, actions and text.
/// Returns its rectangle, for the cursor scrolling back to the actions.
fn header(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let item = &data.item;
    let top = ui.cursor().min;
    let backdrop = item.backdrop_image();
    if let Some(backdrop) = &backdrop {
        // Full width, past the page's padding: `main`'s backdrop ran edge to edge.
        let band = Rect::from_min_max(
            pos2(ui.clip_rect().left(), top.y),
            pos2(ui.clip_rect().right(), top.y + BACKDROP_H),
        );
        image::show_at_width(ui, images, band, Some(backdrop), Some(item), 0, BACKDROP_FILL_W);
        fade_to_background(ui, band);
    }
    ui.add_space(if backdrop.is_some() { BACKDROP_H - 160. } else { 24. });
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 24.;
        let (poster, _) = ui.allocate_exact_size(vec2(DETAIL_POSTER_W, DETAIL_POSTER_H), Sense::hover());
        image::show(
            ui,
            images,
            poster,
            item.primary_image().as_ref(),
            Some(item),
            image::RADIUS,
        );
        ui.vertical(|ui| {
            if backdrop.is_some() {
                // Lines the text up beside the poster, as on `main`.
                ui.add_space(120.);
            }
            info_column(ui, data, actions, selected, asked);
        });
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

/// The title's column: the series line, the title and its actions, the meta line, genres,
/// studios, the synopsis and the tags.
fn info_column(ui: &mut Ui, data: &ItemData, actions: &[ItemAction], selected: Option<Position>, asked: &mut Asked) {
    let item = &data.item;
    let body = TextStyle::Body.resolve(ui.style());
    let small = body.size - 2.;
    let weak = ui.visuals().weak_text_color();
    ui.spacing_mut().item_spacing.y = 8.;
    if matches!(item.kind, BaseItemKind::Episode | BaseItemKind::Season)
        && let Some(series) = &item.series_name
    {
        ui.label(RichText::new(series).size(small).color(weak));
    }

    let episode = data
        .up_next
        .as_ref()
        .map(episode_label)
        .filter(|label| !label.is_empty());
    let actions_width = actions_width(ui, data, actions, episode.as_deref());
    let title_font = FontId::new(body.size * 2.5, body.family.clone());
    let available = ui.available_width();
    let gap = 16.;
    if available >= TITLE_MIN_W + gap + actions_width {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let title_w = available - gap - actions_width;
            ui.vertical(|ui| {
                ui.set_max_width(title_w);
                ui.add(Label::new(RichText::new(&item.name).font(title_font.clone())).wrap());
            });
            ui.add_space((ui.available_width() - actions_width).max(0.));
            actions_row(ui, data, actions, selected, episode.as_deref(), asked);
        });
    } else {
        // A narrow window: the actions go under the title rather than squeeze it to a word a
        // line.
        ui.add(Label::new(RichText::new(&item.name).font(title_font)).wrap());
        actions_row(ui, data, actions, selected, episode.as_deref(), asked);
    }

    let meta = meta_line(item);
    if !meta.is_empty() {
        ui.label(RichText::new(meta).size(small).color(weak));
    }
    if !item.genres.is_empty() {
        ui.label(RichText::new(item.genres.join(", ")).size(small).color(weak));
    }
    if !item.studios.is_empty() {
        let studios: Vec<&str> = item.studios.iter().map(|studio| studio.name.as_str()).collect();
        ui.label(RichText::new(studios.join(", ")).size(small).color(weak));
    }
    if let Some(overview) = &item.overview
        && !overview.is_empty()
    {
        ui.add_space(8.);
        ui.scope(|ui| {
            ui.set_max_width(820.);
            ui.add(Label::new(overview.clone()).wrap());
        });
    }
    if !item.tags.is_empty() {
        ui.add_space(16.);
        ui.scope(|ui| {
            ui.set_max_width(820.);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4., 4.);
                for tag in &item.tags {
                    egui::Frame::NONE
                        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.label(RichText::new(tag).size(small).color(weak));
                        });
                }
            });
        });
    }
}

/// What an action button shows: its icon, its tooltip, and the label playing it carries.
struct ActionLook {
    icon: MaterialIcon,
    tooltip: String,
    label: Option<String>,
    primary: bool,
}

fn action_look(action: ItemAction, item: &BaseItemDto, episode: Option<&str>) -> ActionLook {
    let labelled = |verb: &str| {
        Some(match episode {
            Some(episode) => format!("{verb} {episode}"),
            None => verb.to_owned(),
        })
    };
    match action {
        ItemAction::Resume(position) => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: format!("Resume from {}", timestamp(position.into())),
            label: labelled("Resume"),
            primary: true,
        },
        ItemAction::PlayFromStart => ActionLook {
            icon: ICON_REPLAY,
            tooltip: "Play from beginning".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::Play => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: "Play".to_owned(),
            label: labelled("Play"),
            primary: true,
        },
        ItemAction::PlayAll => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: "Play all".to_owned(),
            label: Some("Play all".to_owned()),
            primary: true,
        },
        ItemAction::Shuffle => ActionLook {
            icon: ICON_SHUFFLE,
            tooltip: "Shuffle".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::Trailer => ActionLook {
            icon: if item.local_trailer_count.unwrap_or(0) > 0 {
                ICON_MOVIE
            } else {
                ICON_OPEN_IN_NEW
            },
            tooltip: "Trailer".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::More => ActionLook {
            icon: ICON_MORE_HORIZ,
            tooltip: "More".to_owned(),
            label: None,
            primary: false,
        },
        // Drawn as toggles, which take their icon from the item's state, and the More menu's
        // entries, which have their own icons.
        ItemAction::TogglePlayed
        | ItemAction::ToggleFavorite
        | ItemAction::AddToPlaylist
        | ItemAction::EditPlaylist
        | ItemAction::DeletePlaylist => ActionLook {
            icon: ICON_MORE_HORIZ,
            tooltip: String::new(),
            label: None,
            primary: false,
        },
    }
}

/// How wide the actions row is, for the title to know what's left of its line. A pixel over is
/// fine: the row right-aligns itself in what the title leaves.
fn actions_width(ui: &Ui, data: &ItemData, actions: &[ItemAction], episode: Option<&str>) -> f32 {
    let body = TextStyle::Button.resolve(ui.style());
    let mut width = 0.;
    for (index, &action) in actions.iter().enumerate() {
        if index > 0 {
            width += ACTION_GAP - 2. * ACTION_RING_GAP;
        }
        width += ACTION_SIZE + 2. * ACTION_RING_GAP;
        if let Some(label) = action_look(action, &data.item, episode).label {
            // The icon and the button's padding around the label, roughly: only the choice
            // between one line and two depends on it.
            let text = ui
                .fonts_mut(|fonts| fonts.layout_no_wrap(label, body.clone(), Color32::WHITE))
                .size()
                .x;
            width += text + ACTION_SIZE;
        }
    }
    width
}

/// The buttons under the title: playing it, named, then icon toggles and the More menu.
fn actions_row(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    episode: Option<&str>,
    asked: &mut Asked,
) {
    let item = &data.item;
    let played = is_played(item);
    let favorite = is_favorite(item);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = ACTION_GAP - 2. * ACTION_RING_GAP;
        for (index, &action) in actions.iter().enumerate() {
            let ringed = selected
                == Some(Position {
                    section: ACTIONS,
                    index,
                });
            let response = match action {
                ItemAction::TogglePlayed => {
                    let icon = menu::played_icon(played).size(ACTION_ICON);
                    toggle(ui, icon, played, "Watched")
                }
                ItemAction::ToggleFavorite => {
                    let icon = menu::favorite_icon(favorite).size(ACTION_ICON);
                    toggle(ui, icon, favorite, "Favorite")
                }
                _ => {
                    let look = action_look(action, item, episode);
                    let response = ui.add(action_button(ui, &look)).on_hover_text(look.tooltip.as_str());
                    if action == ItemAction::More {
                        asked.more = Some(response.clone());
                    }
                    response
                }
            };
            if ringed {
                ui.painter().rect_stroke(
                    response.rect.expand(ACTION_RING_GAP),
                    CornerRadius::same(8),
                    Stroke::new(2., ui.visuals().strong_text_color()),
                    StrokeKind::Outside,
                );
            }
            if response.clicked() {
                asked.action = Some(action);
            }
        }
    });
}

/// An action button: its icon, and a label for the actions that are named. Not focusable: the
/// page's keys are the browse cursor's (`keys.rs`).
fn action_button(ui: &Ui, look: &ActionLook) -> egui::Button<'static> {
    let body = TextStyle::Button.resolve(ui.style());
    let color = if look.primary {
        ui.visuals().panel_fill
    } else {
        ui.visuals().text_color()
    };
    let mut job = LayoutJob::default();
    job.append(
        look.icon.codepoint,
        0.,
        TextFormat {
            font_id: FontId::new(ACTION_ICON, look.icon.font_family()),
            color,
            ..Default::default()
        },
    );
    if let Some(label) = &look.label {
        job.append(
            &format!("  {label}"),
            0.,
            TextFormat {
                font_id: body,
                color,
                ..Default::default()
            },
        );
    }
    let button = egui::Button::new(WidgetText::LayoutJob(job.into()))
        .sense(Sense::CLICK)
        .min_size(vec2(ACTION_SIZE, ACTION_SIZE));
    if look.primary {
        button.fill(ui.visuals().strong_text_color()).stroke(Stroke::NONE)
    } else {
        button
    }
}

/// Watched and Favorite: pressed while on, with the icon filled in.
fn toggle(ui: &mut Ui, icon: RichText, on: bool, tooltip: &str) -> Response {
    let button = egui::Button::new(icon)
        .selected(on)
        .sense(Sense::CLICK)
        .min_size(vec2(ACTION_SIZE, ACTION_SIZE));
    ui.add(button).on_hover_text(tooltip)
}

/// The version label's column and the picker, when the item has more than one version.
fn pickers_row(ui: &mut Ui, data: &ItemData, selected: Option<Position>, version: usize, asked: &mut Asked) -> Rect {
    let top = ui.cursor().min;
    let body = TextStyle::Body.resolve(ui.style());
    let sources = &data.item.media_sources;
    let current = version.min(sources.len() - 1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.;
        let (label, _) = ui.allocate_exact_size(vec2(PICKER_LABEL_W, PICKER_H), Sense::hover());
        ui.painter().text(
            label.left_center(),
            Align2::LEFT_CENTER,
            "Version",
            FontId::new(body.size - 2., body.family.clone()),
            ui.visuals().weak_text_color(),
        );
        let ringed = selected
            == Some(Position {
                section: PICKERS,
                index: 0,
            });
        let name = version_label(&sources[current], current);
        let response = picker(ui, &name, ringed);
        asked.version_picker = Some(response);
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

/// The version select: a bordered box with the version's name and a caret. Clicking it opens
/// the list of versions; the keyboard changes the choice on the cursor instead.
fn picker(ui: &mut Ui, name: &str, ringed: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(PICKER_W, PICKER_H), Sense::CLICK);
    let fill = if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    let stroke = Stroke::new(1., ui.visuals().widgets.inactive.bg_stroke.color);
    ui.painter()
        .rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
    let caret = 16.;
    let text = rect.shrink2(vec2(10., 2.));
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(
                text.min,
                vec2((text.width() - caret).max(0.), text.height()),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.add(Label::new(RichText::new(name).size(PICKER_H * 0.5)).truncate());
    ui.painter().text(
        pos2(rect.right() - 10. - caret / 2., rect.center().y),
        Align2::CENTER_CENTER,
        ICON_ARROW_DROP_DOWN.codepoint,
        FontId::new(caret, ICON_ARROW_DROP_DOWN.font_family()),
        ui.visuals().weak_text_color(),
    );
    if ringed {
        ui.painter().rect_stroke(
            rect.expand(ACTION_RING_GAP),
            CornerRadius::same(8),
            Stroke::new(2., ui.visuals().strong_text_color()),
            StrokeKind::Outside,
        );
    }
    response
}

fn version_label(source: &MediaSourceInfo, index: usize) -> String {
    source.name.clone().unwrap_or_else(|| format!("Version {}", index + 1))
}

/// The people shelf: each one's photo or initials, role and job, opening their page.
fn people_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let people = &data.item.people;
    shelf::shelf_with_gaps(ui, "people", "Cast & crew", BIG_CARD_GAP, SHELF_TITLE_GAP, |ui| {
        for (index, person) in people.iter().enumerate() {
            let is_selected = cursor.is_selected(PEOPLE, index);
            let response = card::person(ui, images, person, BIG_CARD_SCALE, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.open = Some(Route::Person {
                    id: person.id.clone(),
                    name: person.name.clone(),
                });
            }
        }
    })
}

/// Trailers and extras, played from the start when chosen.
fn extras_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let extras = &data.extras;
    shelf::shelf_with_gaps(
        ui,
        "extras",
        "Extras",
        crate::widgets::text::CARD_GAP,
        SHELF_TITLE_GAP,
        |ui| {
            for (index, extra) in extras.iter().enumerate() {
                let is_selected = cursor.is_selected(EXTRAS, index);
                let response = card::landscape(ui, images, extra, is_selected);
                if is_selected && moved {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    asked.extra = Some(extra.clone());
                }
            }
        },
    )
}

/// "More like this": larger cards, opening their pages, with the item menu.
fn similar_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let similar = &data.similar;
    shelf::shelf_with_gaps(ui, "similar", "More like this", BIG_CARD_GAP, SHELF_TITLE_GAP, |ui| {
        for (index, item) in similar.iter().enumerate() {
            let is_selected = cursor.is_selected(SIMILAR, index);
            let response = card::poster_scaled(ui, images, item, BIG_CARD_SCALE, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.open = Some(Route::for_item(item));
            }
            if let Some(action) = menu::item_menu(&response, item) {
                asked.menu = Some((action, item.clone()));
            }
        }
    })
}

/// Fades the backdrop into the page's background at its bottom, as `main`'s gradient did.
fn fade_to_background(ui: &Ui, band: Rect) {
    let background = ui.visuals().panel_fill;
    // `main`'s first stop: the background at 30 %, so the image shows through at the top.
    let top = Color32::from_rgba_unmultiplied(background.r(), background.g(), background.b(), 77);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(pos2(band.left(), band.top()), top);
    mesh.colored_vertex(pos2(band.right(), band.top()), top);
    mesh.colored_vertex(pos2(band.left(), band.bottom()), background);
    mesh.colored_vertex(pos2(band.right(), band.bottom()), background);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// One row of the More or version menus: `icon` (none for the versions) before the label, the
/// keyboard's ring while it's highlighted.
fn menu_row(ui: &mut Ui, icon: Option<MaterialIcon>, label: &str, highlighted: bool) -> Response {
    let body = TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    if let Some(icon) = icon {
        job.append(
            icon.codepoint,
            0.,
            TextFormat {
                font_id: FontId::new(body.size + 2., icon.font_family()),
                color,
                ..Default::default()
            },
        );
        job.append("  ", 0., TextFormat::default());
    }
    job.append(
        label,
        0.,
        TextFormat {
            font_id: body,
            color,
            ..Default::default()
        },
    );
    // Not focusable: the page reads the menu's keys itself, and egui's focus pass must not
    // hand Enter to a button behind the menu.
    let response = ui.add(
        egui::Button::new(WidgetText::LayoutJob(job.into()))
            .frame(false)
            .sense(Sense::CLICK),
    );
    if highlighted {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(4),
            Stroke::new(1.5, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    response
}

fn more_entry(action: ItemAction) -> (MaterialIcon, &'static str) {
    match action {
        ItemAction::AddToPlaylist => (ICON_PLAYLIST_ADD, "Add to playlist…"),
        ItemAction::EditPlaylist => (ICON_EDIT, "Edit…"),
        _ => (ICON_DELETE, "Delete playlist…"),
    }
}

/// Up and Down in a menu wrap round, as its highlight did on `main`. A menu opened with a
/// click shows the first entry going down, the last going up.
fn step_highlight(current: Option<usize>, step: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match current {
        Some(index) => (index as i32 + step).rem_euclid(len as i32) as usize,
        None if step > 0 => 0,
        None => len - 1,
    })
}

/// A popup's id, scoped to the page: the shell gives every page its own id scope, so stacked
/// item pages don't share a menu.
fn more_menu_id(ui: &Ui) -> Id {
    ui.make_persistent_id("item_more_menu")
}

fn versions_menu_id(ui: &Ui) -> Id {
    ui.make_persistent_id("item_versions")
}

/// Opens a popup, or closes it when it's open: what clicking its button does.
fn toggle_popup(ctx: &egui::Context, id: Id) {
    if egui::Popup::is_id_open(ctx, id) {
        egui::Popup::close_id(ctx, id);
    } else {
        egui::Popup::open_id(ctx, id);
    }
    ctx.request_repaint();
}

/// Cancels the focus move egui queued for a Tab or arrow pressed while a page menu is open:
/// egui's focus pass would otherwise hand the keyboard to a widget behind the menu (Enter would
/// then press that as well).
fn cancel_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
}

/// Ctrl/Cmd+F in an open page menu: closes it and asks the shell for the search box, as the
/// account menu does (U1). egui keeps the key while a popup is open, so the shell's search
/// binding never sees it. Returns whether it was the search key.
fn menu_search_key(ctx: &egui::Context, id: Id, nav: &mut Vec<NavRequest>) -> bool {
    if !keys::take_exact(ctx, egui::Modifiers::COMMAND, egui::Key::F) {
        return false;
    }
    egui::Popup::close_id(ctx, id);
    nav.push(NavRequest::OpenSearch);
    true
}

impl Page for ItemPage {
    fn title(&self) -> &str {
        &self.name
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
        let more_id = more_menu_id(ui);
        let versions_id = versions_menu_id(ui);
        self.menu_back_keys(cx.ctx, [more_id, versions_id]);
        Self::versions_menu_keys(cx.ctx, versions_id, cx.nav);
        self.more_menu_keys(more_id, cx);
        if self.loaded_at != Some(cx.services.user_data) {
            self.reload(cx);
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            match result {
                Ok(data) => {
                    if !data.item.name.is_empty() {
                        self.name = data.item.name.clone();
                    }
                    self.data = Load::Loaded(data);
                }
                Err(err) => self.data = Load::Failed(cx.services.handle_error(&err)),
            }
        }
        // Marks the server refused: take back what the page showed ahead of it, however long
        // ago the refusal came.
        self.take_back_refused_marks(&cx.services.refused_marks);
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
            Load::Loaded(_) => {}
        }
        if matches!(self.data, Load::Loaded(_)) {
            self.show_content(ui, cx);
        } else {
            // Nothing to hang the menus from while the page loads or failed; egui would keep
            // the keyboard for a popup that isn't drawn.
            egui::Popup::close_id(cx.ctx, more_menu_id(ui));
            egui::Popup::close_id(cx.ctx, versions_menu_id(ui));
            self.more_highlight = None;
            self.more_requested = false;
        }

        if let Some(dialog) = &mut self.add_to_playlist
            && !dialog.show(cx)
        {
            self.add_to_playlist = None;
        }
        if let Some(dialog) = &mut self.edit_playlist
            && !dialog.show(cx)
        {
            self.edit_playlist = None;
        }
        if let Some(dialog) = &mut self.delete_playlist
            && !dialog.show(cx)
        {
            self.delete_playlist = None;
        }
    }
}

impl Browse for ItemPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// The sections in the order they appear on the page. The children rows and the open
    /// season's episodes are U7's, and empty until then, so the cursor skips them.
    fn sections(&self) -> Vec<Section> {
        let Load::Loaded(data) = &self.data else {
            return Vec::new();
        };
        let actions = item_actions(&data.item, data.up_next.as_ref()).len();
        let pickers = usize::from(data.item.media_sources.len() > 1);
        vec![
            Section::row(actions),
            Section::grid(pickers, 1),
            Section::row(chapters(&data.item).len()),
            Section::row(0),
            Section::row(0),
            Section::row(data.item.people.len()),
            Section::row(data.extras.len()),
            Section::row(data.similar.len()),
        ]
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        let Load::Loaded(data) = &self.data else {
            return;
        };
        match position.section {
            ACTIONS => {
                let actions = item_actions(&data.item, data.up_next.as_ref());
                if let Some(&action) = actions.get(position.index) {
                    // Opened from the keyboard, the menu highlights its first entry.
                    if action == ItemAction::More {
                        self.more_highlight = Some(0);
                    }
                    self.run_action(action, cx);
                }
            }
            PICKERS => {
                let count = data.item.media_sources.len();
                if count > 1 {
                    self.version = (self.version + 1) % count;
                }
            }
            CHAPTERS => {
                if let Some(chapter) = chapters(&data.item).get(position.index) {
                    let start = chapter.start_position_ticks;
                    self.play(Some(start), cx);
                }
            }
            PEOPLE => {
                if let Some(person) = data.item.people.get(position.index) {
                    cx.nav.push(NavRequest::Push(Route::Person {
                        id: person.id.clone(),
                        name: person.name.clone(),
                    }));
                }
            }
            EXTRAS => {
                if let Some(extra) = data.extras.get(position.index) {
                    let extra = extra.clone();
                    cx.services.play(&extra, PlayOptions::default());
                }
            }
            SIMILAR => {
                if let Some(item) = data.similar.get(position.index) {
                    cx.nav.push(NavRequest::Push(Route::for_item(item)));
                }
            }
            // U7's rows.
            _ => {}
        }
    }

    /// Left and Right on the version picker step it, and stay on it at either end.
    fn adjust(&mut self, position: Position, forward: bool, _cx: &mut PageCx) -> bool {
        if position.section != PICKERS {
            return false;
        }
        let Load::Loaded(data) = &self.data else {
            return true;
        };
        let count = data.item.media_sources.len();
        if count > 1 {
            self.version = if forward {
                (self.version + 1).min(count - 1)
            } else {
                self.version.saturating_sub(1)
            };
        }
        true
    }
}

async fn load(client: Client, item_id: String) -> jellyfin::Result<ItemData> {
    let item = client
        .item(
            &item_id,
            &[
                ItemField::Overview,
                ItemField::Genres,
                ItemField::MediaSources,
                ItemField::People,
                ItemField::Studios,
                ItemField::Tags,
                ItemField::RemoteTrailers,
                ItemField::CanDelete,
                ItemField::Chapters,
            ],
        )
        .await?;
    let children = match item.kind {
        BaseItemKind::Series => client.seasons(&item.id, &[]).await?.items,
        BaseItemKind::Season => {
            let series_id = item.series_id.as_deref().unwrap_or_default();
            client
                .episodes(series_id, Some(&item.id), &[ItemField::Overview])
                .await?
                .items
        }
        BaseItemKind::BoxSet => collection_items(&client, &item.id, &[ItemField::PrimaryImageAspectRatio]).await?,
        BaseItemKind::Playlist => client.playlist_items(&item.id, &[ItemField::Overview]).await?.items,
        _ => Vec::new(),
    };
    let (extras, similar, up_next) = tokio::join!(
        extras(&client, &item),
        similar(&client, &item),
        up_next(&client, &item, &children)
    );
    Ok(ItemData {
        item,
        children,
        extras,
        similar,
        up_next,
    })
}

/// The episode Play starts on a series' or season's page. Without it there's no Play button,
/// so a failure only leaves that out.
async fn up_next(client: &Client, item: &BaseItemDto, children: &[BaseItemDto]) -> Option<BaseItemDto> {
    match item.kind {
        BaseItemKind::Season => first_unwatched(children).cloned(),
        BaseItemKind::Series => episodes_of(client, item)
            .await
            .inspect_err(|err| tracing::warn!("couldn't load the series' episodes: {err}"))
            .ok()
            .and_then(|episodes| first_unwatched(&episodes).cloned()),
        _ => None,
    }
}

/// Trailers first, then the other extras. They're secondary, so a failure leaves them out.
async fn extras(client: &Client, item: &BaseItemDto) -> Vec<BaseItemDto> {
    let count = |count: Option<u32>| count.unwrap_or(0) > 0;
    let (trailers, features) = tokio::join!(
        async {
            match count(item.local_trailer_count) {
                true => client.local_trailers(&item.id).await,
                false => Ok(Vec::new()),
            }
        },
        async {
            match count(item.special_feature_count) {
                true => client.special_features(&item.id).await,
                false => Ok(Vec::new()),
            }
        },
    );
    let mut extras = trailers
        .inspect_err(|err| tracing::warn!("couldn't load trailers: {err}"))
        .unwrap_or_default();
    extras.extend(
        features
            .inspect_err(|err| tracing::warn!("couldn't load extras: {err}"))
            .unwrap_or_default(),
    );
    extras
}

/// "More like this" for movies and shows.
async fn similar(client: &Client, item: &BaseItemDto) -> Vec<BaseItemDto> {
    if !matches!(item.kind, BaseItemKind::Movie | BaseItemKind::Series) {
        return Vec::new();
    }
    client
        .similar(&item.id, SIMILAR_LIMIT, &[ItemField::PrimaryImageAspectRatio])
        .await
        .map(|result| result.items)
        .inspect_err(|err| tracing::warn!("couldn't load similar items: {err}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers};
    use jellyfin::Ticks;
    use jellyfin::models::BaseItemDto;

    use super::{ItemAction, ItemData, ItemPage, item_actions, menu_search_key, step_highlight};
    use crate::item_menu::{Mark, is_favorite, is_played};
    use crate::nav::NavRequest;
    use crate::widgets::menu::{REFUSED_MARKS_KEPT, RefusedMarks};
    use crate::widgets::text::Load;

    fn movie(resume: bool, local_trailers: u32) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": "movie",
            "Name": "Movie",
            "Type": "Movie",
            "LocalTrailerCount": local_trailers,
            "UserData": {
                "Played": false,
                "PlaybackPositionTicks": if resume { 10_000_000 } else { 0 },
                "IsFavorite": false,
            },
        }))
        .expect("movie")
    }

    #[test]
    fn the_actions_follow_the_item_state() {
        let fresh = item_actions(&movie(false, 0), None);
        assert_eq!(
            fresh,
            [
                ItemAction::Play,
                ItemAction::TogglePlayed,
                ItemAction::ToggleFavorite,
                // A movie can go in a playlist, so it has the More menu.
                ItemAction::More,
            ]
        );
        let partway = item_actions(&movie(true, 1), None);
        assert_eq!(
            partway,
            [
                ItemAction::Resume(Ticks(10_000_000)),
                ItemAction::PlayFromStart,
                ItemAction::Trailer,
                ItemAction::TogglePlayed,
                ItemAction::ToggleFavorite,
                ItemAction::More,
            ]
        );
    }

    /// Puts one key press in the context's input and returns it.
    fn press(ctx: &egui::Context, key: Key, modifiers: Modifiers) -> Event {
        ctx.input_mut(|input| {
            input.events = vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]
        });
        ctx.input(|input| input.events[0].clone())
    }

    /// Ctrl, or Cmd on macOS, as egui-winit reports it.
    fn command() -> Modifiers {
        if cfg!(target_os = "macos") {
            Modifiers {
                mac_cmd: true,
                command: true,
                ..Default::default()
            }
        } else {
            Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            }
        }
    }

    #[test]
    fn backspace_closes_an_open_menu_and_is_otherwise_the_shells() {
        let ctx = egui::Context::default();
        let mut page = ItemPage::new("movie".into(), "Movie".into(), 0);
        let (more, versions) = (egui::Id::unique("more"), egui::Id::unique("versions"));

        // No menu open: Backspace stays in the input for the shell's Back.
        let event = press(&ctx, Key::Backspace, Modifiers::NONE);
        page.menu_back_keys(&ctx, [more, versions]);
        assert_eq!(ctx.input(|input| input.events.clone()), [event]);

        // With one open it closes it and takes the key, as the account menu does.
        egui::Popup::open_id(&ctx, more);
        page.menu_back_keys(&ctx, [more, versions]);
        assert!(!egui::Popup::is_id_open(&ctx, more));
        assert!(ctx.input(|input| input.events.is_empty()));
    }

    #[test]
    fn tab_stays_in_the_version_list_while_it_is_open() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("versions");
        let mut nav = Vec::new();

        // Closed: Tab is the page's (its next section), not the list's.
        let event = press(&ctx, Key::Tab, Modifiers::NONE);
        ItemPage::versions_menu_keys(&ctx, id, &mut nav);
        assert_eq!(ctx.input(|input| input.events.clone()), [event]);

        // Open: egui would move the keyboard behind the list with it; the list keeps it.
        egui::Popup::open_id(&ctx, id);
        press(&ctx, Key::Tab, Modifiers::NONE);
        ItemPage::versions_menu_keys(&ctx, id, &mut nav);
        assert!(ctx.input(|input| input.events.is_empty()));
        assert!(nav.is_empty());
    }

    #[test]
    fn ctrl_f_closes_a_menu_and_asks_for_search() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("more");
        egui::Popup::open_id(&ctx, id);
        let mut nav = Vec::new();
        press(&ctx, Key::F, command());
        assert!(menu_search_key(&ctx, id, &mut nav));
        assert!(!egui::Popup::is_id_open(&ctx, id));
        assert_eq!(nav, [NavRequest::OpenSearch]);
        assert!(ctx.input(|input| input.events.is_empty()));

        // A plain F isn't the search key (and isn't taken).
        let event = press(&ctx, Key::F, Modifiers::NONE);
        let mut nav = Vec::new();
        assert!(!menu_search_key(&ctx, id, &mut nav));
        assert!(nav.is_empty());
        assert_eq!(ctx.input(|input| input.events.clone()), [event]);
    }

    fn loaded_page(played: bool, favorite: bool, marks_seen: u64) -> ItemPage {
        let item: BaseItemDto = serde_json::from_value(serde_json::json!({
            "Id": "movie",
            "Name": "Movie",
            "Type": "Movie",
            "UserData": {
                "Played": played,
                "PlaybackPositionTicks": 0,
                "IsFavorite": favorite,
            },
        }))
        .expect("movie");
        let mut page = ItemPage::new("movie".into(), "Movie".into(), marks_seen);
        page.data = Load::Loaded(ItemData {
            item,
            children: Vec::new(),
            extras: Vec::new(),
            similar: Vec::new(),
            up_next: None,
        });
        page
    }

    fn shown(page: &ItemPage) -> (bool, bool) {
        let Load::Loaded(data) = &page.data else {
            panic!("the page should be loaded");
        };
        (is_played(&data.item), is_favorite(&data.item))
    }

    #[test]
    fn a_page_takes_back_every_refusal_it_missed_once() {
        let mut refused = RefusedMarks::default();
        refused.push("movie".into(), Mark::Played(true));
        refused.push("elsewhere".into(), Mark::Favorite(true));
        refused.push("movie".into(), Mark::Favorite(true));

        // It showed both ahead of the server; the refusals came while it was off screen.
        let mut page = loaded_page(true, true, 0);
        page.take_back_refused_marks(&refused);
        assert_eq!(shown(&page), (false, false));

        // Its sequence caught up: the same log doesn't take a later mark back.
        page.show_mark("movie", Mark::Played(true), false);
        page.show_mark("movie", Mark::Favorite(true), false);
        page.take_back_refused_marks(&refused);
        assert_eq!(shown(&page), (true, true));

        // A page made after the refusals starts at the log's sequence and takes nothing back.
        let mut fresh = loaded_page(true, true, refused.sequence());
        fresh.take_back_refused_marks(&refused);
        assert_eq!(shown(&fresh), (true, true));

        // Only the newest refusals are kept.
        let mut many = RefusedMarks::default();
        for i in 0..REFUSED_MARKS_KEPT + 5 {
            many.push(format!("item-{i}"), Mark::Played(true));
        }
        assert_eq!(many.after(0).count(), REFUSED_MARKS_KEPT);
        assert_eq!(many.sequence(), (REFUSED_MARKS_KEPT + 5) as u64);
    }

    #[test]
    fn a_menu_highlight_wraps_round() {
        assert_eq!(step_highlight(Some(0), 1, 3), Some(1));
        assert_eq!(step_highlight(Some(2), 1, 3), Some(0));
        assert_eq!(step_highlight(Some(0), -1, 3), Some(2));
        // A menu opened with a click: down starts at the top, up at the bottom.
        assert_eq!(step_highlight(None, 1, 3), Some(0));
        assert_eq!(step_highlight(None, -1, 3), Some(2));
        assert_eq!(step_highlight(Some(0), 1, 0), None);
    }
}
