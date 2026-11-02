//! A pill-shaped button that counts something down: its label, and a ring at the right end that
//! closes round a mark as the time runs out. The player's skip button is one, so the ring says
//! how long is left to skip, and so is the up-next card's *Play now* while autoplay counts down
//! (without autoplay it keeps its place for the ring but draws only the mark). It takes no
//! keyboard focus, as the player's keys are `keys.rs`'s.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::accesskit::Role;
use egui::{Color32, CursorIcon, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;

use crate::theme::text;
use crate::widgets::round_button;

const HEIGHT: f32 = 44.;
/// The room left of the label, and between the label and the ring.
const PAD_LEFT: f32 = 20.;
const GAP: f32 = 12.;
/// The ring's radius, its stroke, and the room between it and the pill's right end.
const RING_RADIUS: f32 = 14.;
const RING_WIDTH: f32 = 3.;
const PAD_RIGHT: f32 = 8.;
const ICON: f32 = 16.;
/// Straight lines in a full ring: enough for it to read as a curve at the render's scale.
const RING_STEPS: f32 = 64.;

/// The colours of a pill, which are the caller's since a pill may sit over the video.
#[derive(Clone, Copy)]
pub struct Colours {
    pub fill: Color32,
    pub hover: Color32,
    /// The label, the mark and the ring's closed part.
    pub ink: Color32,
    /// The ring's open part.
    pub track: Color32,
}

/// Adds the pill to `ui`, with `fraction` of its ring closed, from the top clockwise, or with
/// no ring for `None`.
pub fn show(ui: &mut Ui, label: &str, icon: MaterialIcon, fraction: Option<f32>, colours: Colours) -> Response {
    let font = text::font(ui, "base");
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, colours.ink);
    let width = PAD_LEFT + galley.size().x + GAP + 2. * RING_RADIUS + RING_WIDTH + PAD_RIGHT;
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::CLICK);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    let fill = if response.hovered() {
        colours.hover
    } else {
        colours.fill
    };
    ui.painter().rect_filled(rect, (HEIGHT / 2.) as u8, fill);
    let text_at = pos2(rect.left() + PAD_LEFT, rect.center().y - galley.size().y / 2.);
    ui.painter().galley(text_at, galley, colours.ink);
    ring(ui, rect, fraction, icon, colours);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn ring(ui: &Ui, rect: Rect, fraction: Option<f32>, icon: MaterialIcon, colours: Colours) {
    let center = pos2(
        rect.right() - PAD_RIGHT - RING_WIDTH / 2. - RING_RADIUS,
        rect.center().y,
    );
    // The track is a line like the arc rather than a `circle_stroke`, which egui draws outside
    // its radius: the arc, centred on it, would run 1.5 points inside the track.
    let painter = ui.painter();
    if let Some(fraction) = fraction {
        let track = Stroke::new(RING_WIDTH, colours.track);
        painter.add(Shape::closed_line(arc(center, 1.), track));
        let fraction = fraction.clamp(0., 1.);
        if fraction > 0. {
            painter.add(Shape::line(arc(center, fraction), Stroke::new(RING_WIDTH, colours.ink)));
        }
    }
    round_button::glyph(ui, center, icon, ICON, colours.ink);
}

/// The points of `fraction` of the ring round `center`, clockwise from the top.
fn arc(center: Pos2, fraction: f32) -> Vec<Pos2> {
    let steps = (RING_STEPS * fraction).ceil() as usize;
    (0..=steps)
        .map(|step| {
            let angle = TAU * fraction * step as f32 / steps as f32 - FRAC_PI_2;
            center + RING_RADIUS * vec2(angle.cos(), angle.sin())
        })
        .collect()
}

/// How far `position` is through a span from `start` to `end`, for a pill's ring.
pub fn progress(position: f64, start: f64, end: f64) -> f32 {
    if end <= start {
        return 1.;
    }
    (((position - start) / (end - start)) as f32).clamp(0., 1.)
}
