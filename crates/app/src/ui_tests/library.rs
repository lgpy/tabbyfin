//! The library grid and its toolbar through the keyboard: the browse cursor (hidden until the
//! first key, moved by the arrows and Tab, Enter opening the card under it) and the toolbar's
//! pickers stepping to their next choice. The grid's items are the captured `/Items` fixture.

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use jellyfin::models::{BaseItemDto, CollectionType, QueryResult};

use crate::config::{Config, LibraryView};
use crate::nav::{NavRequest, Route};
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, fixture_json, session};

/// The library id the tests remember their view under, in their own config directory.
const LIBRARY: &str = "ui-test-movies";

/// A library page over the mocked server, on screen and ready to be driven.
fn library_harness(server: &FixtureServer, id: &str) -> PageHarness {
    let session = session(server);
    let id = id.to_owned();
    PageHarness::new(session, move |cx| {
        crate::views::open(
            Route::Library {
                id,
                name: "Movies".into(),
                collection_type: Some(CollectionType::Movies),
                newest_first: false,
            },
            cx,
        )
    })
}

/// A mocked server whose library has the fixture's two movies.
fn fixture_server() -> FixtureServer {
    let server = FixtureServer::start();
    server.json("GET", "/Items", "items_movies");
    server.json("GET", "/Items/Filters", "item_filters");
    server
}

/// The fixture's movies, in the grid's order (sorted by name).
fn movies() -> Vec<BaseItemDto> {
    let items: QueryResult<BaseItemDto> = fixture_json("items_movies");
    items.items
}

/// Frames until the grid and the toolbar are both there: the item count comes with the first
/// page, the year picker with the library's filter values.
fn wait_for_page(ui: &mut PageHarness) {
    // The year picker is a combo box, so its text is the node's value rather than its label.
    ui.step_until("the library page", |harness| {
        harness.query_by_label("2 items").is_some() && harness.query_by_value("Any year").is_some()
    });
}

fn saved_view() -> LibraryView {
    Config::load().library_views.get(LIBRARY).cloned().unwrap_or_default()
}

/// The cursor stays hidden until a key is pressed, so the first Enter shows it instead of
/// opening whatever it was on; the arrows then move it and Enter opens that card.
#[test]
fn the_cursor_shows_on_the_first_key_and_opens_the_card() {
    let _config = ConfigDir::new("library-cursor");
    let server = fixture_server();
    let mut ui = library_harness(&server, LIBRARY);
    wait_for_page(&mut ui);
    let movies = movies();

    ui.press(Key::Enter);
    assert!(ui.state().nav.is_empty(), "Enter opened a card the cursor wasn't on");
    assert!(ui.state().handled.last().copied().unwrap_or_default());

    ui.press(Key::ArrowRight);
    ui.press(Key::Enter);
    assert_eq!(
        ui.state().nav,
        [NavRequest::Push(Route::for_item(&movies[1]))],
        "Enter on the second card"
    );
}

/// Up from the grid's first row moves to the toolbar's control in the same column (it's one
/// row), where Enter presses it; Up again has nowhere to go on the page, so the key is left to
/// the shell, which moves into the top bar.
#[test]
fn up_from_the_grid_reaches_the_toolbar_and_then_the_top_bar() {
    let _config = ConfigDir::new("library-up");
    let server = fixture_server();
    let mut ui = library_harness(&server, LIBRARY);
    wait_for_page(&mut ui);

    // The first Enter only shows the cursor (on the first card); one step right puts it on the
    // second, one column in from the left.
    ui.press(Key::Enter);
    ui.press(Key::ArrowRight);
    ui.press(Key::ArrowUp);
    assert!(
        ui.state().handled.last().copied().unwrap_or_default(),
        "Up left the page"
    );
    // The toolbar ends with the clear button, the sort picker and the order button, six controls
    // on from the rating picker the cursor came up to: Enter flips the order and the page
    // remembers it.
    for _ in 0..6 {
        ui.press(Key::ArrowRight);
    }
    ui.press(Key::Enter);
    assert!(saved_view().descending, "Enter on the order button sorted descending");
    // Nothing is above the toolbar's only row: Up goes to the top bar.
    ui.press(Key::ArrowUp);
    assert!(
        !ui.state().handled.last().copied().unwrap_or_default(),
        "the page kept Up"
    );
    // The next Down shows the cursor again where it was rather than moving it.
    ui.press(Key::ArrowDown);
    assert!(ui.state().handled.last().copied().unwrap_or_default());
    ui.press(Key::Enter);
    assert!(!saved_view().descending, "the order button was still selected");
}

