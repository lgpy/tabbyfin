//! A person, an account or a group as one row of a list: their avatars, a title and a quieter
//! line under it, and at the right end an optional ✕ to remove them or a compact `brand` action.
//! The sign-in screen lists its saved accounts this way and the SyncPlay panel the groups to join.
//! [`person`] is the quieter row for a list of people that isn't a list of buttons.
//!
//! The row is one button (named to AccessKit by its label); the ✕ is a second one after it, so
//! Tab reaches the row and then its removal. The action is part of the row: a click anywhere on
//! the row presses it.

use egui::accesskit::Role;
use egui::{Align2, Color32, CornerRadius, Response, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::icons::ICON_CLOSE;

use crate::theme::{self, text};
use crate::widgets::avatar;
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};

const HEIGHT: f32 = 56.;
const AVATAR_RADIUS: f32 = 18.;
/// The avatars of a row with several people: smaller, so three fit, and overlapping.
const STACK_RADIUS: f32 = 14.;
/// How far each avatar of a stack sits from the one before it.
const STACK_STEP: f32 = 18.;
/// At most this many avatars in a stack.
const STACK_MAX: usize = 3;
/// The room left of the avatar, and between it and the text.
const INSET: f32 = 12.;
/// The ✕'s square at the row's right end.
const REMOVE_SIZE: f32 = 32.;
/// The action's compact button, the toasts' size.
const ACTION_H: f32 = 30.;
const ACTION_RADIUS: u8 = 10;
/// A [`person`] row's height and avatar.
const PERSON_HEIGHT: f32 = 36.;
const PERSON_RADIUS: f32 = 14.;

pub struct AvatarRow<'a> {
    /// Whose avatars it shows: one for a person, up to three overlapping for a group.
    names: &'a [&'a str],
    title: &'a str,
    subtitle: &'a str,
    /// What a screen reader says for the row.
    label: &'a str,
    /// A dot in this colour before the subtitle: the state of what the row stands for.
    status: Option<Color32>,
    /// The ✕'s tooltip, or `None` for a row that can't be removed.
    remove: Option<&'a str>,
    /// A compact `brand` button's label at the row's right end, saying what the row does.
    action: Option<&'a str>,
    /// The keyboard is on it, for a list that keeps its own keyboard rather than egui's focus.
    highlighted: bool,
    focus: Focus,
}

/// What a row's buttons did this frame.
pub struct RowResponse {
    pub row: Response,
    pub remove: Option<Response>,
}

impl<'a> AvatarRow<'a> {
    pub fn new(names: &'a [&'a str], title: &'a str, subtitle: &'a str, label: &'a str) -> Self {
        Self {
            names,
            title,
            subtitle,
            label,
            status: None,
            remove: None,
            action: None,
            highlighted: false,
            focus: Focus::Tab,
        }
    }

    pub fn status(mut self, color: Color32) -> Self {
        self.status = Some(color);
        self
    }

    /// A ✕ at the right end with this tooltip.
    pub fn remove(mut self, tooltip: &'a str) -> Self {
        self.remove = Some(tooltip);
        self
    }

    /// A compact `brand` button with this label at the right end, in place of a ✕.
    pub fn action(mut self, label: &'a str) -> Self {
        self.action = Some(label);
        self
    }

    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    pub fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// Draws the row across the `ui`'s width, its buttons disabled while `enabled` is false.
    pub fn show(self, ui: &mut Ui, enabled: bool) -> RowResponse {
        let tokens = theme::tokens_of(ui);
        let sense = match (enabled, self.focus) {
            (false, _) => Sense::hover(),
            (true, Focus::Tab) => Sense::click(),
            (true, Focus::Skip) => Sense::CLICK,
        };
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), sense);
        row.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, self.label));

        let focused = ui.memory(|memory| memory.focused()) == Some(row.id);
        let active = self.highlighted || focused;
        let painter = ui.painter();
        let fill = if row.hovered() || active {
            tokens.inset_hover
        } else {
            tokens.inset
        };
        painter.rect_filled(rect, tokens.radius_control, fill);
        if active {
            focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
        }

        let x = match self.names {
            [name] => {
                let center = pos2(rect.left() + INSET + AVATAR_RADIUS, rect.center().y);
                avatar::paint(ui, center, AVATAR_RADIUS, name);
                center.x + AVATAR_RADIUS + INSET
            }
            names => {
                let shown = &names[..names.len().min(STACK_MAX)];
                let mut center = pos2(rect.left() + INSET + STACK_RADIUS, rect.center().y);
                for (index, name) in shown.iter().enumerate() {
                    if index > 0 {
                        center.x += STACK_STEP;
                    }
                    // A ring of the row's fill parts each avatar from the one under it.
                    painter.circle_filled(center, STACK_RADIUS + 2., fill);
                    avatar::paint(ui, center, STACK_RADIUS, name);
                }
                center.x + STACK_RADIUS + INSET
            }
        };
        painter.text(
            pos2(x, rect.center().y - 9.),
            Align2::LEFT_CENTER,
            self.title,
            text::font(ui, "base"),
            tokens.ink,
        );
        let mut subtitle_x = x;
        if let Some(color) = self.status {
            painter.circle_filled(pos2(x + 3., rect.center().y + 10.), 3., color);
            subtitle_x += 10.;
        }
        painter.text(
            pos2(subtitle_x, rect.center().y + 10.),
            Align2::LEFT_CENTER,
            self.subtitle,
            text::font(ui, "xs"),
            tokens.ink_muted,
        );

        if let Some(action) = self.action {
            let font = text::font(ui, "sm");
            let galley = painter.layout_no_wrap(action.to_owned(), font, tokens.brand_foreground);
            let width = galley.size().x + 2. * INSET;
            let button = egui::Rect::from_min_size(
                pos2(rect.right() - INSET - width, rect.center().y - ACTION_H / 2.),
                vec2(width, ACTION_H),
            );
            let fill = if enabled && row.hovered() {
                tokens.brand_hover
            } else {
                tokens.brand
            };
            painter.rect_filled(button, CornerRadius::same(ACTION_RADIUS), fill);
            painter.galley(button.center() - galley.size() / 2., galley, tokens.brand_foreground);
        }

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

/// A person in a list that isn't a list of buttons (who's in a SyncPlay group): a small avatar,
/// their name, and `aside` after it in the muted ink ("you").
pub fn person(ui: &mut Ui, name: &str, aside: Option<&str>) -> Response {
    let tokens = theme::tokens_of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), PERSON_HEIGHT), Sense::hover());
    response.widget_info(|| {
        let label = match aside {
            Some(aside) => format!("{name}, {aside}"),
            None => name.to_owned(),
        };
        WidgetInfo::labeled(Role::ListItem, true, label)
    });
    let center = pos2(rect.left() + PERSON_RADIUS, rect.center().y);
    avatar::paint(ui, center, PERSON_RADIUS, name);
    let painter = ui.painter();
    let x = center.x + PERSON_RADIUS + 10.;
    let name_rect = painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        text::font(ui, "base"),
        tokens.ink,
    );
    if let Some(aside) = aside {
        painter.text(
            pos2(name_rect.right() + 6., rect.center().y),
            Align2::LEFT_CENTER,
            aside,
            text::font(ui, "xs"),
            tokens.ink_muted,
        );
    }
    response
}
