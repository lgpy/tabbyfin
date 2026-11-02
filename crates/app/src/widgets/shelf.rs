//! A titled row of cards that scrolls sideways, with a page back and forward: `main`'s shelves.
//!
//! `id` only has to be unique among the page's own rows and scroll areas: the shell scopes every
//! page's ids, so two pages can both have a `"cast"` shelf and keep their scroll positions apart.
//!
//! The title is `xl` in `ink`. The arrows are round buttons at the title row's right end; they
//! show once there's more to see than fits and dim at either end. They need the
//! row's content and viewport widths, which egui only knows after laying it out, so they come
//! from the last frame (as they did on `main`) and the first frame shows none. An arrow glides
//! the row a page over, as `main`'s `glide_by` did: a second click while it moves aims further,
//! and with the animations setting off it jumps.
//!
//! The row is `main`'s `rail_drag.rs` on egui: the wheel keeps scrolling the page (the row takes
//! none of it), a press and a move drags the row with the mouse and the closed hand shows while
//! it moves, a drag doesn't open the card it ends on, and the row follows the pointer 1:1 and
//! stops with it (no egui coasting after the release, no rubber band past the ends, which `main`
//! had neither of). The keyboard's `scroll_to_me` still brings a card into view, through egui's
//! own scroll animation.

use std::hash::Hash;

use egui::containers::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::style::KineticScrollStyle;
use egui::{Align, Layout, Rect, RichText, ScrollArea, Ui};
use egui_material_icons::icons::{ICON_CHEVRON_LEFT, ICON_CHEVRON_RIGHT};

use crate::smooth_scroll::Glide;
use crate::theme;
use crate::theme::text;
use crate::widgets::round_button::RoundButton;

/// A shelf's side padding, `main`'s `px_6`: the page adds none of its own, so the row can
/// scroll a card to the window's edge. Item pages that already inset their content pass 0.
pub const PADDING: f32 = 24.;
/// Under the row, `main`'s `pb_2`, so the cards keep clear of the next shelf.
const BOTTOM_PADDING: f32 = 8.;
/// The title row's height, the arrows' size, so the shelf keeps it when they hide.
const TITLE_H: f32 = ARROW;
/// The page arrows' diameter and their icon's size.
const ARROW: f32 = 32.;
const ARROW_ICON: f32 = 20.;

/// A titled row of cards, with the space between its cards and under its title given (Home's
/// shelves are wider apart than the item pages'), inset `padding` from the page's edges. Returns
/// the row's rectangle, for a page that scrolls its shelves into view as the keyboard cursor
/// moves onto them.
pub fn shelf_with_gaps(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    gap: f32,
    title_gap: f32,
    padding: f32,
    cards: impl FnOnce(&mut Ui),
) -> Rect {
    shelf_inner(ui, id, title, None, gap, title_gap, padding, cards)
}

/// [`shelf_with_gaps`] with a title that is a link: `opened` is set when it's clicked, for a
/// row that is a page's front (Home's "Latest Movies" leads to the library).
#[allow(clippy::too_many_arguments)]
pub fn shelf_linked(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    opened: &mut bool,
    gap: f32,
    title_gap: f32,
    padding: f32,
    cards: impl FnOnce(&mut Ui),
) -> Rect {
    shelf_inner(ui, id, title, Some(opened), gap, title_gap, padding, cards)
}

