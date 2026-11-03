//! The cozy look's window background: a gradient from [`Tokens::backdrop_top`] to
//! [`Tokens::backdrop_bottom`], and the soft light pooled behind the mascot. It's under the
//! sign-in screen and under the whole shell (the top bar and every page); the bar has no fill of
//! its own, so a page that scrolls under it draws [`scroll_edge`].
//!
//! egui has no gradients, so these are meshes with a colour per vertex, which the GPU blends.

use std::f32::consts::TAU;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, vec2};

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

/// Fills the whole window with the backdrop, under the system's bars and cutouts too, which
/// egui's safe area (Android's insets) keeps the panels out of. The same as [`fill`] on
/// `ui.max_rect()` where there's no safe area.
pub fn fill_window(ui: &Ui, tokens: &Tokens) {
    let window = ui.ctx().viewport_rect();
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(window);
    fill(&painter, window, tokens);
}

/// The backdrop's colour at height `y` of `window`, the rectangle [`fill`] painted: what
/// lies under that point of the page, for something that fades into it. Vertex colours blend
/// in gamma space, so this does too.
pub fn color_at(window: Rect, y: f32, tokens: &Tokens) -> Color32 {
    let t = ((y - window.top()) / window.height().max(1.)).clamp(0., 1.);
    tokens.backdrop_top.lerp_to_gamma(tokens.backdrop_bottom, t)
}

/// A shape covering `rect` with the backdrop as it is at that height of `window`, at `top` opacity at
/// its top edge and opaque at its bottom: a picture that melts into the page below it (an
/// item's banner). The backdrop is a gradient, so a flat colour would leave a step.
pub fn fade_into(rect: Rect, window: Rect, top: f32, tokens: &Tokens) -> Shape {
    wash(rect, window, top, 1., tokens)
}

/// A shape covering `rect` with the backdrop as it is at that height of `window`, at `top`
/// opacity at its top edge and `bottom` at its bottom: a veil of the window's colour over a
/// picture, so what's drawn on it stays readable (the top bar over an item's banner).
pub fn wash(rect: Rect, window: Rect, top: f32, bottom: f32, tokens: &Tokens) -> Shape {
    let upper = color_at(window, rect.top(), tokens).gamma_multiply(top);
    let lower = color_at(window, rect.bottom(), tokens).gamma_multiply(bottom);
    gradient(rect, upper, lower)
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
    painter.add(gradient(rect, top, bottom));
}

/// [`vertical`]'s mesh.
fn gradient(rect: Rect, top: Color32, bottom: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// The glow around `center`, fading out at `radius`.
pub fn glow(painter: &Painter, center: Pos2, radius: f32, tokens: &Tokens) {
    let mut mesh = Mesh::default();
    for ring in 0..=GLOW_RINGS {
        let at = ring as f32 / GLOW_RINGS as f32;
        let color = tokens.glow.gamma_multiply(ring_falloff(at));
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

/// A ring's brightness `at` its share of the glow's radius: (1 - t²)³, bright in the middle,
/// and so flat as it reaches zero that the outer rings' alpha rounds to a step of 1 at most,
/// which a light backdrop would otherwise show as a faint edge.
fn ring_falloff(at: f32) -> f32 {
    (1. - at * at).powi(3)
}

/// How bright [`glow`]'s mesh is at `point`: its rings' falloff, interpolated between them as
/// the GPU does.
fn glow_falloff(center: Pos2, radius: f32, point: Pos2) -> f32 {
    let offset = point - center;
    let at = vec2(offset.x, offset.y / GLOW_SQUASH).length() / radius.max(f32::EPSILON);
    if at >= 1. {
        return 0.;
    }
    let ring = at * GLOW_RINGS as f32;
    let inner = ring.floor();
    let t = ring - inner;
    let [a, b] = [inner, inner + 1.].map(|ring| ring_falloff(ring / GLOW_RINGS as f32));
    a + (b - a) * t
}

/// Grid cells across each side of a [`veil`]: fine enough that the glow's curve under it doesn't
/// show as facets.
const VEIL_CELLS: u32 = 16;

/// A shape covering `rect` with what's under it, the backdrop and a [`glow`] drawn at
/// `glow_alpha` around `glow_center`, at `cover` opacity. Drawn over something opaque, it fades
/// that something as one piece: egui fades each shape on its own, so faded parts that overlap
/// show through each other and stack up brighter (the mascot as a page fades in).
pub fn veil(
    rect: Rect,
    window: Rect,
    (glow_center, glow_radius, glow_alpha): (Pos2, f32, f32),
    cover: f32,
    tokens: &Tokens,
) -> Shape {
    let mut mesh = Mesh::default();
    let side = VEIL_CELLS + 1;
    for row in 0..side {
        for column in 0..side {
            let point = rect.min + vec2(column as f32, row as f32) / VEIL_CELLS as f32 * rect.size();
            let glow = tokens
                .glow
                .gamma_multiply(glow_falloff(glow_center, glow_radius, point) * glow_alpha);
            let under = color_at(window, point.y, tokens).blend(glow);
            mesh.colored_vertex(point, under.gamma_multiply(cover));
        }
    }
    for row in 0..VEIL_CELLS {
        for column in 0..VEIL_CELLS {
            let a = row * side + column;
            mesh.add_triangle(a, a + 1, a + side);
            mesh.add_triangle(a + 1, a + side + 1, a + side);
        }
    }
    Shape::mesh(mesh)
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

    #[test]
    fn the_glow_falloff_matches_the_rings() {
        let center = Pos2::ZERO;
        assert_eq!(glow_falloff(center, 100., center), 1.);
        assert_eq!(glow_falloff(center, 100., Pos2::new(100., 0.)), 0.);
        assert_eq!(glow_falloff(center, 100., Pos2::new(0., 90.)), 0.);
        // On a ring it's the ring's own value; between two it's halfway.
        assert!((glow_falloff(center, 100., Pos2::new(30., 0.)) - ring_falloff(0.3)).abs() < 1e-5);
        let between = glow_falloff(center, 100., Pos2::new(35., 0.));
        assert!((between - (ring_falloff(0.3) + ring_falloff(0.4)) / 2.).abs() < 1e-5);
    }

    /// Over nothing but the backdrop and the glow, a full veil is what's already there.
    #[test]
    fn a_full_veil_is_the_backdrop_and_glow_under_it() {
        let tokens = crate::theme::tokens(&egui::Visuals::dark());
        let window = Rect::from_min_size(Pos2::ZERO, vec2(400., 400.));
        let rect = Rect::from_center_size(Pos2::new(200., 200.), vec2(96., 96.));
        let Shape::Mesh(mesh) = veil(rect, window, (rect.center(), 200., 1.), 1., tokens) else {
            panic!("a veil is a mesh");
        };
        let middle = &mesh.vertices[(VEIL_CELLS / 2 * (VEIL_CELLS + 1) + VEIL_CELLS / 2) as usize];
        assert_eq!(middle.pos, rect.center());
        assert_eq!(middle.color, color_at(window, 200., tokens).blend(tokens.glow));
        assert_eq!(middle.color.a(), 255);
    }
}
