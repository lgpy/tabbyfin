use crate::client::Query;
use crate::models::{
    BaseItemDto, BaseItemKind, ItemField, ItemFilter, ItemFilterValues, ItemSortBy, MediaSegmentDto, QueryResult,
    wire_list,
};
use crate::{Client, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    fn as_str(self) -> &'static str {
        match self {
            SortOrder::Ascending => "Ascending",
            SortOrder::Descending => "Descending",
        }
    }
}

/// Parameters for `GET /Items`.
#[derive(Debug, Clone, Default)]
pub struct ItemsQuery {
    pub parent_id: Option<String>,
    pub include_item_types: Vec<BaseItemKind>,
    pub exclude_item_types: Vec<BaseItemKind>,
    /// Always sent explicitly: since 12.0 the server picks its own default when filters are
    /// combined with `includeItemTypes`, so leaving it out gives version-dependent results.
    pub recursive: bool,
    pub ids: Vec<String>,
    /// Items a person appears in.
    pub person_ids: Vec<String>,
    pub filters: Vec<ItemFilter>,
    /// Items with any of these genres (by name).
    pub genres: Vec<String>,
    /// Items released in any of these years.
    pub years: Vec<i32>,
    /// Video width bounds, for resolution filters. The server's own `isHd` and `is4K` filters
    /// aren't usable: `is4K=false` matches nothing on 10.11 and 12.2, and 12.2's `isHd` returns
    /// the hidden items of alternate versions too.
    pub min_width: Option<u32>,
    pub max_width: Option<u32>,
    pub search_term: Option<String>,
    /// Items whose sort name comes before this, ignoring case (used to count them).
    pub name_less_than: Option<String>,
    pub sort_by: Vec<ItemSortBy>,
    pub sort_order: Option<SortOrder>,
    pub fields: Vec<ItemField>,
    pub start_index: Option<u32>,
    pub limit: Option<u32>,
}

impl ItemsQuery {
    fn to_query(&self) -> Query {
        let mut q = Query::new();
        q.opt("parentId", self.parent_id.as_ref())
            .list(
                "includeItemTypes",
                &wire_list(&self.include_item_types, BaseItemKind::as_str),
            )
            .list(
                "excludeItemTypes",
                &wire_list(&self.exclude_item_types, BaseItemKind::as_str),
            )
            .push("recursive", self.recursive)
            .list("ids", &self.ids)
            .list("personIds", &self.person_ids)
            .list("filters", &wire_list(&self.filters, ItemFilter::as_str))
            .list("years", &self.years.iter().map(i32::to_string).collect::<Vec<_>>())
            .opt("minWidth", self.min_width)
            .opt("maxWidth", self.max_width)
            .opt("searchTerm", self.search_term.as_ref())
            .opt("nameLessThan", self.name_less_than.as_ref())
            .list("sortBy", &wire_list(&self.sort_by, ItemSortBy::as_str))
            .opt("sortOrder", self.sort_order.map(SortOrder::as_str))
            .list("fields", &wire_list(&self.fields, ItemField::as_str))
            .opt("startIndex", self.start_index)
            .opt("limit", self.limit);
        // Genre names may contain commas, so this list is the one the server splits on `|`.
        if !self.genres.is_empty() {
            q.push("genres", self.genres.join("|"));
        }
        q
    }
}

fn fields_query(fields: &[ItemField]) -> Query {
    let mut q = Query::new();
    q.list("fields", &wire_list(fields, ItemField::as_str));
    q
}

impl Client {
    /// The user's libraries (and other top-level views), in the user's configured order.
    pub async fn user_views(&self) -> Result<QueryResult<BaseItemDto>> {
        self.get("/UserViews", &Query::new()).await
    }

