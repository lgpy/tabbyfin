//! Notifications in the window's bottom-right corner: info, success and error, each a small
//! surface in the popups' frame. Each goes away by itself after a few seconds (not while the
//! pointer is on it), or with its close button, and can carry one action button at its right
//! end (the update notice's Download). Its buttons are
//! `Sense::CLICK` widgets, not focusable (U8's rule for its dialogs'): a Tab with a toast over
//! the player mustn't give egui the focus, or egui would keep the player's keys (`keys.rs`).
//!
//! Anything can raise one: signed-in code pushes `AppEvent::Toast` into `Services::events`, and
//! the app, which owns [`Toasts`], shows them.

use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_CLOSE, ICON_ERROR, ICON_INFO};

use crate::theme::{self, SPACE, text};
use crate::widgets::popup;

/// How long a toast stays up.
const SHOWN_FOR: Duration = Duration::from_secs(5);
/// Errors stay longer: they often say what to do next.
const ERROR_SHOWN_FOR: Duration = Duration::from_secs(8);
/// How long a toast stays after the pointer leaves it.
const AFTER_HOVER: Duration = Duration::from_secs(2);
/// The oldest go first when more pile up than this.
const MAX_SHOWN: usize = 4;
const WIDTH: f32 = 340.0;
/// From the window's corner.
const MARGIN: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Success,
    Error,
}

/// What a toast's button does. The app carries it out when [`Toasts::show`] returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastAction {
    OpenUrl(String),
}

#[derive(Clone, Debug)]
pub struct Toast {
    kind: Kind,
    title: Option<String>,
    text: String,
    action: Option<(String, ToastAction)>,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Kind::Info, text.into())
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::new(Kind::Success, text.into())
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Kind::Error, text.into())
    }

    fn new(kind: Kind, text: String) -> Self {
        Self {
            kind,
            title: None,
            text,
            action: None,
        }
    }

    /// A bold line above the text.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A compact button at the toast's right end. Clicking it closes the toast.
    pub fn action(mut self, label: impl Into<String>, action: ToastAction) -> Self {
        self.action = Some((label.into(), action));
        self
    }

    fn shown_for(&self) -> Duration {
        match self.kind {
            Kind::Error => ERROR_SHOWN_FOR,
            Kind::Info | Kind::Success => SHOWN_FOR,
        }
    }
}

struct Shown {
    /// Keeps each toast's widgets apart.
    id: u64,
    toast: Toast,
    until: Instant,
    /// Where it was drawn last frame, to tell whether the pointer is on it.
    rect: Option<egui::Rect>,
}

#[derive(Default)]
pub struct Toasts {
    shown: Vec<Shown>,
    next_id: u64,
}

impl Toasts {
    pub fn push(&mut self, toast: Toast) {
        self.push_at(toast, Instant::now());
    }

    fn push_at(&mut self, toast: Toast, now: Instant) {
        match toast.kind {
            Kind::Error => tracing::warn!("toast: {}", toast.text),
            Kind::Info | Kind::Success => tracing::info!("toast: {}", toast.text),
        }
        let until = now + toast.shown_for();
        self.shown.push(Shown {
            id: self.next_id,
            toast,
            until,
            rect: None,
        });
        self.next_id += 1;
        if self.shown.len() > MAX_SHOWN {
            self.shown.remove(0);
        }
    }

    /// Drops the toasts whose time is up. Returns how long until the next one goes.
    fn expire(&mut self, now: Instant) -> Option<Duration> {
        self.shown.retain(|shown| shown.until > now);
        self.shown.iter().map(|shown| shown.until - now).min()
    }

    /// Draws the toasts over everything, newest nearest the corner. Returns the action whose
    /// button was clicked, for the app to carry out.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ToastAction> {
        let now = Instant::now();
        // Before expiring: a toast under the pointer stays, even when the pointer hasn't moved
        // since the frame that drew it.
        if let Some(pointer) = ctx.pointer_hover_pos() {
            for shown in &mut self.shown {
                if shown.rect.is_some_and(|rect| rect.contains(pointer)) {
                    shown.until = shown.until.max(now + AFTER_HOVER);
                }
            }
        }
        if let Some(next) = self.expire(now) {
            ctx.request_repaint_after(next);
        }
        if self.shown.is_empty() {
            return None;
        }
        let mut clicked = None;
        let mut closed = Vec::new();
        egui::Area::new(Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, [-MARGIN, -MARGIN])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = GAP;
                for shown in &mut self.shown {
                    let drawn = ui.push_id(shown.id, |ui| draw(ui, &shown.toast)).inner;
                    if drawn.acted
                        && let Some((_, action)) = &shown.toast.action
                    {
                        clicked = Some(action.clone());
                        closed.push(shown.id);
                    }
                    if drawn.closed {
                        closed.push(shown.id);
                    }
                    shown.rect = Some(drawn.rect);
                }
            });
        self.shown.retain(|shown| !closed.contains(&shown.id));
        clicked
    }
}

