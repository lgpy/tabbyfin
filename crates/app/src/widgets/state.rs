//! What a page shows besides its content: a spinner while it loads, a line of text, an empty
//! state, and a failed load with *Retry*. `main` put the mascot in the spinner and the empty
//! state; the mascot needs tessellation (R3), so loaders are spinners until then.

use egui::{RichText, TextStyle, Ui};

/// A centred spinner, for a page that's still loading.
pub fn spinner(ui: &mut Ui) {
    ui.centered_and_justified(|ui| {
        ui.spinner();
    });
}

/// A centred line of muted text: "Your server has no libraries yet.", "This library has no
/// movies."
pub fn message(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new(message).color(crate::theme::tokens_of(ui).muted_foreground));
    });
}

/// A page with nothing on it. `main` had the confused mascot over the message; that comes with
/// R3.
pub fn empty(ui: &mut Ui, message: impl Into<String>) {
    self::message(ui, message);
}

/// A failed load: the message, and a *Retry* button under it. Returns whether Retry was pressed;
/// the page loads again when it was.
pub fn error(ui: &mut Ui, message: impl Into<String>) -> bool {
    let message = message.into();
    // The message and the button are one block, centred together: a justified widget would
    // centre only a single one.
    let line = ui.text_style_height(&TextStyle::Body);
    let gap = line * 0.5;
    let height = line + gap + ui.spacing().interact_size.y + 2. * ui.spacing().item_spacing.y;
    let mut retry = false;
    ui.vertical_centered(|ui| {
        ui.add_space(((ui.available_height() - height) * 0.5).max(0.));
        ui.label(RichText::new(message).color(crate::theme::tokens_of(ui).muted_foreground));
        ui.add_space(gap);
        retry = ui.button("Retry").clicked();
    });
    retry
}
