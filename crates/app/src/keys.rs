//! Every key binding, and who gets a key.
//!
//! **egui or the app.** egui moves focus between widgets by itself (Tab, and the arrows once a
//! widget has focus) and presses the focused widget with Space or Enter. On a page with the
//! browse cursor that would fight the cursor: Tab is its next section, Enter opens its card. So
//! each frame, before egui sees the input, `App::raw_input_hook` gives the keyboard to one of
//! them, from the state the last frame left (as egui's own focus does):
//!
//! - **egui**, while it's busy with keys ([`egui_wants_keys`]): a widget has focus (a text field,
//!   or anything reached with Tab where the page lets egui's focus work), or a modal dialog, menu
//!   or popup is open. Every key is egui's then: Tab and the arrows move focus, Space and Enter
//!   press, Esc closes the popup or drops the focus. The code that owns such a widget reads the
//!   keys it uses itself (the search box's Down and Enter, a dialog's Up and Down), and a text
//!   field that uses Up and Down stops egui moving focus with them
//!   (`Memory::set_focus_lock_filter`).
//! - **the app**, otherwise: [`take`] removes every press bound in an active [`Scope`] from
//!   egui's input, so egui never sees them, and the shell routes them before anything is drawn.
//!   Keys without a binding stay with egui.
//!
//! The active scopes are the top page's (`Page::scope`) and the shell's. A page without the
//! cursor (Settings) has no scope of its own, so Tab, the arrows, Space and Enter stay with egui
//! there and its focus works as usual, while Esc and Backspace still go back.
//!
//! **Routing order**, for the keys the app took: the top bar while the keyboard is in it, then
//! the page (its scope's actions), then the shell's (Back, search). Dialogs, menus and focused
//! text fields come before all of them, through egui as above.

use egui::{Key, Modifiers};

/// Where a binding applies, like a key context on `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The signed-in frame, under every page.
    Shell,
    /// A page with the browse cursor (`browse::Browse`).
    Browse,
    /// The player page.
    Player,
}

/// What a bound key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    // Shell.
    Back,
    OpenSearch,
    // Browse.
    Move(crate::browse::Move),
    Activate,
    // Player.
    TogglePause,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    ToggleMute,
    NextEpisode,
    PreviousEpisode,
    NextChapter,
    PrevChapter,
    /// The skip button or the up-next card's Play now.
    Skip,
    ToggleFullscreen,
    /// Fullscreen first, then the up-next card, then stop.
    ExitPlayer,
    SpeedUp,
    SpeedDown,
    SubtitleDelayUp,
    SubtitleDelayDown,
}

pub struct Binding {
    pub scope: Scope,
    pub key: Key,
    pub modifiers: Modifiers,
    pub action: Action,
}

const fn bind(scope: Scope, modifiers: Modifiers, key: Key, action: Action) -> Binding {
    Binding {
        scope,
        key,
        modifiers,
        action,
    }
}

const NONE: Modifiers = Modifiers::NONE;

/// Every key binding. `COMMAND` is Ctrl, or Cmd on macOS.
pub const BINDINGS: &[Binding] = {
    use crate::browse::Move;
    use Action::*;
    use Scope::*;
    &[
        bind(Shell, Modifiers::COMMAND, Key::F, OpenSearch),
        bind(Shell, NONE, Key::Escape, Back),
        bind(Shell, NONE, Key::Backspace, Back),
        bind(Shell, Modifiers::ALT, Key::ArrowLeft, Back),
        bind(Browse, NONE, Key::ArrowLeft, Move(Move::Left)),
        bind(Browse, NONE, Key::ArrowRight, Move(Move::Right)),
        bind(Browse, NONE, Key::ArrowUp, Move(Move::Up)),
        bind(Browse, NONE, Key::ArrowDown, Move(Move::Down)),
        bind(Browse, NONE, Key::Tab, Move(Move::NextSection)),
        bind(Browse, Modifiers::SHIFT, Key::Tab, Move(Move::PrevSection)),
        bind(Browse, NONE, Key::Enter, Activate),
        bind(Player, NONE, Key::Space, TogglePause),
        bind(Player, NONE, Key::ArrowRight, SeekForward),
        bind(Player, NONE, Key::ArrowLeft, SeekBackward),
        bind(Player, NONE, Key::ArrowUp, VolumeUp),
        bind(Player, NONE, Key::ArrowDown, VolumeDown),
        bind(Player, NONE, Key::M, ToggleMute),
        bind(Player, Modifiers::SHIFT, Key::N, NextEpisode),
        bind(Player, Modifiers::SHIFT, Key::P, PreviousEpisode),
        bind(Player, NONE, Key::PageDown, NextChapter),
        bind(Player, NONE, Key::PageUp, PrevChapter),
        bind(Player, NONE, Key::Enter, Skip),
        bind(Player, NONE, Key::S, Skip),
        bind(Player, NONE, Key::F, ToggleFullscreen),
        bind(Player, NONE, Key::Escape, ExitPlayer),
        bind(Player, NONE, Key::CloseBracket, SpeedUp),
        bind(Player, NONE, Key::OpenBracket, SpeedDown),
        bind(Player, NONE, Key::X, SubtitleDelayUp),
        bind(Player, NONE, Key::Z, SubtitleDelayDown),
    ]
};