/// A click on the page puts the cursor away, as for mouse users: the next Enter only shows it
/// again.
#[test]
fn a_click_puts_the_cursor_away() {
    let _config = ConfigDir::new("library-click");
    let server = fixture_server();
    let mut ui = library_harness(&server, LIBRARY);
    wait_for_page(&mut ui);
    let movies = movies();

    ui.press(Key::Enter);
    ui.press(Key::ArrowRight);
    // Below the first row of cards, inside the grid's scroll area.
    ui.click_at(egui::pos2(500., 640.));
    ui.press(Key::Enter);
    assert!(ui.state().nav.is_empty(), "the click left the cursor on the card");
    ui.press(Key::Enter);
    assert_eq!(ui.state().nav, [NavRequest::Push(Route::for_item(&movies[1]))]);
}

/// Frames until the last `/Items` request asked for `wanted`: the page asks on tokio, so a key
/// that changed the view doesn't mean its request has gone out yet.
fn wait_for_sorts(ui: &mut PageHarness, server: &FixtureServer, wanted: &str) {
    let start = std::time::Instant::now();
    while !server
        .sorts_requested()
        .last()
        .is_some_and(|sort| sort.contains(wanted))
    {
        assert!(
            start.elapsed() < crate::ui_tests::TIMEOUT,
            "no request sorted by {wanted}: {:?}",
            server.sorts_requested()
        );
        ui.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The toolbar from the keyboard: Shift+Tab from the grid, Enter stepping a picker through its
/// choices (wrapping round at the end), Left and Right moving between the controls. Each step
/// is remembered in the config and asked of the server.
#[test]
fn the_toolbar_steps_through_its_choices() {
    let _config = ConfigDir::new("library-toolbar");
    let server = fixture_server();
    let mut ui = library_harness(&server, LIBRARY);
    wait_for_page(&mut ui);

    // The first Shift+Tab only shows the cursor on the grid; the second moves it to the
    // toolbar's first control, the progress picker. Enter steps the filter.
    ui.press_modifiers(Modifiers::SHIFT, Key::Tab);
    ui.press_modifiers(Modifiers::SHIFT, Key::Tab);
    ui.press(Key::Enter);
    assert_eq!(saved_view().filter, crate::views::library::FILTERS[1].0);

    // Right six times (rating, genres and tags, year, resolution and *Clear filters*, which
    // the filter just brought): the sort picker.
    for _ in 0..6 {
        ui.press(Key::ArrowRight);
    }
    ui.press(Key::Enter);
    let view = saved_view();
    assert_eq!(view.sort, crate::views::library::SORTS[1].0, "the sort stepped on");
    assert_eq!(
        view.descending,
        crate::views::library::SORTS[1].3,
        "the sort's own order"
    );
    wait_for_sorts(&mut ui, &server, "DateCreated");
    // The picker shows the choice, and five more steps wrap round to the first.
    ui.step_until("the new sort", |harness| harness.query_by_value("Date added").is_some());
    for _ in 0..crate::views::library::SORTS.len() - 1 {
        ui.press(Key::Enter);
    }
    assert_eq!(
        saved_view().sort,
        crate::views::library::SORTS[0].0,
        "stepping wraps round"
    );

    // Right once: the order button. Enter flips the order.
    ui.press(Key::ArrowRight);
    let descending = saved_view().descending;
    ui.press(Key::Enter);
    assert_eq!(saved_view().descending, !descending);
}
