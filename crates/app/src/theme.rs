//! The app's look: the theme tokens, dark, light or the system's theme, text size, zoom,
//! animations, and the colours over video (`over_video`).
//!
//! The tokens ([`Tokens`]) are `main`'s: gpui-kit 0.7.3's default light and dark themes with the
//! overrides `main`'s `theme.rs` made on top (radius 10 and 12, and the light theme's
//! `muted_foreground` of `0x525252`). They're the one place colours, radii and spacing come
//! from; [`apply`] builds egui's [`egui::Visuals`] and [`egui::Spacing`] out of them, and
//! widgets that paint themselves read [`tokens`] instead of literals. The player is the
//! exception: it stays dark whatever the theme, and draws with [`over_video`].
//!
//! The text-style table ([`text`]) is `main`'s gpui scale (`text_xs` … `text_2xl`) rather than
//! egui's: it's what [`apply_text_size`] scales, and what widgets name instead of adding steps to
//! egui's body size.
//!
//! On `main` this was `theme.rs`, `zoom.rs` and `motion.rs`. The zoom keys live here now, with
//! egui in charge of the theme it draws in: on Windows and macOS `ctx.set_theme` with
//! [`egui::ThemePreference::System`] follows the system's light or dark appearance as it changes.
//! On Linux egui gets no system theme from winit, so [`SystemTheme`] reads the desktop's from the
//! XDG portal and follows the portal's changes (as `main` did).

use std::sync::Mutex;

use egui::{Color32, FontId, Stroke, TextStyle};

use crate::config::{Config, ThemePreference};
use crate::task::Task;

/// Text sizes the settings offer, in percent.
pub const TEXT_SIZES: [u16; 5] = [90, 100, 115, 130, 150];

/// What "the system's theme" is: egui's own support where winit reports the system theme, and
/// on Linux light, which is what `main` showed when the desktop said nothing, until the
/// [`SystemTheme`] the app keeps alongside sets the desktop's.
#[cfg(not(target_os = "linux"))]
const SYSTEM_PREFERENCE: egui::ThemePreference = egui::ThemePreference::System;
#[cfg(target_os = "linux")]
const SYSTEM_PREFERENCE: egui::ThemePreference = egui::ThemePreference::Light;

/// The zoom levels the zoom keys step through, as on `main`, 1.0 being 100 %.
const ZOOM_LEVELS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.];

/// Applies the config's appearance: theme, zoom, text size and animations. Called through
/// `App::apply_appearance`, at launch and when Settings changes one of them, which also puts the
/// desktop's colour scheme back on Linux.
pub fn apply(ctx: &egui::Context, config: &Config) {
    ctx.set_theme(match config.theme {
        ThemePreference::System => SYSTEM_PREFERENCE,
        ThemePreference::Light => egui::ThemePreference::Light,
        ThemePreference::Dark => egui::ThemePreference::Dark,
    });
    // egui keeps a style per theme; both get the app's look, so switching between them (or
    // following the desktop's) never shows egui's own.
    ctx.set_visuals_of(egui::Theme::Light, visuals(&Tokens::LIGHT));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&Tokens::DARK));
    // The app's zoom keys (`keys.rs`) step through [`ZOOM_LEVELS`] and save the choice; egui's
    // own would step by 10 % and forget it between launches.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    ctx.set_zoom_factor(config.ui_zoom.map_or(1., snap));
    let spacing = spacing();
    // No selectable text: `main` had none, and a card's title sits over the card's own click
    // target, where a selectable label would take its clicks, right-clicks and drags (and show
    // an I-beam). A truncated label still shows its full text on hover.
    ctx.all_styles_mut(|style| {
        style.interaction.selectable_labels = false;
        style.spacing = spacing.clone();
    });
    // A press that moves further than this is a drag, not a click: `main`'s shelves used 4 px.
    // (A shelf still scrolls along from the first pixel; only the click is withheld.)
    ctx.options_mut(|options| options.input_options.max_click_dist = 4.);
    apply_text_size(ctx, config.text_size);
    apply_animations(ctx, config.animations.unwrap_or(true));
}

/// The zoom a config asks for, snapped to a level.
pub fn zoom(config: &Config) -> f32 {
    config.ui_zoom.map_or(1., snap)
}

/// One step up (`1`) or down (`-1`) the zoom levels from `current`, saved as `ui_zoom` for the
/// next launch. Returns the zoom in effect, which is `current` at either end of the levels.
///
/// `current` is the caller's, because `ctx.zoom_factor` only changes at the start of the next
/// pass: two keys in one frame would otherwise both step from the same level.
pub fn step_zoom(ctx: &egui::Context, current: f32, direction: i32) -> f32 {
    let index = ZOOM_LEVELS.iter().position(|level| *level == current).unwrap_or(4) as i32;
    let next = (index + direction).clamp(0, ZOOM_LEVELS.len() as i32 - 1) as usize;
    choose_zoom(ctx, current, ZOOM_LEVELS[next])
}

/// Back to 100 %, from `current` (see [`step_zoom`]).
pub fn reset_zoom(ctx: &egui::Context, current: f32) -> f32 {
    choose_zoom(ctx, current, 1.)
}

/// Sets the UI zoom, which scales everything egui draws, the layout with it, and remembers it.
fn choose_zoom(ctx: &egui::Context, current: f32, zoom: f32) -> f32 {
    if current == zoom {
        return zoom;
    }
    ctx.set_zoom_factor(zoom);
    let mut config = Config::load();
    config.ui_zoom = (zoom != 1.).then_some(zoom);
    if let Err(err) = config.save() {
        tracing::warn!("couldn't save the zoom: {err}");
    }
    zoom
}

/// The level nearest a saved zoom, so stepping from it lands on the levels.
fn snap(zoom: f32) -> f32 {
    ZOOM_LEVELS
        .into_iter()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.)
}

/// The text size factor for a setting: the chosen percentage, or the system's.
pub fn text_scale(setting: Option<u16>) -> f32 {
    let scale = match setting {
        Some(percent) => f32::from(percent) / 100.,
        None => system_text_scale(),
    };
    scale.clamp(0.75, 2.)
}