/// The room inside a toast, and between toasts.
const PAD: f32 = 14.;
const GAP: f32 = 8.;
/// A toast's corner radius: a popup's (`widgets::popup::RADIUS`).
const RADIUS: u8 = popup::RADIUS;
/// The kind's icon, and the room after it.
const ICON: f32 = 20.;
const ICON_GAP: f32 = 10.;
/// The close button's circle, and the action button's height and corners.
const CLOSE: f32 = 28.;
const BUTTON_H: f32 = 30.;
const BUTTON_RADIUS: u8 = 10;

/// What a toast's buttons did this frame, and where it was drawn.
struct Drawn {
    rect: Rect,
    acted: bool,
    closed: bool,
}

/// One toast: a small surface with the kind's icon on its first line, the title in the ink and
/// the text under it in the muted ink (the text alone is in the ink), and at its right end the
/// action as a compact brand button and a round close button. Laid out by hand so the buttons
/// centre on the toast however many lines the text takes.
fn draw(ui: &mut Ui, toast: &Toast) -> Drawn {
    let tokens = theme::tokens_of(ui);
    let painter = ui.painter().clone();
    let action = toast
        .action
        .as_ref()
        .map(|(label, _)| painter.layout_no_wrap(label.clone(), text::font(ui, "sm"), tokens.brand_foreground));
    let button_w = action.as_ref().map(|galley| galley.size().x + 2. * SPACE.md);
    // The close button's circle reaches into the padding, so its icon lines up with the text's
    // edge rather than the circle's.
    let close_in = 4.;
    let right = PAD - close_in + CLOSE;
    let text_w = WIDTH - PAD - ICON - ICON_GAP - right - button_w.map_or(0., |w| w + ICON_GAP) - ICON_GAP;
    let title = toast
        .title
        .as_ref()
        .map(|title| painter.layout(title.clone(), text::font(ui, "base"), tokens.ink, text_w));
    let body_color = if title.is_some() { tokens.ink_muted } else { tokens.ink };
    let body = painter.layout(toast.text.clone(), text::font(ui, "sm"), body_color, text_w);
    let title_h = title.as_ref().map_or(0., |galley| galley.size().y + SPACE.xxs);
    let content_h = (title_h + body.size().y)
        .max(CLOSE)
        .max(if action.is_some() { BUTTON_H } else { 0. });
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, content_h + 2. * PAD), Sense::hover());

    let radius = CornerRadius::same(RADIUS);
    painter.add(theme::popup_shadow(tokens).as_shape(rect, radius));
    painter.rect(
        rect,
        radius,
        tokens.surface,
        Stroke::new(1., tokens.surface_border),
        StrokeKind::Inside,
    );

    let (glyph, color) = icon(toast.kind, tokens);
    let first_line = title
        .as_ref()
        .unwrap_or(&body)
        .rows
        .first()
        .map_or(ICON, |row| row.height());
    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + PAD, rect.top() + PAD + (first_line - ICON) / 2.),
        Vec2::splat(ICON),
    );
    let icon_response = ui.interact(icon_rect, ui.scope_id().with("icon"), Sense::hover());
    icon_response.widget_info(|| WidgetInfo::labeled(Role::Image, true, kind_label(toast.kind)));
    paint_icon(&painter, icon_rect.center(), glyph, ICON, color);

    let x = icon_rect.right() + ICON_GAP;
    let mut y = rect.top() + PAD;
    if let Some(title) = title {
        let height = title.size().y;
        line(ui, "title", Rect::from_min_size(pos2(x, y), title.size()), title.text());
        painter.galley(pos2(x, y), title, tokens.ink);
        y += height + SPACE.xxs;
    }
    line(ui, "text", Rect::from_min_size(pos2(x, y), body.size()), body.text());
    painter.galley(pos2(x, y), body, body_color);

    let close_rect = Rect::from_center_size(
        pos2(rect.right() - PAD + close_in - CLOSE / 2., rect.center().y),
        Vec2::splat(CLOSE),
    );
    let close = ui
        .interact(close_rect, ui.scope_id().with("close"), Sense::CLICK)
        .on_hover_cursor(CursorIcon::PointingHand);
    close.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Close"));
    let close = close.on_hover_text("Close");
    if close.hovered() {
        painter.circle_filled(close_rect.center(), CLOSE / 2., tokens.inset_hover);
    }
    let close_color = if close.hovered() { tokens.ink } else { tokens.ink_muted };
    paint_icon(&painter, close_rect.center(), ICON_CLOSE, 18., close_color);

    let mut acted = false;
    if let (Some(galley), Some(width), Some((label, _))) = (action, button_w, &toast.action) {
        let button_rect = Rect::from_center_size(
            pos2(close_rect.left() - SPACE.xs - width / 2., rect.center().y),
            vec2(width, BUTTON_H),
        );
        // `Sense::CLICK` without `FOCUSABLE`, like the close button (see the module's note).
        let button = ui
            .interact(button_rect, ui.scope_id().with("action"), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand);
        button.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
        let fill = if button.hovered() {
            tokens.brand_hover
        } else {
            tokens.brand
        };
        painter.rect_filled(button_rect, CornerRadius::same(BUTTON_RADIUS), fill);
        painter.galley(
            button_rect.center() - galley.size() / 2.,
            galley,
            tokens.brand_foreground,
        );
        acted = button.clicked();
    }

    Drawn {
        rect,
        acted,
        closed: close.clicked(),
    }
}

