//! A button's contents with a material icon before the label, as `main`'s buttons had them: the
//! library toolbar's, the SyncPlay panel's toggles. And the cozy look's buttons: [`brand`] for a
//! screen's main action (or [`danger`] when it deletes something), [`quiet`] for the ones beside
//! it, and [`brand_icon`] and [`quiet_icon`] for a page's row of actions (the item page's).

use egui::accesskit::Role;
use egui::text::LayoutJob;
use egui::{
    Align, Button, Color32, FontId, Rect, Response, RichText, Spinner, Stroke, TextFormat, TextStyle, Ui, Vec2,
    WidgetInfo, WidgetText, pos2, vec2,
};
use egui_material_icons::MaterialIcon;

use crate::theme::{self, text};
use crate::widgets::focus::{self, Place};
use crate::widgets::round_button;

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

/// The icon's size on [`brand_icon`] and [`quiet_icon`].
pub const ICON_SIZE: f32 = 22.;
/// [`brand_icon`]'s room each side of its icon and label.
const ICON_PADDING: f32 = 18.;

/// [`brand`] as wide as `icon` and `label` need, for a main action on a page rather than a panel
/// (the item page's Play). Clicks only: the page's keys are its own, so `ringed` draws the
/// keyboard's ring while its cursor is on it.
pub fn brand_icon(ui: &mut Ui, icon: MaterialIcon, label: &str, ringed: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let content = icon_text(icon, ICON_SIZE, label, text::font(ui, "base"), tokens.brand_foreground);
    let response = ui
        .scope(|ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
                visuals.weak_bg_fill = tokens.brand;
                visuals.bg_stroke = Stroke::NONE;
            }
            widgets.hovered.weak_bg_fill = tokens.brand_hover;
            widgets.active.weak_bg_fill = tokens.brand_hover;
            ui.spacing_mut().button_padding.x = ICON_PADDING;
            ui.add(
                Button::new(content)
                    .corner_radius(tokens.radius_control)
                    .sense(egui::Sense::CLICK)
                    .min_size(vec2(COZY_HEIGHT, COZY_HEIGHT)),
            )
        })
        .inner;
    if ringed {
        focus::ring(ui, response.rect, tokens.radius_control as f32, Place::Outside);
    }
    response
}

/// How wide [`brand_icon`] draws `label`, for a row that lays itself out before drawing.
pub fn brand_icon_width(ui: &Ui, label: &str) -> f32 {
    let label = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::font(ui, "base"), Color32::WHITE)
        .size()
        .x;
    2. * ICON_PADDING + ICON_SIZE + GAP + label
}

/// A [`quiet`] square holding an icon, [`COZY_HEIGHT`] across: outlined on the surface, the
/// icon in `ink_muted` (`ink` under the pointer). `on` makes it a toggle, which while on is
/// `inset_hover` with its outline and icon in `brand`. `icon_scale` grows the icon (a pop as a
/// toggle turns on). Clicks only, named and tooltipped by `label`; `ringed` draws the
/// keyboard's ring for a page that keeps its own cursor.
pub fn quiet_icon(
    ui: &mut Ui,
    icon: MaterialIcon,
    label: &str,
    on: Option<bool>,
    icon_scale: f32,
    ringed: bool,
) -> Response {
    let tokens = theme::tokens_of(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(COZY_HEIGHT), egui::Sense::CLICK);
    match on {
        Some(on) => response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, label)),
        None => response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label)),
    }
    let on = on == Some(true);
    let hovered = response.hovered();
    let fill = if on || hovered {
        tokens.inset_hover
    } else {
        tokens.surface
    };
    let outline = if on { tokens.brand } else { tokens.surface_border };
    ui.painter().rect(
        rect,
        tokens.radius_control,
        fill,
        Stroke::new(1., outline),
        egui::StrokeKind::Inside,
    );
    let color = if on {
        tokens.brand
    } else if hovered {
        tokens.ink
    } else {
        tokens.ink_muted
    };
    round_button::glyph(ui, rect.center(), icon, ICON_SIZE * icon_scale, color);
    if ringed {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Outside);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
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
