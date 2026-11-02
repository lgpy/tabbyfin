//! The Settings page by keyboard and pointer: Tab goes through the search box and the menu,
//! then puts egui's focus on a picker, and Left and Right step it, saving the same value a click
//! gives. A picker only steps towards a choice it has, so an arrow at either end of its list
//! does nothing. The page shows one section at a time, the one a menu entry picks. A device
//! setting off its default has an undo button that puts it back. The server's settings load
//! from the user's configuration and save back to it.

use egui::Key;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};

use crate::config::{Config, ThemePreference};
use crate::nav::Route;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, PageRun, fixture_json, session};
use crate::views::settings::Section;

fn settings_harness(server: &FixtureServer) -> PageHarness {
    PageHarness::new(session(server), |cx| crate::views::open(Route::Settings, cx))
}

/// The server's side of the page: the user as `authenticate.json` has them, the server's
/// languages, and a configuration update that's accepted.
fn serve_the_users_settings(server: &FixtureServer) {
    let auth: serde_json::Value = fixture_json("authenticate");
    server.json_value("GET", "/Users/Me", &auth["User"]);
    server.json("GET", "/Localization/Cultures", "cultures");
    server.status("POST", "/Users/Configuration", 204);
}

/// Tabs from the search box, past every menu entry, to the first control of what's shown.
fn tab_from_the_search_box(ui: &mut PageHarness) {
    for _ in 0..Section::ALL.len() + 1 {
        ui.press(Key::Tab);
    }
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

    // The first Tab is the search box, and the first control after the menu is the theme
    // picker. It shows the default (dark), the last of the three, so Right has nowhere to go;
    // Left steps to light, and again to the system's scheme.
    ui.press(Key::Tab);
    tab_from_the_search_box(&mut ui);
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

    // The theme is off its default now, so the next Tab is its undo button, and the one after
    // the text size picker, on the system's size: Right steps to 90 % and Left back to following
    // the system, which is the key's absence in the file.
    ui.press(Key::Tab);
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

/// Whether `title`'s menu entry is the one marked.
fn marked(harness: &Harness<'static, PageRun>, title: &str) -> bool {
    harness
        .query_all_by_label(title)
        .find(|node| node.accesskit_node().role() == Role::Button)
        .unwrap_or_else(|| panic!("{title}'s menu entry"))
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

fn shows(harness: &Harness<'static, PageRun>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some()
}

fn click_menu_entry(ui: &mut PageHarness, title: &str) {
    ui.harness()
        .query_all_by_label(title)
        .find(|node| node.accesskit_node().role() == Role::Button)
        .unwrap_or_else(|| panic!("{title}'s menu entry"))
        .click();
    // The frame that takes the click draws the menu before it, so the next one marks it.
    ui.step();
    ui.step();
}

/// The page opens on Appearance, and a menu entry shows its section, the device's or the
/// server's, in place of the one before.
#[test]
fn the_menu_shows_one_section_at_a_time() {
    let _config = ConfigDir::new("settings-menu");
    let server = FixtureServer::start();
    serve_the_users_settings(&server);
    let mut ui = settings_harness(&server);
    ui.step();
    assert!(shows(ui.harness(), "Theme"));
    assert!(marked(ui.harness(), "Appearance"), "Appearance is marked at the start");
    assert!(
        !shows(ui.harness(), "Audio language"),
        "the server's sections aren't shown"
    );

    click_menu_entry(&mut ui, "Languages");
    ui.step_until("the user's languages", |harness| shows(harness, "Audio language"));
    assert!(marked(ui.harness(), "Languages"));
    assert!(!marked(ui.harness(), "Appearance"), "only the section shown is marked");
    assert!(!shows(ui.harness(), "Theme"));
    assert!(
        shows(ui.harness(), "Jellyfin server"),
        "the tag says where they're kept"
    );

    click_menu_entry(&mut ui, "Home");
    ui.step_until("Home", |harness| shows(harness, "Leave watched items out of Latest"));
    assert!(!shows(ui.harness(), "Audio language"));
}

/// A setting off its default has an undo button, and the button leaves its key out of the file
/// again, so the default applies; a setting on its default has none.
#[test]
fn the_undo_button_puts_a_setting_back_to_its_default() {
    let _config = ConfigDir::new("settings-reset");
    let mut config = Config::load();
    config.theme = ThemePreference::Light;
    config.save().unwrap();
    let server = FixtureServer::start();
    let mut ui = settings_harness(&server);
    ui.step();

    let undo = || "Back to the default";
    assert_eq!(ui.harness().query_all_by_label(undo()).count(), 1, "the theme's alone");
    ui.harness().get_by_label(undo()).click();
    // The button goes in the frame after the one that takes its click.
    ui.step();
    ui.step();
    assert_eq!(Config::load().theme, ThemePreference::Dark);
    assert!(ui.harness().query_by_label(undo()).is_none());

    // A flag too: off its default it's saved, and the undo button drops the key.
    ui.harness().get_by_role_and_label(Role::Switch, "Animations").click();
    ui.step();
    ui.step();
    assert_eq!(Config::load().animations, Some(false));
    ui.harness().get_by_label(undo()).click();
    ui.step();
    ui.step();
    assert_eq!(Config::load().animations, None);
    let switch = ui.harness().get_by_role_and_label(Role::Switch, "Animations");
    assert_eq!(
        switch.accesskit_node().toggled(),
        Some(Toggled::True),
        "on, the default"
    );
}

/// A flag is a switch that saves what it's flipped to, and the account is one row naming the
/// user and where they're signed in.
#[test]
fn a_switch_saves_and_the_account_is_one_row() {
    let _config = ConfigDir::new("settings-switch");
    let server = FixtureServer::start();
    let mut ui = settings_harness(&server);
    ui.step();

    let switch = ui.harness().get_by_role_and_label(Role::Switch, "Animations");
    assert_eq!(switch.accesskit_node().toggled(), Some(Toggled::True), "on by default");
    switch.click();
    ui.step();
    assert_eq!(Config::load().animations, Some(false));
    let switch = ui.harness().get_by_role_and_label(Role::Switch, "Animations");
    assert_eq!(switch.accesskit_node().toggled(), Some(Toggled::False));

    click_menu_entry(&mut ui, "Account");
    let account = ui.harness().query_all_by_role(Role::Button).find(|node| {
        node.accesskit_node()
            .label()
            .is_some_and(|label| label.starts_with("Signed in as dev on "))
    });
    assert!(account.is_some(), "the account row");
}

/// A search shows the rows it finds from both sides, and the server's subtitle mode, stepped
/// with the keyboard, goes to the server with the rest of the user's configuration as it was.
#[test]
fn a_searched_server_setting_saves_to_the_server() {
    let _config = ConfigDir::new("settings-server");
    let server = FixtureServer::start();
    serve_the_users_settings(&server);
    let mut ui = settings_harness(&server);
    ui.step();

    // "subtitle" is on both sides: the device's look of subtitles, the server's choice of them.
    ui.harness().get_by_role(Role::TextInput).focus();
    ui.step();
    ui.harness().get_by_role(Role::TextInput).type_text("subtitle");
    ui.step_until("the server's subtitle rows", |harness| {
        harness.query_all_by_label("Subtitle mode").next().is_some()
            && harness.query_all_by_label("Theme").next().is_none()
    });
    assert!(
        ui.harness().query_all_by_label("Colour").next().is_some(),
        "the device's subtitles"
    );
    assert!(ui.harness().query_all_by_label("Theme").next().is_none(), "not a match");

    // Esc in the box clears the search, and the section shown before it is back alone.
    ui.press(Key::Escape);
    ui.step();
    assert!(ui.harness().query_all_by_label("Theme").next().is_some());
    assert!(ui.harness().query_all_by_label("Subtitle mode").next().is_none());

    // Only the subtitle mode's description says "smart", so it's the one control after the
    // menu. It's on Default; Right steps it to Smart.
    ui.harness().get_by_role(Role::TextInput).focus();
    ui.step();
    ui.harness().get_by_role(Role::TextInput).type_text("smart");
    ui.step_until("the subtitle mode", |harness| {
        harness.query_all_by_label("Subtitle mode").next().is_some()
            && harness.query_all_by_label("Theme").next().is_none()
    });
    tab_from_the_search_box(&mut ui);
    ui.press(Key::ArrowRight);
    ui.step_until("the saved configuration", |_| {
        !server.bodies("POST", "/Users/Configuration").is_empty()
    });
    let saved = server.bodies("POST", "/Users/Configuration");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["SubtitleMode"], "Smart");
    // What the page doesn't show goes back as the server had it.
    assert_eq!(saved[0]["CastReceiverId"], "F007D354");
    assert_eq!(saved[0]["HidePlayedInLatest"], true);
    // The device's config is untouched.
    assert_eq!(Config::load().theme, ThemePreference::Dark);
}

/// Enter on a picker egui's focus is on opens its list from the keyboard, on its first row; Down
/// and Enter pick from it as from any menu, and the choice saves as a step would.
#[test]
fn enter_opens_a_pickers_list_and_the_keyboard_picks_from_it() {
    let _config = ConfigDir::new("settings-picker-list");
    let server = FixtureServer::start();
    let mut ui = settings_harness(&server);
    ui.step();

    ui.press(Key::Tab);
    tab_from_the_search_box(&mut ui);
    ui.press(Key::Enter);
    ui.step();
    // The list is the theme's choices, the default (dark) checked.
    let dark = ui.harness().get_by_label("Dark");
    assert_eq!(dark.accesskit_node().role(), Role::MenuItemRadio);
    ui.press(Key::ArrowDown);
    ui.press(Key::Enter);
    ui.step();
    assert_eq!(Config::load().theme, ThemePreference::Light);
    assert!(ui.harness().query_by_label("Dark").is_none(), "the list closed");
}
