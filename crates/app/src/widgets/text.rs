//! Text for cards and pages (titles, episode labels, times, runtimes), the poster and
//! landscape image requests a card makes, and the loading state pages keep. The cards and
//! shelves themselves are `widgets/card.rs` and `widgets/shelf.rs`.

use std::time::Duration;

use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType};
use jellyfin::{ImageRequest, Ticks};

use crate::i18n::tr;

/// Poster and landscape card sizes at their default scale, and the space between cards.
pub const POSTER_W: f32 = 150.;
pub const POSTER_H: f32 = 225.;
pub const LANDSCAPE_W: f32 = 260.;
pub const LANDSCAPE_H: f32 = 146.;
pub const CARD_GAP: f32 = 16.;
/// Height of the text block under a card (title + subtitle lines).
pub const CARD_TEXT_H: f32 = 44.;
/// Cast and crew cards' sizes at their default scale.
pub const PERSON_W: f32 = 120.;
pub const PERSON_H: f32 = 180.;

/// What a page shows while its data loads, once it's there, or after the request failed (with
/// the message for the user).
#[derive(Debug, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Loaded(T),
    Failed(String),
}

/// Tall image for an item. An episode's own image is a landscape still that a poster would
/// crop, so it shows its series' poster, as Jellyfin's web client does (Latest lists a single
/// new episode on its own rather than under its series).
pub fn poster_image(item: &BaseItemDto) -> Option<ImageRequest> {
    let series_poster = || {
        let series_id = item.series_id.as_ref()?;
        let tag = item.series_primary_image_tag.clone()?;
        Some(ImageRequest::new(series_id, ImageType::Primary, Some(tag)))
    };
    match item.kind {
        BaseItemKind::Episode => series_poster().or_else(|| item.primary_image()),
        _ => item.primary_image(),
    }
}

/// Wide image for an item: its own thumb/backdrop, then the parent's, then the primary image
/// (episode screen grabs are already landscape).
pub fn landscape_image(item: &BaseItemDto) -> Option<ImageRequest> {
    item.image(ImageType::Thumb)
        .or_else(|| {
            let id = item.parent_thumb_item_id.as_ref()?;
            let tag = item.parent_thumb_image_tag.clone()?;
            (item.kind != BaseItemKind::Episode).then(|| ImageRequest::new(id, ImageType::Thumb, Some(tag)))
        })
        .or_else(|| {
            (item.kind == BaseItemKind::Episode)
                .then(|| item.image(ImageType::Primary))
                .flatten()
        })
        .or_else(|| item.backdrop_image())
        .or_else(|| item.image(ImageType::Primary))
}

/// Title and secondary line for a card. Episodes show the series as the title.
pub fn card_text(item: &BaseItemDto) -> (String, String) {
    match item.kind {
        BaseItemKind::Episode => (
            item.series_name.clone().unwrap_or_else(|| item.name.clone()),
            format!("{} · {}", episode_label(item), item.name),
        ),
        BaseItemKind::Season => (
            item.name.clone(),
            item.user_data
                .as_ref()
                .and_then(|d| d.unplayed_item_count)
                .filter(|n| *n > 0)
                .map(|n| tr!("widget-unwatched", count = (n as usize)))
                .unwrap_or_default(),
        ),
        _ => (
            item.name.clone(),
            item.production_year.map(|y| y.to_string()).unwrap_or_default(),
        ),
    }
}

/// `S1:E2` (`T1:E2` in Portuguese), or an empty string when numbering is unknown.
pub fn episode_label(item: &BaseItemDto) -> String {
    match (item.parent_index_number, item.index_number) {
        (Some(s), Some(e)) => tr!("item-episode-label", season = s, episode = e),
        (None, Some(e)) => tr!("item-episode-label-short", episode = e),
        _ => String::new(),
    }
}

/// `1:02:03`, `4:05`.
pub fn timestamp(time: Duration) -> String {
    let secs = time.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1h 42m`, `45m`.
pub fn runtime_text(ticks: Ticks) -> String {
    let minutes = (ticks.as_secs_f64() / 60.).round() as u64;
    match (minutes / 60, minutes % 60) {
        (0, m) => tr!("item-runtime-minutes", minutes = m),
        (h, 0) => tr!("item-runtime-hours", hours = h),
        (h, m) => tr!("item-runtime-hours-minutes", hours = h, minutes = m),
    }
}

/// A person's role as the server names it (`GuestStar`), in the interface's language. `Actor`,
/// `Unknown` and any kind this has no message for show the server's own text.
pub fn kind_label(kind: &str) -> String {
    match kind {
        "Director" => tr!("person-director"),
        "Writer" => tr!("person-writer"),
        "Producer" => tr!("person-producer"),
        "GuestStar" => tr!("person-guest-star"),
        "Composer" => tr!("person-composer"),
        "Conductor" => tr!("person-conductor"),
        "Lyricist" => tr!("person-lyricist"),
        "Arranger" => tr!("person-arranger"),
        "Engineer" => tr!("person-engineer"),
        "Mixer" => tr!("person-mixer"),
        "Remixer" => tr!("person-remixer"),
        "Creator" => tr!("person-creator"),
        "Artist" => tr!("person-artist"),
        "AlbumArtist" => tr!("person-album-artist"),
        "Author" => tr!("person-author"),
        "Illustrator" => tr!("person-illustrator"),
        "Penciller" => tr!("person-penciller"),
        "Inker" => tr!("person-inker"),
        "Colorist" => tr!("person-colorist"),
        "Letterer" => tr!("person-letterer"),
        "CoverArtist" => tr!("person-cover-artist"),
        "Editor" => tr!("person-editor"),
        "Translator" => tr!("person-translator"),
        other => other.to_owned(),
    }
}

/// "Episode 3", "episode 03": the name Jellyfin gives an episode without a title.
pub fn is_placeholder_episode_name(name: &str) -> bool {
    name.split_once(' ').is_some_and(|(word, number)| {
        word.eq_ignore_ascii_case("episode") && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{is_placeholder_episode_name, kind_label};

    /// Every kind Jellyfin's `PersonKind` has, bar `Actor` and `Unknown`, reads as a message.
    #[test]
    fn the_credit_kinds_of_a_cast_row_have_messages() {
        for kind in [
            "Director",
            "Writer",
            "Producer",
            "GuestStar",
            "Composer",
            "Conductor",
            "Lyricist",
            "Arranger",
            "Engineer",
            "Mixer",
            "Remixer",
            "Creator",
            "Artist",
            "AlbumArtist",
            "Author",
            "Illustrator",
            "Penciller",
            "Inker",
            "Colorist",
            "Letterer",
            "CoverArtist",
            "Editor",
            "Translator",
        ] {
            let label = kind_label(kind);
            assert!(!label.starts_with("person-") && !label.is_empty(), "{kind}: {label}");
        }
        assert_eq!(kind_label("Actor"), "Actor");
        assert_eq!(kind_label("CoverArtist"), "Cover artist");
    }

    #[test]
    fn jellyfins_names_for_untitled_episodes() {
        assert!(is_placeholder_episode_name("Episode 3"));
        assert!(is_placeholder_episode_name("episode 03"));
        assert!(!is_placeholder_episode_name("Episode"));
        assert!(!is_placeholder_episode_name("Episode IV"));
        assert!(!is_placeholder_episode_name("Frozen Bonds"));
        assert!(!is_placeholder_episode_name("Episode 3: The Return"));
    }
}
