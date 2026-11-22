//! The watch history page: the played items under the day they were last played, asked for
//! newest first, and a card opening its item.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

use crate::nav::{NavRequest, Route};
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, block_on, session};

#[test]
fn played_items_are_listed_under_their_day_newest_first() {
    let _config = ConfigDir::new("history-page");
    let server = FixtureServer::start();
    // The resume fixture holds one episode, last played on 14 October.
    server.json("GET", "/Items", "resume");
    let mut ui = PageHarness::new(session(&server), |cx| crate::views::open(Route::History, cx));
    ui.step_until("the history", |harness| {
        harness
            .query_by_role_and_label(Role::Button, "Test Show S02E01")
            .is_some()
    });
    assert!(ui.harness().query_by_label("2026-10-14").is_some(), "its day's heading");

    let mock = server.mock.clone();
    let requests = block_on(async move { mock.received_requests().await.unwrap_or_default() });
    let query = requests
        .iter()
        .find(|request| request.url.path() == "/Items")
        .expect("the list request")
        .url
        .query()
        .unwrap_or_default()
        .to_owned();
    assert!(query.contains("filters=IsPlayed"), "{query}");
    assert!(query.contains("sortBy=DatePlayed"), "{query}");
    assert!(query.contains("sortOrder=Descending"), "{query}");

    ui.harness()
        .get_by_role_and_label(Role::Button, "Test Show S02E01")
        .click();
    for _ in 0..3 {
        ui.step();
    }
    assert!(matches!(
        ui.state().nav.as_slice(),
        [NavRequest::Push(Route::Item { .. })]
    ));
}
