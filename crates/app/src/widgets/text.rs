//! Text for cards and pages (titles, episode labels, times, runtimes), the poster and
//! landscape image requests a card makes, and the loading state pages keep. The cards and
//! shelves themselves are `widgets/card.rs` and `widgets/shelf.rs` (W1).

use std::time::Duration;

use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType};
use jellyfin::{ImageRequest, Ticks};

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
                .map(|n| format!("{n} unwatched"))
                .unwrap_or_default(),
        ),
        _ => (
            item.name.clone(),
            item.production_year.map(|y| y.to_string()).unwrap_or_default(),
        ),
    }
}

/// `S1:E2`, or an empty string when numbering is unknown.
pub fn episode_label(item: &BaseItemDto) -> String {
    match (item.parent_index_number, item.index_number) {
        (Some(s), Some(e)) => format!("S{s}:E{e}"),
        (None, Some(e)) => format!("E{e}"),
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
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// `GuestStar` -> `Guest star`.
pub fn kind_label(kind: &str) -> String {
    let mut label = String::new();
    for (i, c) in kind.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            label.push(' ');
            label.extend(c.to_lowercase());
        } else {
            label.push(c);
        }
    }
    label
}

/// "Episode 3", "episode 03": the name Jellyfin gives an episode without a title.
pub fn is_placeholder_episode_name(name: &str) -> bool {
    name.split_once(' ').is_some_and(|(word, number)| {
        word.eq_ignore_ascii_case("episode") && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::is_placeholder_episode_name;

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