/// A key press the app took from egui (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Press {
    /// What this press does in `scope`, if it's bound there.
    pub fn action(self, scope: Scope) -> Option<Action> {
        BINDINGS
            .iter()
            .find(|binding| binding.scope == scope && self.matches(binding))
            .map(|binding| binding.action)
    }

    /// Modifiers must match exactly (Alt+Left is Back, Left alone moves the cursor), except on
    /// punctuation, which some layouts only type with Shift or AltGr.
    fn matches(self, binding: &Binding) -> bool {
        if self.key != binding.key {
            return false;
        }
        match self.key {
            Key::OpenBracket | Key::CloseBracket | Key::Plus | Key::Minus | Key::Equals => {
                self.modifiers.matches_logically(binding.modifiers)
            }
            _ => self.modifiers.matches_exact(binding.modifiers),
        }
    }
}

/// Whether egui keeps the keyboard this frame: a widget has focus, or a modal, menu or popup
/// is open. Judged from the last frame's state, like egui's own focus handling.
pub fn egui_wants_keys(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.focused().is_some() || memory.top_modal_layer().is_some())
        || egui::Popup::is_any_open(ctx)
}

/// Takes the presses bound in `scopes` out of `input`, unless egui wants the keyboard. Called
/// from `App::raw_input_hook`, before egui sees the input. Releases stay in: egui ignores the
/// release of a key it never saw pressed.
pub fn take(ctx: &egui::Context, input: &mut egui::RawInput, scopes: &[Scope]) -> Vec<Press> {
    if scopes.is_empty() || egui_wants_keys(ctx) {
        return Vec::new();
    }
    take_bound(input, scopes)
}

fn take_bound(input: &mut egui::RawInput, scopes: &[Scope]) -> Vec<Press> {
    let mut taken = Vec::new();
    input.events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = *event
        else {
            return true;
        };
        let press = Press { key, modifiers };
        let bound = scopes.iter().any(|&scope| press.action(scope).is_some());
        if bound {
            taken.push(press);
        }
        !bound
    });
    taken
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, RawInput};

    use super::{Action, BINDINGS, Press, Scope, take_bound};
    use crate::browse::Move;

    fn press(key: Key, modifiers: Modifiers) -> Press {
        Press { key, modifiers }
    }

    fn key_event(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn modifiers_tell_bindings_apart() {
        let alt = Modifiers::ALT;
        assert_eq!(press(Key::ArrowLeft, alt).action(Scope::Shell), Some(Action::Back));
        assert_eq!(press(Key::ArrowLeft, alt).action(Scope::Browse), None);
        assert_eq!(
            press(Key::ArrowLeft, Modifiers::NONE).action(Scope::Browse),
            Some(Action::Move(Move::Left))
        );
        assert_eq!(
            press(Key::Tab, Modifiers::SHIFT).action(Scope::Browse),
            Some(Action::Move(Move::PrevSection))
        );
        assert_eq!(press(Key::N, Modifiers::NONE).action(Scope::Player), None);
    }

    #[test]
    fn ctrl_f_is_search_where_ctrl_is_command() {
        // What egui-winit reports for Ctrl outside macOS, and for Cmd on macOS.
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let cmd = Modifiers {
            mac_cmd: true,
            command: true,
            ..Default::default()
        };
        assert_eq!(press(Key::F, ctrl).action(Scope::Shell), Some(Action::OpenSearch));
        assert_eq!(press(Key::F, cmd).action(Scope::Shell), Some(Action::OpenSearch));
        assert_eq!(press(Key::F, Modifiers::NONE).action(Scope::Shell), None);
    }

    #[test]
    fn punctuation_ignores_the_shift_a_layout_needs() {
        assert_eq!(
            press(Key::CloseBracket, Modifiers::SHIFT).action(Scope::Player),
            Some(Action::SpeedUp)
        );
    }

    #[test]
    fn no_key_is_bound_twice_in_a_scope() {
        for (i, a) in BINDINGS.iter().enumerate() {
            for b in &BINDINGS[i + 1..] {
                assert!(
                    !(a.scope == b.scope && a.key == b.key && a.modifiers == b.modifiers),
                    "{:?} {:?} is bound twice in {:?}",
                    a.modifiers,
                    a.key,
                    a.scope
                );
            }
        }
    }

    #[test]
    fn only_bound_presses_are_taken() {
        let mut input = RawInput {
            events: vec![
                key_event(Key::Tab, Modifiers::NONE, true),
                key_event(Key::Tab, Modifiers::NONE, false),
                key_event(Key::Space, Modifiers::NONE, true),
                Event::Text(" ".into()),
                key_event(Key::Escape, Modifiers::NONE, true),
            ],
            ..Default::default()
        };
        let taken = take_bound(&mut input, &[Scope::Browse, Scope::Shell]);
        assert_eq!(
            taken,
            [press(Key::Tab, Modifiers::NONE), press(Key::Escape, Modifiers::NONE)]
        );
        // Space isn't bound on a browsing page, so egui keeps it (and the Tab release).
        assert_eq!(
            input.events,
            [
                key_event(Key::Tab, Modifiers::NONE, false),
                key_event(Key::Space, Modifiers::NONE, true),
                Event::Text(" ".into()),
            ]
        );
    }
}
