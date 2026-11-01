//! The cozy look's window background: a gradient from [`Tokens::backdrop_top`] to
//! [`Tokens::backdrop_bottom`], and the soft light pooled behind the mascot. It's under the
//! sign-in screen and under the whole shell (the top bar and every page); the bar has no fill of
//! its own, so a page that scrolls under it draws [`scroll_edge`].
//!
//! egui has no gradients, so these are meshes with a colour per vertex, which the GPU blends.

use std::f32::consts::TAU;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Stroke, Ui, vec2};

use crate::theme::Tokens;

/// Points around each ring of the glow: enough that its edge never shows a corner.
const GLOW_STEPS: u32 = 96;
/// Rings from the centre out. Each steps the alpha down a smooth curve, so the light fades
/// without the visible bands that a few stacked translucent circles leave.
const GLOW_RINGS: u32 = 10;
/// How much flatter the glow is than a circle: light spreading over a surface.
const GLOW_SQUASH: f32 = 0.85;

/// How far a page scrolls before [`scroll_edge`] is fully drawn: it comes in with the first
/// points of a scroll rather than appearing at once.
const EDGE_IN: f32 = 12.;

/// Fills `rect` with the backdrop's gradient.
pub fn fill(painter: &Painter, rect: Rect, tokens: &Tokens) {
    vertical(painter, rect, tokens.backdrop_top, tokens.backdrop_bottom);
}

/// The backdrop's colour at height `y` of `window`, the rectangle [`fill`] painted: what
/// lies under that point of the page, for something that fades into it. Vertex colours blend
/// in gamma space, so this does too.
pub fn color_at(window: Rect, y: f32, tokens: &Tokens) -> Color32 {
    let t = ((y - window.top()) / window.height().max(1.)).clamp(0., 1.);
    tokens.backdrop_top.lerp_to_gamma(tokens.backdrop_bottom, t)
}

/// Covers `rect` with the backdrop as it is at that height of `window`, at `top` opacity at
/// its top edge and opaque at its bottom: a picture that melts into the page below it (an
/// item's banner). The backdrop is a gradient, so a flat colour would leave a step.
pub fn fade_into(painter: &Painter, rect: Rect, window: Rect, top: f32, tokens: &Tokens) {
    let upper = color_at(window, rect.top(), tokens).gamma_multiply(top);
    let lower = color_at(window, rect.bottom(), tokens);
    vertical(painter, rect, upper, lower);
}

/// The hairline between the top bar (or anything fixed above a page) and content scrolled
/// under it: nothing while `area` is at its top, then a 1 pt `surface_border` line across the
/// area's top edge, coming in over the first [`EDGE_IN`] points of a scroll. The bar has no fill
/// of its own, so without it a scrolled page's rows would look cut off at an invisible line.
/// Call it after the scroll area, with the `Ui` it was shown in, so the line is over the content.
pub fn scroll_edge<R>(ui: &Ui, area: &egui::scroll_area::ScrollAreaOutput<R>) {
    let shown = edge_opacity(area.state.clamped_offset().y);
    if shown <= 0. {
        return;
    }
    let color = crate::theme::tokens_of(ui).surface_border.gamma_multiply(shown);
    let rect = area.inner_rect;
    ui.painter()
        .hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1., color));
}

/// How much of [`scroll_edge`]'s line shows with the page scrolled `offset` points down.
fn edge_opacity(offset: f32) -> f32 {
    (offset / EDGE_IN).clamp(0., 1.)
}

/// A vertical gradient over `rect`, `top` to `bottom`.
fn vertical(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// The glow around `center`, fading out at `radius`.
pub fn glow(painter: &Painter, center: Pos2, radius: f32, tokens: &Tokens) {
    let mut mesh = Mesh::default();
    for ring in 0..=GLOW_RINGS {
        let at = ring as f32 / GLOW_RINGS as f32;
        // (1 - t²)³: bright in the middle, and so flat as it reaches zero that the outer rings'
        // alpha rounds to a step of 1 at most, which a light backdrop would otherwise show as
        // a faint edge.
        let falloff = (1. - at * at).powi(3);
        let color = tokens.glow.gamma_multiply(falloff);
        for step in 0..GLOW_STEPS {
            let angle = step as f32 / GLOW_STEPS as f32 * TAU;
            let offset = vec2(angle.cos(), angle.sin() * GLOW_SQUASH) * radius * at;
            mesh.colored_vertex(center + offset, color);
        }
    }
    for ring in 0..GLOW_RINGS {
        for step in 0..GLOW_STEPS {
            let a = ring * GLOW_STEPS + step;
            let b = ring * GLOW_STEPS + (step + 1) % GLOW_STEPS;
            mesh.add_triangle(a, b, a + GLOW_STEPS);
            mesh.add_triangle(b, b + GLOW_STEPS, a + GLOW_STEPS);
        }
    }
    painter.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_comes_in_over_the_first_points_of_a_scroll() {
        assert_eq!(edge_opacity(0.), 0.);
        assert_eq!(edge_opacity(EDGE_IN / 2.), 0.5);
        assert_eq!(edge_opacity(EDGE_IN), 1.);
        assert_eq!(edge_opacity(500.), 1.);
    }

    #[test]
    fn the_backdrop_colour_runs_from_top_to_bottom() {
        let tokens = crate::theme::tokens(&egui::Visuals::dark());
        let window = Rect::from_min_size(Pos2::ZERO, vec2(100., 200.));
        assert_eq!(color_at(window, 0., tokens), tokens.backdrop_top);
        assert_eq!(color_at(window, 200., tokens), tokens.backdrop_bottom);
        // Past either end it holds the end's colour.
        assert_eq!(color_at(window, -50., tokens), tokens.backdrop_top);
        assert_eq!(color_at(window, 900., tokens), tokens.backdrop_bottom);
    }
}
