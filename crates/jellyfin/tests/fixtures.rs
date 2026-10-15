//! Real responses from Jellyfin 12.2 and 10.11 (captured with `dev/capture-fixtures.py` from
//! the dev servers) must keep parsing into the client's models.

use std::path::{Path, PathBuf};

use jellyfin::models::{
    AuthenticationResult, BaseItemDto, BaseItemKind, ItemFilterValues, MediaSegmentDto, PublicSystemInfo, QueryResult,
};
use jellyfin::{GroupInfo, PlaybackInfoResponse};
use serde::de::DeserializeOwned;

const VERSIONS: [&str; 2] = ["12.2", "10.11"];

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
        let _: AuthenticationResult = load(version, "authenticate");
        for name in ["user_views", "items_movies", "seasons", "episodes", "resume", "next_up"] {
            let _: QueryResult<BaseItemDto> = load(version, name);
        }
        for name in ["item_movie", "item_series"] {
            let _: BaseItemDto = load(version, name);
        }
        let _: Vec<BaseItemDto> = load(version, "latest");
        let filters: ItemFilterValues = load(version, "item_filters");
        assert!(filters.genres.contains(&"Science Fiction".to_owned()), "{version}");
        assert!(filters.years.contains(&2024), "{version}");
        let _: QueryResult<MediaSegmentDto> = load(version, "media_segments");
        let _: Vec<GroupInfo> = load(version, "syncplay_list");
        let _: PlaybackInfoResponse = load(version, "playback_info_direct");
        let _: PlaybackInfoResponse = load(version, "playback_info_transcode");
    }
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
