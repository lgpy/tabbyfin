//! Hand-written DTOs covering only the fields the app uses. Unknown fields are ignored, and
//! missing or `null` collections decode as empty, so server additions don't break decoding.

use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Deserializer};

use crate::Ticks;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
    pub id: Option<String>,
    pub server_name: Option<String>,
    pub version: Option<String>,
    pub product_name: Option<String>,
    pub local_address: Option<String>,
    pub startup_wizard_completed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticationResult {
    pub user: UserDto,
    pub access_token: String,
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserDto {
    pub id: String,
    pub name: String,
    pub server_id: Option<String>,
    pub primary_image_tag: Option<String>,
    #[serde(default)]
    pub configuration: Option<UserConfiguration>,
}

/// The user's preferences kept by the server (the web client's Settings → Home and Display).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserConfiguration {
    /// Library ids left out of "My Media". `/UserViews` already leaves them out.
    #[serde(default, deserialize_with = "nullable")]
    pub my_media_excludes: Vec<String>,
    /// Library ids without a "Latest" row on Home.
    #[serde(default, deserialize_with = "nullable")]
    pub latest_items_excludes: Vec<String>,
}

/// A client's stored display preferences. Only the free-form `CustomPrefs` are used.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayPreferences {
    #[serde(default, deserialize_with = "map_skip_nulls")]
    pub custom_prefs: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QuickConnectResult {
    #[serde(default)]
    pub authenticated: bool,
    /// Kept by the client to poll for approval and to log in.
    pub secret: String,
    /// Shown to the user, who enters it in another signed-in client.
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase", bound(deserialize = "T: Deserialize<'de>"))]
pub struct QueryResult<T> {
    #[serde(default, deserialize_with = "nullable")]
    pub items: Vec<T>,
    #[serde(default)]
    pub total_record_count: u32,
    #[serde(default)]
    pub start_index: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItemDto {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub server_id: Option<String>,
    #[serde(rename = "Type", default)]
    pub kind: BaseItemKind,
    #[serde(default, deserialize_with = "nullable")]
    pub is_folder: bool,
    pub collection_type: Option<CollectionType>,
    pub parent_id: Option<String>,

    pub overview: Option<String>,
    pub production_year: Option<i32>,
    pub premiere_date: Option<String>,
    pub official_rating: Option<String>,
    pub community_rating: Option<f32>,
    pub run_time_ticks: Option<Ticks>,
    #[serde(default, deserialize_with = "nullable")]
    pub genres: Vec<String>,

    pub series_id: Option<String>,
    pub series_name: Option<String>,
    pub season_id: Option<String>,
    pub season_name: Option<String>,
    /// Episode number, or season number for seasons.
    pub index_number: Option<i32>,
    /// Season number for episodes.
    pub parent_index_number: Option<i32>,
    pub child_count: Option<u32>,
    pub recursive_item_count: Option<u32>,
    /// A playlist entry's id, for moving or removing it. Present on playlist items; on 10.11
    /// and 12.2 it's the item's own id, so duplicate entries can't be told apart.
    pub playlist_item_id: Option<String>,
    /// Only present when requested with [`crate::models::ItemField::CanDelete`]. Not a
    /// permission check for playlists: other users' shared playlists report `true` too.
    pub can_delete: Option<bool>,

    pub user_data: Option<UserItemDataDto>,

    #[serde(default, deserialize_with = "map_skip_nulls")]
    pub image_tags: HashMap<ImageType, String>,
    #[serde(default, deserialize_with = "nullable")]
    pub backdrop_image_tags: Vec<String>,
    /// Image type -> image tag -> blurhash.
    #[serde(default, deserialize_with = "nullable")]
    pub image_blur_hashes: HashMap<ImageType, HashMap<String, Option<String>>>,
    pub primary_image_aspect_ratio: Option<f64>,
    pub series_primary_image_tag: Option<String>,
    pub parent_backdrop_item_id: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub parent_backdrop_image_tags: Vec<String>,
    pub parent_thumb_item_id: Option<String>,
    pub parent_thumb_image_tag: Option<String>,
    pub parent_logo_item_id: Option<String>,
    pub parent_logo_image_tag: Option<String>,

    /// Only present when requested with [`crate::models::ItemField::MediaSources`].
    /// More than one entry means multiple versions (movies, and episodes since 12.0).
    #[serde(default, deserialize_with = "nullable")]
    pub media_sources: Vec<MediaSourceInfo>,
    /// Cast and crew. Only present when requested with [`crate::models::ItemField::People`].
    #[serde(default, deserialize_with = "nullable")]
    pub people: Vec<BaseItemPerson>,
    /// Only present when requested with [`crate::models::ItemField::Chapters`].
    #[serde(default, deserialize_with = "nullable")]
    pub chapters: Vec<ChapterInfo>,
    /// Only present when requested with [`crate::models::ItemField::Studios`].
    #[serde(default, deserialize_with = "nullable")]
    pub studios: Vec<NameGuidPair>,
    /// Only present when requested with [`crate::models::ItemField::Tags`].
    #[serde(default, deserialize_with = "nullable")]
    pub tags: Vec<String>,
    /// Trailers on the web (usually YouTube). Only present when requested with
    /// [`crate::models::ItemField::RemoteTrailers`].
    #[serde(default, deserialize_with = "nullable")]
    pub remote_trailers: Vec<MediaUrl>,
    /// Trailer files next to the item (`GET /Items/{id}/LocalTrailers`).
    pub local_trailer_count: Option<u32>,
    /// Extras such as featurettes and deleted scenes (`GET /Items/{id}/SpecialFeatures`).
    pub special_feature_count: Option<u32>,
    /// Seek-bar preview tiles, by media source id and then thumbnail width. Only present when
    /// requested with [`crate::models::ItemField::Trickplay`], and generated.
    #[serde(default, deserialize_with = "nullable")]
    pub trickplay: HashMap<String, HashMap<String, TrickplayInfo>>,
}

/// The genres and years found among a library's items (`GET /Items/Filters`), to filter by.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemFilterValues {
    #[serde(default, deserialize_with = "nullable")]
    pub genres: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub years: Vec<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NameGuidPair {
    pub name: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaUrl {
    pub url: String,
    pub name: Option<String>,
}

/// One set of trickplay tiles: JPEG sheets of `tile_width` × `tile_height` thumbnails, each
/// `width` × `height`, one every `interval` ms. Sheet `n` holds thumbnails from
/// `n × tile_width × tile_height`, row by row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrickplayInfo {
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub thumbnail_count: u32,
    /// Milliseconds between thumbnails.
    pub interval: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItemPerson {
    pub id: String,
    pub name: String,
    /// The character, for actors.
    pub role: Option<String>,
    /// `Actor`, `Director`, `Writer`, `Producer`, `GuestStar`…
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub primary_image_tag: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChapterInfo {
    #[serde(default, deserialize_with = "nullable")]
    pub start_position_ticks: Ticks,
    pub name: Option<String>,
}

/// A stretch of an item marked by a segment provider plugin (e.g. Intro Skipper).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSegmentDto {
    #[serde(rename = "Type", default)]
    pub kind: MediaSegmentType,
    pub start_ticks: Ticks,
    pub end_ticks: Ticks,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserItemDataDto {
    /// Resume position. Since 12.0 this is the only place it is reported
    /// (`MediaSourceInfo.PlaybackPositionTicks` was removed).
    #[serde(default, deserialize_with = "nullable")]
    pub playback_position_ticks: Ticks,
    pub played_percentage: Option<f64>,
    #[serde(default, deserialize_with = "nullable")]
    pub play_count: u32,
    #[serde(default, deserialize_with = "nullable")]
    pub is_favorite: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub played: bool,
    pub unplayed_item_count: Option<u32>,
    pub last_played_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSourceInfo {
    pub id: String,
    /// Version label, e.g. `1080p`.
    pub name: Option<String>,
    pub container: Option<String>,
    pub size: Option<i64>,
    pub bitrate: Option<i64>,
    pub run_time_ticks: Option<Ticks>,
    #[serde(default, deserialize_with = "nullable")]
    pub supports_direct_play: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub supports_direct_stream: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub supports_transcoding: bool,
    /// Server-relative HLS URL, set when the server will only transcode (see `PlaybackInfo`).
    /// Carries an `ApiKey`; use [`crate::Client::transcode_url`], which removes it.
    pub transcoding_url: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub media_streams: Vec<MediaStream>,
    pub default_audio_stream_index: Option<i32>,
    pub default_subtitle_stream_index: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    pub index: i32,
    #[serde(rename = "Type", default)]
    pub kind: MediaStreamType,
    pub codec: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub is_default: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub is_forced: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub is_external: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub is_text_subtitle_stream: bool,
    /// How the server delivers a subtitle in this playback (`PlaybackInfo` responses only):
    /// `External` as a file at `delivery_url`, `Encode` burned into a transcode, `Embed`.
    pub delivery_method: Option<String>,
    /// Server-relative URL for subtitles delivered as files.
    pub delivery_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
}

/// Defines an enum that decodes from Jellyfin's string values, maps anything unrecognized to
/// `Other`, and can be turned back into the wire string for query parameters.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant,)+
            #[default]
            #[serde(other)]
            Other,
        }

        impl $name {
            /// The wire string, or `None` for `Other`.
            pub fn as_str(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => Some($wire),)+
                    Self::Other => None,
                }
            }
        }
    };
}