/// A line of a toast's text to AccessKit: the galleys are painted, so each gets a node of its own.
fn line(ui: &Ui, salt: &str, rect: Rect, text: &str) {
    ui.interact(rect, ui.scope_id().with(salt), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
}

/// A glyph centred by its own bounds: the icon font's glyphs sit low in their line.
fn paint_icon(painter: &egui::Painter, center: Pos2, icon: MaterialIcon, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(icon.codepoint.to_owned(), FontId::new(size, icon.font_family()), color);
    let glyph = galley.rows.first().map_or(galley.rect, |row| row.rect());
    painter.galley(center - glyph.center().to_vec2(), galley, color);
}

/// A toast's icon and colour: the theme's info, success and danger colours.
fn icon(kind: Kind, tokens: &theme::Tokens) -> (MaterialIcon, Color32) {
    match kind {
        Kind::Info => (ICON_INFO, tokens.info),
        Kind::Success => (ICON_CHECK_CIRCLE, tokens.success),
        Kind::Error => (ICON_ERROR, tokens.danger),
    }
}

/// What a screen reader says for a toast's icon.
fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Info => "Information",
        Kind::Success => "Success",
        Kind::Error => "Error",
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ERROR_SHOWN_FOR, MAX_SHOWN, SHOWN_FOR, Toast, Toasts};

    #[test]
    fn errors_stay_longer() {
        let now = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push_at(Toast::info("saved"), now);
        toasts.push_at(Toast::error("failed"), now);
        assert_eq!(toasts.expire(now), Some(SHOWN_FOR));
        assert_eq!(
            toasts.expire(now + SHOWN_FOR),
            Some(ERROR_SHOWN_FOR - SHOWN_FOR),
            "the info toast went"
        );
        assert_eq!(toasts.shown.len(), 1);
        assert_eq!(toasts.expire(now + ERROR_SHOWN_FOR), None);
        assert!(toasts.shown.is_empty());
    }

    #[test]
    fn the_oldest_go_when_too_many_pile_up() {
        let now = Instant::now();
        let mut toasts = Toasts::default();
        for i in 0..MAX_SHOWN + 2 {
            toasts.push_at(Toast::info(i.to_string()), now + Duration::from_millis(i as u64));
        }
        let texts: Vec<_> = toasts.shown.iter().map(|shown| shown.toast.text.as_str()).collect();
        assert_eq!(texts, ["2", "3", "4", "5"]);
    }
}
