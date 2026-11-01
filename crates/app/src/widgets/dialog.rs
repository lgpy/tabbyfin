//! What every dialog shares: an `egui::Modal` over the page, the page under it dimmed with the
//! theme's plum scrim, a surface `width` points wide, a heading against its left edge (a dialog
//! isn't a centred form), and the buttons in a row at the bottom right: the main one `brand` (or
//! `danger` when it deletes something) and *Cancel* `quiet`. The dialogs themselves
//! (`views/dialogs.rs`) draw what's between and read their own keys: egui keeps the keyboard
//! while a modal is open (`keys.rs`), which is also why their buttons take no focus.

use egui::{Align, Id, Layout, ModalResponse, Response, Ui};

use crate::theme::{self, text};
use crate::widgets::button::{self, Focus, State};
use crate::widgets::surface;

/// A button's room either side of its label: wide enough for the loading spinner at its right
/// end (`button::State::Loading`) to clear a centred label.
const BUTTON_PAD: f32 = 32.;
/// The narrowest a button gets, so *OK*-sized labels still make a target.
const BUTTON_MIN: f32 = 96.;

/// A dialog under `id`, `width` points wide outside, titled `title` with `subtitle` under it,
/// and `contents` under the heading.
pub fn show<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    title: &str,
    subtitle: Option<&str>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    let tokens = theme::tokens(&ctx.style_of(ctx.theme()).visuals);
    egui::Modal::new(id)
        .backdrop_color(tokens.overlay)
        .frame(surface::frame_of(tokens))
        .show(ctx, |ui| {
            ui.set_width(width - 2. * f32::from(surface::PADDING));
            surface::heading_left(ui, title, subtitle);
            ui.add_space(surface::GAP_GROUP);
            contents(ui)
        })
}

/// The dialog's buttons, right-aligned on a row of their own: `add` adds them right to left, so
/// the one that does the dialog's work comes first and *Cancel* after it.
pub fn buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(surface::GAP_GROUP);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = surface::GAP_ITEM;
        add(ui);
    });
}

/// The dialog's main button, which does its work: `Loading` while that runs.
pub fn main(ui: &mut Ui, label: &str, state: State) -> Response {
    let width = button_width(ui, label);
    button::brand(ui, label, width, state, Focus::Skip)
}

/// [`main`] for a dialog whose work deletes something.
pub fn danger(ui: &mut Ui, label: &str, state: State) -> Response {
    let width = button_width(ui, label);
    button::danger(ui, label, width, state, Focus::Skip)
}

/// *Cancel*: closes the dialog, as Esc and a click on the backdrop do.
pub fn cancel(ui: &mut Ui) {
    let width = button_width(ui, "Cancel");
    if button::quiet(ui, "Cancel", width, State::Ready, Focus::Skip).clicked() {
        ui.close();
    }
}

/// A dialog button's width: its label and the room either side, at least [`BUTTON_MIN`].
fn button_width(ui: &Ui, label: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::font(ui, "base"), egui::Color32::PLACEHOLDER);
    (galley.size().x + 2. * BUTTON_PAD).max(BUTTON_MIN)
}
