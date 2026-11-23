//! Real responses from Jellyfin 12.2 and 10.11 (captured with `dev/capture-fixtures.py` from
//! the dev servers) must keep parsing into the client's models.

use std::path::{Path, PathBuf};

use jellyfin::models::{
    AuthenticationResult, BaseItemDto, BaseItemKind, Culture, ItemFilterValues, MediaSegmentDto, MediaSegmentType,
    PublicSystemInfo, QueryResult, SubtitleMode,
};
use jellyfin::{GroupInfo, GroupState, PlaybackInfoResponse, Ticks};
use serde::Serialize;
use serde::de::DeserializeOwned;

const VERSIONS: [&str; 2] = ["12.2", "10.11"];

/// The folder of fixtures written by hand, for responses the dev servers can't produce (no
/// SyncPlay group, no segment provider plugin, nothing watched yet to be "next up"). They follow
/// the models and the field names of the captured files, and are loaded like a version.
const HANDWRITTEN: &str = "handwritten";

fn dir(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(version)
}

/// Parses a fixture, naming the file and the JSON path on failure.
fn load<T: DeserializeOwned>(version: &str, name: &str) -> T {
    let path = dir(version).join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let de = &mut serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(de)
        .unwrap_or_else(|err| panic!("{version}/{name}.json at `{}`: {}", err.path(), err.inner()))
}

#[test]
fn every_fixture_parses() {
    for version in VERSIONS {
        let info: PublicSystemInfo = load(version, "public_info");
        let reported = info.server_version().expect("a server version");
        assert!(reported.is_supported(), "{version}: {reported}");
        let auth: AuthenticationResult = load(version, "authenticate");
        let config = auth.user.configuration.expect("the user's configuration");
        assert_eq!(config.subtitle_mode, SubtitleMode::Default, "{version}");
        assert!(config.remember_audio_selections, "{version}");
        let cultures: Vec<Culture> = load(version, "cultures");
        assert!(
            cultures
                .iter()
                .any(|culture| culture.three_letter_iso_language_name.as_deref() == Some("jpn")),
            "{version}"
        );
        for name in ["user_views", "items_movies", "seasons", "episodes", "resume", "next_up"] {
            let _: QueryResult<BaseItemDto> = load(version, name);
        }
        for name in ["item_movie", "item_series"] {
            let _: BaseItemDto = load(version, name);
        }
        // `latest` is empty on 12.2, `resume` on 10.11 and `next_up` on both (the dev media has
        // nothing in progress to continue): those parse as empty lists, and the handwritten
        // fixtures below cover the types that no captured file fills.
        let _: Vec<BaseItemDto> = load(version, "latest");
        let filters: ItemFilterValues = load(version, "item_filters");
        assert!(filters.genres.contains(&"Science Fiction".to_owned()), "{version}");
        assert!(filters.years.contains(&2024), "{version}");
        assert!(filters.tags.contains(&"multi-version".to_owned()), "{version}");
        assert!(filters.official_ratings.contains(&"PG-13".to_owned()), "{version}");
        // Both are empty when captured (`media_segments` needs a plugin, `syncplay_list` a group):
        // `handwritten_fixtures_carry_what_the_app_reads` checks the filled shapes.
        let _: QueryResult<MediaSegmentDto> = load(version, "media_segments");
        let _: Vec<GroupInfo> = load(version, "syncplay_list");
        let _: PlaybackInfoResponse = load(version, "playback_info_direct");
        let _: PlaybackInfoResponse = load(version, "playback_info_transcode");
    }
}

