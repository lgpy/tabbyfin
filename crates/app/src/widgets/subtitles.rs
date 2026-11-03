//! Subtitle cues drawn over the video, for a player that leaves them to the UI (Media3 on
//! Android; mpv draws its own). They follow mpv's defaults so both players look alike: text at
//! the bottom of the video, sized to its height, in the colour and style from Settings and the
//! size from the player's menu; pictures (PGS and the like) where the cue puts them.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, TextureHandle, TextureOptions, Ui, pos2, vec2};
use player::{Cue, Cues};

use crate::config::{SubtitleColor, SubtitleStyle};

/// mpv's `sub-font-size` (55) and `sub-margin-y` (22), in pixels of a 720-pixel-high video.
const FONT_SIZE: f32 = 55. / 720.;
const MARGIN: f32 = 22. / 720.;
/// The smallest text, in points, whatever the video's size.
const MIN_FONT: f32 = 14.;
/// How much of the video's width a line may take before it wraps.
const WRAP: f32 = 0.9;

pub struct Subtitles {
    color: Color32,
    style: SubtitleStyle,
    /// The pictures of the cues shown, and the change of cues they're for.
    images: Vec<Option<TextureHandle>>,
    loaded: Option<u64>,
}

impl Subtitles {
    pub fn new(color: SubtitleColor, style: SubtitleStyle) -> Self {
        let color = match color {
            SubtitleColor::White => Color32::WHITE,
            SubtitleColor::Yellow => Color32::from_rgb(0xFF, 0xE1, 0x4D),
        };
        Self {
            color,
            style,
            images: Vec::new(),
            loaded: None,
        }
    }

    /// Draws `cues` over the video, which is letterboxed in `rect`. `changed` counts the changes
    /// of cues, so pictures are uploaded once; `scale` is the player's subtitle size.
    pub fn show(&mut self, ui: &Ui, rect: Rect, cues: &Cues, changed: u64, scale: f32) {
        if self.loaded != Some(changed) {
            self.load(ui, cues);
            self.loaded = Some(changed);
        }
        let video = fit(rect, cues.aspect);
        let text: Vec<&str> = cues
            .cues
            .iter()
            .filter_map(|cue| match cue {
                Cue::Text(text) => Some(text.as_str()),
                Cue::Image(_) => None,
            })
            .collect();
        if !text.is_empty() {
            self.text(ui, video, &text.join("\n"), scale);
        }
        for (cue, texture) in cues.cues.iter().zip(&self.images) {
            if let (Cue::Image(image), Some(texture)) = (cue, texture) {
                let [left, top, width, height] = image.rect;
                let size = vec2(width * video.width(), height * video.height());
                // No height: the picture's own aspect ratio.
                let size = if height > 0. {
                    size
                } else {
                    vec2(size.x, size.x * image.height as f32 / image.width.max(1) as f32)
                };
                let min = video.min + vec2(left * video.width(), top * video.height());
                let uv = Rect::from_min_max(Pos2::ZERO, pos2(1., 1.));
                ui.painter()
                    .image(texture.id(), Rect::from_min_size(min, size), uv, Color32::WHITE);
            }
        }
    }

    fn load(&mut self, ui: &Ui, cues: &Cues) {
        self.images = cues
            .cues
            .iter()
            .enumerate()
            .map(|(i, cue)| match cue {
                Cue::Image(image) => {
                    let size = [image.width as usize, image.height as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
                    Some(
                        ui.ctx()
                            .load_texture(format!("subtitle-{i}"), pixels, TextureOptions::LINEAR),
                    )
                }
                Cue::Text(_) => None,
            })
            .collect();
    }

    /// Text at the bottom of the video, each line centred, wrapped to most of its width.
    fn text(&self, ui: &Ui, video: Rect, text: &str, scale: f32) {
        let size = (video.height() * FONT_SIZE * scale).max(MIN_FONT);
        let painter = ui.painter();
        let mut job = egui::text::LayoutJob::simple(
            text.to_owned(),
            FontId::proportional(size),
            self.color,
            video.width() * WRAP,
        );
        job.halign = egui::Align::Center;
        let galley = painter.layout_job(job);
        // Centred lines are laid out around x = 0.
        let origin = pos2(
            video.center().x,
            video.bottom() - video.height() * MARGIN - galley.size().y,
        );
        match self.style {
            SubtitleStyle::Box => {
                let pad = vec2(size * 0.3, size * 0.1);
                let back = galley.rect.translate(origin.to_vec2()).expand2(pad);
                painter.rect_filled(back, CornerRadius::same(4), Color32::from_black_alpha(0xB0));
            }
            SubtitleStyle::Outline => {
                // mpv's outline (3 of 55) and a shadow under it.
                let outline = (size * 3. / 55.).max(1.);
                let shadow = Color32::from_black_alpha(0x80);
                painter.galley_with_override_text_color(origin + vec2(outline, outline) * 1.5, galley.clone(), shadow);
                for step in 0..8 {
                    let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                    let offset = vec2(angle.cos(), angle.sin()) * outline;
                    painter.galley_with_override_text_color(origin + offset, galley.clone(), Color32::BLACK);
                }
            }
        }
        painter.galley(origin, galley, self.color);
    }
}

/// The largest rectangle of `aspect` (width over height) centred in `rect`; `rect` itself when
/// the aspect is unknown.
fn fit(rect: Rect, aspect: f32) -> Rect {
    if aspect <= 0. || rect.height() <= 0. {
        return rect;
    }
    let size = if rect.width() > rect.height() * aspect {
        vec2(rect.height() * aspect, rect.height())
    } else {
        vec2(rect.width(), rect.width() / aspect)
    };
    Rect::from_center_size(rect.center(), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_the_video_in_the_view() {
        let portrait = Rect::from_min_size(Pos2::ZERO, vec2(400., 800.));
        assert_eq!(fit(portrait, 2.), Rect::from_min_size(pos2(0., 300.), vec2(400., 200.)));
        let wide = Rect::from_min_size(Pos2::ZERO, vec2(1000., 400.));
        assert_eq!(fit(wide, 2.), Rect::from_min_size(pos2(100., 0.), vec2(800., 400.)));
        assert_eq!(fit(wide, 0.), wide);
    }
}