#[allow(clippy::too_many_arguments)]
fn shelf_inner(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    opened: Option<&mut bool>,
    gap: f32,
    title_gap: f32,
    padding: f32,
    cards: impl FnOnce(&mut Ui),
) -> Rect {
    // The row's top before the title takes its place: a page scrolls the whole row into view
    // (its title and cards together), as `main`'s `browse::reveal` scrolled a whole section.
    let top = ui.cursor().min;
    let id = ui.make_persistent_id(id);
    let mut state: State = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let now = ui.input(|input| input.time);
    let animations = theme::animations(ui.ctx());
    // One page is what shows at once, so no card is ever skipped. The arrows move what shows
    // between the row's side paddings, as `main`'s did.
    let page = (state.viewport - 2. * padding).max(0.);
    let max = (state.content - state.viewport).max(0.);
    let arrows = Arrows::of(state);
    // How far an arrow asks the row to go this frame.
    let mut distance = 0.;
    ui.horizontal(|ui| {
        // As tall as the arrows, so the row keeps its height when they hide.
        ui.set_min_height(TITLE_H);
        if padding > 0. {
            ui.add_space(padding);
        }
        match opened {
            Some(opened) => {
                let heading = text::font(ui, "xl");
                let tokens = theme::tokens_of(ui);
                // A chevron after the title says it leads somewhere. It's the icon font's
                // outlined chevron, the row's own paging arrows' glyph, centred on the title's
                // line and a little smaller, so its stroke stays as light as the heading's.
                let centred = |font_id, color| egui::text::TextFormat {
                    font_id,
                    color,
                    valign: Align::Center,
                    ..Default::default()
                };
                let mut job = egui::text::LayoutJob::default();
                job.append(title, 0., centred(heading.clone(), tokens.ink));
                job.append(
                    ICON_CHEVRON_RIGHT.codepoint,
                    2.,
                    centred(
                        egui::FontId::new(heading.size * 0.9, ICON_CHEVRON_RIGHT.outlined().font_family()),
                        tokens.muted_foreground,
                    ),
                );
                let response = ui
                    .add(egui::Label::new(job).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    // The underline says it's a link.
                    let rect = response.rect;
                    let y = rect.bottom() - 2.;
                    let stroke = egui::Stroke::new(1., ui.visuals().text_color());
                    ui.painter().hline(rect.x_range(), y, stroke);
                }
                *opened |= response.clicked();
            }
            None => {
                ui.label(
                    RichText::new(title)
                        .font(text::font(ui, "xl"))
                        .color(theme::tokens_of(ui).ink),
                );
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !arrows.shown {
                return;
            }
            if padding > 0. {
                ui.add_space(padding);
            }
            ui.spacing_mut().item_spacing.x = 4.;
            if arrow(ICON_CHEVRON_RIGHT, "Scroll right", !arrows.at_end)
                .show(ui)
                .clicked()
            {
                distance = page;
            }
            if arrow(ICON_CHEVRON_LEFT, "Scroll left", !arrows.at_start)
                .show(ui)
                .clicked()
            {
                distance = -page;
            }
        });
    });
    ui.add_space(title_gap);

    // `main`'s glide: a click starts one (or aims a running one further), and each frame of one
    // moves the row on. `main` also stopped when something else had scrolled the row since its
    // last step (`glide.last_set`); that check comes after the area is drawn, where egui has
    // applied a drag or the keyboard.
    let mut jump = None;
    if distance != 0. {
        if animations {
            match &mut state.glide {
                Some(glide) if glide.last_set == state.offset => glide.aim(distance, max),
                _ => state.glide = Some(Glide::new(state.offset, distance, max, now)),
            }
        } else {
            // `main` jumped with reduced motion (the animations setting).
            state.glide = None;
            jump = Some((state.offset + distance).clamp(0., max));
        }
    } else if !animations {
        state.glide = None;
    }
    let mut offset = jump;
    if let Some(glide) = &mut state.glide {
        let (next, done) = glide.step(now);
        offset = Some(next.clamp(0., max));
        if done {
            state.glide = None;
        }
    }

    let mut area = ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        // The arrows show where the row is, as on `main`; a bar under every shelf would not.
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        // `main`'s rail dragging: a press on the row and a move scrolls it with the mouse. The
        // wheel is the page's, as on `main`, where the row undid the sideways scroll the toolkit
        // gave it.
        .scroll_source(ScrollSource {
            drag: DragScroll::Always,
            mouse_wheel: false,
            scroll_bar: false,
        })
        // The closed hand `main` showed while a row was being dragged.
        .on_drag_cursor(egui::CursorIcon::Grabbing);
    if let Some(offset) = offset {
        // Clamped here: egui would take the overshoot and spring back, which reads as the row
        // jumping past its end.
        area = area.horizontal_scroll_offset(offset);
    }
    // The row follows the pointer 1:1 and stops with it, as `main`'s rail drag did: no coasting
    // after a release, no rubber band past the ends (which are egui's defaults).
    let kinetic = ui.style().spacing.scroll.kinetic;
    ui.style_mut().spacing.scroll.kinetic = KineticScrollStyle {
        decay_time: 0.,
        rubber_band: false,
        ..kinetic
    };
    let out = area.show(ui, |ui| {
        // `main`'s `px_6` is inside the row, so a card scrolls to the window's edge and the
        // arrows' page is what shows between the paddings.
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(padding as i8, 0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    cards(ui);
                });
            });
    });
    ui.style_mut().spacing.scroll.kinetic = kinetic;
    let mut measured = State {
        offset: out.state.clamped_offset().x,
        content: out.content_size.x,
        viewport: out.inner_rect.width(),
        glide: state.glide,
    };
    if measured
        .glide
        .is_some_and(|glide| (glide.last_set - measured.offset).abs() > 0.01)
    {
        // A drag, a scroll bar or the keyboard's reveal moved the row: the glide stops there, as
        // `main`'s did when the offset wasn't the one it had set.
        measured.glide = None;
    }
    if measured != state || measured.glide.is_some() {
        // The arrows were drawn from the previous frame's widths: a row that just grew, or that
        // a key or a flung drag scrolled to an end, needs one more frame to show or grey them.
        // A running glide needs each frame too. Only a change or a glide asks for it, so an idle
        // row costs nothing.
        ui.ctx().request_repaint();
    }
    ui.data_mut(|data| data.insert_temp(id, measured));
    // `main`'s `pb_2` under the row.
    ui.add_space(BOTTOM_PADDING);
    // The row's own height, not its viewport's: the viewport is clipped to what's on screen, so
    // a row half below the page's edge would look like it ends at the edge and never scroll.
    Rect::from_min_max(top, egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()))
}

/// A page button: a round button, dimmed at its end of the row.
fn arrow(icon: egui_material_icons::MaterialIcon, label: &str, enabled: bool) -> RoundButton<'_> {
    RoundButton::icon(icon.outlined(), label)
        .size(ARROW, ARROW_ICON)
        .enabled(enabled)
}

/// The row's scroll offset and widths, from the last frame it was drawn.
#[derive(Clone, Copy, Default, PartialEq)]
struct State {
    offset: f32,
    content: f32,
    viewport: f32,
    /// The arrows' glide, while it runs.
    glide: Option<Glide>,
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
            ..Default::default()
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
