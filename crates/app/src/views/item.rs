//! Detail page for a movie, series, season or episode, and for collections and playlists
//! (whose entries can be reordered and removed here). The page, its header, actions, pickers and
//! shelves are U6's; its seasons, the open season's episodes and playlist entries are U7's.
//!
//! The page follows `views/home.rs`: data loads through a [`Task`] polled in `show` and what's on
//! screen stays until new data arrives, the page loads again when `services.user_data` moves on,
//! the browse cursor moves over its sections, and what the drawing asked for is carried out
//! after the frame.
//!
//! A series shows its seasons as a grid, and the season chosen opens its episodes in a band
//! under them; a season's page and a playlist's list their children as rows with the image,
//! heading, runtime, three lines of overview and the Watched toggle. A playlist's rows also move
//! and remove their entry: on screen at once, then on the server one edit at a time in the order
//! they were made. The queue is the services' (`item_menu.rs`), so leaving the page doesn't lose
//! an edit on its way.
//!
//! Watched and Favorite show at once and go back if the server refuses: the services remember
//! the refusal in a numbered log (`Services::refused_marks`) and the page takes the change back
//! whenever it next draws, since a reload would fail too while the server is down. The icons pop
//! as the item turns watched or favorite, as `main`'s did ([`crate::motion`]).
//!
//! The More and version menus are [`PopupMenu`]s, so the keyboard can walk them, as it could
//! on `main`; Enter on More opens its menu with the first entry highlighted.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, Direction, FontId, Id, Label, Layout, Margin, Rect, RectAlign, Response,
    RichText, Sense, Stroke, StrokeKind, TextFormat, TextStyle, Ui, UiBuilder, pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_CLOSE, ICON_DELETE, ICON_EDIT, ICON_MORE_HORIZ, ICON_MOVIE,
    ICON_OPEN_IN_NEW, ICON_PLAY_ARROW, ICON_PLAYLIST_ADD, ICON_REPLAY, ICON_SHUFFLE, ICON_STAR,
};
use jellyfin::models::{BaseItemDto, BaseItemKind, ChapterInfo, ImageType, ItemField, ItemSortBy, MediaSourceInfo};
use jellyfin::{Client, ImageRequest, ItemsQuery, Ticks};

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::images::ImageCache;
use crate::item_menu::{
    Mark, PlaylistEdit, can_add_to_playlist, episodes_of, first_unwatched, is_favorite, is_playable, is_played,
    resume_point,
};
use crate::keys::{Action, Scope};
use crate::motion;
use crate::nav::{NavRequest, Route};
use crate::playback::PlayOptions;
use crate::task::Task;
use crate::theme;
use crate::theme::text;
use crate::toasts::Toast;
use crate::views::dialogs::{AddToPlaylist, DeletePlaylist, EditPlaylist};
use crate::views::{Page, PageCx};
use crate::widgets::menu::{self, MenuAction, RefusedMarks};
use crate::widgets::popup::{Entry, Opened, PopupMenu};
use crate::widgets::text::{
    CARD_GAP, LANDSCAPE_H, LANDSCAPE_W, Load, POSTER_W, card_text, episode_label, landscape_image, poster_image,
    runtime_text, timestamp,
};
use crate::widgets::{backdrop, button, card, image, mascot, select, shelf, state};

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
const BACKDROP_H: f32 = 420.;
/// The width the backdrop is fetched at, as `main` did. A fixed width keeps the image cache key
/// (and so the download and its cache entry) the same while the window is resized, maximised or
/// zoomed; it is scaled to the band it fills.
const BACKDROP_FILL_W: u32 = 1280;
/// The narrowest the title gets beside the actions before they move under it.
const TITLE_MIN_W: f32 = 280.;
/// The widest the synopsis and the tags run, for a readable line in a wide window. Only a cap:
/// `set_max_width` would widen a narrower column past the window's edge.
const TEXT_MAX_W: f32 = 820.;
const DETAIL_POSTER_W: f32 = 200.;
const DETAIL_POSTER_H: f32 = 300.;
/// An episode's still in the header, as tall as two thirds of the poster.
const DETAIL_STILL_H: f32 = 200.;
const PICKER_W: f32 = 260.;
const PICKER_H: f32 = 28.;
/// The version label's column, as on `main`.
const PICKER_LABEL_W: f32 = 90.;
/// Between the page's sections.
const SECTION_GAP: f32 = 24.;
const PAGE_PADDING: f32 = 24.;
const BOTTOM_PADDING: f32 = 32.;
/// Cast & crew, "More like this" and the seasons: larger cards, further apart, like Home's.
const BIG_CARD_SCALE: f32 = 1.2;
const BIG_CARD_GAP: f32 = 24.;
/// Between a shelf's title and its cards.
const SHELF_TITLE_GAP: f32 = 16.;
const ACTION_SIZE: f32 = 48.;
const ACTION_ICON: f32 = 24.;
/// The side padding of an action button, level with the 48-point height's own margins.
const ACTION_PADDING: f32 = 16.;
/// The item's title, `main`'s fixed 40-point `text_size` (it didn't follow the text size).
const TITLE_SIZE: f32 = 40.;
/// Between the action buttons, whose cursor rings take 3 px on each side.
const ACTION_GAP: f32 = 16.;
const ACTION_RING_GAP: f32 = 3.;
/// The More menu's width.
const MENU_W: f32 = 220.;

/// A row of a season's episodes or a playlist's entries, as `main` drew it: its image, the text
/// beside it, then the Watched toggle (and, in a playlist, the entry's buttons).
const ROW_PAD: f32 = 8.;
const ROW_GAP: f32 = 16.;
/// Between the heading, the runtime line and the overview.
const ROW_LINE_GAP: f32 = 4.;
const ENTRY_BUTTON: f32 = 28.;
const WATCHED_SIZE: f32 = 32.;
/// How many lines of an entry's overview show, as `main`'s `line_clamp(3)`.
const OVERVIEW_ROWS: usize = 3;
/// The band the open season's episodes sit in, tinted like `main`'s `muted` at 35 %.
const BAND_ALPHA: u8 = 89;

/// Cursor sections, top to bottom, as on `main`.
const ACTIONS: usize = 0;
/// The version picker; Left and Right change it.
const PICKERS: usize = 1;
const CHAPTERS: usize = 2;
/// A series' seasons grid, or a season's or playlist's rows.
const CHILDREN: usize = 3;
/// The open season's episodes.
const EPISODES: usize = 4;
const PEOPLE: usize = 5;
const EXTRAS: usize = 6;
const SIMILAR: usize = 7;

/// A series' season opened on its page, and its episodes once loaded.
struct OpenSeason {
    id: String,
    name: String,
    episodes: Load<Vec<BaseItemDto>>,
}

/// The band of a season the keyboard just opened, and what its reveal still has to do: it's
/// scrolled into view as it opens, and again when its episodes are there, since the loaded band
/// is taller than the spinner's line and the first scroll would leave its cards' bottoms below
/// the fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BandReveal {
    /// The band has just opened: bring its spinner (or its episodes, if they were that quick)
    /// into view.
    Opening,
    /// The spinner is in view and the episodes are on their way.
    Waiting,
    /// The episodes arrived (or their failure did), so the band is at its final height.
    Loaded,
}

/// What a playlist entry's buttons (or Alt+Up, Alt+Down and Delete) do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryAction {
    Up,
    Down,
    Remove,
}

/// When the Watched and Favorite toggles last turned on: `main`'s `Fill`.
#[derive(Default)]
struct Fills {
    played: Fill,
    favorite: Fill,
}

