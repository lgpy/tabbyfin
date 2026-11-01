//! A person or account as one row of a list: their avatar, a title and a quieter line under it,
//! and an optional ✕ to remove them. The sign-in screen lists its saved accounts this way.
//!
//! The row is one button (focusable, named to AccessKit by [`AvatarRow::label`]); the ✕ is a
//! second one after it, so Tab reaches the row and then its removal.

use egui::accesskit::Role;
use egui::{Align2, Response, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::icons::ICON_CLOSE;

use crate::theme::{self, text};
use crate::widgets::avatar;
use crate::widgets::focus::{self, Place};

const HEIGHT: f32 = 56.;
const AVATAR_RADIUS: f32 = 18.;
/// The room left of the avatar, and between it and the text.
const INSET: f32 = 12.;
/// The ✕'s square at the row's right end.
const REMOVE_SIZE: f32 = 32.;

pub struct AvatarRow<'a> {
    /// Whose avatar it shows.
    pub name: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// What a screen reader says for the row.
    pub label: &'a str,
    /// The ✕'s tooltip, or `None` for a row that can't be removed.
    pub remove: Option<&'a str>,
}

/// What a row's buttons did this frame.
pub struct RowResponse {
    pub row: Response,
    pub remove: Option<Response>,
}

impl AvatarRow<'_> {
    /// Draws the row across the `ui`'s width, its buttons disabled while `enabled` is false.
    pub fn show(self, ui: &mut Ui, enabled: bool) -> RowResponse {
        let tokens = theme::tokens_of(ui);
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), sense);
        row.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, self.label));

        let painter = ui.painter();
        let fill = if row.hovered() || row.has_focus() {
            tokens.inset_hover
        } else {
            tokens.inset
        };
        painter.rect_filled(rect, tokens.radius_control, fill);
        if row.has_focus() {
            focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
        }
        let center = pos2(rect.left() + INSET + AVATAR_RADIUS, rect.center().y);
        avatar::paint(ui, center, AVATAR_RADIUS, self.name);
        let x = center.x + AVATAR_RADIUS + INSET;
        painter.text(
            pos2(x, rect.center().y - 9.),
            Align2::LEFT_CENTER,
            self.title,
            text::font(ui, "base"),
            tokens.ink,
        );
        painter.text(
            pos2(x, rect.center().y + 10.),
            Align2::LEFT_CENTER,
            self.subtitle,
            text::font(ui, "xs"),
            tokens.ink_muted,
        );

        let remove = self.remove.map(|tooltip| {
            let square = egui::Rect::from_center_size(
                pos2(rect.right() - INSET - REMOVE_SIZE / 2. + 4., rect.center().y),
                egui::Vec2::splat(REMOVE_SIZE),
            );
            let button = egui::Button::new(ICON_CLOSE).frame_when_inactive(false);
            // A child that isn't allocated in `ui`, so the ✕ doesn't move the row's cursor.
            let mut inside = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(square)
                    .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
            );
            inside.add_enabled(enabled, button).on_hover_text(tooltip)
        });
        RowResponse { row, remove }
    }
}
