//! The app's text styles beyond plain body text, as `RichText` for `ui.label` (or a `Label`
//! that truncates). Each is a size from the text table (`theme::text`, so the text-size setting
//! scales it) and a cozy ink. Where the ink differs (the player, which stays on
//! `theme::over_video`), the caller recolours it with `.color(…)`.

use egui::{RichText, Ui};

use crate::theme::{self, text};

/// A line of secondary text: what a view is waiting for, why there's nothing to show, a field's
/// label, an aside. `sm` in `ink_muted`.
pub fn muted(ui: &Ui, line: impl Into<String>) -> RichText {
    RichText::new(line)
        .font(text::font(ui, "sm"))
        .color(theme::tokens_of(ui).ink_muted)
}

/// A small note under a title or a list: a panel's status, a hint. `xs` in `ink_muted`.
pub fn note(ui: &Ui, line: impl Into<String>) -> RichText {
    RichText::new(line)
        .font(text::font(ui, "xs"))
        .color(theme::tokens_of(ui).ink_muted)
}

/// A panel's or a group's title. `base`, strong, in `ink`.
pub fn title(ui: &Ui, line: impl Into<String>) -> RichText {
    RichText::new(line)
        .font(text::font(ui, "base"))
        .color(theme::tokens_of(ui).ink)
        .strong()
}

/// A small heading over a section of a menu or a panel: `xs`, strong and in capitals, in
/// `ink_muted`.
pub fn overline(ui: &Ui, line: &str) -> RichText {
    RichText::new(line.to_uppercase())
        .font(text::font(ui, "xs"))
        .color(theme::tokens_of(ui).ink_muted)
        .strong()
}
