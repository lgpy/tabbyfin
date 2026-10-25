//! Audio and subtitle track choices of a media source, for the player's settings menu, and the
//! versions of an item for its page's picker. The pickers themselves arrive with U10 (tracks)
//! and U6 (versions).

use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

/// Subtitle picker value for "Off".
pub const SUBTITLES_OFF: i32 = -1;

/// One entry in a version or track picker.
#[derive(Clone)]
pub struct Choice {
    pub label: String,
    /// Media source position for versions; Jellyfin stream index for tracks.
    pub value: i32,
}

fn label(stream: &MediaStream) -> String {
    stream
        .display_title
        .clone()
        .or_else(|| stream.language.clone())
        .unwrap_or_else(|| format!("Track {}", stream.index))
}

/// The audio and subtitle choices of a media source: its audio tracks, and "Off" followed by
/// its subtitle tracks. Empty for a kind the source lacks.
pub fn track_choices(source: &MediaSourceInfo) -> (Vec<Choice>, Vec<Choice>) {
    let choices = |kind| -> Vec<Choice> {
        source
            .media_streams
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| Choice {
                label: label(s),
                value: s.index,
            })
            .collect()
    };
    let audio = choices(MediaStreamType::Audio);
    let mut subtitles = choices(MediaStreamType::Subtitle);
    if !subtitles.is_empty() {
        subtitles.insert(
            0,
            Choice {
                label: "Off".to_owned(),
                value: SUBTITLES_OFF,
            },
        );
    }
    (audio, subtitles)
}