/// The app's text styles, `main`'s gpui scale in points at 100 %: `text_xs` (12), `text_sm`
/// (14), `text_base` (16), `text_lg` (18), `text_xl` (20) and `text_2xl` (24). They're installed
/// into [`egui::Style::text_styles`] as the named styles [`NAMED`] (`"xs"` … `"2xl"`), alongside
/// egui's own (`Small` is `xs`, `Body` and `Button` are `sm`, `Heading` is `lg`), so a widget
/// names the size it wants instead of adding points to the body's. [`apply_text_size`] scales
/// every one of them.
///
/// The families are egui's bundled ones (proportional and monospace) and the material-icons
/// font: `main` drew with the system UI font, which egui doesn't have without bundling a copy.
///
/// Text that `main` drew at a fixed `dp` size stays fixed: the item page's 40-point title, the
/// letter bar's 11, the action icons' 24. The setting makes text bigger, not the layout.
pub mod text {
    use egui::{FontFamily, FontId, TextStyle, Ui};

    /// `text_xs`: card subtitles, tags, the smallest hints.
    pub const XS: f32 = 12.;
    /// `text_sm`: card titles, menu and button labels, the muted lines under a page's title.
    pub const SM: f32 = 14.;
    /// `text_base`: an episode card's title, the synopsis, inline icons.
    pub const BASE: f32 = 16.;
    /// `text_lg`: shelf and page headings.
    pub const LG: f32 = 18.;
    /// `text_xl`.
    pub const XL: f32 = 20.;
    /// `text_2xl`: the sign-in title, a person card's initials.
    pub const XXL: f32 = 24.;
    /// gpui-kit's monospace size.
    pub const MONO: f32 = 13.;

    /// The named styles the table installs, with their 100 % sizes.
    pub const NAMED: [(&str, f32); 6] = [
        ("xs", XS),
        ("sm", SM),
        ("base", BASE),
        ("lg", LG),
        ("xl", XL),
        ("2xl", XXL),
    ];

    /// The named style for one of [`NAMED`]'s sizes.
    pub fn named(name: &str) -> TextStyle {
        TextStyle::Name(name.into())
    }

    /// The font `name` (a [`NAMED`] entry) resolves to under `ui`'s style, scaled by the text
    /// size. Falls back to the body font if the table isn't installed (a bare `egui::Context` in
    /// a unit test).
    pub fn font(ui: &Ui, name: &str) -> FontId {
        font_of(ui.style(), name)
    }

    /// [`font`] for something with only the [`egui::Style`] at hand (a `Ui`'s painter, a
    /// `LayoutJob`).
    pub fn font_of(style: &egui::Style, name: &str) -> FontId {
        style
            .text_styles
            .get(&named(name))
            .cloned()
            .unwrap_or_else(|| TextStyle::Body.resolve(style))
    }

    /// An icon in a line of text, in `family` (the material-icons font): at `text_base`, as
    /// `main`'s inline icons took the text size they sat in.
    pub fn icon(ui: &Ui, family: FontFamily) -> FontId {
        let mut font = font(ui, "base");
        font.family = family;
        font
    }
}

/// Scales the app's text-style table ([`text`]), which sizes text and the controls drawn around
/// it. Fixed layout sizes don't follow, as on `main`: the setting makes text bigger, not the
/// window's chrome.
fn apply_text_size(ctx: &egui::Context, setting: Option<u16>) {
    let scale = text_scale(setting);
    ctx.all_styles_mut(|style| {
        // From the 100 % sizes every time, so applying this again doesn't compound.
        let scaled = |size: f32| FontId::new(size * scale, egui::FontFamily::Proportional);
        style.text_styles.insert(TextStyle::Small, scaled(text::XS));
        style.text_styles.insert(TextStyle::Body, scaled(text::SM));
        style.text_styles.insert(TextStyle::Button, scaled(text::SM));
        style.text_styles.insert(TextStyle::Heading, scaled(text::LG));
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(text::MONO * scale, egui::FontFamily::Monospace),
        );
        for (name, size) in text::NAMED {
            style.text_styles.insert(text::named(name), scaled(size));
        }
    });
}

/// The app's colours, radii and spacing: `main`'s gpui-kit theme. One set per theme; [`apply`]
/// turns them into egui's [`egui::Visuals`], and widgets that paint themselves ask [`tokens`]
/// for the rest (the success green of a watched tick, the accent of a highlighted row).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,

    /// The page and top bar's background.
    pub background: Color32,
    /// Normal text.
    pub foreground: Color32,
    /// A box or chip a little apart from the background.
    pub muted: Color32,
    /// Secondary text: card subtitles, hints, a row's description.
    pub muted_foreground: Color32,
    /// Separators and outlines.
    pub border: Color32,
    /// The border of an input.
    pub input: Color32,

    /// A button at rest, and under the pointer and pressed.
    pub button: Color32,
    pub button_hover: Color32,
    pub button_active: Color32,

    /// The app's main colour: primary buttons and a card's progress fill (egui's own checked
    /// checkboxes draw their box from [`Self::secondary`] and their tick from the foreground).
    pub primary: Color32,
    pub primary_foreground: Color32,
    pub primary_hover: Color32,

    /// The second colour: egui's checkbox boxes and slider rails, and them pressed.
    pub secondary: Color32,
    pub secondary_active: Color32,

    /// A row's highlight: menus, toggles, lists.
    pub accent: Color32,
    pub accent_foreground: Color32,

    /// Text selected with the pointer, in a field.
    pub selection: Color32,

    /// Errors: a failed toast, an error line.
    pub danger: Color32,
    /// A watched tick.
    pub success: Color32,
    pub warning: Color32,
    pub info: Color32,
    /// gpui-kit's base red, which is the favorite heart (`danger` is the error red).
    pub red: Color32,

    /// Menus, popups and dialogs.
    pub popover: Color32,
    /// A dialog's scrim, which `main`'s dialogs and sheets drew the page under.
    pub overlay: Color32,

    /// The radius of controls (`main`'s theme override of gpui-kit's 6).
    pub radius: u8,
    /// The radius of cards, dialogs and popups (`main`'s override of 8).
    pub radius_lg: u8,
}

