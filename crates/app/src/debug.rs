//! Dev aids in debug builds, for driving the app without touching the desktop's mouse or
//! keyboard (dev/README.md).
//!
//! - `TABBYFIN_KEYS="down right enter wait escape"` presses keys after startup, on any screen
//!   (sign-in too). `wait` pauses for a second; `down*40` presses a key 40 times, 30 ms apart (a
//!   held key, roughly). Keys are named as on `main`: `ctrl-f`, `shift-tab`, `alt-left`, `pageup`,
//!   `]`, `a`. They're injected into egui's input from `App::raw_input_hook`, so they take the
//!   same path as real keys (`keys.rs`), and printable ones type their text into a focused field.
//!   A press is only injected in a pass that will draw: eframe runs logic-only passes for a window
//!   the compositor won't show, and input given to one of those only reaches the next pass, where
//!   presses bunch up out of time order. So the presses wait for drawing to resume, one per pass
//!   (dev/README.md, `docs/egui-migration.md` §6).
//! - `TABBYFIN_ROUTE` opens a page on top of Home at startup ([`Start`]).

use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers};

use crate::nav::Route;

/// Time for the window to open and the session to restore before the first key.
const FIRST_KEY_AFTER: Duration = Duration::from_secs(3);
const WAIT: Duration = Duration::from_secs(1);
/// Between the presses of `key*N`.
const REPEAT_EVERY: Duration = Duration::from_millis(30);
/// After each key (or its last repeat), for its effect to show.
const BETWEEN_KEYS: Duration = Duration::from_millis(250);

/// The `TABBYFIN_KEYS` script, played against the clock.
pub struct KeyScript {
    start: Instant,
    /// Each press and when it's due after `start`, in order.
    presses: Vec<(Duration, ScriptKey)>,
    next: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptKey {
    key: Key,
    modifiers: Modifiers,
    /// What the key types, for a focused text field.
    text: Option<String>,
    /// Logged on the first press: the key as written, and how many times.
    log: Option<String>,
}

impl KeyScript {
    /// The script from `TABBYFIN_KEYS`, timed from now.
    pub fn from_env() -> Option<Self> {
        let script = std::env::var("TABBYFIN_KEYS").ok()?;
        Some(Self {
            start: Instant::now(),
            presses: schedule(&script),
            next: 0,
        })
    }

    /// The next press that's due, taken out of the script: the caller injects it into a pass
    /// that draws. At most one per pass, so a script held back while the window wasn't drawn
    /// plays out one key per frame rather than all at once.
    pub fn take_due(&mut self) -> Option<ScriptKey> {
        let (at, press) = self.presses.get(self.next)?;
        if *at > self.start.elapsed() {
            return None;
        }
        if let Some(log) = &press.log {
            tracing::info!("debug key: {log}");
        }
        self.next += 1;
        Some(press.clone())
    }

