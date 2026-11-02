//! The pieces of the player's prompts that the countdown pill (`countdown_pill.rs`) doesn't
//! draw: a quiet pill beside it (the up-next card's *Cancel*) and the resume prompt's pill, a
//! note with a compact action inside its right end. All of them are 44 tall and fully round, so
//! the prompts that share the player's corner read as one family. Their colours are the
//! caller's, since they sit over the video, and none takes keyboard focus (the player's keys are
//! `keys.rs`'s).

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, CursorIcon, Frame, Margin, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2,
};
use egui_material_icons::MaterialIcon;

use crate::theme::text;
use crate::widgets::round_button;

/// A prompt pill's height, the countdown pill's.
pub const HEIGHT: f32 = 44.;
/// The room either side of a quiet pill's label.
const QUIET_PAD: f32 = 20.;
/// The action inside a note: its height, its room before the icon and after the label, and the
/// icon's size.
const ACTION_H: f32 = 36.;
const ACTION_PAD: [f32; 2] = [14., 16.];
const ICON: f32 = 18.;
const ICON_GAP: f32 = 8.;

/// A quiet pill's colours.
#[derive(Clone, Copy)]
pub struct Quiet {
    pub ink: Color32,
    pub outline: Color32,
    /// The fill under the pointer.
    pub hover: Color32,
}

/// A pill outlined in `outline` with its label in `ink`: an action beside a countdown pill that
/// isn't the main one.
pub fn quiet(ui: &mut Ui, label: &str, colours: Quiet) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::font(ui, "base"), colours.ink);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 2. * QUIET_PAD, HEIGHT), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    let fill = if response.hovered() {
        colours.hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same((HEIGHT / 2.) as u8),
        fill,
        Stroke::new(1., colours.outline),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2., galley, colours.ink);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A note's colours.
#[derive(Clone, Copy)]
pub struct Note {
    /// The pill's own frame: its fill, outline and shadow (the radius and margin are set here).
    pub frame: Frame,
    pub ink: Color32,
    /// The action's fill, under the pointer, and its label and icon.
    pub action: Color32,
    pub action_hover: Color32,
    pub action_ink: Color32,
}

/// A pill holding `text` and, inside its right end, a compact action with `icon` and `label`.
/// Returns the action's response; the rest of the pill takes clicks so they don't reach what's
/// behind it, but does nothing with them.
pub fn note(ui: &mut Ui, note: &str, icon: MaterialIcon, label: &str, colours: Note) -> Response {
    let inset = ((HEIGHT - ACTION_H) / 2.) as i8;
    colours
        .frame
        .corner_radius(CornerRadius::same((HEIGHT / 2.) as u8))
        .inner_margin(Margin {
            left: 20,
            right: inset,
            top: inset,
            bottom: inset,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.;
                ui.label(
                    egui::RichText::new(note)
                        .font(text::font(ui, "base"))
                        .color(colours.ink),
                );
                action(ui, icon, label, colours)
            })
            .inner
        })
        .inner
}

fn action(ui: &mut Ui, icon: MaterialIcon, label: &str, colours: Note) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::font(ui, "base"), colours.action_ink);
    let width = ACTION_PAD[0] + ICON + ICON_GAP + galley.size().x + ACTION_PAD[1];
    let (rect, response) = ui.allocate_exact_size(vec2(width, ACTION_H), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    let fill = if response.hovered() {
        colours.action_hover
    } else {
        colours.action
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same((ACTION_H / 2.) as u8), fill);
    let icon_at = pos2(rect.left() + ACTION_PAD[0] + ICON / 2., rect.center().y);
    round_button::glyph(ui, icon_at, icon, ICON, colours.action_ink);
    let text_at = pos2(
        rect.left() + ACTION_PAD[0] + ICON + ICON_GAP,
        rect.center().y - galley.size().y / 2.,
    );
    ui.painter().galley(text_at, galley, colours.action_ink);
    response.on_hover_cursor(CursorIcon::PointingHand)
}
