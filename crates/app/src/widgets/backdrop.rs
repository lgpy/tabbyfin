//! The cozy look's window background: a gradient from [`Tokens::backdrop_top`] to
//! [`Tokens::backdrop_bottom`], and the soft light pooled behind the mascot.
//!
//! egui has no gradients, so both are meshes with a colour per vertex, which the GPU blends.

use std::f32::consts::TAU;

use egui::{Mesh, Painter, Pos2, Rect, vec2};

use crate::theme::Tokens;

/// Points around each ring of the glow: enough that its edge never shows a corner.
const GLOW_STEPS: u32 = 96;
/// Rings from the centre out. Each steps the alpha down a smooth curve, so the light fades
/// without the visible bands that a few stacked translucent circles leave.
const GLOW_RINGS: u32 = 10;
/// How much flatter the glow is than a circle: light spreading over a surface.
const GLOW_SQUASH: f32 = 0.85;

/// Fills `rect` with the backdrop's gradient.
pub fn fill(painter: &Painter, rect: Rect, tokens: &Tokens) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), tokens.backdrop_top);
    mesh.colored_vertex(rect.right_top(), tokens.backdrop_top);
    mesh.colored_vertex(rect.right_bottom(), tokens.backdrop_bottom);
    mesh.colored_vertex(rect.left_bottom(), tokens.backdrop_bottom);
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
