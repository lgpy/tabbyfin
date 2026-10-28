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
//!   or popup is open. Every key is egui's then — except the app's own ([`Scope::App`], the
//!   zoom), which are taken first whatever holds the focus: Ctrl/Cmd with `=`, `-` and `0` type
//!   and press nothing. The code that owns such a widget reads the
//!   keys it uses itself (the search box's Esc, Down, Enter and Tab, a menu's arrow keys and
//!   Enter, a dialog's Up and Down), and a text field that uses Up and Down stops egui moving
//!   focus with them (`Memory::set_focus_lock_filter`).
//! - **the app**, otherwise: [`take`] removes every press bound in an active [`Scope`] from
//!   egui's input, so egui never sees them, and the shell routes them before anything is drawn.
//!   Keys without a binding stay with egui.
//!
//! The active scopes are the top page's (`Page::scope`), the app's and the shell's. A page
//! without the cursor (Settings) has no scope of its own, so Tab, the arrows, Space and Enter
//! stay with egui there and its focus works as usual, while Esc and Backspace still go back.
//!
//! **Routing order**, for the keys the app took: the app's own (`Scope::App`, the zoom) before
//! anything is drawn, then the top bar while the keyboard is in it, then the page (its scope's
//! actions), then the shell's (Back, search). Dialogs, menus and focused text fields come before
//! all of them, through egui as above.

use egui::{Key, Modifiers};

/// Where a binding applies, like a key context on `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Whatever is on screen, sign-in included: the app-wide keys (zoom).
    App,
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
    // App.
    ZoomIn,
    ZoomOut,
    ResetZoom,
    // Shell.
    Back,
    OpenSearch,
    // Browse.
    Move(crate::browse::Move),
    Activate,
    /// The item page's rows: W toggles the highlighted one watched.
    ToggleWatched,
    /// A playlist's highlighted entry: Alt+Up, Alt+Down and Delete move and remove it.
    MoveEntryUp,
    MoveEntryDown,
    RemoveEntry,
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
        bind(App, Modifiers::COMMAND, Key::Equals, ZoomIn),
        // The `+` key itself, and the keypad's.
        bind(App, Modifiers::COMMAND, Key::Plus, ZoomIn),
        bind(App, Modifiers::COMMAND, Key::Minus, ZoomOut),
        bind(App, Modifiers::COMMAND, Key::Num0, ResetZoom),
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
        // The item page's rows (`views/item.rs`); other pages leave these unused.
        bind(Browse, NONE, Key::W, ToggleWatched),
        bind(Browse, Modifiers::ALT, Key::ArrowUp, MoveEntryUp),
        bind(Browse, Modifiers::ALT, Key::ArrowDown, MoveEntryDown),
        bind(Browse, NONE, Key::Delete, RemoveEntry),
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