    pub async fn items(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItemDto>> {
        self.get("/Items", &query.to_query()).await
    }

    /// The genres and years among the items of a library (or folder) of the given types.
    pub async fn item_filter_values(
        &self,
        parent_id: &str,
        include_item_types: &[BaseItemKind],
    ) -> Result<ItemFilterValues> {
        let mut q = Query::new();
        q.push("parentId", parent_id)
            .list("includeItemTypes", &wire_list(include_item_types, BaseItemKind::as_str));
        self.get("/Items/Filters", &q).await
    }

    /// One item with the user's data. Request [`ItemField::MediaSources`] for versions/tracks.
    pub async fn item(&self, item_id: &str, fields: &[ItemField]) -> Result<BaseItemDto> {
        self.get(&format!("/Items/{item_id}"), &fields_query(fields)).await
    }

    /// Continue Watching: partially played videos, most recent first.
    pub async fn resume(&self, limit: u32, fields: &[ItemField]) -> Result<QueryResult<BaseItemDto>> {
        let mut q = fields_query(fields);
        q.push("mediaTypes", "Video").push("limit", limit);
        self.get("/UserItems/Resume", &q).await
    }

    /// The next unwatched episode of each series in progress.
    pub async fn next_up(&self, limit: u32, fields: &[ItemField]) -> Result<QueryResult<BaseItemDto>> {
        let mut q = fields_query(fields);
        q.push("limit", limit);
        self.get("/Shows/NextUp", &q).await
    }

    /// Recently added items in a library. Episodes are grouped under their series by default.
    pub async fn latest(&self, parent_id: &str, limit: u32, fields: &[ItemField]) -> Result<Vec<BaseItemDto>> {
        let mut q = fields_query(fields);
        q.push("parentId", parent_id).push("limit", limit);
        self.get("/Items/Latest", &q).await
    }

    pub async fn seasons(&self, series_id: &str, fields: &[ItemField]) -> Result<QueryResult<BaseItemDto>> {
        self.get(&format!("/Shows/{series_id}/Seasons"), &fields_query(fields))
            .await
    }

    /// Episodes of a series, optionally limited to one season.
    pub async fn episodes(
        &self,
        series_id: &str,
        season_id: Option<&str>,
        fields: &[ItemField],
    ) -> Result<QueryResult<BaseItemDto>> {
        let mut q = fields_query(fields);
        q.opt("seasonId", season_id);
        self.get(&format!("/Shows/{series_id}/Episodes"), &q).await
    }

    /// The episode after `episode_id` in its series (across seasons), if any.
    pub async fn next_episode(&self, series_id: &str, episode_id: &str) -> Result<Option<BaseItemDto>> {
        let mut q = Query::new();
        q.push("startItemId", episode_id).push("limit", 2);
        let episodes: QueryResult<BaseItemDto> = self.get(&format!("/Shows/{series_id}/Episodes"), &q).await?;
        let mut items = episodes.items.into_iter();
        Ok(match items.next() {
            Some(first) if first.id == episode_id => items.next(),
            // The start item itself is missing (e.g. removed): don't guess.
            _ => None,
        })
    }

    /// The collections (box sets) that contain the item. Jellyfin 12.0 added this; older
    /// servers answer 404, which comes back as no collections.
    pub async fn item_collections(&self, item_id: &str) -> Result<Vec<BaseItemDto>> {
        match self
            .get::<QueryResult<BaseItemDto>>(&format!("/Items/{item_id}/Collections"), &Query::new())
            .await
        {
            Ok(result) => Ok(result.items),
            Err(Error::NotFound) => Ok(Vec::new()),
            Err(err) => Err(err),
        }
    }

    /// Trailer files that belong to the item.
    pub async fn local_trailers(&self, item_id: &str) -> Result<Vec<BaseItemDto>> {
        self.get(&format!("/Items/{item_id}/LocalTrailers"), &Query::new())
            .await
    }

    /// Featurettes, deleted scenes, interviews and other extras of the item.
    pub async fn special_features(&self, item_id: &str) -> Result<Vec<BaseItemDto>> {
        self.get(&format!("/Items/{item_id}/SpecialFeatures"), &Query::new())
            .await
    }

    /// Items like this one (by genre, studio, tags…). The server leaves out watched ones.
    pub async fn similar(&self, item_id: &str, limit: u32, fields: &[ItemField]) -> Result<QueryResult<BaseItemDto>> {
        let mut q = fields_query(fields);
        q.push("limit", limit);
        self.get(&format!("/Items/{item_id}/Similar"), &q).await
    }

    /// A playlist's entries, in playlist order.
    pub async fn playlist_items(&self, playlist_id: &str, fields: &[ItemField]) -> Result<QueryResult<BaseItemDto>> {
        self.get(&format!("/Playlists/{playlist_id}/Items"), &fields_query(fields))
            .await
    }

    /// Intro, outro and similar segments, from the server's segment provider plugins. Empty
    /// when none are installed.
    pub async fn media_segments(&self, item_id: &str) -> Result<Vec<MediaSegmentDto>> {
        let segments: QueryResult<MediaSegmentDto> =
            self.get(&format!("/MediaSegments/{item_id}"), &Query::new()).await?;
        Ok(segments.items)
    }

    /// Marks an item (or every episode of a series/season) as watched.
    pub async fn mark_played(&self, item_id: &str) -> Result<()> {
        self.post_no_content::<()>(&format!("/UserPlayedItems/{item_id}"), &Query::new(), None)
            .await
    }

    pub async fn mark_unplayed(&self, item_id: &str) -> Result<()> {
        self.delete(&format!("/UserPlayedItems/{item_id}")).await
    }

    pub async fn mark_favorite(&self, item_id: &str) -> Result<()> {
        self.post_no_content::<()>(&format!("/UserFavoriteItems/{item_id}"), &Query::new(), None)
            .await
    }

    pub async fn unmark_favorite(&self, item_id: &str) -> Result<()> {
        self.delete(&format!("/UserFavoriteItems/{item_id}")).await
    }
}

#[cfg(test)]
mod tests {
    use super::ItemsQuery;

    #[test]
    fn genres_are_split_on_pipes_and_years_on_commas() {
        let query = ItemsQuery {
            genres: vec!["Action, Adventure".into(), "Drama".into()],
            years: vec![2024, 2025],
            max_width: Some(1199),
            ..Default::default()
        };
        let params = query.to_query().0;
        let get = |key| params.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("genres"), Some("Action, Adventure|Drama"));
        assert_eq!(get("years"), Some("2024,2025"));
        assert_eq!(get("maxWidth"), Some("1199"));
        assert_eq!(get("minWidth"), None);
    }
}
