//! A button's contents with a material icon before the label, as `main`'s buttons had them: the
//! library toolbar's, the item page's actions, the SyncPlay panel's toggles.

use egui::text::LayoutJob;
use egui::{Align, Button, Color32, FontId, TextFormat, TextStyle, Ui, WidgetText};
use egui_material_icons::MaterialIcon;

use crate::theme::{self, text};

/// The room between the icon and the label: fixed, where spaces would be the font's width.
const GAP: f32 = 8.;

/// `icon` at `icon_size`, then `label` (none when empty) in `font`, both in `color` and centred
/// on the line's middle: left to the baseline, a large icon would sit low beside its label.
/// Kept as a `LayoutJob` so a selected button doesn't recolour it.
pub fn icon_text(icon: MaterialIcon, icon_size: f32, label: &str, font: FontId, color: Color32) -> WidgetText {
    let format = |font_id| TextFormat {
        font_id,
        color,
        valign: Align::Center,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append(icon.codepoint, 0., format(FontId::new(icon_size, icon.font_family())));
    if !label.is_empty() {
        job.append(label, GAP, format(font));
    }
    WidgetText::LayoutJob(job.into())
}

/// A button with `icon` before `label` at the text's own sizes, in the foreground colour.
pub fn with_icon(ui: &Ui, icon: MaterialIcon, label: &str) -> Button<'static> {
    let font = TextStyle::Button.resolve(ui.style());
    let icon_size = text::icon(ui, icon.font_family()).size;
    Button::new(icon_text(icon, icon_size, label, font, theme::tokens_of(ui).foreground))
}