impl Fills {
    /// Starts each icon's pop whose toggle went from off to on.
    fn update(&mut self, before: (bool, bool), after: (bool, bool), now: f64) {
        self.played.update(before.0, after.0, now);
        self.favorite.update(before.1, after.1, now);
    }
}

/// When a toggle last turned on, so its icon pops for [`motion::POP`] after that and not when the
/// page shows again later.
#[derive(Default)]
struct Fill {
    at: Option<f64>,
}

impl Fill {
    /// Starts the fill if `before` was off and `now` is on.
    fn update(&mut self, before: bool, now_on: bool, now: f64) {
        if !before && now_on {
            self.at = Some(now);
        }
    }

    /// The icon's scale at `now`: popping just after it turned on, at its size after that.
    fn scale(&self, now: f64) -> f32 {
        self.at.map_or(1., |at| motion::pop_scale(now - at))
    }

    /// Whether the pop is still running, so the page asks for the next frame.
    fn popping(&self, now: f64) -> bool {
        self.at.is_some_and(|at| now - at < motion::POP)
    }
}

pub struct ItemPage {
    /// The top bar's title: the route's name until the item loads.
    name: String,
    item_id: String,
    data: Load<ItemData>,
    loading: Option<Task<jellyfin::Result<ItemData>>>,
    /// The `services.user_data` the data was loaded at; `None` before the first load or after
    /// Retry.
    loaded_at: Option<u64>,
    /// The Watched and Favorite icons fill in when the item turns watched or favorite.
    fills: Fills,
    /// The chosen media source, an index into the item's versions. Used only when there is
    /// more than one (`selected_version`).
    version: usize,
    cursor: Cursor,
    /// Season cards per row from the last frame: the keys use the layout the page last showed,
    /// since the column count only comes out of its width.
    columns: usize,
    /// A series' season whose episodes show in a band under the seasons.
    open_season: Option<OpenSeason>,
    /// A band the page still has to scroll into view, set by Enter on a season card: the band
    /// opens below the fold at the default window size, so without this the key looks like a
    /// no-op until the next one.
    reveal_band: Option<BandReveal>,
    /// The open season's episodes request; the next season's replaces it, which cancels it.
    episodes: Option<Task<jellyfin::Result<jellyfin::models::QueryResult<BaseItemDto>>>>,
    /// The More menu was asked for this frame, by the button or by Enter on the cursor (which
    /// starts its highlight on the first entry): its popup opens before the frame draws it.
    more_requested: Option<Opened>,
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
            fills: Fills::default(),
            version: 0,
            cursor: Cursor::default(),
            columns: 1,
            open_season: None,
            reveal_band: None,
            episodes: None,
            more_requested: None,
            marks_seen,
            add_to_playlist: None,
            edit_playlist: None,
            delete_playlist: None,
        }
    }

    /// Fetches the item again. What's on screen stays up until the new data arrives; a load
    /// still running is replaced, which cancels it.
    fn reload(&mut self, cx: &mut PageCx) {
        // A reload in the middle of playlist edits would show the playlist half-edited; the
        // services bump `user_data` when its last edit is done, which loads it again.
        if cx.services.playlist_edits_pending(&self.item_id) {
            return;
        }
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
        self.show_mark(&item_id, change, false, cx.ctx.input(|input| input.time));
        cx.services.mark(item_id, change);
    }

    /// Shows `change` on each copy of the item on the page (itself, a child or an episode of
    /// the open season), or with `undo`, takes it back from those still showing it. As on
    /// `main`, the item's own toggle pops its icon when it turns on.
    fn show_mark(&mut self, item_id: &str, change: Mark, undo: bool, now: f64) {
        let Load::Loaded(data) = &mut self.data else {
            return;
        };
        let before = (is_played(&data.item), is_favorite(&data.item));
        let episodes = match self.open_season.as_mut().map(|season| &mut season.episodes) {
            Some(Load::Loaded(episodes)) => Some(episodes),
            _ => None,
        };
        for item in std::iter::once(&mut data.item)
            .chain(data.children.iter_mut())
            .chain(episodes.into_iter().flatten())
            .filter(|item| item.id == item_id)
        {
            if !undo {
                change.apply(item);
            } else if change.shown_on(item) {
                change.undone().apply(item);
            }
        }
        let after = (is_played(&data.item), is_favorite(&data.item));
        self.fills.update(before, after, now);
    }

    /// Opens `season`'s episodes under the seasons, or closes them if they're open. Returns
    /// whether it opened them.
    fn toggle_season(&mut self, season: &BaseItemDto, cx: &mut PageCx) -> bool {
        let opened = !self.open_season.as_ref().is_some_and(|open| open.id == season.id);
        if opened {
            self.open_season = Some(OpenSeason {
                id: season.id.clone(),
                name: season.name.clone(),
                episodes: Load::Loading,
            });
            self.load_episodes(cx);
        } else {
            self.open_season = None;
            // Its episodes aren't shown any more, so their request goes with them.
            self.episodes = None;
            // A band that isn't drawn has nothing left to reveal.
            self.reveal_band = None;
        }
        cx.ctx.request_repaint();
        opened
    }

    /// Fetches the open season's episodes, unless another season opens meanwhile (its request
    /// replaces this one, which cancels it).
    fn load_episodes(&mut self, cx: &mut PageCx) {
        let Some(open) = &self.open_season else {
            return;
        };
        let client = cx.services.session.client.clone();
        let series_id = self.item_id.clone();
        let season_id = open.id.clone();
        self.episodes = Some(Task::spawn(cx.ctx, async move {
            client
                .episodes(&series_id, Some(&season_id), &[ItemField::Overview])
                .await
        }));
    }

    /// The open season's episodes, once loaded.
    fn open_episodes(&self) -> &[BaseItemDto] {
        match self.open_season.as_ref().map(|open| &open.episodes) {
            Some(Load::Loaded(episodes)) => episodes,
            _ => &[],
        }
    }

    /// The children's layout: a series' or collection's cards in the grid last shown, and a
    /// season's or playlist's entries one per row.
    fn children_section(&self, data: &ItemData) -> Section {
        match data.item.kind {
            BaseItemKind::Series | BaseItemKind::BoxSet => Section::grid(data.children.len(), self.columns),
            BaseItemKind::Season | BaseItemKind::Playlist => Section::grid(data.children.len(), 1),
            _ => Section::row(0),
        }
    }

    /// Moves or removes a playlist entry on screen (its highlight following it), ready to be
    /// sent to the server. `None` when there's nothing to do: not a playlist, an end of the
    /// list, or an entry the server didn't give an id.
    fn move_entry(&mut self, index: usize, action: EntryAction) -> Option<PlaylistEdit> {
        let Load::Loaded(data) = &mut self.data else {
            return None;
        };
        if data.item.kind != BaseItemKind::Playlist || index >= data.children.len() {
            return None;
        }
        let entry_id = data.children[index].playlist_item_id.clone()?;
        let to = match action {
            EntryAction::Up if index > 0 => index - 1,
            EntryAction::Down if index + 1 < data.children.len() => index + 1,
            EntryAction::Up | EntryAction::Down => return None,
            EntryAction::Remove => {
                data.children.remove(index);
                // The cursor stays on the row that took this one's place.
                let len = data.children.len();
                if self.cursor.is_selected(CHILDREN, index) && len > 0 {
                    self.cursor.show_at(Position {
                        section: CHILDREN,
                        index: index.min(len - 1),
                    });
                }
                return Some(PlaylistEdit::Remove { entry_id });
            }
        };
        let entry = data.children.remove(index);
        data.children.insert(to, entry);
        if self.cursor.is_selected(CHILDREN, index) {
            self.cursor.show_at(Position {
                section: CHILDREN,
                index: to,
            });
        }
        Some(PlaylistEdit::Move { entry_id, to })
    }

    /// Moves or removes a playlist entry: on screen right away, then on the server. The services
    /// queue it (they send them one at a time in order, whatever page is on top).
    fn edit_entry(&mut self, index: usize, action: EntryAction, cx: &mut PageCx) {
        let Some(edit) = self.move_entry(index, action) else {
            return;
        };
        cx.services.edit_playlist(self.item_id.clone(), edit);
    }

    /// What Alt+Up, Alt+Down and Delete do to a playlist's highlighted entry.
    fn entry_action(action: Action) -> Option<EntryAction> {
        match action {
            Action::MoveEntryUp => Some(EntryAction::Up),
            Action::MoveEntryDown => Some(EntryAction::Down),
            Action::RemoveEntry => Some(EntryAction::Remove),
            _ => None,
        }
    }

    /// The children rows' keys: W toggles the highlighted entry watched, and a playlist's entries
    /// also move and remove. Returns whether the page used the key.
    fn children_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        let Some(position) = self.cursor.selected() else {
            return false;
        };
        if position.section != CHILDREN {
            return false;
        }
        let Load::Loaded(data) = &self.data else {
            return false;
        };
        if action == Action::ToggleWatched {
            let Some(child) = data.children.get(position.index) else {
                return false;
            };
            let (id, played) = (child.id.clone(), is_played(child));
            self.set_played(id, !played, cx);
            return true;
        }
        if data.item.kind != BaseItemKind::Playlist {
            return false;
        }
        let Some(action) = Self::entry_action(action) else {
            return false;
        };
        self.edit_entry(position.index, action, cx);
        true
    }

    /// Takes back what this page showed ahead of the server for every mark it was refused since
    /// it last looked, then catches up with the log. The page may have been off screen when the
    /// server refused (under another page, behind the player, or with the window hidden), so the
    /// refusal waits for it here instead of being lost.
    fn take_back_refused_marks(&mut self, refused: &RefusedMarks, now: f64) {
        let refusals: Vec<(String, Mark)> = refused
            .after(self.marks_seen)
            .map(|entry| (entry.item_id.clone(), entry.mark))
            .collect();
        self.marks_seen = refused.sequence();
        // Newest first, so two quick toggles both refused unwind to where they started.
        for (item_id, mark) in refusals.into_iter().rev() {
            self.show_mark(&item_id, mark, true, now);
        }
    }

    /// Does what a button, a card or a menu entry asked for.
    fn run_action(&mut self, action: ItemAction, cx: &mut PageCx) {
        match action {
            ItemAction::More => {
                self.more_requested.get_or_insert(Opened::Pointer);
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

    /// The More menu while it's open: the item's less frequent actions. Returns the one picked,
    /// for the caller to carry out once the menu is closed.
    fn more_menu(&mut self, response: &Response, id: Id, cx: &mut PageCx) -> Option<ItemAction> {
        let Load::Loaded(data) = &self.data else {
            return None;
        };
        let actions = more_actions(&data.item);
        let entries: Vec<Entry> = actions.iter().map(|&action| more_entry(action)).collect();
        let shown = PopupMenu::new(id)
            .align(RectAlign::BOTTOM_END)
            .width(MENU_W)
            .show(response, &entries);
        if shown.search {
            cx.nav.push(NavRequest::OpenSearch);
        }
        shown.chosen.map(|index| actions[index])
    }

    /// The version picker's menu while it's open: an item's versions, the chosen one checked.
    /// The keyboard can walk it once it's open; on the cursor, the picker changes the choice
    /// without it (Left and Right step, Enter cycles).
    fn versions_menu(&mut self, response: &Response, id: Id, cx: &mut PageCx) {
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let sources = &data.item.media_sources;
        let current = self.version.min(sources.len().saturating_sub(1));
        let entries: Vec<Entry> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| Entry::new(version_label(source, index)).checked(index == current))
            .collect();
        let shown = PopupMenu::new(id).width(PICKER_W).show(response, &entries);
        if shown.search {
            cx.nav.push(NavRequest::OpenSearch);
        }
        if let Some(index) = shown.chosen {
            self.version = index;
            cx.ctx.request_repaint();
        }
    }

    fn show_content(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let more_id = more_menu_id(ui);
        let versions_id = versions_menu_id(ui);
        // The season cards' column count comes from the page's width, which the padded content
        // area has; the keys then use the layout the page last showed.
        let columns = grid_columns((ui.available_width() - 2. * PAGE_PADDING).max(1.));
        self.columns = columns;
        let Load::Loaded(data) = &self.data else {
            return;
        };
        let actions = item_actions(&data.item, data.up_next.as_ref());
        let version = self.version;
        let open = self.open_season.as_ref().map(|season| season.id.clone());
        // Rows of cards only for a series' seasons and a collection's items; the rows under a
        // season's or playlist's heading are one per line.
        let children_columns = match data.item.kind {
            BaseItemKind::Series | BaseItemKind::BoxSet => columns,
            _ => 1,
        };
        let moved = self.cursor.take_reveal();
        let selected = self.cursor.selected();
        let images = &mut cx.services.images;
        let cursor = &mut self.cursor;
        let fills = &self.fills;
        let mut asked = Asked::default();
        // The page's own scroll target, set once every shelf is drawn: the next scroll area to
        // finish takes a target, and a shelf's row would be it (U3).
        let mut target = None;

        let area = egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: PAGE_PADDING as i8,
                    right: PAGE_PADDING as i8,
                    top: 0,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let header = header(ui, data, &actions, selected, fills, images, &mut asked);
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
                    let mut children_title = None;
                    let mut children_row = None;
                    if let Some(title) = children_heading(data) {
                        ui.add_space(SECTION_GAP);
                        children_title = Some(ui.label(RichText::new(title).heading().strong()).rect);
                        ui.add_space(12.);
                        children_row = match data.item.kind {
                            BaseItemKind::Series | BaseItemKind::BoxSet => {
                                season_rows(ui, data, columns, open.as_deref(), cursor, images, &mut asked)
                            }
                            _ => {
                                let playlist = data.item.kind == BaseItemKind::Playlist;
                                entry_rows(ui, data, playlist, cursor, images, &mut asked)
                            }
                        };
                    }
                    let mut episodes_row = None;
                    if let Some(open) = &self.open_season {
                        ui.add_space(SECTION_GAP);
                        // The band rises in each time a season opens, as `main`'s did.
                        episodes_row = Some(motion::enter(ui, ("season-episodes", open.id.as_str()), 0.0, |ui| {
                            episodes_band(ui, open, moved, cursor, images, &mut asked)
                        }));
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
                        // A key moved the cursor: its card comes into view, and a band that
                        // was still waiting for its reveal has been left behind.
                        self.reveal_band = None;
                        target = match selected {
                            Some(position) if position.section == ACTIONS => Some((header, Some(Align::TOP))),
                            Some(position) if position.section == PICKERS => pickers.map(|rect| (rect, None)),
                            Some(position) if position.section == CHAPTERS => chapters_row.map(|rect| (rect, None)),
                            // The heading comes into view with their first row, as `main`'s
                            // reveal scrolled the title and then the row: the least scroll
                            // that shows both, rather than pushing the header away.
                            Some(position) if position.section == CHILDREN => children_row.map(|row| {
                                let first_row = position.index / children_columns.max(1) == 0;
                                let rect = match children_title {
                                    Some(title) if first_row => title.union(row),
                                    _ => row,
                                };
                                (rect, None)
                            }),
                            // The band with its gap, scrolled as little as shows it: a short
                            // window keeps the seasons above in view.
                            Some(position) if position.section == EPISODES => {
                                episodes_row.map(|rect| (with_gap_above(rect, SECTION_GAP), None))
                            }
                            Some(position) if position.section == PEOPLE => people_row.map(|rect| (rect, None)),
                            Some(position) if position.section == EXTRAS => extras_row.map(|rect| (rect, None)),
                            Some(position) if position.section == SIMILAR => similar_row.map(|rect| (rect, None)),
                            _ => None,
                        };
                    } else if let Some(reveal) = self.reveal_band
                        && let Some(band) = episodes_row
                        && reveal != BandReveal::Waiting
                    {
                        // Enter opened a season: the least scroll that shows its row with the
                        // band, once as it opens and once more when its episodes arrive (the
                        // loaded band is taller than the spinner's line).
                        target = Some((band_reveal(children_row, band, ui.clip_rect().height()), None));
                        // The loaded band gets a reveal of its own when its episodes arrive.
                        self.reveal_band = if reveal == BandReveal::Opening {
                            Some(BandReveal::Waiting)
                        } else {
                            None
                        };
                    }
                    if let Some((rect, align)) = target {
                        ui.scroll_to_rect(rect, align);
                    }
                });
        });
        backdrop::scroll_edge(ui, &area);

        if let Some(action) = asked.action {
            // A second click on More closes its menu, as its trigger did on `main`.
            if action == ItemAction::More && PopupMenu::is_open(cx.ctx, more_id) {
                PopupMenu::close(cx.ctx, more_id);
            } else {
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
        if let Some(season) = asked.season {
            // A click leaves the band where it opens, as on `main`: only the keyboard's Enter
            // scrolls to it, and a click that moved the page under the pointer would surprise.
            self.toggle_season(&season, cx);
        }
        if let Some((index, action)) = asked.entry {
            self.edit_entry(index, action, cx);
        }
        if let Some((item_id, played)) = asked.played {
            self.set_played(item_id, played, cx);
        }
        if let Some(start) = asked.play_from {
            self.play(Some(start), cx);
        }
        if let Some(extra) = asked.extra {
            cx.services.play(&extra, PlayOptions::default());
        }
        if let Some(response) = &asked.version_picker {
            if response.clicked() {
                PopupMenu::toggle(cx.ctx, versions_id, Opened::Pointer);
            }
            self.versions_menu(response, versions_id, cx);
        } else {
            PopupMenu::close(cx.ctx, versions_id);
        }

        if let Some(response) = &asked.more {
            if let Some(opened) = self.more_requested.take() {
                PopupMenu::open(cx.ctx, more_id, opened);
            }
            if let Some(action) = self.more_menu(response, more_id, cx) {
                self.run_action(action, cx);
            }
        } else {
            PopupMenu::close(cx.ctx, more_id);
            self.more_requested = None;
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
    shelf::shelf_with_gaps(ui, "chapters", "Chapters", CARD_GAP, SHELF_TITLE_GAP, 0., |ui| {
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

/// The heading over the item's children: a series' seasons, a collection's items, a season's
/// episodes or a playlist's entries, as on `main`. Nothing when there are none.
fn children_heading(data: &ItemData) -> Option<&'static str> {
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

/// Poster cards at [`BIG_CARD_SCALE`] that fit `available` points in a row, [`BIG_CARD_GAP`]
/// between them: `main`'s count for a series' seasons.
fn grid_columns(available: f32) -> usize {
    let card = POSTER_W * BIG_CARD_SCALE;
    (((available + BIG_CARD_GAP) / (card + BIG_CARD_GAP)).floor() as usize).max(1)
}

/// `rect` with `gap` points added above it, for a scroll that leaves the section's gap.
fn with_gap_above(rect: Rect, gap: f32) -> Rect {
    Rect::from_min_max(pos2(rect.left(), rect.top() - gap), rect.max)
}

/// What a season the keyboard just opened scrolls to: the union of the season's row and its
/// episodes band, so the least scroll shows which season opened and its episodes (`None`
/// rather than `TOP`, as the children's heading is revealed). A window too short to hold both
/// shows the band alone, with the section's gap above it, as a move onto the band does.
fn band_reveal(row: Option<Rect>, band: Rect, visible: f32) -> Rect {
    match row.map(|row| row.union(band)) {
        Some(union) if union.height() <= visible => union,
        _ => with_gap_above(band, SECTION_GAP),
    }
}

/// A series' seasons (or a collection's items) as poster cards in rows of `columns`, the open
/// season highlighted like the cursor's. A season card opens its episodes here; a collection's
/// items open their pages. Right-clicking one gives the item menu, as on `main`. Returns the
/// row holding the cursor, for the page to scroll into view with its heading.
#[allow(clippy::too_many_arguments)]
fn season_rows(
    ui: &mut Ui,
    data: &ItemData,
    columns: usize,
    open: Option<&str>,
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
                let index = row * columns + column;
                let is_open = open == Some(child.id.as_str());
                let selected = cursor.is_selected(CHILDREN, index);
                let response = card::poster_scaled(ui, images, child, BIG_CARD_SCALE, selected || is_open);
                if response.clicked() {
                    if child.kind == BaseItemKind::Season {
                        asked.season = Some(child.clone());
                    } else {
                        asked.open = Some(Route::for_item(child));
                    }
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

/// A season's episodes or a playlist's entries, one row each: its image, its heading and
/// runtime, three lines of overview, the Watched toggle and, in a playlist, the entry's buttons.
/// A click (or Enter) opens the item; the toggle and the buttons don't, as on `main`. Returns
/// the row holding the cursor, for the page to scroll into view with the heading.
fn entry_rows(
    ui: &mut Ui,
    data: &ItemData,
    playlist: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Option<Rect> {
    let len = data.children.len();
    let mut selected_rect = None;
    for (index, entry) in data.children.iter().enumerate() {
        let selected = cursor.is_selected(CHILDREN, index);
        let response = entry_row(ui, entry, index, len, playlist, selected, images, asked);
        if selected {
            selected_rect = Some(response.rect);
            ui.painter().rect_stroke(
                response.rect,
                CornerRadius::same(8),
                Stroke::new(3., theme::tokens_of(ui).foreground),
                StrokeKind::Inside,
            );
        }
        if response.clicked() {
            asked.open = Some(Route::for_item(entry));
        }
        if let Some(action) = menu::item_menu(&response, entry) {
            asked.menu = Some((action, entry.clone()));
        }
    }
    selected_rect
}

/// One row of [`entry_rows`]. Returns the row's response, which covers the whole row; the
/// Watched toggle and the entry buttons are added after it, so they take the clicks on them.
#[allow(clippy::too_many_arguments)]
fn entry_row(
    ui: &mut Ui,
    entry: &BaseItemDto,
    index: usize,
    len: usize,
    playlist: bool,
    selected: bool,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Response {
    // `main`'s rows: a `text_base` heading with `text_sm` under it.
    let small = text::font(ui, "sm");
    let heading_font = text::font(ui, "base");
    let weak = theme::tokens_of(ui).muted_foreground;
    let width = ui.available_width();
    let inner_width = width - 2. * ROW_PAD;
    // The entry buttons, and the gap between them and the Watched toggle.
    let buttons_w = if playlist { 3. * ENTRY_BUTTON + ROW_GAP } else { 0. };
    let text_width = (inner_width - LANDSCAPE_W - 2. * ROW_GAP - buttons_w - WATCHED_SIZE).max(1.);
    let overview = entry
        .overview
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(|text| overview_galley(ui, text, text_width));
    let heading_h = ui.fonts_mut(|fonts| fonts.row_height(&heading_font));
    let runtime_h = ui.fonts_mut(|fonts| fonts.row_height(&small));
    let mut text_h = heading_h + ROW_LINE_GAP + runtime_h;
    if let Some(galley) = &overview {
        text_h += ROW_LINE_GAP + galley.size().y;
    }
    let height = LANDSCAPE_H.max(text_h) + 2. * ROW_PAD;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if selected || ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), theme::tokens_of(ui).accent);
    }
    let inner = rect.shrink(ROW_PAD);
    // The row's content goes in a Ui of its own, so that placing the toggle and the buttons in
    // their rects moves that Ui's cursor rather than the page's (`scope_builder` would
    // otherwise leave the page's cursor inside the row, and the next one would overlap it).
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    let image_rect = Rect::from_min_size(inner.min, vec2(LANDSCAPE_W, LANDSCAPE_H));
    image::show(
        &row,
        images,
        image_rect,
        landscape_image(entry).as_ref(),
        Some(entry),
        image::RADIUS,
    );

    // The text beside the image: episodes read "S1:E2 · Name"; a movie in a playlist its name,
    // and the year on the line below.
    let (name, episode_line) = card_text(entry);
    let is_episode = entry.kind == BaseItemKind::Episode;
    let heading = if is_episode { episode_line } else { name };
    let runtime = [
        entry
            .production_year
            .filter(|_| !is_episode)
            .map(|year| year.to_string()),
        entry.run_time_ticks.map(runtime_text),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    // A very narrow window (or a big zoom) can leave nothing beside the image; the column keeps
    // a point rather than turning the rect inside out.
    let text_left = image_rect.right() + ROW_GAP;
    let text_rect = Rect::from_min_max(
        pos2(text_left, inner.top()),
        pos2(
            (inner.right() - buttons_w - WATCHED_SIZE - ROW_GAP).max(text_left + 1.),
            inner.bottom(),
        ),
    );
    let mut text_ui = row.new_child(
        UiBuilder::new()
            .max_rect(text_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    text_ui.spacing_mut().item_spacing.y = ROW_LINE_GAP;
    text_ui.add(Label::new(RichText::new(&heading).font(heading_font).strong()).truncate());
    if !runtime.is_empty() {
        text_ui.add(Label::new(RichText::new(&runtime).font(small).color(weak)).truncate());
    }
    if let Some(galley) = overview {
        text_ui.add(Label::new(galley).selectable(false));
    }

    // The Watched toggle at the row's right end, pressed while watched like the page's own.
    let played = is_played(entry);
    let watched_rect = Rect::from_min_size(
        pos2(inner.right() - WATCHED_SIZE, inner.top()),
        vec2(WATCHED_SIZE, WATCHED_SIZE),
    );
    let toggle = row
        .scope_builder(UiBuilder::new().max_rect(watched_rect), |ui| {
            // On, the toggle's accent fill would merge with the row's own accent highlight:
            // a muted outline keeps it readable.
            let button = egui::Button::new(menu::played_icon(ui, played))
                .sense(Sense::CLICK)
                .min_size(vec2(WATCHED_SIZE, WATCHED_SIZE));
            let mut button = theme::selected(ui, played, button);
            if played {
                button = button.stroke(Stroke::new(1., theme::tokens_of(ui).muted_foreground));
            }
            ui.add(button)
        })
        .inner
        .on_hover_text("Watched (W)");
    if toggle.clicked() {
        // Don't open the item underneath.
        asked.played = Some((entry.id.clone(), !played));
    }

    if playlist {
        // Up, Down and Remove, left to right, as on `main`; added right to left here.
        let buttons = [
            (EntryAction::Remove, ICON_CLOSE, "Remove from playlist (Delete)"),
            (EntryAction::Down, ICON_ARROW_DOWNWARD, "Move down (Alt+Down)"),
            (EntryAction::Up, ICON_ARROW_UPWARD, "Move up (Alt+Up)"),
        ];
        let mut right = inner.right() - WATCHED_SIZE - ROW_GAP;
        for (action, icon, tooltip) in buttons {
            right -= ENTRY_BUTTON;
            let enabled = match action {
                EntryAction::Up => index > 0,
                EntryAction::Down => index + 1 < len,
                EntryAction::Remove => true,
            };
            let at = Rect::from_min_size(
                pos2(right, inner.top() + (WATCHED_SIZE - ENTRY_BUTTON) * 0.5),
                vec2(ENTRY_BUTTON, ENTRY_BUTTON),
            );
            let button = row
                .scope_builder(UiBuilder::new().max_rect(at), |ui| {
                    ui.add_enabled(
                        enabled,
                        egui::Button::new(icon.outlined().rich_text().size(18.))
                            .frame(false)
                            .sense(Sense::CLICK)
                            .min_size(vec2(ENTRY_BUTTON, ENTRY_BUTTON)),
                    )
                })
                .inner
                .on_hover_text(tooltip)
                .on_disabled_hover_text(tooltip);
            if button.clicked() {
                asked.entry = Some((index, action));
            }
        }
    }
    response
}

/// `text` wrapped to `width`, cut off after [`OVERVIEW_ROWS`] lines with an ellipsis, as
/// `main`'s `line_clamp(3)` did. Laid out here rather than through a `Label`: a label replaces
/// the job's wrapping, and with it the row limit.
fn overview_galley(ui: &Ui, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.,
        TextFormat {
            font_id: self::text::font(ui, "sm"),
            color: theme::tokens_of(ui).foreground,
            ..Default::default()
        },
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = OVERVIEW_ROWS;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The open season's episodes, as a band under the seasons: the season's name, then its episode
/// cards, a spinner while they load or a line when there are none. Returns the band's rectangle,
/// for the cursor scrolling it into view.
fn episodes_band(
    ui: &mut Ui,
    open: &OpenSeason,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let top = ui.cursor().min;
    // The band's background goes under what's drawn in it, but its height is only known once
    // it's drawn: a placeholder takes its place in the layer and is filled in at the end.
    let background = ui.painter().add(egui::Shape::Noop);
    egui::Frame::NONE
        .inner_margin(Margin {
            left: PAGE_PADDING as i8,
            right: PAGE_PADDING as i8,
            top: 16,
            bottom: 16,
        })
        .show(ui, |ui| match &open.episodes {
            Load::Loaded(episodes) if !episodes.is_empty() => {
                // Keyed by the season: each season's row starts at its first episode.
                shelf::shelf_with_gaps(
                    ui,
                    ("season-episodes", open.id.as_str()),
                    &open.name,
                    BIG_CARD_GAP,
                    SHELF_TITLE_GAP,
                    // The band already insets its content.
                    0.,
                    |ui| {
                        for (index, episode) in episodes.iter().enumerate() {
                            let selected = cursor.is_selected(EPISODES, index);
                            let response = card::episode(ui, images, episode, BIG_CARD_SCALE, selected);
                            if selected && moved {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                asked.open = Some(Route::for_item(episode));
                            }
                            if let Some(action) = menu::item_menu(&response, episode) {
                                asked.menu = Some((action, episode.clone()));
                            }
                        }
                    },
                );
            }
            Load::Loaded(_) => band_note(ui, "No episodes in this season.", false),
            Load::Loading => band_note(ui, "", true),
            Load::Failed(message) => band_note(ui, message, false),
        });
    let rect = Rect::from_min_max(
        pos2(ui.clip_rect().left(), top.y),
        pos2(ui.clip_rect().right(), ui.cursor().min.y),
    );
    let muted = theme::tokens_of(ui).muted;
    let band = Color32::from_rgba_unmultiplied(muted.r(), muted.g(), muted.b(), BAND_ALPHA);
    ui.painter()
        .set(background, egui::Shape::rect_filled(rect, CornerRadius::ZERO, band));
    // `main`'s `mb_6`: room between the band and the shelf under it.
    ui.add_space(SECTION_GAP);
    rect
}

/// A line in the episodes band at the height the cards would take: the mascot while they load,
/// or a message.
fn band_note(ui: &mut Ui, message: &str, loading: bool) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), LANDSCAPE_H),
        Layout::centered_and_justified(Direction::LeftToRight),
        |ui| {
            if loading {
                mascot::loader(ui, mascot::PAGE);
            } else {
                ui.label(RichText::new(message).color(theme::tokens_of(ui).muted_foreground));
            }
        },
    );
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
    /// A season card was clicked: its episodes open under the seasons, or close.
    season: Option<BaseItemDto>,
    /// A playlist entry's buttons were used: move or remove the entry at the index.
    entry: Option<(usize, EntryAction)>,
    /// An entry's Watched toggle was pressed: the item and the state to show.
    played: Option<(String, bool)>,
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
    fills: &Fills,
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
        let (request, size) = header_image(item);
        let (poster, _) = ui.allocate_exact_size(size, Sense::hover());
        image::show(ui, images, poster, request.as_ref(), Some(item), image::RADIUS);
        ui.vertical(|ui| {
            if backdrop.is_some() {
                // Lines the text up beside the poster, as on `main`.
                ui.add_space(120.);
            }
            info_column(ui, data, actions, selected, fills, asked);
        });
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

/// The header's picture and its size: the item's poster, or for an episode its own still in a
/// wide frame (an episode's image is a screen grab, which a tall poster would crop), and its
/// series' poster when it has no image of its own.
fn header_image(item: &BaseItemDto) -> (Option<ImageRequest>, egui::Vec2) {
    let poster = vec2(DETAIL_POSTER_W, DETAIL_POSTER_H);
    if item.kind != BaseItemKind::Episode {
        return (item.primary_image(), poster);
    }
    match item.image(ImageType::Primary) {
        // A tall image is a poster after all, and goes in the poster's frame.
        Some(own) => match item.primary_image_aspect_ratio {
            Some(ratio) if ratio < 1. => (Some(own), poster),
            // Episode stills are 16:9 when the server doesn't say.
            ratio => {
                let ratio = ratio.unwrap_or(16. / 9.).clamp(1., 2.4) as f32;
                (Some(own), vec2(DETAIL_STILL_H * ratio, DETAIL_STILL_H))
            }
        },
        None => (poster_image(item), poster),
    }
}

/// The title's column: the series line, the title and its actions, the meta line, genres,
/// studios, the synopsis and the tags.
fn info_column(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    fills: &Fills,
    asked: &mut Asked,
) {
    let item = &data.item;
    let weak = theme::tokens_of(ui).muted_foreground;
    ui.spacing_mut().item_spacing.y = 8.;
    if matches!(item.kind, BaseItemKind::Episode | BaseItemKind::Season)
        && let Some(series) = &item.series_name
    {
        // `main`'s series line took the body size; the lines under the title are `text_sm`. It
        // opens the series, underlined under the pointer like a link.
        let label = Label::new(RichText::new(series).font(text::font(ui, "base")).color(weak));
        match &item.series_id {
            Some(id) => {
                let response = ui
                    .add(label.sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    let rect = response.rect;
                    ui.painter()
                        .hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1., weak));
                }
                if response.clicked() {
                    asked.open = Some(Route::Item {
                        id: id.clone(),
                        name: series.clone(),
                    });
                }
            }
            None => {
                ui.add(label);
            }
        }
    }

    let episode = data
        .up_next
        .as_ref()
        .map(episode_label)
        .filter(|label| !label.is_empty());
    let actions_width = actions_width(ui, data, actions, episode.as_deref());
    // `main`'s 40-point title: a fixed size, so the text size setting doesn't move the header.
    let title_font = FontId::new(TITLE_SIZE, text::font(ui, "base").family);
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
            actions_row(ui, data, actions, selected, episode.as_deref(), fills, asked);
        });
    } else {
        // A narrow window: the actions go under the title rather than squeeze it to a word a
        // line.
        ui.add(Label::new(RichText::new(&item.name).font(title_font)).wrap());
        actions_row(ui, data, actions, selected, episode.as_deref(), fills, asked);
    }

    let meta = meta_line(item);
    if !meta.is_empty() {
        let font = text::font(ui, "sm");
        // The text font has no ★, so the rating's star is drawn from the icon font.
        let mut job = LayoutJob::default();
        let format = |font_id| TextFormat {
            font_id,
            color: weak,
            valign: Align::Center,
            ..Default::default()
        };
        for (index, part) in meta.split('★').enumerate() {
            if index > 0 {
                job.append(
                    ICON_STAR.codepoint,
                    0.,
                    format(FontId::new(font.size, ICON_STAR.font_family())),
                );
            }
            job.append(part, 0., format(font.clone()));
        }
        ui.label(job);
    }
    if !item.genres.is_empty() {
        ui.label(
            RichText::new(item.genres.join(", "))
                .font(text::font(ui, "sm"))
                .color(weak),
        );
    }
    if !item.studios.is_empty() {
        let studios: Vec<&str> = item.studios.iter().map(|studio| studio.name.as_str()).collect();
        ui.label(RichText::new(studios.join(", ")).font(text::font(ui, "sm")).color(weak));
    }
    if let Some(overview) = &item.overview
        && !overview.is_empty()
    {
        ui.add_space(8.);
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(TEXT_MAX_W));
            // The synopsis is `main`'s `text_base`, a step above the meta lines.
            ui.add(Label::new(RichText::new(overview.clone()).font(text::font(ui, "base"))).wrap());
        });
    }
    if !item.tags.is_empty() {
        ui.add_space(16.);
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(TEXT_MAX_W));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4., 4.);
                for tag in &item.tags {
                    // A frame doesn't wrap as a unit, so its label would break per letter in the
                    // row's leftover space; start a new row when the whole chip doesn't fit.
                    let text_width = ui
                        .painter()
                        .layout_no_wrap(tag.clone(), text::font(ui, "xs"), weak)
                        .size()
                        .x;
                    if ui.available_size_before_wrap().x < text_width + 16. + 1. {
                        ui.end_row();
                    }
                    egui::Frame::NONE
                        .fill(theme::tokens_of(ui).muted)
                        // `main`'s `rounded_lg`, 8 points.
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.add(Label::new(RichText::new(tag).font(text::font(ui, "xs")).color(weak)).extend());
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
    fills: &Fills,
    asked: &mut Asked,
) {
    let item = &data.item;
    let played = is_played(item);
    let favorite = is_favorite(item);
    let now = ui.input(|input| input.time);
    // With the animations setting off the icons are drawn at their size, at once.
    let animations = theme::animations(ui.ctx());
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
                    let icon = menu::played_icon(ui, played);
                    let scale = if animations { fills.played.scale(now) } else { 1. };
                    toggle(ui, icon, played, "Watched", scale)
                }
                ItemAction::ToggleFavorite => {
                    let icon = menu::favorite_icon(ui, favorite);
                    let scale = if animations { fills.favorite.scale(now) } else { 1. };
                    toggle(ui, icon, favorite, "Favorite", scale)
                }
                _ => {
                    let look = action_look(action, item, episode);
                    // A primary action's hover and pressed colours come from the theme's style,
                    // which only a scope can give this button.
                    let response = if look.primary {
                        ui.scope(|ui| {
                            theme::primary_style(ui);
                            ui.spacing_mut().button_padding.x = ACTION_PADDING;
                            ui.add(action_button(ui, &look))
                        })
                        .inner
                    } else {
                        ui.scope(|ui| {
                            ui.spacing_mut().button_padding.x = ACTION_PADDING;
                            ui.add(action_button(ui, &look))
                        })
                        .inner
                    };
                    let response = response.on_hover_text(look.tooltip.as_str());
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
                    Stroke::new(2., theme::tokens_of(ui).foreground),
                    StrokeKind::Outside,
                );
            }
            if response.clicked() {
                asked.action = Some(action);
            }
        }
    });
    if animations && (fills.played.popping(now) || fills.favorite.popping(now)) {
        // The icons are still growing into place: keep the frames coming.
        ui.ctx().request_repaint();
    }
}