wire_enum!(BaseItemKind {
    Movie = "Movie",
    Series = "Series",
    Season = "Season",
    Episode = "Episode",
    BoxSet = "BoxSet",
    Video = "Video",
    Trailer = "Trailer",
    MusicVideo = "MusicVideo",
    Playlist = "Playlist",
    Person = "Person",
    Folder = "Folder",
    CollectionFolder = "CollectionFolder",
    UserView = "UserView",
});

wire_enum!(CollectionType {
    Movies = "movies",
    TvShows = "tvshows",
    Music = "music",
    MusicVideos = "musicvideos",
    HomeVideos = "homevideos",
    BoxSets = "boxsets",
    Playlists = "playlists",
    LiveTv = "livetv",
    Books = "books",
    Photos = "photos",
    Folders = "folders",
});

wire_enum!(ImageType {
    Primary = "Primary",
    Art = "Art",
    Backdrop = "Backdrop",
    Banner = "Banner",
    Logo = "Logo",
    Thumb = "Thumb",
    Disc = "Disc",
    Box = "Box",
    Screenshot = "Screenshot",
    Menu = "Menu",
    Chapter = "Chapter",
    BoxRear = "BoxRear",
    Profile = "Profile",
});

wire_enum!(MediaStreamType {
    Audio = "Audio",
    Video = "Video",
    Subtitle = "Subtitle",
    EmbeddedImage = "EmbeddedImage",
    Data = "Data",
    Lyric = "Lyric",
});

