//! Home's *My Media* row: the libraries are tiles (C9), each a button named by the library,
//! which a click or the cursor's Enter opens.

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use jellyfin::models::{BaseItemDto, QueryResult};

use crate::nav::{NavRequest, Route};
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, fixture_json, session};

/// A mocked server with the fixtures' libraries, resume and latest items.
fn fixture_server() -> FixtureServer {
    let server = FixtureServer::start();
    server.json("GET", "/UserViews", "user_views");
    server.json("GET", "/UserItems/Resume", "resume");
    server.json("GET", "/Shows/NextUp", "next_up");
    server.json("GET", "/Items/Latest", "latest");
    server.json("GET", "/Items", "items_movies");
    server
}

fn views() -> Vec<BaseItemDto> {
    let views: QueryResult<BaseItemDto> = fixture_json("user_views");
    views.items
}

fn home(server: &FixtureServer) -> PageHarness {
    let mut ui = PageHarness::new(session(server), |cx| crate::views::open(Route::Home, cx));
    ui.step_until("Home's shelves", |harness| harness.query_by_label("My Media").is_some());
    ui
}

#[test]
fn the_libraries_are_tiles_a_click_opens() {
    let _config = ConfigDir::new("home-tiles");
    let server = fixture_server();
    let mut ui = home(&server);
    let views = views();
    for view in &views {
        assert!(
            ui.harness().query_by_role_and_label(Role::Button, &view.name).is_some(),
            "a tile for {}",
            view.name
        );
    }
    let movies = views
        .iter()
        .find(|view| view.name == "Movies")
        .expect("a Movies library");
    ui.harness().get_by_role_and_label(Role::Button, "Movies").click();
    for _ in 0..3 {
        ui.step();
    }
    assert_eq!(ui.state().nav, [NavRequest::Push(Route::for_item(movies))]);
}

/// The cursor starts on the first tile, as on any shelf's first card, and Enter opens the one
/// it's on.
#[test]
fn enter_opens_the_tile_under_the_cursor() {
    let _config = ConfigDir::new("home-tiles-cursor");
    let server = fixture_server();
    let mut ui = home(&server);
    let views = views();
    ui.press(Key::Enter);
    assert!(ui.state().nav.is_empty(), "the first Enter only shows the cursor");
    ui.press(Key::ArrowRight);
    ui.press(Key::Enter);
    assert_eq!(ui.state().nav, [NavRequest::Push(Route::for_item(&views[1]))]);
}
