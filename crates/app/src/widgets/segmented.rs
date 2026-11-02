//! One choice out of a few short ones, side by side on a sunk track, the chosen one a pill on it.
//! The player's speeds are one. It takes no keyboard focus, as the player's keys are `keys.rs`'s,
//! and its colours are the caller's, since it may sit over the video.

use egui::accesskit::Role;
use egui::{Color32, CursorIcon, Rect, Sense, Ui, WidgetInfo, pos2, vec2};

use crate::theme::text;

const MIN_HEIGHT: f32 = 32.;
/// The room between the chosen pill and the track.
const INSET: f32 = 2.;

#[derive(Clone, Copy)]
pub struct Colours {
    pub track: Color32,
    /// A choice under the pointer.
    pub hover: Color32,
    pub ink: Color32,
    pub chosen: Color32,
    pub chosen_ink: Color32,
}

/// Adds the choices across `ui`'s width. Each is as wide as its label plus an equal share of the
/// room left, so long labels still fit at a large text size. Returns the index clicked.
pub fn show<S: AsRef<str>>(ui: &mut Ui, labels: &[S], current: Option<usize>, colours: Colours) -> Option<usize> {
    let font = text::font(ui, "sm");
    let height = MIN_HEIGHT.max(font.size + 12.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let radius = (height / 2.) as u8;
    ui.painter().rect_filled(rect, radius, colours.track);
    let galleys: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let color = if current == Some(index) {
                colours.chosen_ink
            } else {
                colours.ink
            };
            (
                ui.painter()
                    .layout_no_wrap(label.as_ref().to_owned(), font.clone(), color),
                color,
            )
        })
        .collect();
    let used: f32 = galleys.iter().map(|(galley, _)| galley.size().x).sum();
    let share = (rect.width() - used).max(0.) / labels.len() as f32;
    let mut chosen = None;
    let mut left = rect.left();
    for (index, (galley, color)) in galleys.into_iter().enumerate() {
        let cell = Rect::from_min_size(pos2(left, rect.top()), vec2(galley.size().x + share, height));
        left = cell.right();
        let response = ui.interact(cell, ui.scope_id().with(("segment", index)), Sense::CLICK);
        let selected = current == Some(index);
        let name = labels[index].as_ref();
        response.widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), selected, name));
        let pill = cell.shrink(INSET);
        let pill_radius = radius.saturating_sub(INSET as u8);
        if selected {
            ui.painter().rect_filled(pill, pill_radius, colours.chosen);
        } else if response.hovered() {
            ui.painter().rect_filled(pill, pill_radius, colours.hover);
        }
        ui.painter().galley(cell.center() - galley.size() / 2., galley, color);
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            chosen = Some(index);
        }
    }
    chosen
}
