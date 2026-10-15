//! Detail page for a movie, series, season or episode, and for collections and playlists
//! (whose entries can be reordered and removed here).

use std::collections::VecDeque;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, BaseItemPerson, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery, Ticks};

use crate::browse::{self, Browse, Cursor, Position, Section, on_browse_actions};
use crate::changes::{Changes, UserDataChanged};
use crate::components::{
    CARD_GAP, LANDSCAPE_H, LANDSCAPE_W, Load, POSTER_W, card_text, centered_spinner, episode_label, error_state,
    landscape_card, landscape_image, person_card, poster_card, remote_image, runtime_text, selection_ring, shelf,
    timestamp,
};
use crate::images::ImageCache;
use crate::playback::{PlayOptions, PlaybackManager, SubtitleChoice};
use crate::router::{Navigator, Route};
use crate::runtime;
use crate::tracks::{Choice, ChoiceSelect, SUBTITLES_OFF, cycle_choice, step_choice, track_pickers};
use crate::views::{add_to_playlist, edit_playlist};

const BACKDROP_H: f32 = 340.;
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
}

/// How many items "More like this" shows.
const SIMILAR_LIMIT: u32 = 12;

/// Version and track choices for a playable item. Defaults come from the server, which
/// applies the user's language preferences.
struct Pickers {
    version: Option<ChoiceSelect>,
    audio: Option<ChoiceSelect>,
    subtitle: Option<ChoiceSelect>,
    _version_changed: Option<Subscription>,
}

pub struct ItemPage {
    client: Client,
    nav: Navigator,
    item_id: String,
    data: Load<ItemData>,
    pickers: Option<Pickers>,
    scroll: ScrollHandle,
    people_scroll: ScrollHandle,
    extras_scroll: ScrollHandle,
    similar_scroll: ScrollHandle,
    cursor: Cursor,
    focus_handle: FocusHandle,
    /// Playlist changes waiting to be sent, in order. They're already shown; the server gets
    /// them one at a time, so moves land in the order they were made.
    playlist_edits: VecDeque<PlaylistEdit>,
    sending_edit: bool,
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
            extras_scroll: ScrollHandle::new(),
            similar_scroll: ScrollHandle::new(),
            cursor: Cursor::default(),
            focus_handle: cx.focus_handle(),
            playlist_edits: VecDeque::new(),
            sending_edit: false,
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
            options.audio_stream_index = selected(&pickers.audio);
            options.subtitle = match selected(&pickers.subtitle) {
                Some(SUBTITLES_OFF) => SubtitleChoice::Off,
                Some(index) => SubtitleChoice::Stream(index),
                None => SubtitleChoice::Auto,
            };
        }
        let item = data.item.clone();
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
    }

    fn set_played(&mut self, item_id: String, played: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.mark(item_id, Mark::Played(played), window, cx);
    }

    /// Sets watched or favorite state on the server, then refreshes the pages showing it.
    fn mark(&mut self, item_id: String, mark: Mark, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime::spawn(async move {
                match mark {
                    Mark::Played(true) => client.mark_played(&item_id).await,
                    Mark::Played(false) => client.mark_unplayed(&item_id).await,
                    Mark::Favorite(true) => client.mark_favorite(&item_id).await,
                    Mark::Favorite(false) => client.unmark_favorite(&item_id).await,
                }
            })
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => Changes::notify_user_data(cx),
                Err(err) => {
                    let message = this.nav.handle_error(&err, cx);
                    window.push_notification(Notification::error(message), cx);
                }
            })
            .ok();
        })
        .detach();
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
                .title(format!("Delete {name}?"))
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
        let sources = data.item.media_sources.clone();
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
        let _version_changed = version.as_ref().map(|select| {
            cx.subscribe_in(
                select,
                window,
                move |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    let SelectEvent::Confirm(Some(index)) = event else {
                        return;
                    };
                    if let Some(source) = sources.get(*index as usize) {
                        let (audio, subtitle) = track_pickers(
                            source,
                            source.default_audio_stream_index,
                            source.default_subtitle_stream_index,
                            window,
                            cx,
                        );
                        if let Some(pickers) = &mut this.pickers {
                            pickers.audio = audio;
                            pickers.subtitle = subtitle;
                        }
                        cx.notify();
                    }
                },
            )
        });
        let first = &data.item.media_sources[0];
        let (audio, subtitle) = track_pickers(
            first,
            first.default_audio_stream_index,
            first.default_subtitle_stream_index,
            window,
            cx,
        );
        self.pickers = Some(Pickers {
            version,
            audio,
            subtitle,
            _version_changed,
        });
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
    let (extras, similar) = tokio::join!(extras(&client, &item), similar(&client, &item));
    Ok(ItemData {
        item,
        children,
        extras,
        similar,
    })
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

fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

fn is_favorite(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.is_favorite)
}

impl ItemPage {
    /// Play / Resume buttons and the watched toggle.
    fn render_actions(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let (played, favorite) = (is_played(item), is_favorite(item));
        let buttons = item_actions(item).into_iter().enumerate().map(|(i, action)| {
            let button = match action {
                ItemAction::Resume(position) => Button::new("resume")
                    .primary()
                    .icon(IconName::Play)
                    .label(format!("Resume from {}", timestamp(position.into()))),
                ItemAction::PlayFromStart => Button::new("play-start").outline().label("Play from beginning"),
                ItemAction::Play => Button::new("play").primary().icon(IconName::Play).label("Play"),
                ItemAction::TogglePlayed => Button::new("toggle-played")
                    .outline()
                    .icon(if played { IconName::CircleCheck } else { IconName::Check })
                    .label(if played { "Watched" } else { "Mark watched" }),
                ItemAction::PlayAll => Button::new("play-all").primary().icon(IconName::Play).label("Play all"),
                ItemAction::Shuffle => Button::new("shuffle").outline().icon(Lucide::Shuffle).label("Shuffle"),
                ItemAction::Trailer => Button::new("trailer")
                    .outline()
                    .icon(if item.local_trailer_count.unwrap_or(0) > 0 {
                        Lucide::Clapperboard
                    } else {
                        Lucide::ExternalLink
                    })
                    .label("Trailer"),
                ItemAction::ToggleFavorite => Button::new("toggle-favorite")
                    .outline()
                    .icon(if favorite { Lucide::HeartOff } else { Lucide::Heart })
                    .label(if favorite { "Remove favorite" } else { "Favorite" }),
                ItemAction::AddToPlaylist => Button::new("add-to-playlist")
                    .outline()
                    .icon(Lucide::ListPlus)
                    .label("Add to playlist"),
                ItemAction::EditPlaylist => Button::new("edit-playlist")
                    .outline()
                    .icon(Lucide::Pencil)
                    .label("Edit"),
                ItemAction::DeletePlaylist => Button::new("delete-playlist")
                    .outline()
                    .icon(Lucide::Trash)
                    .label("Delete playlist"),
            };
            let button = button.on_click(cx.listener(move |this, _, window, cx| this.run_action(action, window, cx)));
            // Room around the button for the keyboard cursor's ring.
            div()
                .relative()
                .p(px(ACTION_RING_GAP))
                .rounded_lg()
                .child(button)
                .when(self.cursor.is_selected(ACTIONS, i), |el| {
                    el.child(selection_ring(cx).rounded_lg())
                })
        });
        h_flex()
            .pt_2()
            .ml(px(-ACTION_RING_GAP))
            .gap(px(8. - ACTION_RING_GAP * 2.))
            .children(buttons)
    }

