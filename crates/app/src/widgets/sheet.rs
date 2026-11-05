//! A bottom sheet: how a popup opens with touch. A menu or panel hung from a button suits a
//! pointer, which put it there and can reach any corner of it; a finger wants the choices at the
//! bottom of the screen, rows tall enough to hit, the page dimmed behind so it's clear what a tap
//! outside does, and a title saying what the list is for (nothing points at its button any more).
//!
//! The sheet rises from the bottom edge, as wide as a phone in portrait and centred on anything
//! wider, its top corners a surface's. It runs under the gesture bar, its contents kept above it,
//! and sits on the soft keyboard while that's up. A handle at the top says it can be pulled down:
//! dragging the handle or the title drags the sheet, and letting go far enough (or with a flick)
//! closes it. A tap on the dimmed page, or Back, closes it too.
//!
//! Whether it's open stays egui's popup memory, as it is for the menu it replaces, so callers
//! open, close and ask about it the same way and `keys::egui_wants_keys` still leaves the keyboard
//! to it. Its contents scroll once they're taller than most of the screen.

use egui::{
    Align2, Color32, CornerRadius, Id, Key, LayerId, Margin, Modifiers, Order, Rect, Sense, Stroke, Ui, pos2, vec2,
};

use crate::layout;
use crate::motion;
use crate::theme::{self, text};

/// The widest a sheet gets: a phone's width in portrait, centred on a landscape phone or tablet.
const MAX_W: f32 = 560.;
/// How much of the screen's height a sheet may take before its contents scroll.
const MAX_SHARE: f32 = 0.85;
/// The handle's size, and the room it's centred in at the top of the sheet.
const HANDLE: egui::Vec2 = vec2(32., 4.);
const HANDLE_ROOM: f32 = 22.;
/// The room under the title, and under the contents above the bottom inset.
const TITLE_GAP: f32 = 8.;
const BOTTOM_PAD: f32 = 12.;
/// How long the sheet takes to rise.
const OPEN: f64 = 0.240;
/// Let go past this share of its height, or moving down faster than this, and it closes.
const DISMISS_SHARE: f32 = 0.3;
const DISMISS_SPEED: f32 = 900.;

/// What a sheet remembers between frames, in egui's temp data.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    /// How far a drag has pulled it down; it eases back when the drag stops short.
    pulled: f32,
}