    /// When the next press is due, for the app to wake up then.
    pub fn next_due(&self) -> Option<Instant> {
        self.presses.get(self.next).map(|(at, _)| self.start + *at)
    }
}

impl ScriptKey {
    /// Presses the key, types its text and releases it, as a person at the keyboard would.
    pub fn inject(&self, input: &mut egui::RawInput) {
        let key = |pressed| Event::Key {
            key: self.key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: self.modifiers,
        };
        input.events.push(key(true));
        if let Some(text) = &self.text {
            input.events.push(Event::Text(text.clone()));
        }
        input.events.push(key(false));
    }
}

/// Every press of `script` with its time from the start, as `main`'s replay timed them.
fn schedule(script: &str) -> Vec<(Duration, ScriptKey)> {
    let mut at = FIRST_KEY_AFTER;
    let mut presses = Vec::new();
    for word in script.split_whitespace() {
        if word == "wait" {
            at += WAIT;
            continue;
        }
        let (spec, times) = match word.split_once('*') {
            Some((spec, times)) => (spec, times.parse().unwrap_or(1)),
            None => (word, 1),
        };
        let Some((key, modifiers, text)) = parse_key(spec) else {
            tracing::warn!("bad debug key {spec:?}");
            at += BETWEEN_KEYS;
            continue;
        };
        for i in 0..times {
            if i > 0 {
                at += REPEAT_EVERY;
            }
            let log = (i == 0).then(|| format!("{spec} x{times}"));
            presses.push((
                at,
                ScriptKey {
                    key,
                    modifiers,
                    text: text.clone(),
                    log,
                },
            ));
        }
        at += BETWEEN_KEYS;
    }
    presses
}

/// A key as `main` named them: modifiers joined with `-` (`ctrl`, `alt`, `shift`, `cmd`, and
/// `secondary` for Cmd on macOS and Ctrl elsewhere), then the key. Modifiers come out as
/// egui-winit reports them, so bindings match as they do for real keys. Also returns the text
/// the key types, if it types any.
fn parse_key(spec: &str) -> Option<(Key, Modifiers, Option<String>)> {
    let mac = cfg!(target_os = "macos");
    let mut modifiers = Modifiers::NONE;
    let mut rest = spec;
    // `ctrl--` is Ctrl and the minus key.
    while let Some((modifier, tail)) = rest.split_once('-')
        && !tail.is_empty()
    {
        match modifier {
            "ctrl" | "control" => {
                modifiers.ctrl = true;
                modifiers.command |= !mac;
            }
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "cmd" | "super" | "win" => {
                modifiers.mac_cmd = mac;
                modifiers.command |= mac;
            }
            "secondary" => {
                modifiers.command = true;
                modifiers.mac_cmd = mac;
                modifiers.ctrl |= !mac;
            }
            _ => return None,
        }
        rest = tail;
    }
    let key = match rest {
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "enter" => Key::Enter,
        "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        name => Key::from_name(name).or_else(|| Key::from_name(&name.to_uppercase()))?,
    };
    let typing = !(modifiers.ctrl || modifiers.alt || modifiers.command || modifiers.mac_cmd);
    let text = match rest {
        "space" => Some(" ".to_owned()),
        name if typing && name.chars().count() == 1 => Some(if modifiers.shift {
            name.to_uppercase()
        } else {
            name.to_owned()
        }),
        _ => None,
    };
    Some((key, modifiers, text))
}

/// What `TABBYFIN_ROUTE` asks for at startup.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// `search:<term>`: types `term` into the search box.
    Search(String),
    /// `settings`, `item:<id>`, `person:<id>`, or `library:<id>[:movies|tvshows]`, opened on
    /// top of Home.
    Open(Route),
    /// `play:<id>[:<audio index>[:<subtitle index or -1>[:<media source id>]]]`: opens the item
    /// and plays it from its resume point. Empty fields keep the defaults.
    Play {
        id: String,
        audio: Option<i32>,
        subtitle: Option<i32>,
        source: Option<String>,
    },
}

impl Start {
    pub fn from_env() -> Option<Self> {
        Self::parse(&std::env::var("TABBYFIN_ROUTE").ok()?)
    }