impl Tokens {
    /// gpui-kit's *Default Light* with `main`'s overrides.
    pub const LIGHT: Self = Self {
        dark: false,
        background: Color32::from_rgb(0xff, 0xff, 0xff),
        foreground: Color32::from_rgb(0x0a, 0x0a, 0x0a),
        muted: Color32::from_rgb(0xf5, 0xf5, 0xf5),
        // gpui-kit's neutral-500 is under the 4.5:1 WCAG AA asks of small text on its grey
        // fills; one step darker (neutral-600) passes, as `main` had it.
        muted_foreground: Color32::from_rgb(0x52, 0x52, 0x52),
        border: Color32::from_rgb(0xe5, 0xe5, 0xe5),
        input: Color32::from_rgb(0xe5, 0xe5, 0xe5),
        button: Color32::from_rgb(0xff, 0xff, 0xff),
        button_hover: Color32::from_rgb(0xf2, 0xf2, 0xf2),
        button_active: Color32::from_rgb(0xec, 0xec, 0xec),
        primary: Color32::from_rgb(0x17, 0x17, 0x17),
        primary_foreground: Color32::from_rgb(0xfa, 0xfa, 0xfa),
        // `background.blend(primary)` at 90 %.
        primary_hover: Color32::from_rgb(0x2e, 0x2e, 0x2e),
        secondary: Color32::from_rgb(0xe5, 0xe5, 0xe5),
        secondary_active: Color32::from_rgb(0xd4, 0xd4, 0xd4),
        accent: Color32::from_rgb(0xf5, 0xf5, 0xf5),
        accent_foreground: Color32::from_rgb(0x17, 0x17, 0x17),
        selection: Color32::from_rgb(0x55, 0xa0, 0xfc),
        danger: Color32::from_rgb(0xef, 0x44, 0x44),
        success: Color32::from_rgb(0x22, 0xc5, 0x5e),
        warning: Color32::from_rgb(0xea, 0xb3, 0x08),
        info: Color32::from_rgb(0x06, 0xb6, 0xd4),
        red: Color32::from_rgb(0xdc, 0x26, 0x26),
        popover: Color32::from_rgb(0xff, 0xff, 0xff),
        overlay: Color32::from_rgba_premultiplied(0, 0, 0, 13),
        radius: 10,
        radius_lg: 12,
    };

    /// gpui-kit's *Default Dark* with `main`'s overrides.
    pub const DARK: Self = Self {
        dark: true,
        background: Color32::from_rgb(0x0a, 0x0a, 0x0a),
        foreground: Color32::from_rgb(0xfa, 0xfa, 0xfa),
        muted: Color32::from_rgb(0x26, 0x26, 0x26),
        muted_foreground: Color32::from_rgb(0xa3, 0xa3, 0xa3),
        border: Color32::from_rgb(0x26, 0x26, 0x26),
        input: Color32::from_rgb(0x2f, 0x2f, 0x2f),
        // gpui-kit blends the input's colour (`#2f2f2f`) into the background at 30, 50 and
        // 70 %; these are the results over `background`.
        button: Color32::from_rgb(0x15, 0x15, 0x15),
        button_hover: Color32::from_rgb(0x1d, 0x1d, 0x1d),
        button_active: Color32::from_rgb(0x24, 0x24, 0x24),
        primary: Color32::from_rgb(0xfa, 0xfa, 0xfa),
        primary_foreground: Color32::from_rgb(0x17, 0x17, 0x17),
        primary_hover: Color32::from_rgb(0xe2, 0xe2, 0xe2),
        secondary: Color32::from_rgb(0x26, 0x26, 0x26),
        secondary_active: Color32::from_rgb(0x21, 0x21, 0x21),
        accent: Color32::from_rgb(0x26, 0x26, 0x26),
        accent_foreground: Color32::from_rgb(0xfa, 0xfa, 0xfa),
        selection: Color32::from_rgb(0x1d, 0x4e, 0xd8),
        danger: Color32::from_rgb(0xf8, 0x71, 0x71),
        success: Color32::from_rgb(0x4a, 0xde, 0x80),
        warning: Color32::from_rgb(0xfa, 0xcc, 0x15),
        info: Color32::from_rgb(0x22, 0xd3, 0xee),
        red: Color32::from_rgb(0xf8, 0x71, 0x71),
        popover: Color32::from_rgb(0x0a, 0x0a, 0x0a),
        overlay: Color32::from_rgba_premultiplied(0, 0, 0, 51),
        radius: 10,
        radius_lg: 12,
    };
}

/// The tokens the theme `visuals` draws in: the light or dark set, by its `dark_mode`. The player
/// forces [`egui::Visuals::dark`] over the picture, so its widgets get the dark tokens too.
pub fn tokens(visuals: &egui::Visuals) -> &'static Tokens {
    if visuals.dark_mode {
        &Tokens::DARK
    } else {
        &Tokens::LIGHT
    }
}

/// [`tokens`] for a `Ui`'s visuals.
pub fn tokens_of(ui: &egui::Ui) -> &'static Tokens {
    tokens(ui.visuals())
}

/// The widget style a primary button draws with, as `main`'s `.primary()` buttons had it:
/// `primary` behind `primary_foreground` text, and `primary_hover` under the pointer or pressed.
/// Apply it in a [`egui::Ui::scope`] around the button.
pub fn primary_style(ui: &mut egui::Ui) {
    let tokens = tokens_of(ui);
    let widgets = &mut ui.visuals_mut().widgets;
    for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
        visuals.weak_bg_fill = tokens.primary;
        visuals.bg_stroke = Stroke::NONE;
        visuals.fg_stroke.color = tokens.primary_foreground;
    }
    widgets.hovered.weak_bg_fill = tokens.primary_hover;
    widgets.active.weak_bg_fill = tokens.primary_hover;
}

/// A primary button of `width` with its label centred, drawn with [`primary_style`] (so it takes
/// `primary_hover` under the pointer and pressed) and disabled while `enabled` is false.
pub fn primary_button(ui: &mut egui::Ui, label: &str, width: f32, enabled: bool) -> egui::Response {
    let colour = tokens_of(ui).primary_foreground;
    let button = egui::Button::new(egui::RichText::new(label).color(colour)).min_size(egui::vec2(width, 0.));
    // A button centres its contents with the layout of the `Ui` drawing it.
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            primary_style(ui);
            ui.add_enabled(enabled, button)
        },
    )
    .inner
}

