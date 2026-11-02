//! What a page shows besides its content: the mascot while it loads, a line of text, an empty
//! state, and a failed load with *Retry*.
//!
//! The mascot always sits in a small glow (`backdrop::glow`, the sign-in glow scaled to the
//! page mascot), since the glow goes with the mascot. An empty page and a failed load show it
//! confused; a failed load then has a danger notice and a `brand` *Retry*.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2, vec2};
use egui_material_icons::MaterialIcon;

use crate::theme::{self, text};
use crate::widgets::button::{self, Focus, State};
use crate::widgets::{backdrop, label, mascot, round_button, surface};

/// The glow's radius behind the page mascot: the sign-in glow (360 behind a 160 mascot) at a
/// smaller spread.
const GLOW_RADIUS: f32 = 200.;

/// How far below the mascot's centre the glow is centred, so it pools under the mascot as on
/// sign-in.
const GLOW_DROP: f32 = 18.;

/// The gap under the mascot, before its line or the notice.
const GAP: f32 = 16.;

/// The widest a failed load's notice gets, so a long message wraps to a block under the mascot
/// rather than a line across the page.
const NOTICE_WIDTH: f32 = 420.;

/// The *Retry* button's width.
const RETRY_WIDTH: f32 = 120.;

/// The mascot in its glow, centred, for a page that's still loading.
pub fn spinner(ui: &mut Ui) {
    centred(ui, true, |ui| glowing(ui, |ui| mascot::loader(ui, mascot::PAGE)));
}

/// A centred line of muted text, for a note that isn't an empty page: "Type in the search box
/// to search.", or a failed load with nothing to retry.
pub fn message(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    centred(ui, false, |ui| {
        ui.label(line(ui, message));
    });
}

/// A page with nothing on it: the confused mascot in its glow over the message. It greets for a
/// few seconds and then holds still (see `widgets::mascot`), so an empty page is idle.
pub fn empty(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    centred(ui, true, |ui| {
        glowing(ui, |ui| mascot::confused(ui, mascot::PAGE));
        ui.add_space(GAP);
        ui.label(line(ui, message));
    });
}

/// A compact empty state without the mascot, for a panel or a popover: `icon` in an `inset_hover`
/// circle over `message` in `label::muted`, centred.
pub fn icon_message(ui: &mut Ui, icon: MaterialIcon, message: &str) {
    let tokens = theme::tokens_of(ui);
    ui.vertical_centered(|ui| {
        ui.add_space(theme::SPACE.sm);
        let (rect, _) = ui.allocate_exact_size(vec2(48., 48.), Sense::hover());
        ui.painter().circle_filled(rect.center(), 24., tokens.inset_hover);
        round_button::glyph(ui, rect.center(), icon, 26., tokens.ink_muted);
        ui.add_space(theme::SPACE.sm);
        ui.label(label::muted(ui, message));
        ui.add_space(theme::SPACE.sm);
    });
}

/// A failed load: the confused mascot in its glow, the message in a danger notice, and a
/// `brand` *Retry* under it. Returns whether Retry was pressed; the page loads again when it was.
pub fn error(ui: &mut Ui, message: impl Into<String>) -> bool {
    let message = message.into();
    let mut retry = false;
    centred(ui, true, |ui| {
        glowing(ui, |ui| mascot::confused(ui, mascot::PAGE));
        ui.add_space(GAP);
        let width = NOTICE_WIDTH.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(width, 0.), Layout::top_down(Align::Center), |ui| {
            ui.set_width(width);
            surface::notice(ui, surface::Kind::Danger, &message);
            ui.add_space(surface::GAP_FIELD);
            retry = button::brand(ui, "Retry", RETRY_WIDTH, State::Ready, Focus::Tab).clicked();
        });
    });
    retry
}

/// The state's line: `base` in `ink_muted`.
fn line(ui: &Ui, message: String) -> RichText {
    RichText::new(message)
        .font(text::font(ui, "base"))
        .color(theme::tokens_of(ui).ink_muted)
}

/// The page mascot drawn by `draw` in the middle of a row, with the glow behind it.
fn glowing(ui: &mut Ui, draw: impl FnOnce(&mut Ui)) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), mascot::PAGE), Sense::hover());
    let square = Rect::from_center_size(row.center(), Vec2::splat(mascot::PAGE));
    let tokens = theme::tokens_of(ui);
    let glow_center = square.center() + vec2(0., GLOW_DROP);
    backdrop::glow(ui.painter(), glow_center, GLOW_RADIUS, tokens);
    let mut mascot = ui.new_child(
        UiBuilder::new()
            .max_rect(square)
            .layout(Layout::top_down(Align::Center)),
    );
    // In a fade (a page coming in), the mascot is drawn whole and a veil of what's under it fades
    // it as one piece: faded part by part, its overlapping parts would stack up brighter. The glow
    // is one mesh with nothing overlapping, so it fades as it is.
    let fade = ui.painter().opacity();
    let veiled = fade > 0. && fade < 1.;
    if veiled {
        mascot.set_opacity(1.);
    }
    draw(&mut mascot);
    if veiled {
        let mut painter = ui.painter().clone();
        painter.set_opacity(1.);
        painter.add(backdrop::veil(
            square,
            ui.ctx().content_rect(),
            (glow_center, GLOW_RADIUS, fade),
            1. - fade,
            tokens,
        ));
    }
}

/// Draws `contents` as one block in the space the `ui` has left, centred across it. Without a
/// mascot the block sits in the middle; with one (`has_mascot`, the block's top) the mascot does, and
/// what's under it hangs below, so it stays put going from the loader to an empty page or a
/// failed load. The block moves up only as far as it must to fit. Its height is last frame's (a
/// wrapped notice's depends on the width), and a frame that measures a different one is drawn
/// again rather than shown off-centre.
fn centred(ui: &mut Ui, has_mascot: bool, contents: impl FnOnce(&mut Ui)) {
    let id = ui.scope_id().with("state-height");
    let height = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.);
    ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Center)), |ui| {
        let room = ui.available_height();
        let top = if has_mascot {
            ((room - mascot::PAGE) * 0.5).min(room - height)
        } else {
            (room - height) * 0.5
        };
        ui.add_space(top.max(0.));
        let top = ui.cursor().top();
        contents(ui);
        let measured = ui.min_rect().bottom() - top;
        if (measured - height).abs() > 0.5 {
            ui.data_mut(|data| data.insert_temp(id, measured));
            ui.ctx().request_discard("a page state measured its height");
        }
    });
}