/// What a download saves must read back as what the server sent: the same PascalCase names,
/// and every value the fixtures hold (enums included) going through JSON unchanged. `item.json`
/// keeps these types, and a round trip that lost a field would lose it in offline playback.
#[test]
fn the_downloaded_models_round_trip_through_json() {
    for version in VERSIONS {
        // The session snapshot `session.json` keeps, for a launch the server can't answer.
        let auth: AuthenticationResult = load(version, "authenticate");
        let user_value = fixture_value(version, "authenticate");
        round_trips(&auth.user, &user_value["User"], version, "authenticate.User");
        let configuration = auth.user.configuration.as_ref().expect("the user's configuration");
        round_trips(
            configuration,
            &user_value["User"]["Configuration"],
            version,
            "authenticate.User.Configuration",
        );
        let info: PublicSystemInfo = load(version, "public_info");
        round_trips(&info, &fixture_value(version, "public_info"), version, "public_info");

        let movie: BaseItemDto = load(version, "item_movie");
        round_trips(&movie, &fixture_value(version, "item_movie"), version, "item_movie");
        // A download keeps one media source (its streams are what the player's track mapping
        // reads), and the segments beside it.
        let direct: PlaybackInfoResponse = load(version, "playback_info_direct");
        let original = fixture_value(version, "playback_info_direct");
        for (index, source) in direct.media_sources.iter().enumerate() {
            round_trips(
                source,
                &original["MediaSources"][index],
                version,
                "playback_info_direct.MediaSources",
            );
        }
        // The captured segments are empty, so this runs on the handwritten ones.
        let segments: QueryResult<MediaSegmentDto> = load(HANDWRITTEN, "media_segments");
        let original = fixture_value(HANDWRITTEN, "media_segments");
        assert!(
            !segments.items.is_empty(),
            "the handwritten media_segments.json has no segments"
        );
        for (index, segment) in segments.items.iter().enumerate() {
            round_trips(segment, &original["Items"][index], version, "media_segments.Items");
        }
        let chapters = &movie.chapters;
        let original = &fixture_value(version, "item_movie")["Chapters"];
        for (index, chapter) in chapters.iter().enumerate() {
            round_trips(chapter, &original[index], version, "item_movie.Chapters");
        }
        let data = movie.user_data.as_ref().expect("the user's data");
        round_trips(
            data,
            &fixture_value(version, "item_movie")["UserData"],
            version,
            "item_movie.UserData",
        );
    }
}

/// A fixture's raw JSON, to compare the models' output with what the server sends.
fn fixture_value(version: &str, name: &str) -> serde_json::Value {
    let path = dir(version).join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{version}/{name}.json: {err}"))
}

/// Serializes `value`, reads it back and serializes it again: both must be the same JSON, named
/// the way the server names its fields, with nothing it sent left as `null`.
fn round_trips<T>(value: &T, original: &serde_json::Value, version: &str, name: &str)
where
    T: Serialize + DeserializeOwned,
{
    let json = serde_json::to_value(value).unwrap_or_else(|err| panic!("{version}/{name}: {err}"));
    let again: T = serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{version}/{name}: {err}"));
    let twice = serde_json::to_value(&again).expect("serializes again");
    assert_eq!(twice, json, "{version}/{name}: the second round trip differs");
    let object = json
        .as_object()
        .unwrap_or_else(|| panic!("{version}/{name}: not an object"));
    for (key, value) in object {
        assert!(
            key.starts_with(char::is_uppercase),
            "{version}/{name}: `{key}` isn't PascalCase"
        );
        let sent = original.get(key);
        if value.is_null() {
            // A field the server filled that came back null wasn't decoded: a wrong name or type.
            assert!(
                sent.is_none_or(serde_json::Value::is_null),
                "{version}/{name}: the server sent `{key}` but it read as null"
            );
        } else {
            // An empty collection may be one the server leaves out; a filled one it can't.
            let empty = match value {
                serde_json::Value::Array(items) => items.is_empty(),
                serde_json::Value::Object(map) => map.is_empty(),
                _ => false,
            };
            assert!(empty || sent.is_some(), "{version}/{name}: the server sends no `{key}`");
        }
    }
}

