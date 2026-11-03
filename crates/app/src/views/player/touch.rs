//! Touch on the video, on Android (docs/android.md, *Touch UI*). A tap shows or hides the
//! controls. A double tap on the left or right third skips back or forward, and each further tap
//! there in quick succession skips again. A drag seeks (sideways, with the seek bar and the
//! preview following), or sets the brightness (up and down on the left half) or the volume (on
//! the right half). With a mouse a click pauses and a double click goes fullscreen instead.
//!
//! While the player page is open the window is the player's: kept on, in landscape and with the
//! system bars hidden ([`Window`]).

use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Ui, Vec2, pos2, vec2};
use egui_material_icons::icons::{
    ICON_BRIGHTNESS_MEDIUM, ICON_FAST_FORWARD, ICON_FAST_REWIND, ICON_VOLUME_OFF, ICON_VOLUME_UP,
};

use super::{PREVIEW_W, PlayerPage, SEEK_STEP, TRACK_H, View};
use crate::playback::UiCommand;
use crate::theme::{self, over_video};
use crate::views::PageCx;
use crate::widgets::round_button;
use crate::widgets::text::timestamp;

/// Whether the video takes touch gestures rather than the mouse's clicks.
pub(super) const TOUCH: bool = cfg!(target_os = "android");

/// What the video senses: drags too, for touch.
pub(super) fn sense() -> Sense {
    if TOUCH {
        Sense::CLICK | Sense::DRAG
    } else {
        Sense::CLICK
    }
}

/// How long after a skip another tap on the same side skips again; its bubble shows that long.
const STREAK: f64 = 0.8;
/// How far a finger moves before its drag picks a kind (seek, volume or brightness).
const DECIDE: f32 = 12.;
/// The seconds a drag across the whole video seeks by.
const SEEK_ACROSS: f64 = 120.;
/// The share of the video's height a drag goes from silent to full (or dark to bright) in.
const LEVEL_ACROSS: f32 = 0.6;
/// The skip bubble's diameter, and the level pill's size and its track's width.
const BUBBLE: f32 = 88.;
const LEVEL_W: f32 = 220.;
const LEVEL_H: f32 = 44.;
const LEVEL_TRACK_W: f32 = 120.;

/// The touch gestures' state.
#[derive(Default)]
pub(super) struct Touch {
    /// Whether the controls showed before the last tap, which a double tap puts back.
    controls_before_tap: bool,
    /// The skips in a row on one side.
    streak: Option<Streak>,
    drag: Option<Drag>,
    /// The window while the page is open (Android).
    _window: Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Back,
    Forward,
}

struct Streak {
    side: Side,
    /// When the last tap skipped.
    last: f64,
    /// How many skips: the bubble shows their total.
    skips: u32,
}

enum Drag {
    /// The finger hasn't moved far enough to tell.
    Undecided,
    /// Seeking from `from`: [`PlayerPage::scrub`] holds where to, so the seek bar follows.
    Seek { from: Duration, duration: Duration },
    /// The volume in percent, from `from`; `value` is what was last asked for.
    Volume { from: f64, value: f64 },
    /// The brightness from 0 to 1.
    Brightness { from: f32, value: f32 },
    /// A drag that does nothing (seeking with no duration known, or from the bars).
    Ignored,
}

