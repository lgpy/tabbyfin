//! Audio and subtitle track choices of a media source, for the player's settings menu, and the
//! versions of an item for its page's picker.

use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

use crate::i18n::tr;

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
        .unwrap_or_else(|| tr!("player-track-number", index = stream.index))
}

/// How many tracks of `kind` `source` has.
pub fn count(source: &MediaSourceInfo, kind: MediaStreamType) -> usize {
    source.media_streams.iter().filter(|s| s.kind == kind).count()
}

/// The tracks that can be shown as a second subtitle: "Off" and the text subtitles except the one
/// shown as the first (a bitmap one can't be styled or placed at the top). `files` is the
/// subtitle streams a transcode carries, the only ones it has. Empty when there's none to pick.
pub fn secondary_choices(source: &MediaSourceInfo, files: Option<&[i32]>, primary: Option<i32>) -> Vec<Choice> {
    let mut choices: Vec<Choice> = source
        .media_streams
        .iter()
        .filter(|s| s.kind == MediaStreamType::Subtitle && s.is_text_subtitle_stream)
        .filter(|s| Some(s.index) != primary)
        .filter(|s| files.is_none_or(|files| files.contains(&s.index)))
        .map(|s| Choice {
            label: label(s),
            value: s.index,
        })
        .collect();
    if !choices.is_empty() {
        choices.insert(
            0,
            Choice {
                label: tr!("player-subtitles-off"),
                value: SUBTITLES_OFF,
            },
        );
    }
    choices
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
                label: tr!("player-subtitles-off"),
                value: SUBTITLES_OFF,
            },
        );
    }
    (audio, subtitles)
}

#[cfg(test)]
mod tests {
    use jellyfin::models::MediaSourceInfo;
    use serde_json::json;

    use super::{SUBTITLES_OFF, secondary_choices};

    fn source() -> MediaSourceInfo {
        let sub = |index: i32, text: bool| json!({ "Index": index, "Type": "Subtitle", "IsTextSubtitleStream": text });
        serde_json::from_value(json!({
            "Id": "s",
            "MediaStreams": [
                { "Index": 0, "Type": "Video" },
                sub(1, true),
                sub(2, false),
                sub(3, true),
                sub(4, true),
            ],
        }))
        .unwrap()
    }

    #[test]
    fn the_second_subtitle_is_off_or_another_text_track() {
        let values = |choices: Vec<super::Choice>| choices.iter().map(|c| c.value).collect::<Vec<_>>();
        // Bitmap tracks (2) can't be the second, and the first track can't be both.
        assert_eq!(
            values(secondary_choices(&source(), None, Some(1))),
            [SUBTITLES_OFF, 3, 4]
        );
        // A transcode has only the tracks it sends as files.
        assert_eq!(
            values(secondary_choices(&source(), Some(&[1, 4]), Some(1))),
            [SUBTITLES_OFF, 4]
        );
        // With nothing to pick there's no list at all.
        assert!(secondary_choices(&source(), Some(&[1]), Some(1)).is_empty());
    }
}