    fn run_action(&mut self, action: ItemAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            ItemAction::Resume(position) => self.play(Some(position), cx),
            ItemAction::PlayFromStart | ItemAction::Play => self.play(None, cx),
            ItemAction::TogglePlayed => {
                let Load::Loaded(data) = &self.data else { return };
                let (id, played) = (data.item.id.clone(), is_played(&data.item));
                self.set_played(id, !played, window, cx);
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
                self.mark(id, Mark::Favorite(!favorite), window, cx);
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
        let poster = remote_image(
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
            .child(div().text_3xl().font_weight(FontWeight::BOLD).child(item.name.clone()))
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
            .when(!item.tags.is_empty(), |el| {
                el.child(h_flex().flex_wrap().gap_1().children(item.tags.iter().map(|tag| {
                    div()
                        .px_2()
                        .rounded_md()
                        .bg(cx.theme().muted)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tag.clone())
                })))
            })
            .child(self.render_actions(item, cx))
            .when_some(item.overview.clone(), |el, overview| {
                el.child(div().pt_2().max_w(px(820.)).child(overview))
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
                        .h(px(BACKDROP_H))
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
                    .pt(px(if has_backdrop { BACKDROP_H - 160. } else { 24. }))
                    .child(
                        div()
                            .flex_none()
                            .w(px(DETAIL_POSTER_W))
                            .h(px(DETAIL_POSTER_H))
                            .rounded_md()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(poster),
                    )
                    .child(info.pt(px(if has_backdrop { 120. } else { 0. }))),
            )
    }

    fn render_pickers(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let pickers = self.pickers.as_ref()?;
        let labeled = [
            ("Version", &pickers.version),
            ("Audio", &pickers.audio),
            ("Subtitles", &pickers.subtitle),
        ];
        let rows = labeled
            .into_iter()
            .filter_map(|(label, select)| Some((label, select.as_ref()?)))
            .enumerate()
            .map(|(i, (label, select))| {
                h_flex()
                    .gap_4()
                    .child(
                        div()
                            .w(px(90.))
                            .flex_none()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .relative()
                            .child(Select::new(select).w(px(PICKER_W)).menu_width(px(PICKER_W * 1.5)))
                            .when(self.cursor.is_selected(PICKERS, i), |el| {
                                el.child(selection_ring(cx).rounded_lg().m(px(-ACTION_RING_GAP)))
                            }),
                    )
            });
        Some(v_flex().px_6().gap_2().text_sm().children(rows))
    }

    /// Season cards in rows of `columns`.
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
                        poster_card(
                            ("season", i),
                            season,
                            self.cursor.is_selected(CHILDREN, i),
                            move |_, window, cx| nav.open(&target, window, cx),
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect();
                h_flex()
                    .px_6()
                    .pb(px(CARD_GAP))
                    .gap(px(CARD_GAP))
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
                let image = remote_image(
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
                let entry_button = |id: &'static str, icon: Lucide, tooltip: &'static str, action: EntryAction| {
                    Button::new((id, i))
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
                    .id(("episode", i))
                    .relative()
                    .items_start()
                    .gap_4()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .when(selected, |el| el.bg(cx.theme().muted))
                    .on_click(move |_, window, cx| nav.open(&target, window, cx))
                    .child(
                        div()
                            .flex_none()
                            .w(px(LANDSCAPE_W))
                            .h(px(LANDSCAPE_H))
                            .rounded_md()
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
                        Button::new(("toggle-played", i))
                            .ghost()
                            .icon(if played { IconName::CircleCheck } else { IconName::Check })
                            .tooltip(if played {
                                "Mark unwatched (W)"
                            } else {
                                "Mark watched (W)"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Don't open the episode underneath.
                                cx.stop_propagation();
                                this.set_played(episode_id.clone(), !played, window, cx);
                            })),
                    )
                    .when(selected, |el| el.child(selection_ring(cx)));
                div().px_4().pb_1().child(row).into_any_element()
            })
            .collect()
    }

    /// The seasons grid or episode list, if the page has one.
    fn children_section(&self, data: &ItemData, window: &Window) -> Section {
        match data.item.kind {
            BaseItemKind::Series | BaseItemKind::BoxSet => {
                let available = window.viewport_size().width - px(48.);
                let columns = ((available + px(CARD_GAP)) / px(POSTER_W + CARD_GAP)).floor() as usize;
                Section::grid(data.children.len(), columns)
            }
            BaseItemKind::Season | BaseItemKind::Playlist => Section::grid(data.children.len(), 1),
            _ => Section::row(0),
        }
    }

    /// Scroll-container child index of the children's title.
    fn children_title_index(&self, data: &ItemData) -> usize {
        1 + usize::from(self.pickers.is_some()) + usize::from(!data.item.people.is_empty())
    }

    /// Scroll-container child index of the extras shelf: after the children's title and rows.
    fn extras_index(&self, data: &ItemData, window: &Window) -> usize {
        let children = self.children_section(data, window);
        let block = match children.len {
            0 => 0,
            len => 1 + len.div_ceil(children.columns),
        };
        self.children_title_index(data) + block
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
                    move |_, _, cx| play_extra(&target, cx),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf("extras", "Extras", cards, &self.extras_scroll, cx))
            .into_any_element()
    }

