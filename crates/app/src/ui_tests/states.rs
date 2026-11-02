//! The page states (`widgets/state.rs`) on real pages: a failed load is a danger notice with a
//! *Retry* that loads again, on a library and on search, and an empty search is the empty state.

use egui_kittest::kittest::Queryable;
use jellyfin::models::CollectionType;

use crate::nav::Route;
use crate::ui_tests::{FixtureServer, PageHarness, session};
use crate::views::Page;
use crate::views::search::SearchPage;

fn library_harness(server: &FixtureServer) -> PageHarness {
    PageHarness::new(session(server), |cx| {
        crate::views::open(
            Route::Library {
                id: "ui-test-movies".into(),
                name: "Movies".into(),
                collection_type: Some(CollectionType::Movies),
                newest_first: false,
            },
            cx,
        )
    })
}

fn search_harness(server: &FixtureServer) -> PageHarness {
    PageHarness::new(session(server), |cx| {
        let mut page = SearchPage::default();
        page.set_search_term("dune", cx);
        Box::new(page)
    })
}

/// Frames until a failed load's *Retry* is up.
fn wait_for_retry(ui: &mut PageHarness) {
    ui.step_until("the failed state", |harness| harness.query_by_label("Retry").is_some());
}

/// Clicks *Retry* and waits for the page to ask the server again.
fn retry(ui: &mut PageHarness, server: &FixtureServer) {
    let before = server.sorts_requested().len();
    ui.harness().get_by_label("Retry").click();
    ui.step_until("the page to load again", |_| server.sorts_requested().len() > before);
}

/// A library the server can't list shows the error as a danger notice (its icon named "Error"
/// to a screen reader), and *Retry* asks for the items again.
#[test]
fn a_failed_library_shows_a_notice_and_retries() {
    let server = FixtureServer::start();
    server.status("GET", "/Items", 500);
    server.json("GET", "/Items/Filters", "item_filters");
    let mut ui = library_harness(&server);
    wait_for_retry(&mut ui);
    assert!(ui.harness().query_by_label("Error").is_some(), "no danger notice");
    retry(&mut ui, &server);
}

/// A failed search had only a line of text; it now has *Retry*, which runs the search again.
#[test]
fn a_failed_search_can_be_retried() {
    let server = FixtureServer::start();
    server.status("GET", "/Items", 500);
    let mut ui = search_harness(&server);
    wait_for_retry(&mut ui);
    assert!(ui.harness().query_by_label("Error").is_some(), "no danger notice");
    retry(&mut ui, &server);
}

/// A search that finds nothing is the empty state, with no Retry.
#[test]
fn an_empty_search_is_the_empty_state() {
    let server = FixtureServer::start();
    server.json_value(
        "GET",
        "/Items",
        &serde_json::json!({ "Items": [], "TotalRecordCount": 0, "StartIndex": 0 }),
    );
    let mut ui = search_harness(&server);
    ui.step_until("no results", |harness| harness.query_by_label("No results.").is_some());
    assert!(ui.harness().query_by_label("Retry").is_none());
    assert!(ui.harness().query_by_label("Error").is_none());
}
