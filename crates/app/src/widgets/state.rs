//! What a page shows besides its content: the mascot while it loads, a line of text, an empty
//! state, and a failed load with *Retry*.
//!
//! The mascot always sits in a small glow (`backdrop::glow`, the sign-in glow scaled to the
//! page mascot), since the glow goes with the mascot. An empty page and a failed load show it
//! confused; a failed load then has a danger notice and a `brand` *Retry*.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2, vec2};

use crate::theme::{self, text};
use crate::widgets::button::{self, Focus, State};
use crate::widgets::{backdrop, mascot, surface};

/// The glow's radius behind the page mascot: the sign-in glow (360 behind a 160 mascot) at a
/// smaller spread, picked by the C8 mockups.
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
    centred(ui, |ui| glowing(ui, |ui| mascot::loader(ui, mascot::PAGE)));
}

/// A centred line of muted text, for a note that isn't an empty page: "Type in the search box
/// to search.", or a failed load with nothing to retry.
pub fn message(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    centred(ui, |ui| {
        ui.label(line(ui, message));
    });
}

/// A page with nothing on it: the confused mascot in its glow over the message. It greets for a
/// few seconds and then holds still (see `widgets::mascot`), so an empty page is idle.
pub fn empty(ui: &mut Ui, message: impl Into<String>) {
    let message = message.into();
    centred(ui, |ui| {
        glowing(ui, |ui| mascot::confused(ui, mascot::PAGE));
        ui.add_space(GAP);
        ui.label(line(ui, message));
    });
}

/// A failed load: the confused mascot in its glow, the message in a danger notice, and a
/// `brand` *Retry* under it. Returns whether Retry was pressed; the page loads again when it was.
pub fn error(ui: &mut Ui, message: impl Into<String>) -> bool {
    let message = message.into();
    let mut retry = false;
    centred(ui, |ui| {
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
    backdrop::glow(
        ui.painter(),
        square.center() + vec2(0., GLOW_DROP),
        GLOW_RADIUS,
        theme::tokens_of(ui),
    );
    let mut mascot = ui.new_child(
        UiBuilder::new()
            .max_rect(square)
            .layout(Layout::top_down(Align::Center)),
    );
    draw(&mut mascot);
}

/// Draws `contents` as one block in the middle of the space the `ui` has left and centred
/// across it. The block's height is last frame's (a wrapped notice's depends on the width), and
/// a frame that measures a different one is drawn again rather than shown off-centre.
fn centred(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    let id = ui.scope_id().with("state-height");
    let height = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.);
    ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Center)), |ui| {
        ui.add_space(((ui.available_height() - height) * 0.5).max(0.));
        let top = ui.cursor().top();
        contents(ui);
        let measured = ui.min_rect().bottom() - top;
        if (measured - height).abs() > 0.5 {
            ui.data_mut(|data| data.insert_temp(id, measured));
            ui.ctx().request_discard("a page state measured its height");
        }
    });
}
