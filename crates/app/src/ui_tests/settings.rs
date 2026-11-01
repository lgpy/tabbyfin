//! The Settings page by keyboard and pointer: Tab goes through the search box and the menu,
//! then puts egui's focus on a picker, and Left and Right step it, saving the same value a click
//! gives. A picker only steps towards a choice it has, so an arrow at either end of its list
//! does nothing. Every section is on the page; a menu entry scrolls to one. The server's
//! settings load from the user's configuration and save back to it.

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

/// Where a section's heading is, and whether its menu entry is the one marked.
fn section_state(harness: &Harness<'static, PageRun>, title: &str) -> (f32, bool) {
    let nodes = || harness.query_all_by_label(title);
    let heading = nodes()
        .find(|node| node.accesskit_node().role() == Role::Label)
        .unwrap_or_else(|| panic!("{title}'s heading"));
    let entry = nodes()
        .find(|node| node.accesskit_node().role() == Role::Button)
        .unwrap_or_else(|| panic!("{title}'s menu entry"));
    (
        heading.rect().top(),
        entry.accesskit_node().toggled() == Some(Toggled::True),
    )
}

fn click_menu_entry(ui: &mut PageHarness, title: &str) {
    ui.harness()
        .query_all_by_label(title)
        .find(|node| node.accesskit_node().role() == Role::Button)
        .unwrap_or_else(|| panic!("{title}'s menu entry"))
        .click();
    ui.step();
}

/// Every section is on the page, the device's and the server's, each store's under its banner.
/// A menu entry scrolls its section to the top, and the menu marks it once it's there.
#[test]
fn the_menu_scrolls_to_a_section() {
    let _config = ConfigDir::new("settings-menu");
    let server = FixtureServer::start();
    serve_the_users_settings(&server);
    let mut ui = settings_harness(&server);
    ui.step_until("the user's languages", |harness| {
        harness.query_all_by_label("Audio language").next().is_some()
    });
    // The first section is at the top and marked; the server's are further down, loaded.
    assert!(ui.harness().query_all_by_label("This device").next().is_some());
    assert!(ui.harness().query_all_by_label("Theme").next().is_some());
    assert!(
        ui.harness()
            .query_all_by_label("Leave watched items out of Latest")
            .next()
            .is_some()
    );
    let (appearance_top, marked) = section_state(ui.harness(), "Appearance");
    assert!(marked, "Appearance is marked at the start");
    let (languages_top, marked) = section_state(ui.harness(), "Languages");
    assert!(!marked);
    assert!(
        languages_top > 700.,
        "Languages starts below the window: {languages_top}"
    );

    click_menu_entry(&mut ui, "Languages");
    ui.step_until("Languages at the top", |harness| {
        let (top, marked) = section_state(harness, "Languages");
        top < 80. && marked
    });
    let (_, marked) = section_state(ui.harness(), "Appearance");
    assert!(!marked, "only the section at the top is marked");

    // Home, the last section, is short, but the page has room for it to reach the top too.
    click_menu_entry(&mut ui, "Home");
    ui.step_until("Home at the top", |harness| {
        let (top, marked) = section_state(harness, "Home");
        top < 80. && marked
    });

    // And back up to the first section, where it was at the start.
    click_menu_entry(&mut ui, "Appearance");
    ui.step_until("Appearance at the top", |harness| {
        let (top, marked) = section_state(harness, "Appearance");
        (top - appearance_top).abs() < 1. && marked
    });
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

    // Esc in the box clears the search, and every section is back.
    ui.press(Key::Escape);
    ui.step();
    assert!(ui.harness().query_all_by_label("Theme").next().is_some());
    assert!(ui.harness().query_all_by_label("Subtitle mode").next().is_some());

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
