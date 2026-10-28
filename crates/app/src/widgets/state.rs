//! What a page shows besides its content: the mascot while it loads, a line of text, an empty
//! state, and a failed load with *Retry*.
//!
//! `main` had the drawn mascot (`widgets/mascot.rs`) in the loader and over the empty state's
//! message; the failed load was this page's own line and button.

use egui::{Layout, RichText, TextStyle, Ui, UiBuilder};

use crate::widgets::mascot;

/// The gap under the confused mascot, `main`'s `gap_3`.
const GAP: f32 = 12.;

/// The mascot centred, for a page that's still loading.
pub fn spinner(ui: &mut Ui) {
    centred(ui, mascot::PAGE, |ui| mascot::loader(ui, mascot::PAGE));
}

/// A centred line of muted text: "Your server has no libraries yet.", "This library has no
/// movies."
pub fn message(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new(message).color(ui.visuals().weak_text_color()));
    });
}

/// A page with nothing on it: the confused mascot over the message. It greets for a few seconds
/// and then holds still (see `widgets::mascot`), so an empty page is idle.
pub fn empty(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    let line = ui.text_style_height(&TextStyle::Body);
    centred(ui, mascot::PAGE + GAP + line, |ui| {
        mascot::confused(ui, mascot::PAGE);
        ui.add_space(GAP);
        ui.label(RichText::new(message).color(ui.visuals().weak_text_color()));
    });
}

/// A failed load: the message, and a *Retry* button under it. Returns whether Retry was pressed;
/// the page loads again when it was.
pub fn error(ui: &mut Ui, message: impl Into<String>) -> bool {
    let message = message.into();
    let line = ui.text_style_height(&TextStyle::Body);
    let gap = line * 0.5;
    // The gaps egui puts between the two widgets as well, so the block is centred by its edges.
    let height = line + gap + ui.spacing().interact_size.y + 2. * ui.spacing().item_spacing.y;
    let mut retry = false;
    centred(ui, height, |ui| {
        ui.label(RichText::new(message).color(ui.visuals().weak_text_color()));
        ui.add_space(gap);
        retry = ui.button("Retry").clicked();
    });
    retry
}

/// Draws `contents`, `height` points tall, as one block in the middle of the space the `ui` has
/// left and centred across it.
fn centred(ui: &mut Ui, height: f32, contents: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().layout(Layout::top_down(egui::Align::Center)), |ui| {
        ui.add_space(((ui.available_height() - height) * 0.5).max(0.));
        contents(ui);
    });
}