/// An action button: its icon, and a label for the actions that are named. Not focusable: the
/// page's keys are the browse cursor's (`keys.rs`).
fn action_button(ui: &Ui, look: &ActionLook) -> egui::Button<'static> {
    let body = TextStyle::Button.resolve(ui.style());
    let tokens = theme::tokens_of(ui);
    let color = if look.primary {
        tokens.primary_foreground
    } else {
        tokens.foreground
    };
    let label = look.label.as_deref().unwrap_or_default();
    egui::Button::new(button::icon_text(look.icon, ACTION_ICON, label, body, color))
        .sense(Sense::CLICK)
        .min_size(vec2(ACTION_SIZE, ACTION_SIZE))
}

/// Watched and Favorite: pressed while on (the theme's accent behind it, as `main`'s checked
/// toggles had), with the icon filled in green or red and a muted outline so the pressed toggle
/// still reads on a highlighted row, whose own fill is the same accent. `scale` is the icon's
/// pop as it turns on, which fits in the button's own size.
fn toggle(ui: &mut Ui, icon: RichText, on: bool, tooltip: &str, scale: f32) -> Response {
    let tokens = theme::tokens_of(ui);
    let mut button = egui::Button::new(icon.size(ACTION_ICON * scale))
        .selected(on)
        .sense(Sense::CLICK)
        .min_size(vec2(ACTION_SIZE, ACTION_SIZE));
    if on {
        // A selected button paints itself with the selection colour; `main`'s toggles used the
        // accent, and the icon keeps its own green or red.
        button = button
            .fill(tokens.accent)
            .stroke(Stroke::new(1., tokens.muted_foreground));
    }
    ui.add(button).on_hover_text(tooltip)
}

