//! Detail page for a movie, series, season or episode, and for collections and playlists
//! (whose entries can be reordered and removed here).

use std::collections::VecDeque;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants, Toggle, ToggleVariants};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, BaseItemPerson, ChapterInfo, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery, Ticks};

use crate::browse::{self, Browse, Cursor, Position, Section, on_browse_actions};
use crate::changes::{Changes, UserDataChanged};
use crate::components::{
    CARD_GAP, LANDSCAPE_H, LANDSCAPE_W, Load, POSTER_W, card_text, centered_message, centered_spinner, chapter_card,
    episode_card, episode_label, error_state, landscape_card, landscape_image, menu_left_focus, menu_popover,
    person_card, poster_card_scaled, remote_image, rounded_image, runtime_text, selection_ring, shelf, shelf_with_rail,
    timestamp,
};
use crate::images::ImageCache;
use crate::item_menu::{
    self, Mark, can_add_to_playlist, episodes_of, first_unwatched, is_favorite, is_playable, is_played, resume_point,
    with_item_menu,
};
use crate::motion;
use crate::playback::{PlayOptions, PlaybackManager};
use crate::rail_drag::RailDrag;
use crate::router::{Navigator, Route};
use crate::runtime;
use crate::smooth_scroll::smooth_wheel;
use crate::tracks::{Choice, ChoiceSelect, cycle_choice, step_choice};
use crate::views::{add_to_playlist, edit_playlist};
use crate::zoom::{dp, dp_px};

const BACKDROP_H: f32 = 340.;
/// The narrowest the title gets beside the actions before they move under it.
const TITLE_MIN_W: f32 = 280.;
const DETAIL_POSTER_W: f32 = 200.;
const DETAIL_POSTER_H: f32 = 300.;
const PICKER_W: f32 = 260.;

struct ItemData {
    item: BaseItemDto,
    /// Seasons of a series, or episodes of a season.
    children: Vec<BaseItemDto>,
    /// Local trailers, then featurettes and other special features.
    extras: Vec<BaseItemDto>,
    /// "More like this".
    similar: Vec<BaseItemDto>,
    /// The episode a series' or season's Play starts ([`item_menu::first_unwatched`]).
    up_next: Option<BaseItemDto>,
}

/// A series' season opened on its page, and its episodes once loaded.
struct OpenSeason {
    id: String,
    name: String,
    episodes: Load<Vec<BaseItemDto>>,
}

/// How many items "More like this" shows.
const SIMILAR_LIMIT: u32 = 12;

/// The version choice for a playable item with more than one. Audio and subtitles aren't
/// picked here: the server's defaults apply (the user's language preferences), and the
/// player's settings menu changes them.
struct Pickers {
    version: Option<ChoiceSelect>,
}

