//! The player page: video, controls, seek bar, menus, the skip button and up-next card. U10
//! fills it; the steps its keys, wheel and remote commands use are here, and the keys
//! themselves ([`on_key`]).

use crate::keys::Action;
use crate::markers;
use crate::playback::{Adjustments, UiCommand};
use crate::services::Services;

/// Seconds per seek key press (and remote rewind or fast-forward).
pub const SEEK_STEP: f64 = 10.;
/// Volume percent per key press, wheel notch or remote volume step.
pub const VOLUME_STEP: f64 = 5.;
/// Wheel lines per volume step. X11 and Windows normalise a wheel notch to three lines, so
/// one notch is one step; a trackpad's pixel deltas are turned into lines first.
pub const SCROLL_LINES_PER_STEP: f32 = 3.;
/// Trackpad pixels per wheel line, for platforms that send pixel deltas (macOS).
pub const PIXELS_PER_LINE: f32 = 30.;
/// The most volume steps one wheel event makes. Windows scales a notch by the user's lines per
/// notch, and its "one screen at a time" setting turns a notch into billions of lines.
pub const MAX_STEPS_PER_SCROLL: f32 = 4.;

/// The player's keys (`keys::Scope::Player`), acting on the running session. The shell calls
/// this while a session is showing, until U10's player page takes it over as its `on_key`
/// (adding what's the page's own: showing the controls after each key). Returns whether the
/// key was used.
pub fn on_key(action: Action, services: &mut Services, ctx: &egui::Context) -> bool {
    let fullscreen = ctx.input(|i| i.viewport().fullscreen).unwrap_or(false);
    let Some(state) = services.playback.now_playing() else {
        // Between sessions or waiting to reconnect: Esc still leaves fullscreen, then stops
        // (cancelling a reconnect).
        match action {
            Action::ExitPlayer if fullscreen => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)),
            Action::ExitPlayer => services.control(UiCommand::Stop),
            Action::ToggleFullscreen => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen)),
            _ => return false,
        }
        return true;
    };
    let volume = state.volume;
    let change_volume = |delta: f64| UiCommand::SetVolume((volume + delta).clamp(0., 100.));
    let mut adjustments = state.adjustments;
    match action {
        Action::TogglePause => services.control(UiCommand::TogglePause),
        Action::SeekForward => services.control(UiCommand::SeekBy(SEEK_STEP)),
        Action::SeekBackward => services.control(UiCommand::SeekBy(-SEEK_STEP)),
        Action::VolumeUp => services.control(change_volume(VOLUME_STEP)),
        Action::VolumeDown => services.control(change_volume(-VOLUME_STEP)),
        Action::ToggleMute => services.control(UiCommand::ToggleMute),
        // Shift+N: the next episode, collection movie or queued item.
        Action::NextEpisode => services.play_next(),
        // Shift+P: the previous episode or collection movie.
        Action::PreviousEpisode => services.play_previous(),
        Action::NextChapter => {
            if let Some(start) = markers::next_chapter(&state.chapters, state.position) {
                services.control(UiCommand::SeekTo(start));
            }
        }
        Action::PrevChapter => {
            if let Some(start) = markers::prev_chapter(&state.chapters, state.position) {
                services.control(UiCommand::SeekTo(start));
            }
        }
        // Enter: skip the intro (or recap, preview…), or play the next episode from the
        // up-next card, or start over from the resume prompt.
        Action::Skip => {
            if let Some(segment) = state.skippable() {
                services.control(UiCommand::SeekTo(segment.end));
            } else if state.show_up_next() {
                services.play_next();
            } else if state.resume_prompt().is_some() {
                services.start_over();
            }
        }
        Action::ToggleFullscreen => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen)),
        // Esc: leave fullscreen first, then dismiss the up-next card, then stop playback (which
        // closes the player).
        Action::ExitPlayer => {
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else if state.show_up_next() {
                if let Some(state) = services.playback.now_playing_mut() {
                    state.next_cancelled = true;
                }
            } else {
                services.control(UiCommand::Stop);
            }
        }
        Action::SpeedUp | Action::SpeedDown => {
            adjustments.speed = adjustments.step_speed(action == Action::SpeedUp);
            services.adjust(adjustments);
        }
        Action::SubtitleDelayUp => {
            adjustments.subtitle_delay += Adjustments::DELAY_STEP;
            services.adjust(adjustments);
        }
        Action::SubtitleDelayDown => {
            adjustments.subtitle_delay -= Adjustments::DELAY_STEP;
            services.adjust(adjustments);
        }
        _ => return false,
    }
    true
}

/// Turns a wheel delta (in lines, positive is scrolling up) into whole volume steps, keeping
/// the leftover lines for the next event. One event makes at most `MAX_STEPS_PER_SCROLL`.
pub fn scroll_steps(pending: f32, lines: f32) -> (f32, i32) {
    let limit = MAX_STEPS_PER_SCROLL * SCROLL_LINES_PER_STEP;
    let total = (pending + lines).clamp(-limit, limit);
    let steps = (total / SCROLL_LINES_PER_STEP).trunc() as i32;
    (total - steps as f32 * SCROLL_LINES_PER_STEP, steps)
}

#[cfg(test)]
mod tests {
    use super::{MAX_STEPS_PER_SCROLL, SCROLL_LINES_PER_STEP, scroll_steps};

    #[test]
    fn a_wheel_notch_is_one_volume_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP);
        assert_eq!(steps, 1);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_fast_flick_keeps_every_step() {
        let (pending, steps) = scroll_steps(0., SCROLL_LINES_PER_STEP * 4.);
        assert_eq!(steps, 4);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn small_deltas_add_up() {
        let (pending, steps) = scroll_steps(0., 1.);
        assert_eq!((pending, steps), (1., 0));
        let (pending, steps) = scroll_steps(pending, 2.);
        assert_eq!((pending, steps), (0., 1));
    }

    #[test]
    fn scrolling_down_is_negative() {
        let (pending, steps) = scroll_steps(0., -2. * SCROLL_LINES_PER_STEP);
        assert_eq!(steps, -2);
        assert_eq!(pending, 0.);
    }

    #[test]
    fn a_huge_delta_is_capped() {
        let (pending, steps) = scroll_steps(0., 4_294_967_295.);
        assert_eq!(steps, MAX_STEPS_PER_SCROLL as i32);
        assert_eq!(pending, 0.);
        let (_, steps) = scroll_steps(0., -4_294_967_295.);
        assert_eq!(steps, -(MAX_STEPS_PER_SCROLL as i32));
    }

    #[test]
    fn scrolling_back_cancels() {
        let (pending, steps) = scroll_steps(0., -1.5);
        assert_eq!(steps, 0);
        let (_, steps) = scroll_steps(pending, 1.5);
        assert_eq!(steps, 0);
    }
}
