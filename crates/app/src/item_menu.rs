//! What an item's menu and actions offer: played and favorite state, resume points, what can be
//! played, marked or added to a playlist. The menu itself is `widgets/menu.rs` and the item
//! page's own buttons are U6's.

use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{Client, Ticks};

#[derive(Debug, Clone, Copy)]
pub enum Mark {
    Played(bool),
    Favorite(bool),
}

impl Mark {
    /// Whether `item` already shows this.
    pub fn shown_on(self, item: &BaseItemDto) -> bool {
        match self {
            Mark::Played(played) => is_played(item) == played,
            Mark::Favorite(favorite) => is_favorite(item) == favorite,
        }
    }

    /// Shows this on `item`, ahead of the server. The rest of its user data (a played item's
    /// resume point) comes with the reload that follows.
    pub fn apply(self, item: &mut BaseItemDto) {
        let data = item.user_data.get_or_insert_default();
        match self {
            Mark::Played(played) => data.played = played,
            Mark::Favorite(favorite) => data.is_favorite = favorite,
        }
    }

    pub fn undone(self) -> Mark {
        match self {
            Mark::Played(played) => Mark::Played(!played),
            Mark::Favorite(favorite) => Mark::Favorite(!favorite),
        }
    }
}

pub fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

pub fn is_favorite(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.is_favorite)
}

/// Where playback would resume, if it's partway through.
pub fn resume_point(item: &BaseItemDto) -> Option<Ticks> {
    item.user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0)
}

pub fn is_playable(item: &BaseItemDto) -> bool {
    matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
    )
}

/// The episode a series' or season's Play starts, out of its episodes in order: the first
/// that isn't watched (resumed if it's partway), or the first if they all are. Specials
/// (season 0) only count when there's nothing else, as with Jellyfin's web client.
pub fn first_unwatched(episodes: &[BaseItemDto]) -> Option<&BaseItemDto> {
    let regular: Vec<_> = episodes.iter().filter(|e| e.parent_index_number != Some(0)).collect();
    let candidates = if regular.is_empty() {
        episodes.iter().collect()
    } else {
        regular
    };
    candidates
        .iter()
        .find(|e| !is_played(e))
        .or(candidates.first())
        .copied()
}

/// All of a series' episodes in order, or a season's.
pub async fn episodes_of(client: &Client, item: &BaseItemDto) -> jellyfin::Result<Vec<BaseItemDto>> {
    Ok(match item.kind {
        BaseItemKind::Series => client.episodes(&item.id, None, &[]).await?.items,
        BaseItemKind::Season => {
            let series_id = item.series_id.as_deref().unwrap_or_default();
            client.episodes(series_id, Some(&item.id), &[]).await?.items
        }
        _ => Vec::new(),
    })
}

/// Movies, episodes and what holds episodes can go in a playlist.
pub fn can_add_to_playlist(item: &BaseItemDto) -> bool {
    is_playable(item) || matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season)
}

/// What has watched and favorite state worth setting from a card.
pub fn can_mark(item: &BaseItemDto) -> bool {
    can_add_to_playlist(item) || item.kind == BaseItemKind::BoxSet
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::first_unwatched;

    fn episode(id: &str, season: i32, played: bool) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": id,
            "Type": "Episode",
            "ParentIndexNumber": season,
            "UserData": { "Played": played, "PlaybackPositionTicks": 0, "IsFavorite": false },
        }))
        .expect("episode")
    }

    fn pick(episodes: &[BaseItemDto]) -> Option<&str> {
        first_unwatched(episodes).map(|e| e.id.as_str())
    }

    #[test]
    fn plays_the_first_unwatched_episode() {
        let episodes = [episode("a", 1, true), episode("b", 1, false), episode("c", 2, false)];
        assert_eq!(pick(&episodes), Some("b"));
    }

    #[test]
    fn starts_over_when_everything_is_watched() {
        let episodes = [episode("a", 1, true), episode("b", 1, true)];
        assert_eq!(pick(&episodes), Some("a"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn skips_specials_unless_there_is_nothing_else() {
        let episodes = [
            episode("special", 0, false),
            episode("a", 1, true),
            episode("b", 1, false),
        ];
        assert_eq!(pick(&episodes), Some("b"));
        let watched = [episode("special", 0, false), episode("a", 1, true)];
        assert_eq!(pick(&watched), Some("a"));
        assert_eq!(pick(&[episode("special", 0, false)]), Some("special"));
    }
}