/// A row in a picker's list, highlighted with `accent` when it's the chosen one: egui's own
/// `selectable_label` paints the text selection's blue.
pub fn selectable_label(ui: &mut egui::Ui, on: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let button = egui::Button::new(text).frame_when_inactive(on);
    ui.add(selected(ui, on, button))
}

/// `main`'s fill for a selected control — a menu row, a toggle, a checked box — which egui would
/// paint in the text selection's colour. The button keeps its `selected` state (AccessKit's
/// selected/toggled flag) and is filled with `accent` when it's on.
pub fn selected<'a>(ui: &egui::Ui, selected: bool, button: egui::Button<'a>) -> egui::Button<'a> {
    let button = button.selected(selected);
    if selected {
        button.fill(tokens_of(ui).accent)
    } else {
        button
    }
}

/// gpui-kit's spacing scale (`main`'s `gap_1`, `p_2`, `px_6`, …), in points. It's the same in
/// both themes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

/// The app's spacing tokens.
pub const SPACE: Space = Space {
    xxs: 2.,
    xs: 4.,
    sm: 8.,
    md: 12.,
    lg: 16.,
    xl: 24.,
    xxl: 32.,
};

/// egui's [`egui::Visuals`] from `tokens`: what egui's own widgets (buttons, checkboxes, text
/// fields, menus, scroll areas, separators) draw with.
fn visuals(tokens: &Tokens) -> egui::Visuals {
    let mut visuals = if tokens.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let corner_radius = egui::CornerRadius::same(tokens.radius);
    visuals.dark_mode = tokens.dark;

    visuals.override_text_color = None;
    // A hint or a card's subtitle: `main`'s muted_foreground, not a dimmed body colour.
    visuals.weak_text_color = Some(tokens.muted_foreground);
    visuals.panel_fill = tokens.background;
    visuals.window_fill = tokens.popover;
    visuals.window_stroke = Stroke::new(1., tokens.border);
    visuals.window_corner_radius = egui::CornerRadius::same(tokens.radius_lg);
    // `main`'s popups were rounded like a dialog (`rounded_lg`).
    visuals.menu_corner_radius = egui::CornerRadius::same(tokens.radius_lg);
    // A text field is the background with an `input` outline, as gpui-kit's inputs are.
    visuals.extreme_bg_color = tokens.background;
    visuals.text_edit_bg_color = Some(tokens.background);
    visuals.faint_bg_color = tokens.muted;
    visuals.code_bg_color = tokens.muted;
    visuals.hyperlink_color = tokens.foreground;
    visuals.warn_fg_color = tokens.warning;
    visuals.error_fg_color = tokens.danger;
    visuals.selection.bg_fill = tokens.selection;
    // Selected text keeps the text colour; the ring around a selected control is the
    // foreground, as `main`'s selection ring was.
    visuals.selection.stroke = Stroke::new(1., tokens.foreground);
    visuals.interact_cursor = None;

    visuals.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: tokens.background,
        weak_bg_fill: tokens.background,
        bg_stroke: Stroke::new(1., tokens.border),
        fg_stroke: Stroke::new(1., tokens.foreground),
        corner_radius,
        expansion: 0.,
    };
    visuals.widgets.inactive = egui::style::WidgetVisuals {
        // Checked boxes and sliders' rails.
        bg_fill: tokens.secondary,
        // A button or a menu row at rest.
        weak_bg_fill: tokens.button,
        bg_stroke: Stroke::new(1., tokens.input),
        fg_stroke: Stroke::new(1., tokens.foreground),
        corner_radius,
        expansion: 0.,
    };
    visuals.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: tokens.secondary,
        weak_bg_fill: tokens.button_hover,
        bg_stroke: Stroke::new(1., tokens.input),
        fg_stroke: Stroke::new(1., tokens.foreground),
        corner_radius,
        expansion: 0.,
    };
    visuals.widgets.active = egui::style::WidgetVisuals {
        // A checkbox or slider held down.
        bg_fill: tokens.secondary_active,
        weak_bg_fill: tokens.button_active,
        bg_stroke: Stroke::new(1., tokens.border),
        // What `strong_text_color` returns: the cursor's ring and a played badge used it for the
        // foreground.
        fg_stroke: Stroke::new(1., tokens.foreground),
        corner_radius,
        expansion: 0.,
    };
    visuals.widgets.open = egui::style::WidgetVisuals {
        bg_fill: tokens.accent,
        weak_bg_fill: tokens.accent,
        bg_stroke: Stroke::new(1., tokens.border),
        fg_stroke: Stroke::new(1., tokens.foreground),
        corner_radius,
        expansion: 0.,
    };
    visuals
}

/// egui's [`egui::Spacing`] from [`SPACE`] and the tokens' radii: the room egui's own widgets
/// leave around themselves, in `main`'s sizes (a 32-point button, 10 points beside its label).
fn spacing() -> egui::Spacing {
    let mut spacing = egui::Spacing {
        item_spacing: egui::vec2(SPACE.sm, SPACE.xs),
        button_padding: egui::vec2(10., SPACE.xs),
        menu_margin: egui::Margin::same(SPACE.xs as i8),
        window_margin: egui::Margin::same(SPACE.sm as i8),
        // `main`'s buttons and text fields are 32 points tall.
        interact_size: egui::vec2(40., 32.),
        icon_width: 16.,
        icon_width_inner: 8.,
        icon_spacing: SPACE.xs,
        indent: SPACE.lg,
        combo_width: 120.,
        text_edit_width: 280.,
        tooltip_width: 400.,
        ..Default::default()
    };
    // `main`'s scrollbar: 6 points, just inside the edge.
    spacing.scroll.bar_width = 6.;
    spacing.scroll.bar_inner_margin = 2.;
    spacing.scroll.bar_outer_margin = 0.;
    spacing
}

