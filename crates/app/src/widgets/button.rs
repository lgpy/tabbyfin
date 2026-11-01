//! A button's contents with a material icon before the label, as `main`'s buttons had them: the
//! library toolbar's, the item page's actions, the SyncPlay panel's toggles. And the cozy look's
//! buttons: [`brand`] for a screen's main action (or [`danger`] when it deletes something),
//! [`quiet`] for the ones beside it.

use egui::text::LayoutJob;
use egui::{
    Align, Button, Color32, FontId, Rect, Response, RichText, Spinner, Stroke, TextFormat, TextStyle, Ui, Vec2,
    WidgetText, pos2, vec2,
};
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

/// The height of the cozy look's buttons: roomier than egui's 32 points.
pub const COZY_HEIGHT: f32 = 42.;

/// Whether a cozy button takes clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Ready,
    Disabled,
    /// What it started is running: disabled, with a spinner at its right end and its label
    /// where it was.
    Loading,
}

/// Whether a cozy button is a stop for Tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// Tab reaches it and Space or Enter presses it, as on the sign-in form.
    Tab,
    /// Clicks only (`Sense::CLICK`): for a dialog, whose keys are its own (`widgets/dialog.rs`),
    /// and for any screen where egui's focus would take Space or Enter from it.
    Skip,
}

/// The cozy look's main action: a [`theme::Tokens::brand`] button `width` wide, its label
/// centred.
pub fn brand(ui: &mut Ui, label: &str, width: f32, state: State, focus: Focus) -> Response {
    let tokens = theme::tokens_of(ui);
    let colors = [tokens.brand, tokens.brand_hover, tokens.brand_foreground];
    filled(ui, label, width, state, focus, colors)
}

/// [`brand`] for a main action that deletes something, in
/// [`theme::Tokens::danger_action`].
pub fn danger(ui: &mut Ui, label: &str, width: f32, state: State, focus: Focus) -> Response {
    let tokens = theme::tokens_of(ui);
    let colors = [
        tokens.danger_action,
        tokens.danger_action_hover,
        tokens.danger_action_foreground,
    ];
    filled(ui, label, width, state, focus, colors)
}

/// A cozy button filled with `fill`, `hover` under the pointer, its label in `ink`.
fn filled(
    ui: &mut Ui,
    label: &str,
    width: f32,
    state: State,
    focus: Focus,
    [fill, hover, ink]: [Color32; 3],
) -> Response {
    let radius = theme::tokens_of(ui).radius_control;
    let label = RichText::new(label).font(text::font(ui, "base")).color(ink);
    centred(ui, width, |ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            visuals.weak_bg_fill = fill;
            visuals.bg_stroke = Stroke::NONE;
        }
        widgets.hovered.weak_bg_fill = hover;
        widgets.active.weak_bg_fill = hover;
        add(ui, cozy(label, width, radius, focus), state, ink)
    })
}

/// A cozy action that isn't the main one: outlined, on the surface, the same size as [`brand`].
pub fn quiet(ui: &mut Ui, label: &str, width: f32, state: State, focus: Focus) -> Response {
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    quiet_in(ui, label, None, width, state, focus, color)
}

/// A [`quiet`] button whose label is in [`theme::Tokens::danger`], for an action that leaves or
/// loses something but isn't the view's main one (*Leave group*).
pub fn quiet_danger(ui: &mut Ui, label: &str, width: f32, state: State, focus: Focus) -> Response {
    let danger = theme::tokens_of(ui).danger;
    quiet_in(ui, label, Some(danger), width, state, focus, danger)
}

fn quiet_in(
    ui: &mut Ui,
    label: &str,
    label_color: Option<Color32>,
    width: f32,
    state: State,
    focus: Focus,
    spinner: Color32,
) -> Response {
    let radius = theme::tokens_of(ui).radius_control;
    let mut label = RichText::new(label).font(text::font(ui, "base"));
    if let Some(color) = label_color {
        label = label.color(color);
    }
    centred(ui, width, |ui| {
        add(ui, cozy(label, width, radius, focus), state, spinner)
    })
}

/// Adds a cozy button in `state`; a loading one's spinner is in `color`, dimmed with the rest
/// of the button, at its right end so the label doesn't move.
fn add(ui: &mut Ui, button: Button<'_>, state: State, color: Color32) -> Response {
    ui.add_enabled_ui(state == State::Ready, |ui| {
        let response = ui.add(button);
        if state == State::Loading {
            let size = 18.;
            let inset = (COZY_HEIGHT - size) / 2.;
            let center = pos2(response.rect.right() - inset - size / 2., response.rect.center().y);
            Spinner::new()
                .size(size)
                .color(color)
                .paint_at(ui, Rect::from_center_size(center, Vec2::splat(size)));
        }
        response
    })
    .inner
}

/// A link-like button in the muted ink, for an aside under a panel ("Use another server").
pub fn subtle(ui: &mut Ui, label: &str) -> Response {
    let tokens = theme::tokens_of(ui);
    let label = RichText::new(label)
        .font(text::font(ui, "sm"))
        .color(tokens.ink_muted)
        .underline();
    ui.add(Button::new(label).frame(false))
}

/// The cozy look's rounder, taller button, with its own radius so a `Ui` that changed the
/// widgets' radius doesn't change it.
fn cozy(label: RichText, width: f32, radius: u8, focus: Focus) -> Button<'static> {
    let button = Button::new(label)
        .corner_radius(radius)
        .min_size(vec2(width, COZY_HEIGHT));
    match focus {
        Focus::Tab => button,
        Focus::Skip => button.sense(egui::Sense::CLICK),
    }
}

/// A button centres its contents with the layout of the `Ui` drawing it.
fn centred(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    ui.allocate_ui_with_layout(vec2(width, COZY_HEIGHT), egui::Layout::top_down(Align::Center), add)
        .inner
}