wire_enum!(
    /// Extra fields to include in item responses (`fields=` query parameter).
    ItemField {
        Overview = "Overview",
        Genres = "Genres",
        People = "People",
        Studios = "Studios",
        Taglines = "Taglines",
        MediaSources = "MediaSources",
        MediaStreams = "MediaStreams",
        PrimaryImageAspectRatio = "PrimaryImageAspectRatio",
        ChildCount = "ChildCount",
        RecursiveItemCount = "RecursiveItemCount",
        DateCreated = "DateCreated",
        ParentId = "ParentId",
        ProviderIds = "ProviderIds",
        SortName = "SortName",
        Chapters = "Chapters",
        Trickplay = "Trickplay",
        Tags = "Tags",
        RemoteTrailers = "RemoteTrailers",
        CanDelete = "CanDelete",
    }
);

wire_enum!(
    MediaSegmentType {
        Unknown = "Unknown",
        Commercial = "Commercial",
        Preview = "Preview",
        Recap = "Recap",
        Outro = "Outro",
        Intro = "Intro",
    }
);

wire_enum!(ItemSortBy {
    Default = "Default",
    SortName = "SortName",
    DateCreated = "DateCreated",
    DatePlayed = "DatePlayed",
    PremiereDate = "PremiereDate",
    ProductionYear = "ProductionYear",
    CommunityRating = "CommunityRating",
    Runtime = "Runtime",
    Random = "Random",
    ParentIndexNumber = "ParentIndexNumber",
    IndexNumber = "IndexNumber",
});

wire_enum!(ItemFilter {
    IsFolder = "IsFolder",
    IsNotFolder = "IsNotFolder",
    IsPlayed = "IsPlayed",
    IsUnplayed = "IsUnplayed",
    IsFavorite = "IsFavorite",
    IsResumable = "IsResumable",
});

/// Wire strings for a list of enum values, dropping `Other`.
pub(crate) fn wire_list<T: Copy>(values: &[T], as_str: fn(T) -> Option<&'static str>) -> Vec<&'static str> {
    values.iter().filter_map(|v| as_str(*v)).collect()
}

/// Treats an explicit `null` like a missing field.
fn nullable<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(de)?.unwrap_or_default())
}

/// Decodes a map whose values may be `null`, dropping those entries.
fn map_skip_nulls<'de, D, K, V>(de: D) -> Result<HashMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Eq + Hash + Deserialize<'de>,
    V: Deserialize<'de>,
{
    let map = Option::<HashMap<K, Option<V>>>::deserialize(de)?.unwrap_or_default();
    Ok(map.into_iter().filter_map(|(k, v)| Some((k, v?))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_nulls_and_unknown_values() {
        let json = r#"{
            "Id": "abc", "Name": "Pilot", "Type": "Episode", "IsFolder": null,
            "CollectionType": "something-new", "Genres": null,
            "ImageTags": {"Primary": "t1", "Logo": null, "FutureType": "x"},
            "UserData": {"PlaybackPositionTicks": 150000000, "Played": false},
            "MediaSources": [{"Id": "ms", "Name": "1080p", "MediaStreams": [
                {"Index": 0, "Type": "Video", "Width": 1920},
                {"Index": 1, "Type": "Hologram"}
            ]}],
            "SomeFieldFromTheFuture": 1
        }"#;
        let item: BaseItemDto = serde_json::from_str(json).unwrap();
        assert_eq!(item.kind, BaseItemKind::Episode);
        assert_eq!(item.collection_type, Some(CollectionType::Other));
        assert!(!item.is_folder && item.genres.is_empty());
        assert_eq!(item.image_tags.get(&ImageType::Primary).map(String::as_str), Some("t1"));
        assert!(!item.image_tags.contains_key(&ImageType::Logo));
        assert_eq!(item.user_data.unwrap().playback_position_ticks, Ticks(150_000_000));
        let streams = &item.media_sources[0].media_streams;
        assert_eq!(streams[0].kind, MediaStreamType::Video);
        assert_eq!(streams[1].kind, MediaStreamType::Other);
    }
}