/// Off makes every widget change instant, scrolling included, as GPUI's reduce-motion flag did
/// for `main`.
fn apply_animations(ctx: &egui::Context, on: bool) {
    let animation_time = if on { egui::Style::default().animation_time } else { 0. };
    let scroll = if on {
        egui::style::ScrollAnimation::default()
    } else {
        egui::style::ScrollAnimation::none()
    };
    ctx.all_styles_mut(|style| {
        style.animation_time = animation_time;
        style.scroll_animation = scroll;
    });
}

/// The desktop's colour scheme, from the XDG desktop portal's settings: what "follow the
/// system" means on Linux, where winit reports no system theme (its X11 backend has none, its
/// Wayland one only knows what the app set). The same source `main` used.
#[cfg(target_os = "linux")]
pub struct SystemTheme {
    /// The watch. Dropping it stops the listening, which is what happens when the preference
    /// stops following the system or the app exits.
    _watch: Task<()>,
    updates: crate::task::Receiver<egui::Theme>,
    /// What the portal said last, to set again after [`apply`] (which knows only light).
    latest: Option<egui::Theme>,
}

#[cfg(target_os = "linux")]
impl SystemTheme {
    /// Starts watching the desktop's colour scheme, off the UI thread.
    pub fn start(ctx: &egui::Context) -> Self {
        let (sender, updates) = crate::task::channel(ctx);
        let watch = Task::spawn(
            ctx,
            watch_colour_scheme(sender, PORTAL_BUS, PORTAL_PATH, PORTAL_SETTINGS),
        );
        Self {
            _watch: watch,
            updates,
            latest: None,
        }
    }

    /// Applies the scheme the portal reported since the last frame. The app keeps this only
    /// while the preference is the system's, so it never overrides a chosen theme.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some(theme) = self.updates.drain().last() {
            tracing::info!(?theme, "the desktop's colour scheme");
            self.latest = Some(theme);
            ctx.set_theme(theme);
        }
    }

    /// Sets the desktop's scheme again, after [`apply`] set its light stand-in.
    pub fn reapply(&self, ctx: &egui::Context) {
        if let Some(theme) = self.latest {
            ctx.set_theme(theme);
        }
    }
}

/// Nothing to watch: egui follows the system theme itself where winit reports it.
#[cfg(not(target_os = "linux"))]
pub struct SystemTheme;

#[cfg(not(target_os = "linux"))]
impl SystemTheme {
    pub fn start(_ctx: &egui::Context) -> Self {
        Self
    }

    pub fn tick(&mut self, _ctx: &egui::Context) {}

    pub fn reapply(&self, _ctx: &egui::Context) {}
}

/// Whether the app has to follow the desktop's colour scheme itself, which is Linux (and there
/// only while the theme preference is the system's).
#[cfg(target_os = "linux")]
pub fn follows_the_desktop(config: &Config) -> bool {
    config.theme == ThemePreference::System
}

#[cfg(not(target_os = "linux"))]
pub fn follows_the_desktop(_config: &Config) -> bool {
    false
}

#[cfg(target_os = "linux")]
const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
#[cfg(target_os = "linux")]
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
#[cfg(target_os = "linux")]
const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
/// The desktop's appearance, whose `color-scheme` is the light or dark preference.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const APPEARANCE: &str = "org.freedesktop.appearance";
/// The key in [`APPEARANCE`] that says which theme the desktop prefers.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const COLOR_SCHEME: &str = "color-scheme";

/// The theme a portal colour scheme asks for: 1 is dark, 2 is light. 0 (no preference), a
/// missing portal and anything else are light, as `main` showed without a signal.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn scheme_theme(scheme: u32) -> egui::Theme {
    match scheme {
        1 => egui::Theme::Dark,
        _ => egui::Theme::Light,
    }
}

/// Reads the desktop's colour scheme and then follows the portal's changes, until the receiver
/// is gone (the app dropped the watch) or the connection does. A portal that can't be read
/// (light stays) is still followed, in case it says something later.
#[cfg(target_os = "linux")]
async fn watch_colour_scheme(sender: crate::task::Sender<egui::Theme>, bus: &str, path: &str, interface: &str) {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!("no session bus, so no desktop colour scheme to follow: {err}");
            return;
        }
    };
    let proxy = match zbus::Proxy::new(&connection, bus, path, interface).await {
        Ok(proxy) => proxy,
        Err(err) => {
            tracing::info!("no desktop portal settings to follow: {err}");
            return;
        }
    };
    // Listening before reading, so a change between the two isn't missed.
    let changes = proxy
        .receive_signal("SettingChanged")
        .await
        .map_err(|err| tracing::info!("couldn't follow the desktop colour scheme's changes: {err}"))
        .ok();
    if let Some(theme) = read_colour_scheme(&proxy).await
        && sender.send(theme).is_err()
    {
        return;
    }
    let Some(mut changes) = changes else {
        return;
    };
    use futures_util::StreamExt as _;
    while let Some(message) = changes.next().await {
        let Some(theme) = colour_scheme_change(&message) else {
            continue;
        };
        if sender.send(theme).is_err() {
            break;
        }
    }
}

/// The desktop's colour scheme as the portal has it now. `ReadOne` arrived in
/// xdg-desktop-portal 1.15; older ones (Ubuntu 22.04 has 1.14) only have `Read`, whose value
/// comes wrapped in one more variant.
#[cfg(target_os = "linux")]
async fn read_colour_scheme(proxy: &zbus::Proxy<'_>) -> Option<egui::Theme> {
    let args = &(APPEARANCE, COLOR_SCHEME);
    let value: zbus::zvariant::OwnedValue = match proxy.call("ReadOne", args).await {
        Ok(value) => value,
        Err(_) => proxy
            .call("Read", args)
            .await
            .map_err(|err| tracing::info!("couldn't read the desktop's colour scheme: {err}"))
            .ok()?,
    };
    Some(scheme_theme(scheme_value(&value)))
}

/// The colour scheme in a portal `SettingChanged` signal, if it's about the appearance.
#[cfg(target_os = "linux")]
fn colour_scheme_change(message: &zbus::Message) -> Option<egui::Theme> {
    let (namespace, key, value): (String, String, zbus::zvariant::OwnedValue) = message.body().deserialize().ok()?;
    (namespace == APPEARANCE && key == COLOR_SCHEME).then(|| scheme_theme(scheme_value(&value)))
}

