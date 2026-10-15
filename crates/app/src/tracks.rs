//! Version and track pickers, shared by the item page (before playing) and the player.

use gpui_kit::component::IndexPath;
use gpui_kit::component::select::{SelectEvent, SelectItem, SelectState};
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

/// Moves a picker to the next or previous choice from the keyboard, as if it were picked from
/// the menu (which only opens with a click): subscribers get the same `Confirm`. Returns false
/// at either end.
pub fn step_choice(select: &ChoiceSelect, forward: bool, window: &mut Window, cx: &mut App) -> bool {
    let current = select.read(cx).selected_index(cx).map_or(0, |ix| ix.row);
    let next = if forward { current + 1 } else { current.wrapping_sub(1) };
    if next == usize::MAX {
        return false;
    }
    select_row(select, next, true, window, cx) || {
        // Past the last choice: the select has no count to check against, so put it back.
        select_row(select, current, false, window, cx);
        false
    }
}

/// Like [`step_choice`] forward, but from the last choice back to the first (for Enter).
pub fn cycle_choice(select: &ChoiceSelect, window: &mut Window, cx: &mut App) {
    if !step_choice(select, true, window, cx) {
        select_row(select, 0, true, window, cx);
    }
}

/// Selects `row`, telling subscribers when `confirm`. Returns false if there's no such row.
fn select_row(select: &ChoiceSelect, row: usize, confirm: bool, window: &mut Window, cx: &mut App) -> bool {
    select.update(cx, |select, cx| {
        select.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
        let value = select.selected_value().copied();
        if confirm && value.is_some() {
            cx.emit(SelectEvent::Confirm(value));
        }
        value.is_some()
    })
}

fn label(stream: &MediaStream) -> SharedString {
    stream
        .display_title
        .clone()
        .or_else(|| stream.language.clone())
        .unwrap_or_else(|| format!("Track {}", stream.index))
        .into()
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
                label: "Off".into(),
                value: SUBTITLES_OFF,
            },
        );
    }
    (audio, subtitles)
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
    let mut picker = |choices: Vec<Choice>, value: Option<i32>| {
        (!choices.is_empty()).then(|| {
            let selected = choices.iter().position(|c| Some(c.value) == value).unwrap_or(0);
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx))
        })
    };
    let (audio_choices, subtitle_choices) = track_choices(source);
    (picker(audio_choices, audio), picker(subtitle_choices, subtitle))
}