/// Takes the presses bound in `scopes` out of `input`. Called from `App::raw_input_hook`, before
/// egui sees the input. Releases stay in: egui ignores the release of a key it never saw pressed.
///
/// The app's own keys are taken even while egui wants the keyboard (a text field with the focus,
/// a menu, a dialog): `main` bound them globally, and Ctrl/Cmd with `=`, `-` and `0` mean
/// nothing to egui's widgets. Everything else waits for egui to be done with the keyboard.
pub fn take(ctx: &egui::Context, input: &mut egui::RawInput, scopes: &[Scope]) -> Vec<Press> {
    let mut taken = if scopes.contains(&Scope::App) {
        take_bound(input, &[Scope::App])
    } else {
        Vec::new()
    };
    if egui_wants_keys(ctx) {
        return taken;
    }
    let page_scopes: Vec<Scope> = scopes.iter().copied().filter(|scope| *scope != Scope::App).collect();
    taken.extend(take_bound(input, &page_scopes));
    taken
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

/// Takes presses of `key` with exactly `modifiers` out of egui's input, for the widgets that
/// read a key themselves while egui keeps the keyboard (the search box, the account menu) and
/// want the plain key only. [`egui::InputState::consume_key`] matches modifiers logically, so
/// Shift or Alt with the key would count as the key alone and, say, Alt+Down would open the
/// results or move the menu highlight. `modifiers` match as in [`Press::action`].
pub fn take_exact(ctx: &egui::Context, modifiers: Modifiers, key: Key) -> bool {
    let mut taken = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key: pressed,
                pressed: true,
                modifiers: pressed_modifiers,
                ..
            } = *event
            else {
                return true;
            };
            let matches = pressed == key && pressed_modifiers.matches_exact(modifiers);
            taken |= matches;
            !matches
        });
    });
    taken
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, RawInput};

    use super::{Action, BINDINGS, Press, Scope, take, take_bound, take_exact};
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
    fn the_item_pages_row_keys_are_the_browsers_alone() {
        assert_eq!(
            press(Key::W, Modifiers::NONE).action(Scope::Browse),
            Some(Action::ToggleWatched)
        );
        assert_eq!(
            press(Key::ArrowUp, Modifiers::ALT).action(Scope::Browse),
            Some(Action::MoveEntryUp)
        );
        assert_eq!(
            press(Key::ArrowDown, Modifiers::ALT).action(Scope::Browse),
            Some(Action::MoveEntryDown)
        );
        assert_eq!(
            press(Key::Delete, Modifiers::NONE).action(Scope::Browse),
            Some(Action::RemoveEntry)
        );
        // Plain Up stays the cursor's, and Alt+Up isn't Up.
        assert_eq!(
            press(Key::ArrowUp, Modifiers::NONE).action(Scope::Browse),
            Some(Action::Move(Move::Up))
        );
        // The rows' keys are the browsing pages' alone.
        assert_eq!(press(Key::W, Modifiers::NONE).action(Scope::Player), None);
        assert_eq!(press(Key::Delete, Modifiers::NONE).action(Scope::Shell), None);
        assert_eq!(press(Key::W, Modifiers::NONE).action(Scope::Shell), None);
    }

    #[test]
    fn punctuation_ignores_the_shift_a_layout_needs() {
        assert_eq!(
            press(Key::CloseBracket, Modifiers::SHIFT).action(Scope::Player),
            Some(Action::SpeedUp)
        );
    }

    #[test]
    fn the_zoom_keys_are_command_and_minus_and_equal() {
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
        for modifiers in [ctrl, cmd] {
            assert_eq!(press(Key::Equals, modifiers).action(Scope::App), Some(Action::ZoomIn));
            // A layout where `+` is Shift and `=` still zooms in.
            let shifted = Modifiers {
                shift: true,
                ..modifiers
            };
            assert_eq!(press(Key::Equals, shifted).action(Scope::App), Some(Action::ZoomIn));
            assert_eq!(press(Key::Plus, modifiers).action(Scope::App), Some(Action::ZoomIn));
            assert_eq!(press(Key::Minus, modifiers).action(Scope::App), Some(Action::ZoomOut));
            assert_eq!(press(Key::Num0, modifiers).action(Scope::App), Some(Action::ResetZoom));
        }
        // No modifier, no zoom: `=` and `-` are ordinary keys elsewhere.
        assert_eq!(press(Key::Equals, Modifiers::NONE).action(Scope::App), None);
        assert_eq!(press(Key::Minus, Modifiers::NONE).action(Scope::App), None);
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
    fn the_zoom_is_taken_from_a_text_field_and_from_an_open_menu() {
        let ctx = egui::Context::default();
        // A widget with the keyboard, as after a click into a text field.
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("search")));
        assert!(super::egui_wants_keys(&ctx));
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let mut input = RawInput {
            events: vec![
                key_event(Key::Equals, ctrl, true),
                key_event(Key::Equals, ctrl, false),
                // Tab belongs to egui while it has the focus.
                key_event(Key::Tab, Modifiers::NONE, true),
            ],
            ..Default::default()
        };
        let taken = take(&ctx, &mut input, &[Scope::Browse, Scope::Shell, Scope::App]);
        assert_eq!(taken, [press(Key::Equals, ctrl)]);
        // Tab and its like stayed with egui.
        assert!(input.events.contains(&key_event(Key::Tab, Modifiers::NONE, true)));
        assert!(!input.events.contains(&key_event(Key::Equals, ctrl, true)));
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

    #[test]
    fn exact_keys_ignore_extra_modifiers() {
        let ctx = egui::Context::default();
        let alt = Modifiers::ALT;
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        ctx.input_mut(|input| {
            input.events = vec![
                key_event(Key::ArrowDown, alt, true),
                key_event(Key::ArrowDown, Modifiers::NONE, true),
                key_event(Key::F, ctrl, true),
                key_event(Key::Tab, Modifiers::SHIFT, true),
            ];
        });
        // Alt+Down isn't Down, Shift+Tab isn't Tab.
        assert!(!take_exact(&ctx, Modifiers::NONE, Key::Tab));
        assert!(take_exact(&ctx, Modifiers::NONE, Key::ArrowDown));
        assert!(!take_exact(&ctx, Modifiers::NONE, Key::ArrowDown));
        assert!(take_exact(&ctx, Modifiers::SHIFT, Key::Tab));
        // Ctrl (or Cmd) and F, which is what the search shortcut binds.
        assert!(take_exact(&ctx, Modifiers::COMMAND, Key::F));
        // Only what matched went: Alt+Down is still egui's.
        assert_eq!(
            ctx.input(|input| input.events.clone()),
            [key_event(Key::ArrowDown, alt, true)]
        );
    }
}