/// The `u32` in a portal settings value, unwrapping the variant `Read` adds; 0 (no preference)
/// when it's another type.
#[cfg(target_os = "linux")]
fn scheme_value(value: &zbus::zvariant::Value) -> u32 {
    match value {
        zbus::zvariant::Value::U32(scheme) => *scheme,
        zbus::zvariant::Value::Value(inner) => scheme_value(inner),
        _ => 0,
    }
}

/// The system's text size as last read.
static SYSTEM_SCALE: Mutex<Option<f32>> = Mutex::new(None);

/// The system's text size: Windows' *Accessibility → Text size*, GNOME's text scaling factor
/// (also set by its *Large Text* switch), KDE's general font size against its default 10 pt;
/// 1.0 elsewhere, before the first read or if it can't be read. Reading it starts processes, so
/// the read itself happens in [`read_system_text_scale`], off the UI thread.
pub fn system_text_scale() -> f32 {
    SYSTEM_SCALE.lock().unwrap().unwrap_or(1.)
}

/// Reads the system text size again without blocking the UI thread (`reg`, `gsettings` and
/// `kdeglobals` are slow). Systems don't tell apps about a change, so this runs at launch and
/// when the window gets the focus back; pass the result to [`note_system_text_scale`].
pub fn refresh_system_text_scale(ctx: &egui::Context) -> Task<f32> {
    Task::spawn(ctx, async {
        tokio::task::spawn_blocking(read_valid_system_text_scale)
            .await
            .unwrap_or(1.)
    })
}

/// Whether the system text size has been read yet.
pub fn system_text_scale_read() -> bool {
    SYSTEM_SCALE.lock().unwrap().is_some()
}

/// Remembers a freshly read system text size, and says whether it changed (from 100 %, which is
/// what was in use before the first read).
pub fn note_system_text_scale(scale: f32) -> bool {
    SYSTEM_SCALE.lock().unwrap().replace(scale).unwrap_or(1.) != scale
}

fn read_valid_system_text_scale() -> f32 {
    read_system_text_scale()
        .filter(|s| s.is_finite() && *s > 0.)
        .unwrap_or(1.)
}

#[cfg(windows)]
fn read_system_text_scale() -> Option<f32> {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW: no console flashing up.
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Accessibility",
            "/v",
            "TextScaleFactor",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    parse_reg_dword(&String::from_utf8_lossy(&output.stdout)).map(|percent| percent as f32 / 100.)
}

#[cfg(target_os = "linux")]
fn read_system_text_scale() -> Option<f32> {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.split(':').any(|d| d == "KDE"));
    if kde {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
        let kdeglobals = std::fs::read_to_string(config.join("kdeglobals")).ok();
        // No `font` entry: KDE's default size, so 100%.
        return Some(
            kdeglobals
                .as_deref()
                .and_then(parse_kde_font_size)
                .unwrap_or(KDE_DEFAULT_POINTS)
                / KDE_DEFAULT_POINTS,
        );
    }
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "text-scaling-factor"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read_system_text_scale() -> Option<f32> {
    None
}

/// KDE's default general font size (Noto Sans 10), which counts as 100%.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const KDE_DEFAULT_POINTS: f32 = 10.;

/// The point size of the general font in `kdeglobals`: `font=Noto Sans,11,-1,5,…` in the
/// `[General]` group.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_kde_font_size(kdeglobals: &str) -> Option<f32> {
    let mut general = false;
    for line in kdeglobals.lines().map(str::trim) {
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general && let Some(value) = line.strip_prefix("font=") {
            return value
                .split(',')
                .nth(1)?
                .trim()
                .parse()
                .ok()
                .filter(|points: &f32| *points > 0.);
        }
    }
    None
}

