//! A blurred picture of the page under a dialog: egui can't blur what's under a layer, so the
//! window is captured once as the dialog opens (`Context::request_screenshot`), shrunk and
//! blurred on a thread of its own, and drawn over the page, under the dialog's scrim. The picture
//! doesn't follow the page after that; the dialog's own contents say what changed.
//!
//! The dialog waits for it (a few frames), so it doesn't open on the sharp page and then
//! frost over. Where nothing answers the capture, it opens on the scrim alone after [`WAIT`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Color32, ColorImage, Context, Id, Order, Rect, TextureHandle, TextureOptions, pos2};

/// The blur, in points: the picture is shrunk to a pixel every [`STEP`] points, then three box
/// passes of [`RADIUS`] pixels run over it, about a 16-point Gaussian. Drawn stretched back with
/// linear filtering, it comes back smooth.
const STEP: f32 = 4.;
const RADIUS: usize = 4;
const PASSES: usize = 3;
/// How long the dialog waits for its picture before opening without it.
const WAIT: Duration = Duration::from_millis(400);

/// What the backdrop is at.
#[derive(Default)]
enum Phase {
    /// Nothing asked for: the dialog is closed.
    #[default]
    Idle,
    /// The capture is asked for; the blurred picture lands in `ready`.
    Waiting {
        since: f64,
        ready: Arc<Mutex<Option<ColorImage>>>,
    },
    /// The picture, of the window as it was at `rect`.
    Shown { texture: TextureHandle, rect: Rect },
    /// No picture: the dialog has the scrim alone.
    Plain,
}

/// A dialog's blurred backdrop, kept by its owner while the dialog is open.
#[derive(Default)]
pub struct Frosted {
    phase: Phase,
}

impl Frosted {
    /// Called each frame the dialog is open, before it's drawn: asks for the picture the first
    /// time, and draws it once it's there. Returns whether the dialog should be drawn this frame
    /// (false while the picture is on its way). `enabled` false (no renderer to capture, as in
    /// the tests) skips the picture.
    pub fn show(&mut self, ctx: &Context, enabled: bool) -> bool {
        let now = ctx.input(|input| input.time);
        if let Phase::Idle = self.phase {
            self.phase = if enabled { capture(ctx, now) } else { Phase::Plain };
        }
        if let Phase::Waiting { since, ready } = &self.phase {
            let image = ready.lock().ok().and_then(|mut ready| ready.take());
            if let Some(image) = image {
                let texture = ctx.load_texture("frosted_backdrop", image, TextureOptions::LINEAR);
                self.phase = Phase::Shown {
                    texture,
                    rect: ctx.viewport_rect(),
                };
            } else if now - since > WAIT.as_secs_f64() {
                self.phase = Phase::Plain;
            } else {
                ctx.request_repaint();
                return false;
            }
        }
        if let Phase::Shown { texture, rect } = &self.phase {
            // Taken at another size (the phone turned), it would be stretched: the scrim alone.
            if (rect.size() - ctx.viewport_rect().size()).length() > 1. {
                self.phase = Phase::Plain;
            } else {
                paint(ctx, texture, *rect);
            }
        }
        true
    }

    /// Lets the picture go, for the next opening to take a new one.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
    }
}

/// Asks for the window's picture, to be blurred off the UI thread as it arrives.
fn capture(ctx: &Context, now: f64) -> Phase {
    let ready = Arc::new(Mutex::new(None));
    let slot = ready.clone();
    let repaint = ctx.clone();
    let factor = (STEP * ctx.pixels_per_point()).round().max(1.) as usize;
    ctx.request_screenshot(move |image: Arc<ColorImage>| {
        std::thread::spawn(move || {
            let blurred = blur(&image, factor, RADIUS);
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(blurred);
            }
            repaint.request_repaint();
        });
    });
    ctx.request_repaint();
    Phase::Waiting { since: now, ready }
}

