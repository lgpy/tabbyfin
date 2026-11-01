//! A round button holding an icon or a person's avatar: the top bar's buttons, the SyncPlay
//! panel's toggles and queue buttons, and the button inside a field's right end.
//!
//! No frame at rest; under the pointer an `inset_hover` circle comes up behind it and the icon
//! goes from `ink_muted` to `ink`. A toggle that is on keeps the circle with its icon in `brand`.

use egui::accesskit::Role;
use egui::{CursorIcon, FontId, Id, Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo};
use egui_material_icons::MaterialIcon;

use crate::theme;
use crate::widgets::avatar;
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};

/// What the button shows.
#[derive(Clone, Copy)]
enum Content<'a> {
    Icon(MaterialIcon),
    /// This name's avatar, 4 points inside the button so its hover circle shows round it.
    Avatar(&'a str),
}

pub struct RoundButton<'a> {
    content: Content<'a>,
    /// What a screen reader says, and the tooltip.
    label: &'a str,
    size: f32,
    icon_size: f32,
    /// `Some` for a toggle: whether it's on.
    on: Option<bool>,
    ringed: bool,
    enabled: bool,
    focus: Focus,
}

impl<'a> RoundButton<'a> {
    /// A 40-point button with a 24-point icon, the top bar's size.
    pub fn icon(icon: MaterialIcon, label: &'a str) -> Self {
        Self::new(Content::Icon(icon), label)
    }

    /// A 40-point button showing `name`'s avatar.
    pub fn avatar(name: &'a str, label: &'a str) -> Self {
        Self::new(Content::Avatar(name), label)
    }

    fn new(content: Content<'a>, label: &'a str) -> Self {
        Self {
            content,
            label,
            size: 40.,
            icon_size: 24.,
            on: None,
            ringed: false,
            enabled: true,
            focus: Focus::Tab,
        }
    }

    /// The button's diameter and its icon's size.
    pub fn size(mut self, size: f32, icon_size: f32) -> Self {
        self.size = size;
        self.icon_size = icon_size;
        self
    }

    /// Makes it a toggle, on or off: on, it keeps its circle and its icon is `brand`.
    pub fn on(mut self, on: bool) -> Self {
        self.on = Some(on);
        self
    }

    /// The keyboard's ring round it, for a list or bar that keeps its own keyboard rather than
    /// egui's focus. A focused button is ringed anyway.
    pub fn ringed(mut self, ringed: bool) -> Self {
        self.ringed = ringed;
        self
    }

    /// A disabled button is dimmed and takes no clicks.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// Adds the button to `ui`'s layout.
    pub fn show(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), self.sense());
        self.paint(ui, rect, response)
    }

    /// The button centred on `center` without taking room in `ui`'s layout, so it can sit over
    /// something else (a field, a row); `id` tells it apart from the others there.
    pub fn show_at(self, ui: &mut Ui, center: Pos2, id: Id) -> Response {
        let rect = Rect::from_center_size(center, Vec2::splat(self.size));
        let response = ui.interact(rect, id, self.sense());
        self.paint(ui, rect, response)
    }

    fn sense(&self) -> Sense {
        match (self.enabled, self.focus) {
            (false, _) => Sense::hover(),
            (true, Focus::Tab) => Sense::click(),
            (true, Focus::Skip) => Sense::CLICK,
        }
    }

    fn paint(self, ui: &Ui, rect: Rect, response: Response) -> Response {
        let tokens = theme::tokens_of(ui);
        let (label, enabled) = (self.label, self.enabled);
        match self.on {
            Some(on) => response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, on, label)),
            None => response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label)),
        }
        let on = self.on == Some(true);
        let hovered = enabled && response.hovered();
        let painter = ui.painter();
        if hovered || on {
            painter.circle_filled(rect.center(), self.size / 2., tokens.inset_hover);
        }
        // The focus by id, not `Response::has_focus`, which is false while the window lacks the
        // OS's focus (`docs/DESIGN.md`, *Behaviour*).
        let focused = ui.memory(|memory| memory.focused()) == Some(response.id);
        let ring = match self.content {
            Content::Icon(icon) => {
                let color = if !enabled {
                    tokens.ink_muted.gamma_multiply(0.45)
                } else if on {
                    tokens.brand
                } else if hovered {
                    tokens.ink
                } else {
                    tokens.ink_muted
                };
                glyph(ui, rect.center(), icon, self.icon_size, color);
                rect
            }
            Content::Avatar(name) => {
                let radius = self.size / 2. - 4.;
                avatar::paint(ui, rect.center(), radius, name);
                Rect::from_center_size(rect.center(), Vec2::splat(2. * radius))
            }
        };
        if self.ringed || focused {
            focus::ring(ui, ring, ring.width() / 2., Place::Outside);
        }
        if !enabled {
            return response;
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(self.label)
    }
}

/// `icon` centred on `center` by its own bounds: the icon font's glyphs sit low in their line,
/// so centring the line would put them below the middle.
pub fn glyph(ui: &Ui, center: Pos2, icon: MaterialIcon, size: f32, color: egui::Color32) {
    let font = FontId::new(size, icon.font_family());
    let galley = ui.painter().layout_no_wrap(icon.codepoint.to_owned(), font, color);
    let bounds = galley.rows.first().map_or(galley.rect, |row| row.rect());
    ui.painter().galley(center - bounds.center().to_vec2(), galley, color);
}
