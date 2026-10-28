//! The Settings page by keyboard: Tab puts egui's focus on a picker and Left and Right step it,
//! saving the same value a click gives.
//!
//! X3's sweep couldn't step the theme or the text size with Tab and the arrows. It wasn't the
//! page: a picker only steps towards a choice it has, and X3 pressed Right on the theme picker
//! while it showed *Dark*, the last of the three, so there was nowhere to go; the earlier runs
//! had used two or three Tabs, still in the top bar's buttons. The test below pins both halves:
//! the page's first Tab is the theme picker and the second the text size picker, and the steps
//! go where the config says.

use egui::Key;

use crate::config::{Config, ThemePreference};
use crate::nav::Route;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, session};

fn settings_harness(server: &FixtureServer) -> PageHarness {
    PageHarness::new(session(server), |cx| crate::views::open(Route::Settings, cx))
}

/// The page's own Tab order, then Left and Right on the two appearance pickers, each step read
/// back from the config file. The page has no key scope, so the keys are egui's, as on a real
/// keyboard.
#[test]
fn tab_and_the_arrows_step_the_appearance_pickers() {
    let _config = ConfigDir::new("settings-pickers");
    let server = FixtureServer::start();
    let mut ui = settings_harness(&server);
    ui.step();

    // Nothing has the focus yet, so the arrows are not a picker's and write nothing.
    ui.press(Key::ArrowRight);
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().theme, ThemePreference::Dark);
    assert_eq!(Config::load().text_size, None);

    // The first Tab is the theme picker. It shows the default (dark), the last of the three, so
    // Right has nowhere to go; Left steps to light, and again to the system's scheme.
    ui.press(Key::Tab);
    ui.press(Key::ArrowRight);
    assert_eq!(
        Config::load().theme,
        ThemePreference::Dark,
        "Right at the end of the list"
    );
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().theme, ThemePreference::Light);
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().theme, ThemePreference::System);
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().theme, ThemePreference::System, "Left at the start");

    // The next Tab is the text size picker, on the system's size: Right steps to 90 % and Left
    // back to following the system, which is the key's absence in the file.
    ui.press(Key::Tab);
    ui.press(Key::ArrowRight);
    assert_eq!(Config::load().text_size, Some(90));
    ui.press(Key::ArrowRight);
    assert_eq!(Config::load().text_size, Some(100));
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().text_size, Some(90));
    ui.press(Key::ArrowLeft);
    assert_eq!(Config::load().text_size, None);
}
