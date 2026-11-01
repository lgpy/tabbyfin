//! A select: a box sunk into the page like a cozy text field, with the chosen value's name and
//! a caret, which opens a
//! [`PopupMenu`] of the choices with the chosen one checked. Every select in the app is one (the
//! library's toolbar, the Settings pickers, the item page's and the player's version pickers),
//! so they look alike and their lists walk with the keyboard like any menu.
//!
//! The box is painted rather than an egui widget so the page decides whether it takes egui's
//! focus: pages with a cursor of their own (the library, the item page, the player) read their
//! keys themselves, while Settings moves between its controls with Tab. It tells AccessKit it's
//! a combo box, with the chosen name as its value.

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, Id, Key, Modifiers, Response, Sense, Stroke, StrokeKind, TextFormat, Ui, Vec2, WidgetInfo,
    pos2, vec2,
};
use egui_material_icons::icons::ICON_ARROW_DROP_DOWN;

use crate::keys;
use crate::theme::{self, text};
use crate::widgets::focus;
use crate::widgets::popup::{Entry, Opened, PopupMenu};

/// The caret's size, and the room left and right of the name and caret.
const CARET: f32 = 16.;
const INSET: f32 = 12.;

/// A select under `id` over `labels` with `selected` chosen, `width` points wide and as tall as
/// the theme's inputs. A click opens its menu; when `focusable`, egui's focus can rest on it and
/// Enter or Space opens the menu from the keyboard. Returns the box's response and the choice
/// picked in the menu.
pub fn select(
    ui: &mut Ui,
    id: Id,
    width: f32,
    name: &str,
    labels: &[&str],
    selected: usize,
    focusable: bool,
) -> (Response, Option<usize>) {
    let current = labels.get(selected).copied().unwrap_or_default();
    let size = vec2(width, ui.spacing().interact_size.y);
    let response = button(ui, size, current, name, focusable);
    let by_pointer = response.is_pointer_button_down_on() || response.hovered();
    // The box keeps egui's focus while its menu is open, so Enter or Space there would also
    // click it: those are the menu's, to pick a row, not the box's, to close it.
    let open = PopupMenu::is_open(ui.ctx(), id);
    if response.clicked() && (by_pointer || !open) {
        // A click from the pointer leaves the highlight to it; Enter or Space on a focused box
        // starts it on the first row, as a menu opened from the keyboard does. That key is
        // taken here, or the menu, drawn below in the same frame, would pick its first row with
        // it.
        let opened = if by_pointer {
            Opened::Pointer
        } else {
            keys::take_exact(ui.ctx(), Modifiers::NONE, Key::Enter);
            keys::take_exact(ui.ctx(), Modifiers::NONE, Key::Space);
            Opened::Keyboard
        };
        PopupMenu::toggle(ui.ctx(), id, opened);
    }
    let entries: Vec<Entry> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| Entry::new(*label).checked(index == selected))
        .collect();
    let chosen = PopupMenu::new(id)
        .width(width.max(160.))
        .show(&response, &entries)
        .chosen;
    (response, chosen)
}

/// The select's box alone, `size` big, showing `value`; `name` is what AccessKit calls it. For
/// a page that opens and draws its own menu from it.
pub fn button(ui: &mut Ui, size: Vec2, value: &str, name: &str, focusable: bool) -> Response {
    let sense = if focusable { Sense::click() } else { Sense::CLICK };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| {
        let mut info = WidgetInfo::new(Role::ComboBox);
        info.enabled = ui.is_enabled();
        info.label = Some(name.to_owned());
        info.current_text_value = Some(value.to_owned());
        info
    });
    // Sunk like a text field: `inset` with the field's outline, which darkens under the pointer
    // as a field's does. The keyboard gets the shared ring.
    let tokens = theme::tokens_of(ui);
    let outline = if response.hovered() {
        tokens.ink_muted
    } else {
        tokens.surface_border
    };
    let radius = tokens.radius_control;
    ui.painter().rect(
        rect,
        CornerRadius::same(radius),
        tokens.inset,
        Stroke::new(1., outline),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        focus::ring(ui, rect, radius as f32, focus::Place::Outside);
    }

    // Painted rather than a `Label`, which would be a second AccessKit node with the value. A
    // value too long for the box ends in an ellipsis.
    let mut job = LayoutJob::single_section(
        value.to_owned(),
        TextFormat {
            font_id: text::font(ui, "sm"),
            color: tokens.ink,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width((rect.width() - 2. * INSET - CARET).max(0.));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(rect.left() + INSET, rect.center().y - galley.size().y / 2.),
        galley,
        tokens.ink,
    );
    ui.painter().text(
        pos2(rect.right() - INSET - CARET / 2. + 4., rect.center().y),
        Align2::CENTER_CENTER,
        ICON_ARROW_DROP_DOWN.codepoint,
        text::icon(ui, ICON_ARROW_DROP_DOWN.font_family()),
        tokens.ink_muted,
    );
    response
}