/// The three responses no captured file fills, written by hand (`tests/fixtures/handwritten/`):
/// what the app reads from each must come out of them set, which an empty list can't show.
#[test]
fn handwritten_fixtures_carry_what_the_app_reads() {
    // A SyncPlay group list: the join dialog shows the name, state and members.
    let groups: Vec<GroupInfo> = load(HANDWRITTEN, "syncplay_list");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].group_name, "Movie night");
    assert!(!groups[0].group_id.is_empty());
    assert_eq!(groups[0].state, GroupState::Playing);
    assert_eq!(groups[0].participants, ["dev", "guest"]);
    assert_eq!(groups[1].state, GroupState::Idle);
    assert!(groups[1].participants.is_empty());

    // Media segments: the skip button reads the kind and the span.
    let segments: QueryResult<MediaSegmentDto> = load(HANDWRITTEN, "media_segments");
    assert_eq!(segments.items.len(), 2);
    assert_eq!(segments.items[0].kind, MediaSegmentType::Intro);
    assert_eq!(segments.items[0].start_ticks, Ticks(0));
    assert_eq!(segments.items[0].end_ticks, Ticks(100_000_000));
    assert_eq!(segments.items[1].kind, MediaSegmentType::Outro);
    assert!(segments.items[1].start_ticks < segments.items[1].end_ticks);

    // Next up: an episode with the fields the home page's card and the player's next-episode
    // autoplay use.
    let next_up: QueryResult<BaseItemDto> = load(HANDWRITTEN, "next_up");
    assert_eq!(next_up.items.len(), 1);
    let episode = &next_up.items[0];
    assert_eq!(episode.kind, BaseItemKind::Episode);
    assert!(!episode.id.is_empty() && !episode.name.is_empty());
    assert_eq!(episode.series_name.as_deref(), Some("Test Show"));
    assert!(episode.series_id.is_some() && episode.season_id.is_some());
    assert_eq!(episode.parent_index_number, Some(1));
    assert_eq!(episode.index_number, Some(2));
    assert!(episode.run_time_ticks.is_some());
    assert!(episode.user_data.is_some());
}

#[test]
fn item_details() {
    for version in VERSIONS {
        let movie: BaseItemDto = load(version, "item_movie");
        assert_eq!(movie.kind, BaseItemKind::Movie);
        // 1080p, 720p and 4K HEVC versions, each with English and Japanese audio.
        assert_eq!(movie.media_sources.len(), 3, "{version}");
        assert!(
            movie.media_sources.iter().all(|s| s.media_streams.len() >= 3),
            "{version}"
        );
        // The server sends `CanDownload` with `/Items/{id}`; downloads depend on it.
        assert!(movie.can_download, "{version}");
        // Intro / Episode / Credits from dev/make-media.sh.
        let chapters: Vec<_> = movie.chapters.iter().filter_map(|c| c.name.as_deref()).collect();
        assert_eq!(chapters, ["Intro", "Episode", "Credits"], "{version}");
        assert!(movie.primary_image().is_some(), "{version}");

        let episodes: QueryResult<BaseItemDto> = load(version, "episodes");
        let first = &episodes.items[0];
        assert_eq!(first.kind, BaseItemKind::Episode);
        assert!(first.series_id.is_some() && first.index_number.is_some(), "{version}");
    }
}

#[test]
fn playback_info() {
    for version in VERSIONS {
        let direct: PlaybackInfoResponse = load(version, "playback_info_direct");
        assert!(direct.play_session_id.is_some(), "{version}");
        assert!(direct.media_sources.iter().all(|s| s.supports_direct_play), "{version}");

        // Above the 2 Mbit/s limit: transcode only, an HLS URL, and text subtitles as files.
        let transcode: PlaybackInfoResponse = load(version, "playback_info_transcode");
        let source = &transcode.media_sources[0];
        assert!(!source.supports_direct_play && source.supports_transcoding, "{version}");
        let url = source.transcoding_url.as_deref().expect("a transcoding URL");
        assert!(url.contains("master.m3u8"), "{version}: {url}");
        assert!(
            source
                .media_streams
                .iter()
                .any(|s| s.delivery_method.as_deref() == Some("External") && s.delivery_url.is_some()),
            "{version}"
        );
    }
}
