//! Notifications in the window's bottom-right corner: info, success and error. Each goes away
//! by itself after a few seconds (not while the pointer is on it), or with its close button,
//! and can carry one action button (the update notice's Download). Its buttons are
//! `Sense::CLICK` widgets, not focusable (U8's rule for its dialogs'): a Tab with a toast over
//! the player mustn't give egui the focus, or egui would keep the player's keys (`keys.rs`).
//!
//! Anything can raise one: signed-in code pushes `AppEvent::Toast` into `Services::events`, and
//! the app, which owns [`Toasts`], shows them.

use std::time::{Duration, Instant};

use egui::{Align2, Id, RichText};
use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_CLOSE, ICON_ERROR, ICON_INFO};

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

    /// A button under the text. Clicking it closes the toast.
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
                for shown in &mut self.shown {
                    let response = ui
                        .push_id(shown.id, |ui| {
                            egui::Frame::popup(ui.style())
                                .show(ui, |ui| {
                                    ui.set_width(WIDTH);
                                    ui.horizontal_top(|ui| {
                                        let (icon, color) = icon(shown.toast.kind, ui);
                                        ui.label(icon.rich_text().size(20.0).color(color));
                                        ui.vertical(|ui| {
                                            // What's left beside the icon, after the close button.
                                            ui.set_width(ui.available_width() - 28.0);
                                            if let Some(title) = &shown.toast.title {
                                                ui.label(RichText::new(title).strong());
                                            }
                                            ui.label(&shown.toast.text);
                                            if let Some((label, action)) = &shown.toast.action
                                                && toast_button(ui, egui::Button::new(label)).clicked()
                                            {
                                                clicked = Some(action.clone());
                                                closed.push(shown.id);
                                            }
                                        });
                                        if toast_button(ui, egui::Button::new(ICON_CLOSE.codepoint).small())
                                            .on_hover_text("Close")
                                            .clicked()
                                        {
                                            closed.push(shown.id);
                                        }
                                    });
                                })
                                .response
                        })
                        .inner;
                    shown.rect = Some(response.rect);
                }
            });
        self.shown.retain(|shown| !closed.contains(&shown.id));
        clicked
    }
}

/// A toast's action or close button, drawn as the button it's given. Not focusable
/// (`Sense::CLICK` without `FOCUSABLE`), as U8's dialogs' buttons aren't: a Tab that focused
/// one over the player would give egui the keyboard and take the player's keys away
/// (`keys.rs`).
fn toast_button(ui: &mut egui::Ui, button: egui::Button<'_>) -> egui::Response {
    ui.add(button.sense(egui::Sense::CLICK))
}

/// A toast's icon and colour, as `main`'s notifications had them: the theme's info, success and
/// danger colours.
fn icon(kind: Kind, ui: &egui::Ui) -> (egui_material_icons::MaterialIcon, egui::Color32) {
    let tokens = crate::theme::tokens_of(ui);
    match kind {
        Kind::Info => (ICON_INFO, tokens.info),
        Kind::Success => (ICON_CHECK_CIRCLE, tokens.success),
        Kind::Error => (ICON_ERROR, tokens.danger),
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
