//! The popup menus (`widgets/popup.rs`) on a real page: a card's right-click menu and the item
//! page's More menu, opened, walked and picked from.

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use jellyfin::models::BaseItemDto;

use crate::nav::Route;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, block_on, fixture_json, session};

/// The series the fixtures give two seasons, and its first season.
const SERIES: &str = "129db440ee6554a42e5d8f1350516bab";
const SEASON_1: &str = "0173092548b085805d91ffea12e3fb36";

fn series_page() -> (FixtureServer, PageHarness) {
    let server = FixtureServer::start();
    server.json("GET", "/Items/129db440ee6554a42e5d8f1350516bab", "item_series");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Seasons", "seasons");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Episodes", "episodes");
    server.status("POST", "/UserFavoriteItems/0173092548b085805d91ffea12e3fb36", 204);
    let session = session(&server);
    let series: BaseItemDto = fixture_json("item_series");
    let name = series.name;
    let mut ui = PageHarness::new(session, move |cx| {
        crate::views::open(
            Route::Item {
                id: SERIES.to_owned(),
                name,
            },
            cx,
        )
    });
    ui.step_until("the series' seasons", |harness| {
        harness.query_by_label("Seasons").is_some()
    });
    (server, ui)
}

/// The labels of the open menu's rows with `role`.
fn rows(ui: &PageHarness, role: Role) -> Vec<String> {
    ui.harness()
        .query_all_by_role(role)
        .filter_map(|node| node.accesskit_node().label())
        .collect()
}

/// A key straight to egui, as the app hands it over while a popup is open
/// (`keys::egui_wants_keys`): the menu reads it, not the page's bindings.
fn menu_key(ui: &mut PageHarness, key: Key) {
    ui.harness.key_press(key);
    ui.step();
}

#[test]
fn a_right_click_opens_the_cards_menu_and_a_row_picks() {
    let _config = ConfigDir::new("menus-context");
    let (server, mut ui) = series_page();

    // The seasons start below the fold: the cursor on them brings them into view.
    ui.press(Key::Tab);
    ui.press(Key::Tab);
    ui.step_until("the first season on screen", |harness| {
        let window = harness.ctx.input(|input| input.viewport_rect());
        window.contains_rect(harness.get_by_label_contains("Season 1").rect())
    });
    let season = ui.harness().get_by_label_contains("Season 1");
    season.click_secondary();
    // A frame for the click, one for the menu's sizing pass, one to draw it.
    for _ in 0..3 {
        ui.step();
    }
    assert_eq!(rows(&ui, Role::MenuItem), ["Play", "Add to playlist…"]);
    // Watched and Favorite say they're toggles, and that both are off.
    assert_eq!(rows(&ui, Role::MenuItemCheckBox), ["Watched", "Favorite"]);
    let favorite = ui.harness().get_by_role_and_label(Role::MenuItemCheckBox, "Favorite");
    assert_eq!(
        favorite.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );

    favorite.click();
    for _ in 0..3 {
        ui.step();
    }
    assert!(
        rows(&ui, Role::MenuItemCheckBox).is_empty(),
        "picking a row closes the menu"
    );
    ui.step_until("the favorite request", |_| {
        let mock = server.mock.clone();
        let requests = block_on(async move { mock.received_requests().await.unwrap_or_default() });
        requests
            .iter()
            .any(|request| request.method.as_str() == "POST" && request.url.path().ends_with(SEASON_1))
    });
}

#[test]
fn enter_on_more_opens_its_menu_for_the_keyboard() {
    let _config = ConfigDir::new("menus-more");
    let (_server, mut ui) = series_page();

    // The first key shows the cursor on the actions row; Right stops at More, the last.
    for _ in 0..10 {
        ui.press(Key::ArrowRight);
    }
    ui.press(Key::Enter);
    ui.step();
    let add = ui.harness().get_by_role_and_label(Role::MenuItem, "Add to playlist…");
    assert!(add.rect().is_positive(), "the menu is open");

    // Tab stays in the menu, and Esc closes it.
    menu_key(&mut ui, Key::Tab);
    assert_eq!(rows(&ui, Role::MenuItem), ["Add to playlist…"]);
    menu_key(&mut ui, Key::Escape);
    ui.step();
    assert!(rows(&ui, Role::MenuItem).is_empty(), "Esc closes the menu");
}
