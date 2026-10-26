//! A titled row of cards that scrolls sideways, with a page back and forward: `main`'s shelves.
//!
//! `id` only has to be unique among the page's own rows and scroll areas: the shell scopes every
//! page's ids, so two pages can both have a `"cast"` shelf and keep their scroll positions apart.
//!
//! The arrows show once there's more to see than fits and grey out at either end. They need the
//! row's content and viewport widths, which egui only knows after laying it out, so they come
//! from the last frame (as they did on `main`) and the first frame shows none.
//!
//! `main`'s `rail_drag.rs` and `smooth_scroll.rs` aren't needed: egui's [`ScrollArea`] drags its
//! contents with the mouse, and a drag never counts as a click on the card it ends on. The wheel
//! keeps scrolling the page rather than the row (egui only scrolls the one enabled direction
//! with Shift), as on `main`; the arrows jump instead of gliding until R5.

use std::hash::Hash;

use egui::containers::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::{Align, Button, Layout, RichText, ScrollArea, Ui};
use egui_material_icons::icons::{ICON_CHEVRON_LEFT, ICON_CHEVRON_RIGHT};

use crate::widgets::text::CARD_GAP;

/// How far under its title a row's cards start (`main`'s `shelf`).
const TITLE_GAP: f32 = 16.;

/// A titled row of cards, [`CARD_GAP`] apart.
pub fn shelf(ui: &mut Ui, id: impl Hash + std::fmt::Debug, title: &str, cards: impl FnOnce(&mut Ui)) {
    shelf_with_gaps(ui, id, title, CARD_GAP, TITLE_GAP, cards);
}

/// [`shelf`] with a different space between the cards and under the title (Home's shelves are
/// wider apart than the item pages').
pub fn shelf_with_gaps(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    gap: f32,
    title_gap: f32,
    cards: impl FnOnce(&mut Ui),
) {
    let id = ui.make_persistent_id(id);
    let state: State = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    // One page is what shows at once, so no card is ever skipped.
    let page = state.viewport;
    let arrows = Arrows::of(state);
    // Where an arrow asks the row to go; `None` leaves it where the mouse or keys left it.
    let mut target = None;
    ui.horizontal(|ui| {
        // As tall as the arrows, so the row keeps its height when they hide.
        ui.set_min_height(ui.spacing().interact_size.y);
        ui.label(RichText::new(title).heading().strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !arrows.shown {
                return;
            }
            let next = ui.add_enabled(!arrows.at_end, arrow(ICON_CHEVRON_RIGHT));
            if next.clicked() {
                target = Some(state.offset + page);
            }
            next.on_hover_text("Scroll right")
                .on_disabled_hover_text("Scroll right");
            let previous = ui.add_enabled(!arrows.at_start, arrow(ICON_CHEVRON_LEFT));
            if previous.clicked() {
                target = Some(state.offset - page);
            }
            previous
                .on_hover_text("Scroll left")
                .on_disabled_hover_text("Scroll left");
        });
    });
    ui.add_space(title_gap);

    let mut area = ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        // The arrows show where the row is, as on `main`; a bar under every shelf would not.
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        // `main`'s rail dragging: a press on the row and a move scrolls it with the mouse.
        .scroll_source(ScrollSource {
            drag: DragScroll::Always,
            ..Default::default()
        })
        // The closed hand `main` showed while a row was being dragged.
        .on_drag_cursor(egui::CursorIcon::Grabbing);
    if let Some(target) = target {
        // Clamped here: egui would take the overshoot and spring back, which reads as the row
        // jumping past its end.
        area = area.horizontal_scroll_offset(target.clamp(0., (state.content - state.viewport).max(0.)));
    }
    let out = area.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            cards(ui);
        });
    });
    let measured = State {
        offset: out.state.clamped_offset().x,
        content: out.content_size.x,
        viewport: out.inner_rect.width(),
    };
    if measured != state {
        // The arrows were drawn from the previous frame's widths: a row that just grew, or that
        // a key or a flung drag scrolled to an end, needs one more frame to show or grey them.
        // Only a change asks for it, so an idle row costs nothing.
        ui.ctx().request_repaint();
    }
    ui.data_mut(|data| data.insert_temp(id, measured));
}

/// A page button, small and frameless.
fn arrow(icon: egui_material_icons::MaterialIcon) -> Button<'static> {
    Button::new(icon.outlined())
        .small()
        .frame(false)
        .min_size(egui::vec2(24., 24.))
}

/// The row's scroll offset and widths, from the last frame it was drawn.
#[derive(Clone, Copy, Default, PartialEq)]
struct State {
    offset: f32,
    content: f32,
    viewport: f32,
}

/// Which page arrows to draw: none when the cards fit, and each one greyed out at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Arrows {
    shown: bool,
    at_start: bool,
    at_end: bool,
}

impl Arrows {
    fn of(state: State) -> Self {
        // Half a point of slack: egui's offsets are fractional.
        Self {
            shown: state.content > state.viewport + 0.5,
            at_start: state.offset <= 0.5,
            at_end: state.offset + state.viewport >= state.content - 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Arrows, State};

    fn arrows(offset: f32, content: f32, viewport: f32) -> Arrows {
        Arrows::of(State {
            offset,
            content,
            viewport,
        })
    }

    #[test]
    fn no_arrows_while_the_cards_fit() {
        let fits = arrows(0., 1000., 1200.);
        assert!(!fits.shown);
        // A row exactly as wide as the viewport has nothing to page.
        assert!(!arrows(0., 1200., 1200.).shown);
    }

    #[test]
    fn arrows_grey_out_at_each_end() {
        let start = arrows(0., 2000., 1200.);
        assert!(start.shown);
        assert!(start.at_start);
        assert!(!start.at_end);
        let middle = arrows(400., 2000., 1200.);
        assert!(!middle.at_start);
        assert!(!middle.at_end);
        // A page past the end is clamped by the scroll area, which lands here.
        let end = arrows(800., 2000., 1200.);
        assert!(!end.at_start);
        assert!(end.at_end);
    }
}
