//! Detail page for a movie, series, season or episode, and for collections and playlists
//! (whose entries can be reordered and removed here). The page, its header, pickers and
//! shelves arrive with U6, its seasons, episodes and playlist entries with U7; what it needs
//! from the server and the actions it offers are here.

use jellyfin::models::{BaseItemDto, BaseItemKind, ChapterInfo, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery, Ticks};

use crate::item_menu::{can_add_to_playlist, is_playable, resume_point};
use crate::widgets::text::{episode_label, runtime_text};

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

#[derive(Clone, Copy)]
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