impl PlayerPage {
    /// The video's taps and drags. `bars` are the controls' bars while they show, which keep
    /// their own taps and drags.
    pub(super) fn touch_input(
        &mut self,
        response: &Response,
        ui: &Ui,
        view: &View,
        cx: &mut PageCx,
        bars: Option<[Rect; 2]>,
    ) {
        let now = ui.input(|input| input.time);
        let over_bars = |pos: egui::Pos2| bars.is_some_and(|bars| bars.iter().any(|bar| bar.contains(pos)));
        let rect = response.rect;
        let tap = response
            .interact_pointer_pos()
            .filter(|&pos| response.clicked() && !over_bars(pos));
        if let Some(pos) = tap {
            let side = side_of(rect, pos);
            let streaking = self
                .touch
                .streak
                .as_ref()
                .is_some_and(|streak| now - streak.last < STREAK && Some(streak.side) == side);
            if streaking && let (Some(streak), Some(side)) = (&mut self.touch.streak, side) {
                streak.last = now;
                streak.skips += 1;
                skip(cx, side);
            } else if response.double_clicked() {
                // The first tap showed or hid the controls; a skip leaves them as they were.
                self.set_controls(self.touch.controls_before_tap);
                if let Some(side) = side {
                    self.touch.streak = Some(Streak {
                        side,
                        last: now,
                        skips: 1,
                    });
                    skip(cx, side);
                }
            } else {
                self.touch.streak = None;
                self.touch.controls_before_tap = self.controls_visible;
                self.set_controls(!self.controls_visible);
            }
        }

        let origin = ui.input(|input| input.pointer.press_origin());
        if response.drag_started() {
            let from_bars = origin.is_some_and(over_bars);
            self.touch.drag = Some(if from_bars { Drag::Ignored } else { Drag::Undecided });
        }
        let mut seeking_began = false;
        if let (Some(drag), Some(origin), Some(pos)) = (&mut self.touch.drag, origin, response.interact_pointer_pos()) {
            let moved = pos - origin;
            if matches!(drag, Drag::Undecided) && moved.length() >= DECIDE {
                *drag = decide(rect, origin, moved, view);
                seeking_began = matches!(drag, Drag::Seek { .. });
            }
            match drag {
                Drag::Seek { from, duration } => {
                    let by = f64::from(moved.x / rect.width()) * SEEK_ACROSS;
                    let to = (from.as_secs_f64() + by).clamp(0., duration.as_secs_f64());
                    self.scrub = Some(Duration::from_secs_f64(to));
                }
                Drag::Volume { from, value } => {
                    let to = (*from + f64::from(-moved.y / (rect.height() * LEVEL_ACROSS)) * 100.)
                        .clamp(0., 100.)
                        .round();
                    if to != *value {
                        *value = to;
                        cx.services.control(UiCommand::SetVolume(to));
                    }
                }
                Drag::Brightness { from, value } => {
                    let to = (*from - moved.y / (rect.height() * LEVEL_ACROSS)).clamp(0.01, 1.);
                    if (to - *value).abs() >= 0.005 {
                        *value = to;
                        set_brightness(to);
                    }
                }
                Drag::Undecided | Drag::Ignored => {}
            }
        }
        // The seek bar shows while a drag seeks.
        if seeking_began {
            self.show_controls();
        }
        if response.drag_stopped() {
            match self.touch.drag.take() {
                Some(Drag::Seek { .. }) => {
                    if let Some(time) = self.scrub.take() {
                        cx.services.control(UiCommand::SeekTo(time));
                    }
                    self.show_controls();
                }
                Some(Drag::Volume { value, .. }) => cx.services.control(UiCommand::SetVolume(value)),
                _ => {}
            }
        }
    }

    /// Shows the controls, or hides them now.
    fn set_controls(&mut self, visible: bool) {
        if visible {
            self.show_controls();
        } else {
            self.controls_visible = false;
        }
    }

