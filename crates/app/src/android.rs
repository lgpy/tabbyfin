//! What winit 0.30 drops on Android, passed on to egui: the window insets (status bar, gesture
//! bar, cutouts and the soft keyboard), as egui's safe area, and the soft keyboard's text, as
//! IME events and its action key as Enter, and the system's Back, as Escape. `MainActivity.java`
//! reports them; [`raw_input`] applies them each frame.
//!
//! The state is process-wide because Java calls in through plain functions with no handle to the
//! app. It's set once, by `android_main`, before eframe starts.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::input::{ImeOptions, InputType, TextInputAction, TextInputState, TextSpan};
use winit::platform::android::activity::ndk::configuration::UiModeNight;

static APP: OnceLock<AndroidApp> = OnceLock::new();
/// Repainted when Java reports a change, since winit wakes nothing for it.
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
/// Left, top, right and bottom system bar insets, then the keyboard's height, in pixels.
static INSETS: [AtomicI32; 5] = [const { AtomicI32::new(0) }; 5];
/// Set when the keyboard's text changed, so it's read only then: android-activity 0.6.1 builds a
/// slice from a null pointer when it's read before the keyboard has had any text.
static TEXT_CHANGED: AtomicBool = AtomicBool::new(false);
/// Set when the keyboard's action key was pressed, which the app takes as Enter.
static ENTER: AtomicBool = AtomicBool::new(false);
/// Whether Back has somewhere to go in the app, as of the last frame; Java asks before passing
/// it on.
static GOES_BACK: AtomicBool = AtomicBool::new(false);
/// Set when Back was passed on, which the app takes as Escape.
static BACK: AtomicBool = AtomicBool::new(false);
/// Whether the keyboard was last set up for a password, which it mustn't suggest or learn from.
static PASSWORD: AtomicBool = AtomicBool::new(false);
static KEYBOARD: Mutex<Keyboard> = Mutex::new(Keyboard {
    finished: String::new(),
    word: String::new(),
    focus: None,
});

/// The keyboard's text as the field has been told it. GameActivity keeps the keyboard's own copy
/// of what was typed into the focused field, and the field holds the real text: each change to
/// the keyboard's copy is passed on as the difference from what it was. It's never written back
/// while typing, since the keyboard may have moved on by then and that would lose what it typed;
/// it's emptied when another field takes the focus.
struct Keyboard {
    /// What the keyboard has finished: its text outside the composing region.
    finished: String,
    /// The word it's composing, which the field shows as IME pre-edit text.
    word: String,
    /// The field the text is going to.
    focus: Option<egui::Id>,
}

pub fn init(app: AndroidApp) {
    let _ = APP.set(app);
}

pub fn set_context(ctx: egui::Context) {
    let _ = CONTEXT.set(ctx);
}

/// Adds the frame's safe area and keyboard text to egui's input.
pub fn raw_input(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let px = |i: usize| INSETS[i].load(Ordering::Relaxed) as f32 / ctx.pixels_per_point();
    raw_input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
        left: px(0),
        top: px(1),
        right: px(2),
        // The keyboard's inset includes the gesture bar beneath it.
        bottom: px(3).max(px(4)),
    }));
    if let Some(app) = APP.get() {
        // winit reports no system theme on Android; the activity's night mode is it.
        raw_input.system_theme = match app.config().ui_mode_night() {
            UiModeNight::Yes => Some(egui::Theme::Dark),
            UiModeNight::No => Some(egui::Theme::Light),
            _ => None,
        };
        keyboard_text(ctx, app, &mut raw_input.events);
        for (flag, key) in [(&ENTER, egui::Key::Enter), (&BACK, egui::Key::Escape)] {
            if flag.swap(false, Ordering::Relaxed) {
                for pressed in [true, false] {
                    raw_input.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
    }
}

/// Turns what the soft keyboard typed into egui events: Backspace for what it deleted, an IME
/// commit for what it finished, and pre-edit text for the word it's composing. Deleting past
/// what the keyboard holds (text that was in the field before) arrives as Backspace key presses,
/// which winit does pass on.
fn keyboard_text(ctx: &egui::Context, app: &AndroidApp, events: &mut Vec<egui::Event>) {
    let mut keyboard = KEYBOARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let focus = ctx.memory(|memory| memory.focused());
    if focus != keyboard.focus {
        keyboard.focus = focus;
        keyboard.finished.clear();
        keyboard.word.clear();
        app.set_text_input_state(TextInputState {
            text: String::new(),
            selection: TextSpan { start: 0, end: 0 },
            compose_region: None,
        });
        return;
    }
    if !TEXT_CHANGED.swap(false, Ordering::Relaxed) {
        return;
    }
    let state = app.text_input_state();
    tracing::debug!(?state, "keyboard text");
    let composing = state
        .compose_region
        .filter(|region| region.start < region.end)
        .map(|region| byte_range(&state.text, region));
    let (finished, word) = match composing {
        Some(range) => (
            format!("{}{}", &state.text[..range.start], &state.text[range.end..]),
            state.text[range].to_owned(),
        ),
        None => (state.text.clone(), String::new()),
    };
    if finished == keyboard.finished && word == keyboard.word {
        return;
    }

    // The pre-edit text goes first, so the deletes and the commit land where it was.
    if !keyboard.word.is_empty() {
        events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        }));
    }
    let kept = keyboard
        .finished
        .chars()
        .zip(finished.chars())
        .take_while(|(old, new)| old == new)
        .count();
    for _ in kept..keyboard.finished.chars().count() {
        events.push(egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    let typed: String = finished.chars().skip(kept).collect();
    if !typed.is_empty() {
        events.push(egui::Event::Ime(egui::ImeEvent::Commit(typed)));
    }
    if !word.is_empty() {
        let chars = word.chars().count();
        events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: word.clone(),
            active_range_chars: Some(chars..chars),
        }));
    }
    keyboard.finished = finished;
    keyboard.word = word;
}

