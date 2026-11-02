//! A value changed a step at a time: its label, optional keys, and a sunk pill of − value +. A
//! reset shows beside the pill once the value has left its default, so a setting at rest offers
//! nothing to undo. It takes no keyboard focus, as the player's keys are `keys.rs`'s, and its
//! colours are the caller's, since it may sit over the video.

use egui::accesskit::Role;
use egui::{Align2, Color32, CursorIcon, Pos2, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_ADD, ICON_REMOVE, ICON_RESTART_ALT};

use crate::theme::text;
use crate::widgets::round_button;

const MIN_HEIGHT: f32 = 36.;
const PILL_W: f32 = 128.;
const PILL_H: f32 = 30.;
/// The − and + circles, as tall as the pill, and their icons.
const BUTTON: f32 = PILL_H;
const ICON: f32 = 18.;
/// The room left of the label, which lines it up with the text of `popup::row`.
const PAD_LEFT: f32 = 8.;
const KEYS_GAP: f32 = 8.;

/// Which of a stepper's buttons was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Down,
    Up,
    Reset,
}

#[derive(Clone, Copy)]
pub struct Colours {
    pub fill: Color32,
    /// A button under the pointer.
    pub hover: Color32,
    pub ink: Color32,
    pub ink_muted: Color32,
}

/// Adds a row across `ui`'s width. `label` also names its buttons to AccessKit ("Decrease delay"),
/// `keys` are the keys that do the same, shown as a muted aside, and `moved` says the value has
/// left its default.
pub fn row(ui: &mut Ui, label: &str, keys: Option<&str>, value: &str, moved: bool, colours: Colours) -> Option<Step> {
    let font = text::font(ui, "sm");
    let height = MIN_HEIGHT.max(font.size + 16.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let y = rect.center().y;
    let painter = ui.painter();
    let name = painter.text(
        pos2(rect.left() + PAD_LEFT, y),
        Align2::LEFT_CENTER,
        label,
        font.clone(),
        colours.ink,
    );
    if let Some(keys) = keys {
        painter.text(
            pos2(name.right() + KEYS_GAP, y),
            Align2::LEFT_CENTER,
            keys,
            text::font(ui, "xs"),
            colours.ink_muted,
        );
    }
    let pill = egui::Rect::from_min_size(pos2(rect.right() - PILL_W, y - PILL_H / 2.), vec2(PILL_W, PILL_H));
    painter.rect_filled(pill, (PILL_H / 2.) as u8, colours.fill);
    painter.text(pill.center(), Align2::CENTER_CENTER, value, font, colours.ink);
    let word = label.to_lowercase();
    let button =
        |center: Pos2, icon, name: String, ink, step| pressed(ui, center, icon, &name, ink, colours).then_some(step);
    let down = button(
        pos2(pill.left() + BUTTON / 2., y),
        ICON_REMOVE,
        format!("Decrease {word}"),
        colours.ink,
        Step::Down,
    );
    let up = button(
        pos2(pill.right() - BUTTON / 2., y),
        ICON_ADD,
        format!("Increase {word}"),
        colours.ink,
        Step::Up,
    );
    let reset = moved
        .then(|| {
            button(
                pos2(pill.left() - BUTTON / 2. - 2., y),
                ICON_RESTART_ALT,
                format!("Reset {word}"),
                colours.ink_muted,
                Step::Reset,
            )
        })
        .flatten();
    down.or(up).or(reset)
}

fn pressed(ui: &Ui, center: Pos2, icon: MaterialIcon, name: &str, ink: Color32, colours: Colours) -> bool {
    let rect = egui::Rect::from_center_size(center, vec2(BUTTON, BUTTON));
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
    if response.hovered() {
        ui.painter().circle_filled(center, BUTTON / 2., colours.hover);
    }
    round_button::glyph(ui, center, icon, ICON, ink);
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}