    /// What the gestures show over the video: the skip bubble, the seek time with its preview,
    /// or the volume or brightness.
    pub(super) fn touch_feedback(&mut self, ui: &Ui, rect: Rect, view: &View) {
        let now = ui.input(|input| input.time);
        if let Some(streak) = &self.touch.streak {
            let age = now - streak.last;
            if age < STREAK {
                skip_bubble(ui, rect, streak, age);
                ui.ctx().request_repaint();
            } else {
                self.touch.streak = None;
            }
        }
        match &self.touch.drag {
            Some(Drag::Seek { from, .. }) => {
                let to = self.scrub.unwrap_or(view.position);
                let preview = self.trickplay.as_mut().and_then(|trickplay| {
                    let aspect = trickplay.aspect_ratio();
                    trickplay.thumbnail(to).map(|texture| (texture.id(), aspect))
                });
                seek_label(ui, rect, *from, to, preview);
            }
            Some(Drag::Volume { value, .. }) => {
                let icon = if *value == 0. { ICON_VOLUME_OFF } else { ICON_VOLUME_UP };
                level(ui, rect, icon, (*value / 100.) as f32);
            }
            Some(Drag::Brightness { value, .. }) => level(ui, rect, ICON_BRIGHTNESS_MEDIUM, *value),
            _ => {}
        }
    }
}

/// The side a tap at `pos` skips to: the left or right third, or `None` in the middle.
fn side_of(rect: Rect, pos: egui::Pos2) -> Option<Side> {
    let third = rect.width() / 3.;
    if pos.x < rect.left() + third {
        Some(Side::Back)
    } else if pos.x > rect.right() - third {
        Some(Side::Forward)
    } else {
        None
    }
}

fn skip(cx: &mut PageCx, side: Side) {
    let by = match side {
        Side::Back => -SEEK_STEP,
        Side::Forward => SEEK_STEP,
    };
    cx.services.control(UiCommand::SeekBy(by));
}

/// A drag's kind, once it has moved far enough: sideways seeks, up and down sets the brightness
/// on the left half and the volume on the right.
fn decide(rect: Rect, origin: egui::Pos2, moved: Vec2, view: &View) -> Drag {
    if moved.x.abs() > moved.y.abs() {
        return match view.duration.filter(|duration| !duration.is_zero()) {
            Some(duration) => Drag::Seek {
                from: view.position,
                duration,
            },
            None => Drag::Ignored,
        };
    }
    if origin.x < rect.center().x {
        let from = brightness();
        Drag::Brightness { from, value: from }
    } else {
        Drag::Volume {
            from: view.volume,
            value: view.volume,
        }
    }
}

/// The round bubble on the skipping side: the arrows over the seconds skipped, fading out.
fn skip_bubble(ui: &Ui, rect: Rect, streak: &Streak, age: f64) {
    let (x, icon, sign) = match streak.side {
        Side::Back => (rect.left() + rect.width() / 6., ICON_FAST_REWIND, "−"),
        Side::Forward => (rect.right() - rect.width() / 6., ICON_FAST_FORWARD, "+"),
    };
    // Full until the last quarter of its time, then fading.
    let fade = ((STREAK - age) / (STREAK * 0.25)).clamp(0., 1.) as f32;
    let center = pos2(x, rect.center().y);
    let painter = ui.painter();
    painter.circle_filled(center, BUBBLE / 2., over_video::SURFACE_SUBTLE.gamma_multiply(fade));
    let ink = over_video::FOREGROUND.gamma_multiply(fade);
    round_button::glyph(ui, center - vec2(0., 12.), icon, 28., ink);
    let seconds = f64::from(streak.skips) * SEEK_STEP;
    painter.text(
        center + vec2(0., 18.),
        Align2::CENTER_CENTER,
        format!("{sign}{seconds:.0} s"),
        theme::text::font(ui, "sm"),
        ink,
    );
}

