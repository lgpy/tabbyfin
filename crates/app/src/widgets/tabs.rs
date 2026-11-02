//! A row of equal tabs, each an icon and a label, with the open one lifted off a sunk track. The
//! player's settings menu has one. It takes no keyboard focus, as the player's keys are
//! `keys.rs`'s, and its colours are the caller's, since it may sit over the video.

use egui::accesskit::Role;
use egui::{Color32, CursorIcon, Rect, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;

use crate::theme::text;
use crate::widgets::round_button;

const MIN_HEIGHT: f32 = 40.;
/// The room between the track's edge and a tab, the track's radius and a tab's.
const INSET: f32 = 3.;
const RADIUS: u8 = 12;
const TAB_RADIUS: u8 = RADIUS - INSET as u8;
const ICON: f32 = 18.;
const ICON_GAP: f32 = 6.;

#[derive(Clone, Copy)]
pub struct Colours {
    pub track: Color32,
    /// The open tab's fill, and the tab under the pointer's.
    pub selected: Color32,
    pub hover: Color32,
    pub ink: Color32,
    pub ink_muted: Color32,
}

/// One tab: what it stands for, its icon and its label.
pub struct Tab<'a, T> {
    pub value: T,
    pub icon: MaterialIcon,
    pub label: &'a str,
}

/// Adds the tabs across `ui`'s width. Returns the tab clicked.
pub fn bar<T: Copy + PartialEq>(ui: &mut Ui, tabs: &[Tab<T>], current: T, colours: Colours) -> Option<T> {
    let font = text::font(ui, "sm");
    let height = MIN_HEIGHT.max(font.size + 22.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, RADIUS, colours.track);
    let width = (rect.width() - 2. * INSET) / tabs.len() as f32;
    let mut chosen = None;
    for (index, tab) in tabs.iter().enumerate() {
        let left = rect.left() + INSET + width * index as f32;
        let cell = Rect::from_min_size(pos2(left, rect.top() + INSET), vec2(width, height - 2. * INSET));
        let id = ui.scope_id().with(("tab", index));
        let response = ui.interact(cell, id, Sense::CLICK);
        let selected = tab.value == current;
        response.widget_info(|| WidgetInfo::selected(Role::Tab, ui.is_enabled(), selected, tab.label));
        let color = if selected { colours.ink } else { colours.ink_muted };
        if selected {
            ui.painter().rect_filled(cell, TAB_RADIUS, colours.selected);
        } else if response.hovered() {
            ui.painter().rect_filled(cell, TAB_RADIUS, colours.hover);
        }
        let galley = ui.painter().layout_no_wrap(tab.label.to_owned(), font.clone(), color);
        let start = cell.center().x - (ICON + ICON_GAP + galley.size().x) / 2.;
        round_button::glyph(ui, pos2(start + ICON / 2., cell.center().y), tab.icon, ICON, color);
        let text_at = pos2(start + ICON + ICON_GAP, cell.center().y - galley.size().y / 2.);
        ui.painter().galley(text_at, galley, color);
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            chosen = Some(tab.value);
        }
    }
    chosen
}