/// The picture over the whole window, on a layer of its own between the page and the dialogs
/// (`Order::Middle`; pages are panels, in `Background`, and the dialog's modal is `Foreground`).
fn paint(ctx: &Context, texture: &TextureHandle, rect: Rect) {
    egui::Area::new(Id::unique("frosted_backdrop"))
        .order(Order::Middle)
        .fixed_pos(rect.min)
        .interactable(false)
        .show(ctx, |ui| {
            let uv = Rect::from_min_max(pos2(0., 0.), pos2(1., 1.));
            ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
        });
}

/// `image` shrunk `factor` times (each pixel the mean of its square) and run through
/// [`PASSES`] box blurs of `radius` pixels each way.
pub fn blur(image: &ColorImage, factor: usize, radius: usize) -> ColorImage {
    let (w, h) = ((image.width() / factor).max(1), (image.height() / factor).max(1));
    let mut buf: Vec<[f32; 3]> = vec![[0.; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.; 3];
            let mut n = 0.;
            for sy in y * factor..((y + 1) * factor).min(image.height()) {
                for sx in x * factor..((x + 1) * factor).min(image.width()) {
                    let c = image.pixels[sy * image.width() + sx];
                    sum[0] += f32::from(c.r());
                    sum[1] += f32::from(c.g());
                    sum[2] += f32::from(c.b());
                    n += 1.;
                }
            }
            let n = f32::max(n, 1.);
            buf[y * w + x] = [sum[0] / n, sum[1] / n, sum[2] / n];
        }
    }
    let mut out = vec![[0.; 3]; w * h];
    for _ in 0..PASSES {
        box_pass(&buf, &mut out, w, h, radius, true);
        box_pass(&out, &mut buf, w, h, radius, false);
    }
    let pixels = buf
        .iter()
        .map(|c| Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8))
        .collect();
    ColorImage::new([w, h], pixels)
}

/// One box blur along rows (`horizontal`) or columns, its edges clamped.
fn box_pass(from: &[[f32; 3]], to: &mut [[f32; 3]], w: usize, h: usize, radius: usize, horizontal: bool) {
    let (lines, length) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let n = (2 * radius + 1) as f32;
    for line in 0..lines {
        let mut sum = [0.; 3];
        for d in 0..2 * radius + 1 {
            let c = from[at(line, d.saturating_sub(radius).min(length - 1))];
            (0..3).for_each(|k| sum[k] += c[k]);
        }
        for i in 0..length {
            to[at(line, i)] = [sum[0] / n, sum[1] / n, sum[2] / n];
            let out = from[at(line, i.saturating_sub(radius))];
            let inn = from[at(line, (i + radius + 1).min(length - 1))];
            (0..3).for_each(|k| sum[k] += inn[k] - out[k]);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Color32, ColorImage};

    use super::blur;

    #[test]
    fn a_blur_shrinks_and_keeps_a_flat_colour() {
        let colour = Color32::from_rgb(40, 120, 200);
        let image = ColorImage::new([64, 32], vec![colour; 64 * 32]);
        let blurred = blur(&image, 8, 4);
        assert_eq!(blurred.size, [8, 4]);
        assert!(blurred.pixels.iter().all(|&c| c == colour));
    }

    #[test]
    fn a_blur_spreads_an_edge() {
        let mut pixels = vec![Color32::BLACK; 64 * 8];
        for y in 0..8 {
            for x in 32..64 {
                pixels[y * 64 + x] = Color32::WHITE;
            }
        }
        let blurred = blur(&ColorImage::new([64, 8], pixels), 2, 4);
        let row: Vec<u8> = (0..32).map(|x| blurred.pixels[x].r()).collect();
        assert_eq!(row[0], 0);
        assert_eq!(row[31], 255);
        assert!(row[15] > 0 && row[15] < 255, "softened at the edge: {row:?}");
    }
}