pub struct ItemPage {
    client: Client,
    nav: Navigator,
    item_id: String,
    data: Load<ItemData>,
    pickers: Option<Pickers>,
    scroll: ScrollHandle,
    people_scroll: ScrollHandle,
    chapters_scroll: ScrollHandle,
    extras_scroll: ScrollHandle,
    similar_scroll: ScrollHandle,
    /// Dragging the shelves sideways.
    rail_drag: RailDrag,
    /// The season of a series whose episodes show in a row under the seasons.
    open_season: Option<OpenSeason>,
    episodes_scroll: ScrollHandle,
    cursor: Cursor,
    focus_handle: FocusHandle,
    /// Playlist changes waiting to be sent, in order. They're already shown; the server gets
    /// them one at a time, so moves land in the order they were made.
    playlist_edits: VecDeque<PlaylistEdit>,
    sending_edit: bool,
    /// The open More menu, and its dismissal.
    more_menu: Option<(Entity<PopupMenu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

/// A change to a playlist's entries, by entry id.
enum PlaylistEdit {
    Move { entry_id: String, to: usize },
    Remove { entry_id: String },
}

/// What a playlist entry's buttons (or Alt+Up, Alt+Down and Delete) do.
#[derive(Clone, Copy)]
enum EntryAction {
    Up,
    Down,
    Remove,
}

impl ItemPage {
    pub fn new(client: Client, nav: Navigator, item_id: String, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            // Resume point and watched state change after playback or marking items watched.
            cx.subscribe(&Changes::global(cx), |this, _, _: &UserDataChanged, cx| this.reload(cx)),
        ];
        let mut this = Self {
            client,
            nav,
            item_id,
            data: Load::Loading,
            pickers: None,
            scroll: ScrollHandle::new(),
            people_scroll: ScrollHandle::new(),
            chapters_scroll: ScrollHandle::new(),
            extras_scroll: ScrollHandle::new(),
            similar_scroll: ScrollHandle::new(),
            rail_drag: RailDrag::default(),
            open_season: None,
            episodes_scroll: ScrollHandle::new(),
            cursor: Cursor::default(),
            focus_handle: cx.focus_handle(),
            playlist_edits: VecDeque::new(),
            sending_edit: false,
            more_menu: None,
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Fetches the item again. The current content stays up until the new data arrives.
    fn reload(&mut self, cx: &mut Context<Self>) {
        // A reload in the middle of playlist edits would show the playlist half-edited; the
        // last edit reloads instead.
        if self.sending_edit {
            return;
        }
        let (client, item_id) = (self.client.clone(), self.item_id.clone());
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(load(client, item_id)).await;
            this.update(cx, |this, cx| {
                this.data = match result {
                    Ok(data) => Load::Loaded(data),
                    Err(err) => Load::Failed(this.nav.handle_error(&err, cx)),
                };
                // Its episodes' watched state and progress may have changed too.
                if let Some(id) = this.open_season.as_ref().map(|season| season.id.clone()) {
                    this.load_episodes(id, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn play(&mut self, start: Option<Ticks>, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        let mut options = PlayOptions {
            start,
            ..Default::default()
        };
        if let Some(pickers) = &self.pickers {
            let selected =
                |select: &Option<ChoiceSelect>| select.as_ref().and_then(|s| s.read(cx).selected_value().copied());
            options.media_source_id = selected(&pickers.version)
                .and_then(|i| data.item.media_sources.get(i as usize))
                .map(|source| source.id.clone());
        }
        // A series or season plays its next episode.
        let item = data.up_next.clone().unwrap_or_else(|| data.item.clone());
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
    }

    /// A series or season, whose Play and Shuffle go through its episodes.
    fn is_show(&self) -> bool {
        matches!(&self.data, Load::Loaded(data) if matches!(data.item.kind, BaseItemKind::Series | BaseItemKind::Season))
    }

    fn set_played(&mut self, item_id: String, played: bool, window: &mut Window, cx: &mut Context<Self>) {
        item_menu::mark(
            self.client.clone(),
            self.nav.clone(),
            item_id,
            Mark::Played(played),
            window,
            cx,
        );
    }

    /// Moves or removes a playlist entry: on screen right away, then on the server.
    fn edit_entry(&mut self, index: usize, action: EntryAction, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &mut self.data else { return };
        if data.item.kind != BaseItemKind::Playlist || index >= data.children.len() {
            return;
        }
        let Some(entry_id) = data.children[index].playlist_item_id.clone() else {
            return;
        };
        let to = match action {
            EntryAction::Up if index > 0 => index - 1,
            EntryAction::Down if index + 1 < data.children.len() => index + 1,
            EntryAction::Up | EntryAction::Down => return,
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
                self.playlist_edits.push_back(PlaylistEdit::Remove { entry_id });
                self.send_edits(window, cx);
                cx.notify();
                return;
            }
        };
        let entry = data.children.remove(index);
        data.children.insert(to, entry);
        if self.cursor.is_selected(CHILDREN, index) {
            self.cursor.show_at(Position {
                section: CHILDREN,
                index: to,
            });
            self.reveal(
                Position {
                    section: CHILDREN,
                    index: to,
                },
                window,
                cx,
            );
        }
        self.playlist_edits.push_back(PlaylistEdit::Move { entry_id, to });
        self.send_edits(window, cx);
        cx.notify();
    }

    /// Sends the next queued playlist edit. When the queue runs dry the page reloads, for the
    /// playlist's new runtime and to match what the server ended up with. A failure drops the
    /// rest and reloads too.
    fn send_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending_edit {
            return;
        }
        let Some(edit) = self.playlist_edits.pop_front() else {
            self.reload(cx);
            return;
        };
        self.sending_edit = true;
        let (client, playlist_id) = (self.client.clone(), self.item_id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime::spawn(async move {
                match edit {
                    PlaylistEdit::Move { entry_id, to } => {
                        client.move_playlist_entry(&playlist_id, &entry_id, to).await
                    }
                    PlaylistEdit::Remove { entry_id } => client.remove_from_playlist(&playlist_id, &[entry_id]).await,
                }
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                this.sending_edit = false;
                if let Err(err) = result {
                    this.playlist_edits.clear();
                    let message = match err {
                        jellyfin::Error::Forbidden => {
                            "This playlist belongs to someone else; it can't be changed.".into()
                        }
                        err => this.nav.handle_error(&err, cx),
                    };
                    window.push_notification(Notification::error(message), cx);
                }
                this.send_edits(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// W on a highlighted episode (or season, or entry) toggles it watched; Alt+Up, Alt+Down
    /// and Delete move and remove a playlist entry.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(position) = self.cursor.selected() else { return };
        let Load::Loaded(data) = &self.data else { return };
        if position.section != CHILDREN {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if keystroke.key == "w" && !modifiers.modified() {
            if let Some(child) = data.children.get(position.index) {
                let (id, played) = (child.id.clone(), is_played(child));
                cx.stop_propagation();
                self.set_played(id, !played, window, cx);
            }
            return;
        }
        if data.item.kind != BaseItemKind::Playlist {
            return;
        }
        let action = match keystroke.key.as_str() {
            "up" if modifiers.alt && !modifiers.control && !modifiers.shift => EntryAction::Up,
            "down" if modifiers.alt && !modifiers.control && !modifiers.shift => EntryAction::Down,
            "delete" if !modifiers.modified() => EntryAction::Remove,
            _ => return,
        };
        cx.stop_propagation();
        self.edit_entry(position.index, action, window, cx);
    }

    /// Asks before deleting the playlist, then goes back to the page before it.
    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        let (client, nav) = (self.client.clone(), self.nav.clone());
        let (item_id, name) = (data.item.id.clone(), data.item.name.clone());
        let page = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (client, nav, page) = (client.clone(), nav.clone(), page.clone());
            let (item_id, name) = (item_id.clone(), name.clone());
            alert
                .title(format!("Delete “{name}”?"))
                .description(DELETE_PLAYLIST_NOTE)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let (client, nav, page) = (client.clone(), nav.clone(), page.clone());
                    let (item_id, name) = (item_id.clone(), name.clone());
                    window
                        .spawn(cx, async move |cx| {
                            let result = runtime::spawn(async move { client.delete_item(&item_id).await }).await;
                            cx.update(|window, cx| match result {
                                Ok(()) => {
                                    window.push_notification(Notification::success(format!("Deleted {name}.")), cx);
                                    Changes::notify_user_data(cx);
                                    if page.upgrade().is_some() {
                                        nav.back(window, cx);
                                    }
                                }
                                Err(err) => {
                                    let message = match err {
                                        jellyfin::Error::Forbidden => {
                                            format!("{name} belongs to someone else; it can't be deleted.").into()
                                        }
                                        err => nav.handle_error(&err, cx),
                                    };
                                    window.push_notification(Notification::error(message), cx);
                                }
                            })
                            .ok();
                        })
                        .detach();
                    true
                })
        });
    }

    fn ensure_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pickers.is_some() {
            return;
        }
        let Load::Loaded(data) = &self.data else { return };
        if data.item.media_sources.is_empty() {
            return;
        }
        let sources = &data.item.media_sources;
        let version = (sources.len() > 1).then(|| {
            let choices = sources
                .iter()
                .enumerate()
                .map(|(i, s)| Choice {
                    label: s.name.clone().unwrap_or_else(|| format!("Version {}", i + 1)).into(),
                    value: i as i32,
                })
                .collect();
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::default()), window, cx))
        });
        self.pickers = Some(Pickers { version });
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

/// A collection's movies (and shows) by release date, as its page lists them. Playback's
/// previous and next movie follow the same order.
pub(crate) async fn collection_items(
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
fn meta_line(item: &BaseItemDto) -> String {
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
fn chapters(item: &BaseItemDto) -> &[ChapterInfo] {
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

impl ItemPage {
    /// The actions on the title's line: playing it, named, then icon buttons. Watched and
    /// Favorite are toggles, pressed while on; the less frequent actions are under More.
    fn render_actions(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let (played, favorite) = (is_played(item), is_favorite(item));
        // Room around each button for the keyboard cursor's ring.
        let ring = |i: usize, element: AnyElement| {
            div()
                .relative()
                .p(dp(ACTION_RING_GAP))
                .rounded_xl()
                .child(element)
                .when(self.cursor.is_selected(ACTIONS, i), |el| {
                    el.child(selection_ring(cx).rounded_xl())
                })
        };
        let icon_button = |id: &'static str, icon: Icon| {
            Button::new(id)
                .outline()
                .size(dp(ACTION_SIZE))
                .child(icon.size(dp(ACTION_ICON)))
        };
        let up_next = match &self.data {
            Load::Loaded(data) => data.up_next.as_ref(),
            _ => None,
        };
        // A series or season names the episode it plays.
        let episode = up_next.map(episode_label).filter(|label| !label.is_empty());
        let buttons: Vec<_> = item_actions(item, up_next)
            .into_iter()
            .enumerate()
            .map(|(i, action)| {
                let (id, icon, tooltip, primary): (&'static str, Icon, SharedString, bool) = match action {
                    ItemAction::TogglePlayed | ItemAction::ToggleFavorite => {
                        let (id, icon, tooltip, on) = if matches!(action, ItemAction::TogglePlayed) {
                            let icon = if played { IconName::CircleCheck } else { IconName::Check };
                            ("toggle-played", Icon::new(icon), "Watched", played)
                        } else {
                            ("toggle-favorite", Icon::new(Lucide::Heart), "Favorite", favorite)
                        };
                        let toggle = Toggle::new(id)
                            .outline()
                            .size(dp(ACTION_SIZE))
                            .icon(icon.size(dp(ACTION_ICON)))
                            .checked(on)
                            .tooltip(tooltip)
                            .on_click(
                                cx.listener(move |this, _: &bool, window, cx| this.run_action(action, window, cx)),
                            );
                        return ring(i, toggle.into_any_element());
                    }
                    ItemAction::More => {
                        let page = cx.entity();
                        let menu = self.more_menu.as_ref().map(|(menu, _)| menu.clone());
                        let trigger = icon_button("more", Icon::new(Lucide::Ellipsis)).tooltip("More");
                        let popover = menu_popover(
                            "more-menu",
                            Anchor::TopRight,
                            menu,
                            trigger,
                            move |open, window, cx| {
                                page.update(cx, |this, cx| {
                                    if *open {
                                        this.open_more(window, cx);
                                    }
                                })
                            },
                            cx,
                        );
                        return ring(i, popover.into_any_element());
                    }
                    // In the More menu.
                    ItemAction::AddToPlaylist | ItemAction::EditPlaylist | ItemAction::DeletePlaylist => {
                        unreachable!("not a button")
                    }
                    ItemAction::Resume(position) => (
                        "resume",
                        IconName::Play.into(),
                        format!("Resume from {}", timestamp(position.into())).into(),
                        true,
                    ),
                    ItemAction::PlayFromStart => (
                        "play-start",
                        Lucide::RotateCcw.into(),
                        "Play from beginning".into(),
                        false,
                    ),
                    ItemAction::Play => ("play", IconName::Play.into(), "Play".into(), true),
                    ItemAction::PlayAll => ("play-all", IconName::Play.into(), "Play all".into(), true),
                    ItemAction::Shuffle => ("shuffle", Lucide::Shuffle.into(), "Shuffle".into(), false),
                    ItemAction::Trailer => (
                        "trailer",
                        if item.local_trailer_count.unwrap_or(0) > 0 {
                            Lucide::Clapperboard
                        } else {
                            Lucide::ExternalLink
                        }
                        .into(),
                        "Trailer".into(),
                        false,
                    ),
                };
                // Playing is the main action, so it's named; the rest are icons.
                let label: Option<SharedString> = match action {
                    ItemAction::Resume(_) | ItemAction::Play => {
                        let verb = if matches!(action, ItemAction::Play) {
                            "Play"
                        } else {
                            "Resume"
                        };
                        Some(match &episode {
                            Some(episode) => format!("{verb} {episode}").into(),
                            None => verb.into(),
                        })
                    }
                    ItemAction::PlayAll => Some("Play all".into()),
                    _ => None,
                };
                let button = Button::new(id)
                    .map(|b| if primary { b.primary() } else { b.outline() })
                    .h(dp(ACTION_SIZE))
                    .child(icon.size(dp(ACTION_ICON)))
                    .map(|b| match label {
                        Some(label) => b
                            .px(dp(ACTION_SIZE / 2.))
                            .gap_2()
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(label)),
                        None => b.w(dp(ACTION_SIZE)),
                    })
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| this.run_action(action, window, cx)));
                ring(i, button.into_any_element())
            })
            .collect();
        h_flex()
            .flex_wrap()
            // The rings' room is outside the column's right edge.
            .mr(dp(-ACTION_RING_GAP))
            .gap(dp(ACTION_GAP - ACTION_RING_GAP * 2.))
            .children(buttons)
    }

    /// Opens the More menu: adding to a playlist, and editing or deleting a playlist.
    fn open_more(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        let actions = more_actions(&data.item);
        let page = cx.entity().downgrade();
        let menu = PopupMenu::build(window, cx, move |mut menu, _, _| {
            for &action in &actions {
                let (label, icon) = match action {
                    ItemAction::AddToPlaylist => ("Add to playlist…", Lucide::ListPlus),
                    ItemAction::EditPlaylist => ("Edit…", Lucide::Pencil),
                    _ => ("Delete playlist…", Lucide::Trash),
                };
                if matches!(action, ItemAction::DeletePlaylist) {
                    menu = menu.separator();
                }
                let page = page.clone();
                menu = menu.item(PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
                    // After the menu has closed and handed focus back, so the dialog keeps it.
                    let page = page.clone();
                    window.defer(cx, move |window, cx| {
                        page.update(cx, |this, cx| this.run_action(action, window, cx)).ok();
                    });
                }));
            }
            menu
        });
        menu.focus_handle(cx).focus(window, cx);
        let dismissed = cx.subscribe_in(&menu, window, |this, menu, _: &DismissEvent, window, cx| {
            if menu_left_focus(menu, window, cx) {
                this.focus_handle.focus(window, cx);
            }
            this.more_menu = None;
            cx.notify();
        });
        self.more_menu = Some((menu, dismissed));
        cx.notify();
    }

    fn run_action(&mut self, action: ItemAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            ItemAction::More => self.open_more(window, cx),
            ItemAction::Resume(position) => self.play(Some(position), cx),
            ItemAction::PlayFromStart | ItemAction::Play => self.play(None, cx),
            ItemAction::TogglePlayed => {
                let Load::Loaded(data) = &self.data else { return };
                let (id, played) = (data.item.id.clone(), is_played(&data.item));
                self.set_played(id, !played, window, cx);
            }
            ItemAction::Shuffle if self.is_show() => {
                let Load::Loaded(data) = &self.data else { return };
                let (client, nav, item) = (self.client.clone(), self.nav.clone(), data.item.clone());
                cx.spawn_in(window, async move |_, cx| {
                    let result = runtime::spawn(async move { episodes_of(&client, &item).await }).await;
                    cx.update(|window, cx| match result {
                        Ok(mut episodes) => {
                            shuffle(&mut episodes);
                            PlaybackManager::global(cx).update(cx, |playback, cx| playback.play_all(episodes, cx));
                        }
                        Err(err) => {
                            let message = nav.handle_error(&err, cx);
                            window.push_notification(Notification::error(message), cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
            ItemAction::PlayAll | ItemAction::Shuffle => {
                let Load::Loaded(data) = &self.data else { return };
                let mut items: Vec<BaseItemDto> = data
                    .children
                    .iter()
                    .filter(|child| {
                        matches!(
                            child.kind,
                            BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
                        )
                    })
                    .cloned()
                    .collect();
                if matches!(action, ItemAction::Shuffle) {
                    shuffle(&mut items);
                }
                if items.is_empty() {
                    window.push_notification(Notification::info("Nothing here to play."), cx);
                    return;
                }
                PlaybackManager::global(cx).update(cx, |playback, cx| playback.play_all(items, cx));
            }
            ItemAction::ToggleFavorite => {
                let Load::Loaded(data) = &self.data else { return };
                let (id, favorite) = (data.item.id.clone(), is_favorite(&data.item));
                item_menu::mark(
                    self.client.clone(),
                    self.nav.clone(),
                    id,
                    Mark::Favorite(!favorite),
                    window,
                    cx,
                );
            }
            ItemAction::AddToPlaylist => {
                let Load::Loaded(data) = &self.data else { return };
                add_to_playlist::open(self.client.clone(), self.nav.clone(), &data.item, window, cx);
            }
            ItemAction::EditPlaylist => {
                let Load::Loaded(data) = &self.data else { return };
                let (id, name) = (data.item.id.clone(), data.item.name.clone());
                edit_playlist::open(self.client.clone(), self.nav.clone(), id, name, window, cx);
            }
            ItemAction::DeletePlaylist => self.confirm_delete(window, cx),
            // A trailer file plays here; a trailer on the web opens in the browser.
            ItemAction::Trailer => {
                let Load::Loaded(data) = &self.data else { return };
                if let Some(trailer) = data.extras.iter().find(|extra| extra.kind == BaseItemKind::Trailer) {
                    play_extra(trailer, cx);
                } else if let Some(trailer) = data.item.remote_trailers.first() {
                    cx.open_url(&trailer.url);
                }
            }
        }
    }

    fn render_header(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let backdrop = item.backdrop_image().map(|r| r.fill_width(1280));
        let has_backdrop = backdrop.is_some();
        let backdrop = remote_image(backdrop, Some(item), cx);
        let poster = rounded_image(
            item.primary_image()
                .map(|r| r.fill_width((DETAIL_POSTER_W * 2.) as u32)),
            Some(item),
            cx,
        );
        let title = match item.kind {
            BaseItemKind::Episode | BaseItemKind::Season => item.series_name.clone().unwrap_or_default(),
            _ => String::new(),
        };

        let info = v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .when(!title.is_empty(), |el| {
                el.child(div().text_color(cx.theme().muted_foreground).child(title))
            })
            // The title, with the actions on the right; in a narrow window the actions go under
            // it rather than squeezing it to a word a line.
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_4()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w(dp(TITLE_MIN_W))
                            .text_size(dp(40.))
                            .line_height(dp(48.))
                            .font_weight(FontWeight::BOLD)
                            .child(item.name.clone()),
                    )
                    .child(self.render_actions(item, cx)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(meta_line(item)),
            )
            .when(!item.genres.is_empty(), |el| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(item.genres.join(", ")),
                )
            })
            .when(!item.studios.is_empty(), |el| {
                let studios: Vec<&str> = item.studios.iter().map(|studio| studio.name.as_str()).collect();
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(studios.join(", ")),
                )
            })
            .when_some(item.overview.clone(), |el, overview| {
                el.child(div().pt_2().max_w(dp(820.)).child(overview))
            })
            // Under the synopsis, as wide as it, set apart from it.
            .when(!item.tags.is_empty(), |el| {
                el.child(
                    h_flex()
                        .pt_4()
                        .max_w(dp(820.))
                        .flex_wrap()
                        .gap_1()
                        .children(item.tags.iter().map(|tag| {
                            div()
                                .px_2()
                                .rounded_lg()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tag.clone())
                        })),
                )
            });

        div()
            .relative()
            .w_full()
            .when(has_backdrop, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(dp(BACKDROP_H))
                        .overflow_hidden()
                        .child(backdrop)
                        .child(div().absolute().inset_0().bg(linear_gradient(
                            180.,
                            linear_color_stop(cx.theme().background.opacity(0.3), 0.),
                            linear_color_stop(cx.theme().background, 1.),
                        ))),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .items_start()
                    .gap_6()
                    .px_6()
                    .pt(dp(if has_backdrop { BACKDROP_H - 160. } else { 24. }))
                    .child(
                        div()
                            .flex_none()
                            .w(dp(DETAIL_POSTER_W))
                            .h(dp(DETAIL_POSTER_H))
                            .rounded_xl()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(poster),
                    )
                    .child(info.pt(dp(if has_backdrop { 120. } else { 0. }))),
            )
    }

    fn render_pickers(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let pickers = self.pickers.as_ref()?;
        let labeled = [("Version", &pickers.version)];
        let rows = labeled
            .into_iter()
            .filter_map(|(label, select)| Some((label, select.as_ref()?)))
            .enumerate()
            .map(|(i, (label, select))| {
                h_flex()
                    .gap_4()
                    .child(
                        div()
                            .w(dp(90.))
                            .flex_none()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .relative()
                            .child(
                                Select::new(select)
                                    .w(rems(PICKER_W / 16.))
                                    .menu_width(rems(PICKER_W * 1.5 / 16.)),
                            )
                            .when(self.cursor.is_selected(PICKERS, i), |el| {
                                el.child(selection_ring(cx).rounded_xl().m(dp(-ACTION_RING_GAP)))
                            }),
                    )
            });
        Some(v_flex().px_6().gap_2().text_sm().children(rows))
    }

    /// Season cards in rows of `columns`.
    /// Opens `season`'s episodes under the seasons, or closes them if they're open.
    fn toggle_season(&mut self, season: &BaseItemDto, cx: &mut Context<Self>) {
        if self.open_season.as_ref().is_some_and(|open| open.id == season.id) {
            self.open_season = None;
        } else {
            self.open_season = Some(OpenSeason {
                id: season.id.clone(),
                name: season.name.clone(),
                episodes: Load::Loading,
            });
            self.episodes_scroll.set_offset(point(px(0.), px(0.)));
            self.load_episodes(season.id.clone(), cx);
        }
        cx.notify();
    }

    /// Fetches the open season's episodes, unless another season opens meanwhile.
    fn load_episodes(&mut self, season_id: String, cx: &mut Context<Self>) {
        let client = self.client.clone();
        let series_id = self.item_id.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn({
                let season_id = season_id.clone();
                async move {
                    client
                        .episodes(&series_id, Some(&season_id), &[ItemField::Overview])
                        .await
                }
            })
            .await;
            this.update(cx, |this, cx| {
                let Some(open) = this.open_season.as_mut().filter(|open| open.id == season_id) else {
                    return;
                };
                open.episodes = match result {
                    Ok(episodes) => Load::Loaded(episodes.items),
                    Err(err) => Load::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The open season's episodes, as a row under the seasons.
    fn render_episodes(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.open_season.as_ref()?;
        let content = match &open.episodes {
            Load::Loaded(episodes) if !episodes.is_empty() => {
                let cards = episodes
                    .iter()
                    .enumerate()
                    .map(|(i, episode)| {
                        let nav = self.nav.clone();
                        let target = episode.clone();
                        let card = episode_card(
                            ("episode", i),
                            episode,
                            BIG_CARD_SCALE,
                            self.cursor.is_selected(EPISODES, i),
                            self.rail_drag.guard(move |_, window, cx| nav.open(&target, window, cx)),
                            cx,
                        );
                        with_item_menu(card, episode, &self.client, &self.nav)
                    })
                    .collect();
                shelf_with_rail(
                    "season-episodes",
                    open.name.clone(),
                    cards,
                    BIG_CARD_GAP,
                    SHELF_TITLE_GAP,
                    &self.episodes_scroll,
                    self.rail_drag.rail(&self.episodes_scroll),
                    cx,
                )
                .into_any_element()
            }
            Load::Loaded(_) => centered_message("No episodes in this season.", cx).into_any_element(),
            Load::Loading => div().h(dp(LANDSCAPE_H)).child(centered_spinner()).into_any_element(),
            Load::Failed(message) => centered_message(message.clone(), cx).into_any_element(),
        };
        // Rises in each time a season opens.
        let id = ElementId::from((ElementId::from("season-episodes"), open.id.clone()));
        // A tinted band across the page, that reads as part of the seasons above.
        let band = div().mb_6().py_4().bg(cx.theme().muted.opacity(0.35)).child(content);
        Some(motion::enter(band, id, Duration::ZERO).into_any_element())
    }

    fn season_rows(&self, seasons: &[BaseItemDto], columns: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        seasons
            .chunks(columns)
            .enumerate()
            .map(|(r, row)| {
                let cards: Vec<AnyElement> = row
                    .iter()
                    .enumerate()
                    .map(|(c, season)| {
                        let i = r * columns + c;
                        let nav = self.nav.clone();
                        let target = season.clone();
                        // A series' seasons open their episodes here; a collection's items
                        // open their pages.
                        let is_season = season.kind == BaseItemKind::Season;
                        let open = self.open_season.as_ref().is_some_and(|open| open.id == season.id);
                        let page = cx.entity();
                        let card = poster_card_scaled(
                            ("season", i),
                            season,
                            BIG_CARD_SCALE,
                            self.cursor.is_selected(CHILDREN, i) || open,
                            move |_, window, cx| {
                                if is_season {
                                    page.update(cx, |this, cx| this.toggle_season(&target, cx));
                                } else {
                                    nav.open(&target, window, cx);
                                }
                            },
                            cx,
                        );
                        with_item_menu(card, season, &self.client, &self.nav)
                    })
                    .collect();
                h_flex()
                    .px_6()
                    .pb(dp(BIG_CARD_GAP))
                    .gap(dp(BIG_CARD_GAP))
                    .items_start()
                    .children(cards)
                    .into_any_element()
            })
            .collect()
    }

    /// Episodes of a season, or a playlist's entries (`playlist`), which get buttons to move and
    /// remove them.
    fn episode_rows(&self, episodes: &[BaseItemDto], playlist: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        episodes
            .iter()
            .enumerate()
            .map(|(i, episode)| {
                let nav = self.nav.clone();
                let target = episode.clone();
                let image = rounded_image(
                    landscape_image(episode).map(|r| r.fill_width((LANDSCAPE_W * 2.) as u32)),
                    Some(episode),
                    cx,
                );
                // Episodes read "S1:E2 · Name"; movies in a playlist their name, and the year.
                let (name, episode_line) = card_text(episode);
                let is_episode = episode.kind == BaseItemKind::Episode;
                let heading = if is_episode { episode_line } else { name };
                let runtime = [
                    episode.production_year.filter(|_| !is_episode).map(|y| y.to_string()),
                    episode.run_time_ticks.map(runtime_text),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                let played = is_played(episode);
                let episode_id = episode.id.clone();
                let selected = self.cursor.is_selected(CHILDREN, i);
                // Keyed by the entry, not its place: a playlist's rows move.
                let key = episode.playlist_item_id.as_deref().unwrap_or(&episode.id);
                let element_id = |name: &str| SharedString::from(format!("{name}-{key}"));
                let entry_button = |id: &'static str, icon: Lucide, tooltip: &'static str, action: EntryAction| {
                    Button::new(element_id(id))
                        .ghost()
                        .icon(icon)
                        .tooltip(tooltip)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.edit_entry(i, action, window, cx);
                        }))
                };
                let entry_buttons = playlist.then(|| {
                    h_flex()
                        .child(
                            entry_button("entry-up", Lucide::ArrowUp, "Move up (Alt+Up)", EntryAction::Up)
                                .disabled(i == 0),
                        )
                        .child(
                            entry_button(
                                "entry-down",
                                Lucide::ArrowDown,
                                "Move down (Alt+Down)",
                                EntryAction::Down,
                            )
                            .disabled(i + 1 == episodes.len()),
                        )
                        .child(entry_button(
                            "entry-remove",
                            Lucide::X,
                            "Remove from playlist (Delete)",
                            EntryAction::Remove,
                        ))
                });
                let row = h_flex()
                    .id(element_id("episode"))
                    .relative()
                    .items_start()
                    .gap_4()
                    .p_2()
                    .rounded_lg()
                    .hover(|style| style.bg(cx.theme().muted))
                    .when(selected, |el| el.bg(cx.theme().muted))
                    .on_click(move |_, window, cx| nav.open(&target, window, cx))
                    .child(
                        div()
                            .flex_none()
                            .w(dp(LANDSCAPE_W))
                            .h(dp(LANDSCAPE_H))
                            .rounded_xl()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(image),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(heading))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(runtime))
                            .when_some(episode.overview.clone(), |el, overview| {
                                el.child(div().text_sm().line_clamp(3).child(overview))
                            }),
                    )
                    .children(entry_buttons)
                    .child(
                        // Pressed while watched, like the page's own Watched.
                        Toggle::new(element_id("toggle-played"))
                            .icon(if played { IconName::CircleCheck } else { IconName::Check })
                            .checked(played)
                            .tooltip("Watched (W)")
                            .on_click(cx.listener(move |this, _: &bool, window, cx| {
                                // Don't open the episode underneath.
                                cx.stop_propagation();
                                this.set_played(episode_id.clone(), !played, window, cx);
                            })),
                    )
                    .when(selected, |el| el.child(selection_ring(cx)));
                div()
                    .px_4()
                    .pb_1()
                    .child(with_item_menu(row, episode, &self.client, &self.nav))
                    .into_any_element()
            })
            .collect()
    }

    /// The seasons grid or episode list, if the page has one.
    fn children_section(&self, data: &ItemData, window: &Window) -> Section {
        match data.item.kind {
            BaseItemKind::Series | BaseItemKind::BoxSet => {
                let available = window.viewport_size().width - dp_px(48., window);
                let card = POSTER_W * BIG_CARD_SCALE;
                let columns =
                    ((available + dp_px(BIG_CARD_GAP, window)) / dp_px(card + BIG_CARD_GAP, window)).floor() as usize;
                Section::grid(data.children.len(), columns)
            }
            BaseItemKind::Season | BaseItemKind::Playlist => Section::grid(data.children.len(), 1),
            _ => Section::row(0),
        }
    }

    /// Scroll-container child index of the children's title, after the chapters.
    fn children_title_index(&self, data: &ItemData) -> usize {
        self.chapters_index() + usize::from(!chapters(&data.item).is_empty())
    }

    /// Scroll-container child index of the open season's episodes: after the children's title
    /// and rows.
    fn episodes_index(&self, data: &ItemData, window: &Window) -> usize {
        let children = self.children_section(data, window);
        let block = match children.len {
            0 => 0,
            len => 1 + len.div_ceil(children.columns),
        };
        self.children_title_index(data) + block
    }

    /// Scroll-container child index of the cast shelf: after the open season's episodes.
    fn people_index(&self, data: &ItemData, window: &Window) -> usize {
        self.episodes_index(data, window) + usize::from(self.open_season.is_some())
    }

    /// Scroll-container child index of the extras shelf.
    fn extras_index(&self, data: &ItemData, window: &Window) -> usize {
        self.people_index(data, window) + usize::from(!data.item.people.is_empty())
    }

    /// Scroll-container child index of the "More like this" shelf.
    fn similar_index(&self, data: &ItemData, window: &Window) -> usize {
        self.extras_index(data, window) + usize::from(!data.extras.is_empty())
    }

    /// Trailers and extras, played from the start when chosen.
    fn render_extras(&self, extras: &[BaseItemDto], cx: &mut Context<Self>) -> AnyElement {
        let cards = extras
            .iter()
            .enumerate()
            .map(|(i, extra)| {
                let target = extra.clone();
                landscape_card(
                    ("extra", i),
                    extra,
                    self.cursor.is_selected(EXTRAS, i),
                    self.rail_drag.guard(move |_, _, cx| play_extra(&target, cx)),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf(
                "extras",
                "Extras",
                cards,
                &self.extras_scroll,
                self.rail_drag.rail(&self.extras_scroll),
                cx,
            ))
            .into_any_element()
    }

    fn render_similar(&self, similar: &[BaseItemDto], cx: &mut Context<Self>) -> AnyElement {
        let cards = similar
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (nav, target) = (self.nav.clone(), item.clone());
                let card = poster_card_scaled(
                    ("similar", i),
                    item,
                    BIG_CARD_SCALE,
                    self.cursor.is_selected(SIMILAR, i),
                    self.rail_drag.guard(move |_, window, cx| nav.open(&target, window, cx)),
                    cx,
                );
                with_item_menu(card, item, &self.client, &self.nav)
            })
            .collect();
        div()
            .pt_6()
            .child(shelf_with_rail(
                "similar",
                "More like this",
                cards,
                BIG_CARD_GAP,
                SHELF_TITLE_GAP,
                &self.similar_scroll,
                self.rail_drag.rail(&self.similar_scroll),
                cx,
            ))
            .into_any_element()
    }

    /// Scroll-container child index of the chapters shelf, after the header and pickers.
    fn chapters_index(&self) -> usize {
        1 + usize::from(self.pickers.is_some())
    }

    /// Chapters to jump into: their image, name and start. Chosen, the item plays from there
    /// (with the version and tracks picked above).
    fn render_chapters(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> AnyElement {
        let cards = chapters(item)
            .iter()
            .enumerate()
            .map(|(i, chapter)| {
                let start = chapter.start_position_ticks;
                chapter_card(
                    ("chapter", i),
                    &item.id,
                    i,
                    chapter,
                    self.cursor.is_selected(CHAPTERS, i),
                    self.rail_drag
                        .guard(cx.listener(move |this, _, _, cx| this.play(Some(start), cx))),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf(
                "chapters",
                "Chapters",
                cards,
                &self.chapters_scroll,
                self.rail_drag.rail(&self.chapters_scroll),
                cx,
            ))
            .into_any_element()
    }

    fn render_people(&self, people: &[BaseItemPerson], cx: &mut Context<Self>) -> AnyElement {
        let cards = people
            .iter()
            .enumerate()
            .map(|(i, person)| {
                let nav = self.nav.clone();
                let route = Route::Person {
                    id: person.id.clone(),
                    name: person.name.clone(),
                };
                person_card(
                    ("person", i),
                    person,
                    BIG_CARD_SCALE,
                    self.cursor.is_selected(PEOPLE, i),
                    self.rail_drag
                        .guard(move |_, window, cx| nav.push(route.clone(), window, cx)),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf_with_rail(
                "people",
                "Cast & crew",
                cards,
                BIG_CARD_GAP,
                SHELF_TITLE_GAP,
                &self.people_scroll,
                self.rail_drag.rail(&self.people_scroll),
                cx,
            ))
            .into_any_element()
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_pickers(window, cx);
        let data = match &self.data {
            Load::Loading => return centered_spinner().into_any_element(),
            Load::Failed(message) => {
                let retry = cx.listener(|this, _, _, cx| {
                    this.data = Load::Loading;
                    this.reload(cx);
                });
                return error_state(message.clone(), retry, cx).into_any_element();
            }
            Load::Loaded(data) => data,
        };
        let item = &data.item;
        let children = self.children_section(data, window);
        let (title, rows) = match item.kind {
            _ if children.len == 0 => (None, Vec::new()),
            BaseItemKind::Series => (Some("Seasons"), self.season_rows(&data.children, children.columns, cx)),
            BaseItemKind::BoxSet => (
                Some("In this collection"),
                self.season_rows(&data.children, children.columns, cx),
            ),
            BaseItemKind::Playlist => (Some("Playlist"), self.episode_rows(&data.children, true, cx)),
            _ => (Some("Episodes"), self.episode_rows(&data.children, false, cx)),
        };

        // The header, pickers, title and rows are the scroll container's direct children,
        // for `browse::reveal` (see `children_title_index`).
        let page = v_flex()
            .id("item")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(smooth_wheel("item", &self.scroll))
            .pb_8()
            .child(self.render_header(item, cx))
            .children(self.render_pickers(cx).map(|pickers| div().pt_6().child(pickers)))
            .when(!chapters(item).is_empty(), |el| {
                el.child(self.render_chapters(item, cx))
            })
            .when_some(title, |el, title| {
                el.child(
                    div()
                        .px_6()
                        .pt_6()
                        .pb_3()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
            })
            .children(rows)
            .children(self.render_episodes(cx))
            .when(!item.people.is_empty(), |el| {
                el.child(self.render_people(&item.people, cx))
            })
            .when(!data.extras.is_empty(), |el| {
                el.child(self.render_extras(&data.extras, cx))
            })
            .when(!data.similar.is_empty(), |el| {
                el.child(self.render_similar(&data.similar, cx))
            });
        self.rail_drag.track(page).into_any_element()
    }
}

/// Fisher-Yates with a random seed (uuid's v4 is the randomness the app already has).
pub(crate) fn shuffle<T>(items: &mut [T]) {
    let mut state = uuid::Uuid::new_v4().as_u128() as u64 | 1;
    for i in (1..items.len()).rev() {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(i, (state % (i as u64 + 1)) as usize);
    }
}

#[derive(Clone, Copy)]
enum ItemAction {
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
fn item_actions(item: &BaseItemDto, up_next: Option<&BaseItemDto>) -> Vec<ItemAction> {
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
fn more_actions(item: &BaseItemDto) -> Vec<ItemAction> {
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

/// Plays a trailer or other extra from the start, as a session of its own.
fn play_extra(extra: &BaseItemDto, cx: &mut App) {
    let extra = extra.clone();
    PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&extra, PlayOptions::default(), cx));
}

const DELETE_PLAYLIST_NOTE: &str =
    "The playlist is removed from the server for everyone it's shared with. What's in it stays in your libraries.";

/// Cast & crew and "More like this": larger cards, further apart, like Home's.
const BIG_CARD_SCALE: f32 = 1.2;
const BIG_CARD_GAP: f32 = 24.;
/// Between a shelf's title and its cards, as [`shelf`] has it.
const SHELF_TITLE_GAP: f32 = 16.;

/// Cursor sections, top to bottom.
const ACTIONS: usize = 0;
/// The version picker; Left and Right change it.
const PICKERS: usize = 1;
const CHAPTERS: usize = 2;
const CHILDREN: usize = 3;
/// The open season's episodes.
const EPISODES: usize = 4;
const PEOPLE: usize = 5;
const EXTRAS: usize = 6;
const SIMILAR: usize = 7;
/// Scroll-container child index of the pickers, after the header.
const PICKERS_INDEX: usize = 1;
/// Padding around each action button, where the cursor's ring goes.
const ACTION_RING_GAP: f32 = 3.;
/// The action buttons, square, and their icons.
const ACTION_SIZE: f32 = 48.;
const ACTION_ICON: f32 = 24.;
/// Between the action buttons.
const ACTION_GAP: f32 = 16.;

impl Browse for ItemPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, window: &Window, _cx: &App) -> Vec<Section> {
        let Load::Loaded(data) = &self.data else {
            return Vec::new();
        };
        vec![
            Section::row(item_actions(&data.item, data.up_next.as_ref()).len()),
            Section::grid(self.picker_selects().len(), 1),
            Section::row(chapters(&data.item).len()),
            self.children_section(data, window),
            Section::row(self.open_episodes().len()),
            Section::row(data.item.people.len()),
            Section::row(data.extras.len()),
            Section::row(data.similar.len()),
        ]
    }

    fn reveal(&mut self, position: Position, window: &mut Window, _cx: &mut Context<Self>) {
        if position.section == ACTIONS {
            self.scroll.set_offset(point(px(0.), px(0.)));
            return;
        }
        let Load::Loaded(data) = &self.data else { return };
        if position.section == PICKERS {
            browse::reveal(&self.scroll, PICKERS_INDEX, px(CARD_GAP));
            return;
        }
        if position.section == CHAPTERS {
            browse::reveal(&self.scroll, self.chapters_index(), px(CARD_GAP));
            browse::reveal(&self.chapters_scroll, position.index, px(24.));
            return;
        }
        if position.section == EPISODES {
            browse::reveal(&self.scroll, self.episodes_index(data, window), px(CARD_GAP));
            browse::reveal(&self.episodes_scroll, position.index, px(24.));
            return;
        }
        if position.section == PEOPLE {
            browse::reveal(&self.scroll, self.people_index(data, window), px(CARD_GAP));
            browse::reveal(&self.people_scroll, position.index, px(24.));
            return;
        }
        if position.section == EXTRAS || position.section == SIMILAR {
            let (index, shelf) = if position.section == EXTRAS {
                (self.extras_index(data, window), &self.extras_scroll)
            } else {
                (self.similar_index(data, window), &self.similar_scroll)
            };
            browse::reveal(&self.scroll, index, px(CARD_GAP));
            browse::reveal(shelf, position.index, px(24.));
            return;
        }
        let title = self.children_title_index(data);
        let row = position.index / self.children_section(data, window).columns;
        if row == 0 {
            browse::reveal(&self.scroll, title, px(0.));
        }
        browse::reveal(&self.scroll, title + 1 + row, px(CARD_GAP));
    }

    fn adjust(&mut self, position: Position, forward: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if position.section != PICKERS {
            return false;
        }
        if let Some(select) = self.picker_selects().get(position.index) {
            step_choice(select, forward, window, cx);
        }
        // At either end the cursor stays put, rather than leaving the picker.
        true
    }

    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        if position.section == ACTIONS {
            if let Some(&action) = item_actions(&data.item, data.up_next.as_ref()).get(position.index) {
                self.run_action(action, window, cx);
            }
        } else if position.section == PICKERS {
            if let Some(select) = self.picker_selects().get(position.index) {
                cycle_choice(select, window, cx);
            }
        } else if position.section == CHAPTERS {
            if let Some(chapter) = chapters(&data.item).get(position.index) {
                let start = chapter.start_position_ticks;
                self.play(Some(start), cx);
            }
        } else if position.section == EPISODES {
            if let Some(episode) = self.open_episodes().get(position.index) {
                self.nav.open(episode, window, cx);
            }
        } else if position.section == PEOPLE {
            if let Some(person) = data.item.people.get(position.index) {
                let route = Route::Person {
                    id: person.id.clone(),
                    name: person.name.clone(),
                };
                self.nav.push(route, window, cx);
            }
        } else if position.section == EXTRAS {
            if let Some(extra) = data.extras.get(position.index) {
                play_extra(extra, cx);
            }
        } else if position.section == SIMILAR {
            if let Some(item) = data.similar.get(position.index) {
                self.nav.open(item, window, cx);
            }
        } else if let Some(child) = data.children.get(position.index) {
            if child.kind == BaseItemKind::Season {
                let child = child.clone();
                self.toggle_season(&child, cx);
            } else {
                self.nav.open(child, window, cx);
            }
        }
    }
}

impl ItemPage {
    /// The open season's episodes, once loaded.
    fn open_episodes(&self) -> &[BaseItemDto] {
        match self.open_season.as_ref().map(|open| &open.episodes) {
            Some(Load::Loaded(episodes)) => episodes,
            _ => &[],
        }
    }

    /// The pickers shown, top to bottom.
    fn picker_selects(&self) -> Vec<ChoiceSelect> {
        let Some(pickers) = &self.pickers else {
            return Vec::new();
        };
        pickers.version.iter().cloned().collect()
    }
}

impl Focusable for ItemPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ItemPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_content(window, cx);
        on_browse_actions(div().size_full().track_focus(&self.focus_handle), cx)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(content)
    }
}