/// The time a seek drag goes to, and by how much, in the middle: under the preview when the
/// server has one.
fn seek_label(ui: &Ui, rect: Rect, from: Duration, to: Duration, preview: Option<(egui::TextureId, f32)>) {
    let painter = ui.painter();
    let time = painter.layout_no_wrap(timestamp(to), theme::text::font(ui, "xl"), over_video::FOREGROUND);
    let by = to.as_secs_f64() - from.as_secs_f64();
    let sign = if by < 0. { "−" } else { "+" };
    let delta = painter.layout_no_wrap(
        format!("{sign}{}", timestamp(Duration::from_secs_f64(by.abs()))),
        theme::text::font(ui, "sm"),
        over_video::MUTED_FOREGROUND,
    );
    let pad = vec2(16., 8.);
    let size = vec2(time.size().x.max(delta.size().x), time.size().y + 2. + delta.size().y) + pad * 2.;
    let label = Rect::from_center_size(rect.center(), size);
    if let Some((texture, aspect)) = preview {
        let image = Rect::from_min_size(
            pos2(rect.center().x - PREVIEW_W / 2., label.top() - 8. - PREVIEW_W / aspect),
            vec2(PREVIEW_W, PREVIEW_W / aspect),
        );
        painter.image(
            texture,
            image,
            Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
            Color32::WHITE,
        );
    }
    painter.rect_filled(label, CornerRadius::same(12), over_video::SURFACE);
    let top = label.top() + pad.y;
    painter.galley(
        pos2(label.center().x - time.size().x / 2., top),
        time.clone(),
        over_video::FOREGROUND,
    );
    painter.galley(
        pos2(label.center().x - delta.size().x / 2., top + time.size().y + 2.),
        delta,
        over_video::MUTED_FOREGROUND,
    );
}

/// The volume or brightness while a drag sets it: a pill at the top middle with the icon, a
/// bar and the percentage.
fn level(ui: &Ui, rect: Rect, icon: egui_material_icons::MaterialIcon, fraction: f32) {
    let fraction = fraction.clamp(0., 1.);
    let pill = Rect::from_center_size(pos2(rect.center().x, rect.top() + 72.), vec2(LEVEL_W, LEVEL_H));
    let painter = ui.painter();
    painter.rect_filled(pill, CornerRadius::same((LEVEL_H / 2.) as u8), over_video::SURFACE);
    round_button::glyph(
        ui,
        pos2(pill.left() + 28., pill.center().y),
        icon,
        22.,
        over_video::FOREGROUND,
    );
    let track = Rect::from_center_size(
        pos2(pill.left() + 52. + LEVEL_TRACK_W / 2., pill.center().y),
        vec2(LEVEL_TRACK_W, TRACK_H),
    );
    let radius = CornerRadius::same(TRACK_H as u8 / 2);
    painter.rect_filled(track, radius, over_video::SEEK_TRACK);
    let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * fraction, track.bottom()));
    painter.rect_filled(filled, radius, over_video::FOREGROUND);
    painter.text(
        pos2(pill.right() - 18., pill.center().y),
        Align2::RIGHT_CENTER,
        format!("{:.0}", fraction * 100.),
        theme::text::font(ui, "sm"),
        over_video::FOREGROUND,
    );
}

/// The screen's brightness, from 0 to 1.
fn brightness() -> f32 {
    #[cfg(target_os = "android")]
    if let Some(value) = player::android::brightness() {
        return value;
    }
    0.5
}

fn set_brightness(value: f32) {
    #[cfg(target_os = "android")]
    player::android::set_brightness(value);
    #[cfg(not(target_os = "android"))]
    let _ = value;
}

/// The window while the player page is open, from its first item to its last (Android): kept
/// on, landscape and full screen, then put back, with the system's brightness, when the page
/// closes. Nothing elsewhere.
struct Window;

impl Default for Window {
    fn default() -> Self {
        #[cfg(target_os = "android")]
        player::android::watching(true);
        Window
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        player::android::watching(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_skips_from_the_outer_thirds_only() {
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(900., 400.));
        assert_eq!(side_of(rect, pos2(10., 200.)), Some(Side::Back));
        assert_eq!(side_of(rect, pos2(299., 200.)), Some(Side::Back));
        assert_eq!(side_of(rect, pos2(450., 200.)), None);
        assert_eq!(side_of(rect, pos2(601., 200.)), Some(Side::Forward));
    }
}