/// The value of a `REG_DWORD` line from `reg query`, e.g. `TextScaleFactor REG_DWORD 0x7d`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_dword(output: &str) -> Option<u32> {
    let line = output.lines().find(|line| line.contains("REG_DWORD"))?;
    let hex = line.split_whitespace().last()?.strip_prefix("0x")?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        SPACE, Tokens, apply_animations, apply_text_size, parse_kde_font_size, parse_reg_dword, scheme_theme, snap,
        text, text_scale, tokens,
    };

    #[test]
    fn reads_the_windows_text_size() {
        let output =
            "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Accessibility\r\n    TextScaleFactor    REG_DWORD    0x7d\r\n";
        assert_eq!(parse_reg_dword(output), Some(125));
        assert_eq!(
            parse_reg_dword("ERROR: The system was unable to find the specified registry key"),
            None
        );
    }

    #[test]
    fn chosen_sizes_win_and_stay_in_range() {
        assert_eq!(text_scale(Some(130)), 1.3);
        assert_eq!(text_scale(Some(400)), 2.);
    }

    #[test]
    fn reads_the_kde_font_size() {
        let kdeglobals = "[General]\nColorScheme=BreezeDark\nfont=Noto Sans,12,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\n[WM]\nfont=Noto Sans,9,-1,5,700,0,0,0,0,0\n";
        assert_eq!(parse_kde_font_size(kdeglobals), Some(12.));
        // The general font only, not the window title's.
        assert_eq!(parse_kde_font_size("[WM]\nfont=Noto Sans,9,-1,5\n"), None);
        assert_eq!(parse_kde_font_size("[General]\nfont=Inter,10.5,-1,5\n"), Some(10.5));
    }

    #[test]
    fn a_saved_zoom_lands_on_a_level() {
        assert_eq!(snap(1.), 1.);
        assert_eq!(snap(0.72), 0.75);
        assert_eq!(snap(1.4), 1.5);
        // Well above the last level.
        assert_eq!(snap(4.), 2.);
    }

    #[test]
    fn animations_off_makes_every_change_instant() {
        let ctx = egui::Context::default();
        apply_animations(&ctx, false);
        let style = ctx.global_style();
        assert_eq!(style.animation_time, 0.);
        assert_eq!(style.scroll_animation, egui::style::ScrollAnimation::none());
        // Turning them back on restores what egui's own style had.
        apply_animations(&ctx, true);
        let style = ctx.global_style();
        assert_eq!(style.animation_time, egui::Style::default().animation_time);
        assert_eq!(style.scroll_animation, egui::style::ScrollAnimation::default());
    }

    #[test]
    fn the_tokens_are_mains_overrides_on_gpui_kits_themes() {
        // `main`'s `theme.rs`: radius 10, radius_lg 12, and a darker muted grey in light.
        for tokens in [Tokens::LIGHT, Tokens::DARK] {
            assert_eq!(tokens.radius, 10);
            assert_eq!(tokens.radius_lg, 12);
        }
        assert_eq!(
            Tokens::LIGHT.muted_foreground,
            egui::Color32::from_rgb(0x52, 0x52, 0x52)
        );
        assert_eq!(Tokens::DARK.muted_foreground, egui::Color32::from_rgb(0xa3, 0xa3, 0xa3));
        // gpui-kit's defaults the pages lean on.
        assert_eq!(Tokens::DARK.background, egui::Color32::from_rgb(0x0a, 0x0a, 0x0a));
        assert_eq!(Tokens::LIGHT.background, egui::Color32::WHITE);
        assert_eq!(Tokens::DARK.primary, egui::Color32::from_rgb(0xfa, 0xfa, 0xfa));
        assert_eq!(Tokens::LIGHT.primary, egui::Color32::from_rgb(0x17, 0x17, 0x17));
        assert_eq!(tokens(&egui::Visuals::dark()), &Tokens::DARK);
        assert_eq!(tokens(&egui::Visuals::light()), &Tokens::LIGHT);
    }

    #[test]
    fn egui_draws_with_the_tokens() {
        for (tokens, dark) in [(Tokens::LIGHT, false), (Tokens::DARK, true)] {
            let visuals = super::visuals(&tokens);
            assert_eq!(visuals.dark_mode, dark);
            assert_eq!(visuals.panel_fill, tokens.background);
            assert_eq!(visuals.window_fill, tokens.popover);
            assert_eq!(visuals.weak_text_color, Some(tokens.muted_foreground));
            assert_eq!(visuals.faint_bg_color, tokens.muted);
            assert_eq!(visuals.error_fg_color, tokens.danger);
            assert_eq!(visuals.selection.bg_fill, tokens.selection);
            assert_eq!(visuals.selection.stroke.color, tokens.foreground);
            assert_eq!(visuals.widgets.noninteractive.fg_stroke.color, tokens.foreground);
            assert_eq!(visuals.widgets.inactive.weak_bg_fill, tokens.button);
            assert_eq!(visuals.widgets.hovered.weak_bg_fill, tokens.button_hover);
            assert_eq!(visuals.widgets.active.bg_fill, tokens.secondary_active);
            // What `strong_text_color` returns, which the cursor rings and primary buttons
            // painted with before the tokens were read directly.
            assert_eq!(visuals.strong_text_color(), tokens.foreground);
            assert_eq!(visuals.widgets.inactive.corner_radius.nw, tokens.radius);
            assert_eq!(visuals.menu_corner_radius.nw, tokens.radius_lg);
        }
    }

    #[test]
    fn the_spacing_is_gpui_kits_scale() {
        let spacing = super::spacing();
        assert_eq!(spacing.item_spacing, egui::vec2(SPACE.sm, SPACE.xs));
        assert_eq!(spacing.button_padding, egui::vec2(10., SPACE.xs));
        // `main`'s medium buttons and inputs are 32 points tall.
        assert_eq!(spacing.interact_size.y, 32.);
    }

    /// The app's table, scaled: every style it installs is exactly the table's size times the
    /// setting, and applying it again doesn't compound.
    #[test]
    fn the_text_size_scales_every_text_style_once() {
        let ctx = egui::Context::default();
        let sizes = |ctx: &egui::Context| -> Vec<f32> {
            let style = ctx.global_style();
            style
                .text_styles()
                .into_iter()
                .map(|text_style| style.text_styles[&text_style].size)
                .collect()
        };
        apply_text_size(&ctx, Some(100));
        let normal = sizes(&ctx);
        // The table's own sizes, not egui's: a body at `text_sm`, a heading at `text_lg`.
        let style = ctx.global_style();
        assert_eq!(style.text_styles[&egui::TextStyle::Body].size, text::SM);
        assert_eq!(style.text_styles[&egui::TextStyle::Small].size, text::XS);
        assert_eq!(style.text_styles[&egui::TextStyle::Heading].size, text::LG);
        for (name, size) in text::NAMED {
            assert_eq!(style.text_styles[&text::named(name)].size, size);
        }
        apply_text_size(&ctx, Some(150));
        let scaled = sizes(&ctx);
        for (normal, scaled) in normal.iter().zip(&scaled) {
            assert_eq!(*scaled, normal * 1.5);
        }
        // The setting is applied again whenever another one changes.
        apply_text_size(&ctx, Some(150));
        assert_eq!(sizes(&ctx), scaled);
        apply_text_size(&ctx, Some(100));
        assert_eq!(sizes(&ctx), normal);
    }

    /// Stand-ins for xdg-desktop-portal's settings interface, served on the session bus under
    /// names of their own so a real portal isn't touched.
    #[cfg(target_os = "linux")]
    mod portal {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{Duration, Instant};

        use zbus::zvariant::{OwnedValue, Value};

        use super::super::{APPEARANCE, COLOR_SCHEME, watch_colour_scheme};

        const PATH: &str = "/org/freedesktop/portal/desktop";

        /// Which reads the fake portal answers.
        #[derive(Clone, Copy)]
        enum Version {
            /// 1.15 and later: `ReadOne`.
            Current,
            /// 1.14 and earlier: only `Read`, its value in a variant of its own.
            Old,
            /// Answers no read, as a portal whose backend is missing.
            Mute,
        }

        struct FakePortal {
            scheme: Arc<AtomicU32>,
            version: Version,
        }

        impl FakePortal {
            fn value(&self, namespace: &str, key: &str) -> zbus::fdo::Result<u32> {
                if namespace != APPEARANCE || key != COLOR_SCHEME {
                    return Err(zbus::fdo::Error::InvalidArgs(format!("{namespace} {key}")));
                }
                Ok(self.scheme.load(Ordering::SeqCst))
            }
        }

        #[zbus::interface(name = "org.freedesktop.portal.Settings")]
        impl FakePortal {
            async fn read_one(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
                match self.version {
                    Version::Current => Ok(OwnedValue::from(self.value(&namespace, &key)?)),
                    Version::Old | Version::Mute => Err(zbus::fdo::Error::UnknownMethod("ReadOne".into())),
                }
            }

            async fn read(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
                match self.version {
                    Version::Current | Version::Old => {
                        let wrapped = Value::new(Value::from(self.value(&namespace, &key)?));
                        Ok(OwnedValue::try_from(wrapped).expect("a plain value"))
                    }
                    Version::Mute => Err(zbus::fdo::Error::Failed("no backend".into())),
                }
            }

            #[zbus(signal)]
            async fn setting_changed(
                emitter: &zbus::object_server::SignalEmitter<'_>,
                namespace: &str,
                key: &str,
                value: OwnedValue,
            ) -> zbus::Result<()>;
        }

        /// Waits for the next colour scheme the watch sends; while waiting, `poke` runs every
        /// 100 ms (to send a change the watch may not be listening for yet).
        fn next(updates: &mut crate::task::Receiver<egui::Theme>, mut poke: impl FnMut()) -> egui::Theme {
            let start = Instant::now();
            loop {
                poke();
                for _ in 0..5 {
                    if let Some(theme) = updates.drain().last() {
                        return theme;
                    }
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "the portal's colour scheme never arrived"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }

        /// Serves a fake portal of `version` starting at dark, and checks what the watch reads
        /// at first (if anything) and that it follows a change to light.
        fn follows(name: &'static str, version: Version, reads: bool) {
            let ctx = egui::Context::default();
            let (sender, mut updates) = crate::task::channel(&ctx);
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let scheme = Arc::new(AtomicU32::new(1));
            let offering = FakePortal {
                scheme: scheme.clone(),
                version,
            };
            let server = crate::runtime::start(async move {
                let connection = zbus::connection::Builder::session()
                    .expect("a session bus")
                    .name(name)
                    .expect("a bus name")
                    .serve_at(PATH, offering)
                    .expect("the fake portal")
                    .build()
                    .await
                    .expect("the fake portal's connection");
                ready_tx.send(connection).expect("the test is still there");
                std::future::pending::<()>().await;
            });
            let connection = ready_rx.recv_timeout(Duration::from_secs(5)).expect("the fake portal");
            let watch = crate::runtime::start(watch_colour_scheme(
                sender,
                name,
                PATH,
                "org.freedesktop.portal.Settings",
            ));

            if reads {
                assert_eq!(next(&mut updates, || {}), egui::Theme::Dark);
            }
            // The desktop switches to light: the portal changes its value and says so.
            scheme.store(2, Ordering::SeqCst);
            let say_light = || {
                let connection = connection.clone();
                crate::runtime::start(async move {
                    let emitter = zbus::object_server::SignalEmitter::new(&connection, PATH).expect("an emitter");
                    FakePortal::setting_changed(&emitter, APPEARANCE, COLOR_SCHEME, OwnedValue::from(2u32))
                        .await
                        .expect("the signal");
                });
            };
            assert_eq!(next(&mut updates, say_light), egui::Theme::Light);

            watch.abort();
            server.abort();
        }

        fn have_a_session_bus() -> bool {
            let there = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
            if !there {
                eprintln!("no session bus here: the portal watch isn't tested");
            }
            there
        }

        #[test]
        fn the_desktops_colour_scheme_is_read_and_followed() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakePortal", Version::Current, true);
            }
        }

        #[test]
        fn an_older_portal_is_read_with_read() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakeOldPortal", Version::Old, true);
            }
        }

        #[test]
        fn a_portal_that_cant_be_read_is_still_followed() {
            if have_a_session_bus() {
                follows("io.github.lgpy.tabbyfin.FakeMutePortal", Version::Mute, false);
            }
        }
    }

    #[test]
    fn a_portal_colour_scheme_maps_to_a_theme() {
        assert_eq!(scheme_theme(1), egui::Theme::Dark);
        assert_eq!(scheme_theme(2), egui::Theme::Light);
        // No preference, and anything the portal grows later.
        assert_eq!(scheme_theme(0), egui::Theme::Light);
        assert_eq!(scheme_theme(7), egui::Theme::Light);
    }
}