/// Takes what a frame decided: whether Back has somewhere to go (`goes_back`), and the keyboard
/// for the focused field (`ime` is the frame's IME output): a password field gets a password
/// keyboard, which shows no suggestions and learns nothing.
pub fn after_frame(ime: Option<egui::output::IMEOutput>, goes_back: bool) {
    GOES_BACK.store(goes_back, Ordering::Relaxed);
    let Some(ime) = ime else {
        return;
    };
    let password = ime.purpose == egui::IMEPurpose::Password;
    if PASSWORD.swap(password, Ordering::Relaxed) == password {
        return;
    }
    if let Some(app) = APP.get() {
        let (input_type, options) = if password {
            (
                InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_VARIATION_PASSWORD,
                ImeOptions::IME_FLAG_NO_PERSONALIZED_LEARNING,
            )
        } else {
            (InputType::TYPE_CLASS_TEXT, ImeOptions::IME_NULL)
        };
        app.set_ime_editor_info(input_type, TextInputAction::Done, options);
    }
}

/// Sets the environment variables listed in the system property `debug.tabbyfin.env`
/// (`NAME=value;NAME=value`), the stand-in for the desktop's environment, which an app on Android
/// doesn't get: `adb shell setprop debug.tabbyfin.env "WGPU_BACKEND=vulkan;RUST_LOG=debug"`, then
/// restart the app. On the emulator, also picks the GL backend unless told otherwise. Call it
/// before any other thread starts.
pub fn env_from_property() {
    let properties = android_system_properties::AndroidSystemProperties::new();
    if let Some(vars) = properties.get("debug.tabbyfin.env") {
        for (name, value) in vars.split(';').filter_map(|pair| pair.split_once('=')) {
            // SAFETY: `android_main` calls this before starting any thread.
            unsafe { std::env::set_var(name.trim(), value.trim()) };
        }
    }
    // The emulator's software Vulkan draws the first frames and then times out on every one
    // after; its GLES works.
    let emulator = properties
        .get("ro.hardware")
        .is_some_and(|hw| hw == "ranchu" || hw == "goldfish");
    if emulator && std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: as above.
        unsafe { std::env::set_var("WGPU_BACKEND", "gl") };
    }
}

/// The phone's model ("Pixel 7"), which Jellyfin shows as the device's name.
pub fn device_name() -> Option<String> {
    android_system_properties::AndroidSystemProperties::new()
        .get("ro.product.model")
        .filter(|model| !model.is_empty())
}

/// The byte range of `span`, which counts UTF-16 units (Java's string indices), in `text`.
fn byte_range(text: &str, span: TextSpan) -> std::ops::Range<usize> {
    let mut units = 0;
    let mut start = text.len();
    let mut end = text.len();
    for (byte, ch) in text.char_indices() {
        if units == span.start {
            start = byte;
        }
        if units == span.end {
            end = byte;
            break;
        }
        units += ch.len_utf16();
    }
    start.min(end)..end
}

fn repaint() {
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_lgpy_tabbyfin_MainActivity_nativeInsets(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    keyboard: i32,
) {
    tracing::debug!(left, top, right, bottom, keyboard, "insets");
    for (slot, value) in INSETS.iter().zip([left, top, right, bottom, keyboard]) {
        slot.store(value, Ordering::Relaxed);
    }
    repaint();
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_lgpy_tabbyfin_MainActivity_nativeTextChanged(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    TEXT_CHANGED.store(true, Ordering::Relaxed);
    repaint();
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_lgpy_tabbyfin_MainActivity_nativeEditorAction(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    ENTER.store(true, Ordering::Relaxed);
    repaint();
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_lgpy_tabbyfin_MainActivity_nativeGoesBack(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) -> u8 {
    GOES_BACK.load(Ordering::Relaxed).into()
}

#[unsafe(no_mangle)]
extern "system" fn Java_io_github_lgpy_tabbyfin_MainActivity_nativeBack(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
) {
    BACK.store(true, Ordering::Relaxed);
    repaint();
}