/// The version label's column and the picker, when the item has more than one version.
fn pickers_row(ui: &mut Ui, data: &ItemData, selected: Option<Position>, version: usize, asked: &mut Asked) -> Rect {
    let top = ui.cursor().min;
    let sources = &data.item.media_sources;
    let current = version.min(sources.len() - 1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.;
        let (label, _) = ui.allocate_exact_size(vec2(PICKER_LABEL_W, PICKER_H), Sense::hover());
        ui.painter().text(
            label.left_center(),
            Align2::LEFT_CENTER,
            "Version",
            text::font(ui, "sm"),
            theme::tokens_of(ui).muted_foreground,
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

/// The version select (`widgets::select`), with the cursor's ring round it. Clicking it opens
/// the list of versions; the keyboard changes the choice on the cursor instead.
fn picker(ui: &mut Ui, name: &str, ringed: bool) -> Response {
    let response = select::button(ui, vec2(PICKER_W, PICKER_H), name, "Version", false);
    if ringed {
        ui.painter().rect_stroke(
            response.rect.expand(ACTION_RING_GAP),
            CornerRadius::same(8),
            Stroke::new(2., theme::tokens_of(ui).foreground),
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
    shelf::shelf_with_gaps(ui, "people", "Cast & crew", BIG_CARD_GAP, SHELF_TITLE_GAP, 0., |ui| {
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
        // The page already insets its content.
        0.,
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
    shelf::shelf_with_gaps(
        ui,
        "similar",
        "More like this",
        BIG_CARD_GAP,
        SHELF_TITLE_GAP,
        0.,
        |ui| {
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
        },
    )
}

/// Fades the backdrop image into the page at its bottom, as `main`'s gradient did: into the
/// window's backdrop at that height, so the picture melts into the gradient rather than ending
/// on a flat band of colour.
fn fade_to_background(ui: &Ui, band: Rect) {
    // `main`'s first stop: the background at 30 %, so the image shows through at the top.
    let window = ui.ctx().content_rect();
    backdrop::fade_into(ui.painter(), band, window, 77. / 255., theme::tokens_of(ui));
}

/// A More menu row. Deleting a playlist can't be taken back, so it's set apart and in red.
fn more_entry(action: ItemAction) -> Entry<'static> {
    match action {
        ItemAction::AddToPlaylist => Entry::new("Add to playlist…").icon(ICON_PLAYLIST_ADD),
        ItemAction::EditPlaylist => Entry::new("Edit…").icon(ICON_EDIT),
        _ => Entry::new("Delete playlist…").icon(ICON_DELETE).danger().separated(),
    }
}

/// A popup's id, scoped to the page: the shell gives every page its own id scope, so stacked
/// item pages don't share a menu.
fn more_menu_id(ui: &Ui) -> Id {
    ui.make_persistent_id("item_more_menu")
}

fn versions_menu_id(ui: &Ui) -> Id {
    ui.make_persistent_id("item_versions")
}

impl Page for ItemPage {
    fn title(&self) -> &str {
        &self.name
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Browse)
    }

    fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        // The children rows' own keys come before the cursor's, as `main`'s `on_key_down` did:
        // W is a letter, not a move.
        if self.children_key(action, cx) {
            return true;
        }
        browse::on_key(self, action, cx)
    }

    fn hide_cursor(&mut self) {
        self.cursor.hide();
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let now = ui.input(|input| input.time);
        if self.loaded_at != Some(cx.services.user_data) {
            self.reload(cx);
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            // An edit in flight would be overwritten by what the server had before it: the
            // playlist shows its edits until the last one bumps `user_data`.
            if !cx.services.playlist_edits_pending(&self.item_id) {
                match result {
                    Ok(data) => {
                        // Marked watched or favorite (here or elsewhere) since it last loaded.
                        if let Load::Loaded(old) = &self.data {
                            self.fills.update(
                                (is_played(&old.item), is_favorite(&old.item)),
                                (is_played(&data.item), is_favorite(&data.item)),
                                now,
                            );
                        }
                        if !data.item.name.is_empty() {
                            self.name = data.item.name.clone();
                        }
                        self.data = Load::Loaded(data);
                        // Its episodes' watched state and progress may have changed too.
                        if self.open_season.is_some() {
                            self.load_episodes(cx);
                        }
                    }
                    Err(err) => self.data = Load::Failed(cx.services.handle_error(&err)),
                }
            }
        }
        if let Some(result) = self.episodes.as_mut().and_then(Task::poll) {
            self.episodes = None;
            if let Some(open) = &mut self.open_season {
                open.episodes = match result {
                    Ok(episodes) => Load::Loaded(episodes.items),
                    Err(err) => Load::Failed(cx.services.handle_error(&err)),
                };
            }
            // The band's height is final now, so the reveal that waited for the episodes can
            // scroll it into view with its season's row.
            if self.reveal_band == Some(BandReveal::Waiting) {
                self.reveal_band = Some(BandReveal::Loaded);
            }
        }
        // Marks the server refused: take back what the page showed ahead of it, however long
        // ago the refusal came.
        self.take_back_refused_marks(&cx.services.refused_marks, now);
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
            // Nothing to hang the menus from while the page loads or failed.
            PopupMenu::close(cx.ctx, more_menu_id(ui));
            PopupMenu::close(cx.ctx, versions_menu_id(ui));
            self.more_requested = None;
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

    /// The sections in the order they appear on the page: a series' or collection's children as
    /// the grid last shown, and the open season's episodes as a row.
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
            self.children_section(data),
            Section::row(self.open_episodes().len()),
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
                        self.more_requested = Some(Opened::Keyboard);
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
            // A season card opens its episodes under the seasons; a collection's card, a
            // season's episode and a playlist's entry open their pages.
            CHILDREN => {
                if let Some(child) = data.children.get(position.index) {
                    if child.kind == BaseItemKind::Season {
                        let child = child.clone();
                        // The band opens below the fold at the default window size: the key
                        // brings it into view, as a move onto the band would.
                        if self.toggle_season(&child, cx) {
                            self.reveal_band = Some(BandReveal::Opening);
                        }
                    } else {
                        cx.nav.push(NavRequest::Push(Route::for_item(child)));
                    }
                }
            }
            EPISODES => {
                if let Some(episode) = self.open_episodes().get(position.index) {
                    cx.nav.push(NavRequest::Push(Route::for_item(episode)));
                }
            }
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
    use egui::{Rect, pos2};
    use jellyfin::Ticks;
    use jellyfin::models::BaseItemDto;

    use super::{CHILDREN, EntryAction, ItemAction, ItemData, ItemPage, band_reveal, item_actions};
    use crate::browse::{self, Position, Section};
    use crate::item_menu::{Mark, PlaylistEdit, is_favorite, is_played};
    use crate::keys::Action;
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
        page.take_back_refused_marks(&refused, 0.);
        assert_eq!(shown(&page), (false, false));

        // Its sequence caught up: the same log doesn't take a later mark back.
        page.show_mark("movie", Mark::Played(true), false, 1.);
        page.show_mark("movie", Mark::Favorite(true), false, 1.);
        page.take_back_refused_marks(&refused, 1.);
        assert_eq!(shown(&page), (true, true));

        // A page made after the refusals starts at the log's sequence and takes nothing back.
        let mut fresh = loaded_page(true, true, refused.sequence());
        fresh.take_back_refused_marks(&refused, 2.);
        assert_eq!(shown(&fresh), (true, true));

        // Only the newest refusals are kept.
        let mut many = RefusedMarks::default();
        for i in 0..REFUSED_MARKS_KEPT + 5 {
            many.push(format!("item-{i}"), Mark::Played(true));
        }
        assert_eq!(many.after(0).count(), REFUSED_MARKS_KEPT);
        assert_eq!(many.sequence(), (REFUSED_MARKS_KEPT + 5) as u64);
    }

    /// A page whose item is of `kind`, with `children` under it.
    fn page_with_children(kind: &str, children: Vec<BaseItemDto>) -> ItemPage {
        let item: BaseItemDto = serde_json::from_value(serde_json::json!({
            "Id": "item",
            "Name": "Item",
            "Type": kind,
        }))
        .expect("item");
        let mut page = ItemPage::new("item".into(), "Item".into(), 0);
        page.data = Load::Loaded(ItemData {
            item,
            children,
            extras: Vec::new(),
            similar: Vec::new(),
            up_next: None,
        });
        page
    }

    /// A playlist entry, whose entry id is its item id (as Jellyfin 12.2 gives them).
    fn entry(id: &str) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": id.to_uppercase(),
            "Type": "Movie",
            "PlaylistItemId": id,
        }))
        .expect("entry")
    }

    fn data(page: &ItemPage) -> &ItemData {
        match &page.data {
            Load::Loaded(data) => data,
            _ => panic!("the page should be loaded"),
        }
    }

    fn order(page: &ItemPage) -> Vec<&str> {
        data(page).children.iter().map(|entry| entry.id.as_str()).collect()
    }

    #[test]
    fn a_series_lays_its_seasons_out_in_the_grid_last_shown() {
        let mut page = page_with_children("Series", vec![entry("s1"), entry("s2"), entry("s3"), entry("s4")]);
        page.columns = 3;
        assert_eq!(page.children_section(data(&page)), Section::grid(4, 3));
        // A season's episodes and a playlist's entries are one per row, so Up and Down walk
        // them.
        let season = page_with_children("Season", vec![entry("e1"), entry("e2")]);
        assert_eq!(season.children_section(data(&season)), Section::grid(2, 1));
        let playlist = page_with_children("Playlist", vec![entry("a")]);
        assert_eq!(playlist.children_section(data(&playlist)), Section::grid(1, 1));
        // Nothing under a movie.
        let movie = page_with_children("Movie", Vec::new());
        assert_eq!(movie.children_section(data(&movie)), Section::row(0));
    }

    #[test]
    fn a_playlist_entry_follows_its_move_and_stops_at_the_ends() {
        let mut page = page_with_children("Playlist", vec![entry("a"), entry("b"), entry("c")]);
        // Down from the top swaps the first two, and the change carries the entry's id.
        assert_eq!(
            page.move_entry(0, EntryAction::Down),
            Some(PlaylistEdit::Move {
                entry_id: "a".into(),
                to: 1,
            })
        );
        assert_eq!(order(&page), ["b", "a", "c"]);
        // At either end there's nothing to send.
        assert_eq!(page.move_entry(0, EntryAction::Up), None);
        assert_eq!(page.move_entry(2, EntryAction::Down), None);
        assert_eq!(order(&page), ["b", "a", "c"]);

        // A highlighted entry's cursor follows it, and the move brings it back into view.
        page.cursor.show_at(Position {
            section: CHILDREN,
            index: 1,
        });
        page.cursor.take_reveal();
        assert_eq!(
            page.move_entry(1, EntryAction::Down),
            Some(PlaylistEdit::Move {
                entry_id: "a".into(),
                to: 2,
            })
        );
        assert_eq!(order(&page), ["b", "c", "a"]);
        assert_eq!(
            page.cursor.selected(),
            Some(Position {
                section: CHILDREN,
                index: 2,
            })
        );
        assert!(page.cursor.take_reveal());

        // Removing it leaves the cursor on the row that takes its place.
        assert_eq!(
            page.move_entry(2, EntryAction::Remove),
            Some(PlaylistEdit::Remove { entry_id: "a".into() })
        );
        assert_eq!(order(&page), ["b", "c"]);
        assert_eq!(
            page.cursor.selected(),
            Some(Position {
                section: CHILDREN,
                index: 1,
            })
        );
    }

    /// A season the keyboard opened scrolls to its row and its band, unless the window is too
    /// short for both, which shows the band alone.
    #[test]
    fn an_opened_bands_reveal_shows_its_season_row_with_the_band() {
        let row = Rect::from_min_max(pos2(0., 100.), pos2(400., 350.));
        let band = Rect::from_min_max(pos2(0., 374.), pos2(1000., 574.));
        // Room for both: the union, so the least scroll shows the season's row and its band.
        assert_eq!(band_reveal(Some(row), band, 600.), row.union(band));
        let band_alone = Rect::from_min_max(pos2(0., 350.), pos2(1000., 574.));
        // A window too short for the two shows the band with the gap above it, as a move onto
        // the band does.
        assert_eq!(band_reveal(Some(row), band, 300.), band_alone);
        // And the same when there's no row to hold in view.
        assert_eq!(band_reveal(None, band, 600.), band_alone);
    }

    #[test]
    fn only_a_playlists_entries_take_the_row_keys() {
        assert_eq!(ItemPage::entry_action(Action::MoveEntryUp), Some(EntryAction::Up));
        assert_eq!(ItemPage::entry_action(Action::MoveEntryDown), Some(EntryAction::Down));
        assert_eq!(ItemPage::entry_action(Action::RemoveEntry), Some(EntryAction::Remove));
        // W toggles watched, on any children row; the move keys are the playlist's alone.
        assert_eq!(ItemPage::entry_action(Action::ToggleWatched), None);
        assert_eq!(ItemPage::entry_action(Action::Move(browse::Move::Up)), None);
    }
}