/// Colours over the video. The player stays dark whatever the theme, since its controls sit on
/// the picture; these are its tokens, so views don't scatter black and white literals.
pub mod over_video {
    use egui::Color32;

    /// Behind the picture, and its letterbox bars.
    pub const BACKDROP: Color32 = Color32::BLACK;

    /// Where the top bar's gradient starts (it fades to [`CLEAR`]).
    pub const SCRIM: Color32 = Color32::from_black_alpha(179);

    /// Where the control bar's gradient starts, a little darker for the seek bar's sake.
    pub const SCRIM_STRONG: Color32 = Color32::from_black_alpha(204);

    pub const CLEAR: Color32 = Color32::from_black_alpha(0);

    /// Cards and labels floating on the picture: the up-next card, the seek preview's time.
    pub const SURFACE: Color32 = Color32::from_black_alpha(204);

    /// Small labels in the bars (Transcoding, the speed).
    pub const SURFACE_SUBTLE: Color32 = Color32::from_black_alpha(128);

    pub const FOREGROUND: Color32 = Color32::WHITE;

    /// White at 70 %.
    pub const MUTED_FOREGROUND: Color32 = Color32::from_rgba_premultiplied(179, 179, 179, 179);

    /// Chapter marks on the seek bar and the preview's outline: white at 60 %.
    pub const MARK: Color32 = Color32::from_rgba_premultiplied(153, 153, 153, 153);

    /// Behind a card's progress bar, over its image. Dark against the fill, with a light outline
    /// ([`TRACK_OUTLINE`]) so the bar's full length still shows on dark images.
    pub const TRACK: Color32 = Color32::from_black_alpha(153);

    /// White at 50 %.
    pub const TRACK_OUTLINE: Color32 = Color32::from_rgba_premultiplied(128, 128, 128, 128);
}