    fn parse(value: &str) -> Option<Self> {
        use jellyfin::models::CollectionType;

        if let Some(term) = value.strip_prefix("search:") {
            return Some(Start::Search(term.to_owned()));
        }
        let mut parts = value.split(':');
        match (parts.next()?, parts.next()) {
            ("settings", _) => Some(Start::Open(Route::Settings)),
            ("item", Some(id)) => Some(Start::Open(Route::Item {
                id: id.into(),
                name: "Item".into(),
            })),
            ("play", Some(id)) => {
                let rest: Vec<&str> = parts.collect();
                let index = |i: usize| rest.get(i).and_then(|part| part.parse().ok());
                Some(Start::Play {
                    id: id.into(),
                    audio: index(0),
                    subtitle: index(1),
                    source: rest.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                })
            }
            ("library", Some(id)) => Some(Start::Open(Route::Library {
                id: id.into(),
                name: "Library".into(),
                collection_type: match parts.next() {
                    Some("movies") => Some(CollectionType::Movies),
                    Some("tvshows") => Some(CollectionType::TvShows),
                    _ => None,
                },
            })),
            ("person", Some(id)) => Some(Start::Open(Route::Person {
                id: id.into(),
                name: "Person".into(),
            })),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use egui::{Event, Key, Modifiers};

    use super::{KeyScript, Start, parse_key, schedule};
    use crate::nav::Route;

    #[test]
    fn keys_parse_as_main_named_them() {
        assert_eq!(parse_key("down"), Some((Key::ArrowDown, Modifiers::NONE, None)));
        assert_eq!(parse_key("shift-tab"), Some((Key::Tab, Modifiers::SHIFT, None)));
        assert_eq!(parse_key("alt-left"), Some((Key::ArrowLeft, Modifiers::ALT, None)));
        assert_eq!(parse_key("pagedown"), Some((Key::PageDown, Modifiers::NONE, None)));
        assert_eq!(
            parse_key("]"),
            Some((Key::CloseBracket, Modifiers::NONE, Some("]".into())))
        );
        assert_eq!(parse_key("a"), Some((Key::A, Modifiers::NONE, Some("a".into()))));
        assert_eq!(parse_key("shift-n"), Some((Key::N, Modifiers::SHIFT, Some("N".into()))));
        assert_eq!(
            parse_key("space"),
            Some((Key::Space, Modifiers::NONE, Some(" ".into())))
        );
        assert_eq!(parse_key("f5"), Some((Key::F5, Modifiers::NONE, None)));
        assert_eq!(parse_key("nonsense"), None);
        assert_eq!(parse_key("hyper-a"), None);
    }

    #[test]
    fn ctrl_types_nothing_and_is_command_outside_macos() {
        let (key, modifiers, text) = parse_key("ctrl-f").unwrap();
        assert_eq!((key, text), (Key::F, None));
        assert!(modifiers.ctrl);
        assert_eq!(modifiers.command, !cfg!(target_os = "macos"));
        let (key, modifiers, _) = parse_key("ctrl--").unwrap();
        assert_eq!(key, Key::Minus);
        assert!(modifiers.ctrl);
    }

    #[test]
    fn the_script_keeps_mains_timing() {
        let at: Vec<Duration> = schedule("right wait enter*3 bogus escape")
            .into_iter()
            .map(|(at, _)| at)
            .collect();
        let ms = Duration::from_millis;
        assert_eq!(
            at,
            [
                ms(3000),
                // 250 ms after `right`, then the second's wait.
                ms(4250),
                ms(4280),
                ms(4310),
                // A bad key still takes its turn.
                ms(4810),
            ]
        );
    }

    #[test]
    fn a_press_is_taken_once_and_only_when_it_is_due() {
        let mut script = KeyScript {
            start: Instant::now(),
            presses: schedule("down"),
            next: 0,
        };
        // The first key waits for the window to open (`FIRST_KEY_AFTER`).
        assert!(script.take_due().is_none());
        script.start -= Duration::from_secs(4);
        let press = script.take_due().expect("due after the first delay");
        let mut input = egui::RawInput::default();
        press.inject(&mut input);
        assert_eq!(input.events.len(), 2);
        assert!(matches!(
            input.events[0],
            Event::Key {
                key: Key::ArrowDown,
                pressed: true,
                ..
            }
        ));
        assert!(matches!(input.events[1], Event::Key { pressed: false, .. }));
        assert!(script.take_due().is_none());
    }

    #[test]
    fn routes_parse() {
        assert_eq!(Start::parse("search:test"), Some(Start::Search("test".into())));
        assert_eq!(Start::parse("settings"), Some(Start::Open(Route::Settings)));
        assert_eq!(
            Start::parse("play:abc::-1"),
            Some(Start::Play {
                id: "abc".into(),
                audio: None,
                subtitle: Some(-1),
                source: None,
            })
        );
        assert_eq!(
            Start::parse("play:abc:1:2:src"),
            Some(Start::Play {
                id: "abc".into(),
                audio: Some(1),
                subtitle: Some(2),
                source: Some("src".into()),
            })
        );
        assert!(matches!(
            Start::parse("library:lib:movies"),
            Some(Start::Open(Route::Library {
                collection_type: Some(jellyfin::models::CollectionType::Movies),
                ..
            }))
        ));
        assert_eq!(
            Start::parse("person:abc"),
            Some(Start::Open(Route::Person {
                id: "abc".into(),
                name: "Person".into(),
            }))
        );
        assert_eq!(Start::parse("item"), None);
        assert_eq!(Start::parse("nowhere"), None);
    }
}