/// Draws popup `id` as a sheet while it's open, titled `title`, with `pad` of room left and right
/// of `content` and `title_inset` before the title. `content` gets a `Ui` of the sheet's inner
/// width, laid out top-down and justified with no room between rows. The sheet closes itself (a
/// tap beside it, Back, a pull down); the caller closes it when a choice is made. Returns what
/// `content` returned, or `None` while it's closed.
pub fn show<R>(
    ctx: &egui::Context,
    id: Id,
    title: Option<&str>,
    pad: f32,
    title_inset: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if !egui::Popup::is_id_open(ctx, id) {
        return None;
    }
    // Nothing calls `Popup::show` for it, which is what would keep egui from closing it at the
    // end of the pass.
    egui::Popup::open_id(ctx, id);

    let state_id = id.with("sheet_state");
    let mut state = ctx.data(|data| data.get_temp::<State>(state_id).unwrap_or_default());
    let started = motion::appear(ctx, id.with("sheet"));
    let now = ctx.input(|input| input.time);
    let t = if theme::animations(ctx) {
        motion::progress(started, now, 0., OPEN)
    } else {
        1.
    };
    let area_id = id.with("sheet_area");
    if started == now {
        // Opened over a dialog (the library's filters), it goes over it.
        ctx.move_to_top(LayerId::new(Order::Foreground, area_id));
    }

    // Down to the window's bottom edge, under the gesture bar, unless the soft keyboard is up:
    // then it sits on the keyboard.
    let content_rect = ctx.content_rect();
    let bottom = if layout::keyboard_open(ctx) {
        content_rect.bottom()
    } else {
        ctx.viewport_rect().bottom().max(content_rect.bottom())
    };
    let bounds = Rect::from_min_max(content_rect.min, pos2(content_rect.right(), bottom));
    let inset = bottom - content_rect.bottom();
    let width = bounds.width().min(MAX_W);
    let height = ctx
        .memory(|memory| memory.area_rect(area_id))
        .map_or(bounds.height() / 2., |rect| rect.height());

    let tokens = theme::tokens(&ctx.global_style().visuals);
    let radius = tokens.radius_surface;
    let frame = egui::Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1., tokens.surface_border))
        .shadow(theme::popup_shadow(tokens))
        .corner_radius(CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: (inset + BOTTOM_PAD) as i8,
        });
    let lowered = height * (1. - t) + state.pulled;
    let area = egui::Modal::default_area(area_id)
        .anchor(Align2::CENTER_BOTTOM, vec2(0., lowered))
        .constrain_to(bounds)
        .constrain(false);
    // The page dims as the sheet rises, and brightens again as it's pulled away.
    let pulled_share = if height > 0. {
        (state.pulled / height).clamp(0., 1.)
    } else {
        0.
    };
    let backdrop = tokens.scrim.gamma_multiply(t * (1. - pulled_share));

    let mut close = false;
    let mut dragging = false;
    let modal = egui::Modal::new(area_id)
        .area(area)
        .frame(frame)
        .backdrop_color(backdrop)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.;

            // The handle and the title are what a finger pulls the sheet down by.
            let grab_top = ui.cursor().top();
            let (handle_room, _) = ui.allocate_exact_size(vec2(width, HANDLE_ROOM), Sense::hover());
            ui.painter().rect_filled(
                Rect::from_center_size(handle_room.center(), HANDLE),
                CornerRadius::same(2),
                tokens.ink_muted.gamma_multiply(0.5),
            );
            if let Some(title) = title {
                ui.horizontal(|ui| {
                    ui.add_space(title_inset);
                    let title = egui::RichText::new(title)
                        .font(text::font(ui, "lg"))
                        .color(tokens.ink)
                        .strong();
                    ui.add(egui::Label::new(title).truncate().selectable(false));
                });
                ui.add_space(TITLE_GAP);
            }
            let grab = Rect::from_min_max(
                pos2(handle_room.left(), grab_top),
                pos2(handle_room.right(), ui.cursor().top()),
            );
            let grabbed = ui.interact(grab, area_id.with("grab"), Sense::DRAG);
            if grabbed.dragged() {
                dragging = true;
                state.pulled = (state.pulled + grabbed.drag_delta().y).max(0.);
            }
            if grabbed.drag_stopped() {
                let speed = ui.input(|input| input.pointer.velocity().y);
                close = state.pulled > height * DISMISS_SHARE || speed > DISMISS_SPEED;
            }

            let max = (content_rect.height() * MAX_SHARE - (ui.cursor().top() - grab_top)).max(120.);
            egui::ScrollArea::vertical()
                .id_salt(area_id.with("scroll"))
                .max_height(max)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(pad as i8, 0))
                        .show(ui, |ui| {
                            ui.set_width(width - 2. * pad);
                            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                                ui.spacing_mut().item_spacing.y = 0.;
                                content(ui)
                            })
                            .inner
                        })
                        .inner
                })
                .inner
        });

    // Short of closing, a pulled sheet eases back up.
    if !dragging && state.pulled > 0. && !close {
        state.pulled = if theme::animations(ctx) && state.pulled > 0.5 {
            ctx.request_repaint();
            state.pulled * 0.7
        } else {
            0.
        };
    }
    if t < 1. {
        ctx.request_repaint();
    }
    let back = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if close || back || modal.backdrop_response.clicked() {
        egui::Popup::close_id(ctx, id);
        state = State::default();
    }
    ctx.data_mut(|data| data.insert_temp(state_id, state));
    Some(modal.inner)
}

/// A line across a sheet, edge to edge past the `pad` its rows have either side.
pub fn separator(ui: &mut Ui, pad: f32, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 17.), Sense::hover());
    ui.painter().hline(
        (rect.left() - pad)..=(rect.right() + pad),
        rect.center().y,
        Stroke::new(1., colour),
    );
}
