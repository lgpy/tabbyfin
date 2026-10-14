//! Version and track pickers, shared by the item page (before playing) and the player.

use gpui_kit::component::IndexPath;
use gpui_kit::component::select::{SelectItem, SelectState};
use gpui_kit::*;
use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

/// Subtitle picker value for "Off".
pub const SUBTITLES_OFF: i32 = -1;

/// One entry in a version or track picker.
#[derive(Clone)]
pub struct Choice {
    pub label: SharedString,
    /// Media source position for versions; Jellyfin stream index for tracks.
    pub value: i32,
}

impl SelectItem for Choice {
    type Value = i32;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &i32 {
        &self.value
    }
}

pub type ChoiceSelect = Entity<SelectState<Vec<Choice>>>;

fn label(stream: &MediaStream) -> SharedString {
    stream
        .display_title
        .clone()
        .or_else(|| stream.language.clone())
        .unwrap_or_else(|| format!("Track {}", stream.index))
        .into()
}

/// Audio and subtitle pickers for a media source, with `audio` / `subtitle` (Jellyfin stream
/// indexes, [`SUBTITLES_OFF`] for none) preselected. `None` for a kind the source lacks.
pub fn track_pickers<V: 'static>(
    source: &MediaSourceInfo,
    audio: Option<i32>,
    subtitle: Option<i32>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (Option<ChoiceSelect>, Option<ChoiceSelect>) {
    let streams = |kind| -> Vec<&MediaStream> { source.media_streams.iter().filter(|s| s.kind == kind).collect() };

    let audio_streams = streams(MediaStreamType::Audio);
    let audio = (!audio_streams.is_empty()).then(|| {
        let choices: Vec<Choice> = audio_streams
            .iter()
            .map(|s| Choice {
                label: label(s),
                value: s.index,
            })
            .collect();
        let selected = choices.iter().position(|c| Some(c.value) == audio).unwrap_or(0);
        cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx))
    });

    let subtitle_streams = streams(MediaStreamType::Subtitle);
    let subtitle = (!subtitle_streams.is_empty()).then(|| {
        let mut choices = vec![Choice {
            label: "Off".into(),
            value: SUBTITLES_OFF,
        }];
        choices.extend(subtitle_streams.iter().map(|s| Choice {
            label: label(s),
            value: s.index,
        }));
        let selected = choices.iter().position(|c| Some(c.value) == subtitle).unwrap_or(0);
        cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx))
    });
    (audio, subtitle)
}