    fn render_similar(&self, similar: &[BaseItemDto], cx: &mut Context<Self>) -> AnyElement {
        let cards = similar
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (nav, target) = (self.nav.clone(), item.clone());
                poster_card(
                    ("similar", i),
                    item,
                    self.cursor.is_selected(SIMILAR, i),
                    move |_, window, cx| nav.open(&target, window, cx),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf("similar", "More like this", cards, &self.similar_scroll, cx))
            .into_any_element()
    }

    /// Scroll-container child index of the cast shelf.
    fn people_index(&self) -> usize {
        1 + usize::from(self.pickers.is_some())
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
                    self.cursor.is_selected(PEOPLE, i),
                    move |_, window, cx| nav.push(route.clone(), window, cx),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        div()
            .pt_6()
            .child(shelf("people", "Cast & crew", cards, &self.people_scroll, cx))
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
        v_flex()
            .id("item")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb_8()
            .child(self.render_header(item, cx))
            .children(self.render_pickers(cx).map(|pickers| div().pt_6().child(pickers)))
            .when(!item.people.is_empty(), |el| {
                el.child(self.render_people(&item.people, cx))
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
            .when(!data.extras.is_empty(), |el| {
                el.child(self.render_extras(&data.extras, cx))
            })
            .when(!data.similar.is_empty(), |el| {
                el.child(self.render_similar(&data.similar, cx))
            })
            .into_any_element()
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
enum Mark {
    Played(bool),
    Favorite(bool),
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
}

/// The buttons under an item's title, in order.
fn item_actions(item: &BaseItemDto) -> Vec<ItemAction> {
    let playable = matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
    );
    let resume_at = item
        .user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0);
    let mut actions = match (playable, resume_at) {
        _ if matches!(item.kind, BaseItemKind::BoxSet | BaseItemKind::Playlist) => {
            vec![ItemAction::PlayAll, ItemAction::Shuffle]
        }
        (false, _) => Vec::new(),
        (true, Some(position)) => vec![ItemAction::Resume(position), ItemAction::PlayFromStart],
        (true, None) => vec![ItemAction::Play],
    };
    if item.local_trailer_count.unwrap_or(0) > 0 || !item.remote_trailers.is_empty() {
        actions.push(ItemAction::Trailer);
    }
    actions.push(ItemAction::TogglePlayed);
    actions.push(ItemAction::ToggleFavorite);
    if matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video | BaseItemKind::Series | BaseItemKind::Season
    ) {
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

/// Cursor sections, top to bottom.
const ACTIONS: usize = 0;
/// Version, audio and subtitle pickers, one per row; Left and Right change them.
const PICKERS: usize = 1;
const PEOPLE: usize = 2;
const CHILDREN: usize = 3;
const EXTRAS: usize = 4;
const SIMILAR: usize = 5;
/// Scroll-container child index of the pickers, after the header.
const PICKERS_INDEX: usize = 1;
/// Padding around each action button, where the cursor's ring goes.
const ACTION_RING_GAP: f32 = 3.;

impl Browse for ItemPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, window: &Window, _cx: &App) -> Vec<Section> {
        let Load::Loaded(data) = &self.data else {
            return Vec::new();
        };
        vec![
            Section::row(item_actions(&data.item).len()),
            Section::grid(self.picker_selects().len(), 1),
            Section::row(data.item.people.len()),
            self.children_section(data, window),
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
        if position.section == PEOPLE {
            browse::reveal(&self.scroll, self.people_index(), px(CARD_GAP));
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
            if let Some(&action) = item_actions(&data.item).get(position.index) {
                self.run_action(action, window, cx);
            }
        } else if position.section == PICKERS {
            if let Some(select) = self.picker_selects().get(position.index) {
                cycle_choice(select, window, cx);
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
            self.nav.open(child, window, cx);
        }
    }
}

impl ItemPage {
    /// The pickers shown, top to bottom.
    fn picker_selects(&self) -> Vec<ChoiceSelect> {
        let Some(pickers) = &self.pickers else {
            return Vec::new();
        };
        [&pickers.version, &pickers.audio, &pickers.subtitle]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
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
