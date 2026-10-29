//! What every dialog shares: an `egui::Modal` over the page, dimmed with the theme's `overlay`
//! as `main`'s dialogs dimmed it, a fixed width, the title, and the buttons in a row at the
//! bottom right. The dialogs themselves (`views/dialogs.rs`, `views/value_filters.rs`) draw
//! what's between and read their own keys: egui keeps the keyboard while a modal is open
//! (`keys.rs`), which is also why their buttons take no focus.

use egui::{Align, Button, Id, Layout, ModalResponse, Response, Sense, Ui};

use crate::theme;

/// The room under the title and above the buttons.
const TITLE_GAP: f32 = 8.;
const BUTTONS_GAP: f32 = 12.;

/// A dialog under `id`, `width` points wide, titled `title`, with `contents` under the title.
pub fn show<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    title: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    let backdrop = theme::tokens(&ctx.style_of(ctx.theme()).visuals).overlay;
    egui::Modal::new(id).backdrop_color(backdrop).show(ctx, |ui| {
        ui.set_width(width);
        ui.heading(title);
        ui.add_space(TITLE_GAP);
        contents(ui)
    })
}

/// The dialog's buttons, right-aligned on a row of their own: `add` adds them right to left, so
/// the one that does the dialog's work comes first and *Cancel* after it.
pub fn buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(BUTTONS_GAP);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

/// A dialog's button, which takes no focus (Space and Enter are the dialog's).
pub fn button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(enabled, Button::new(label).sense(Sense::CLICK))
}

/// *Cancel*: closes the dialog, as Esc and a click on the backdrop do.
pub fn cancel(ui: &mut Ui) {
    if button(ui, "Cancel", true).clicked() {
        ui.close();
    }
}
