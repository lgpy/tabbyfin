//! The item page's seasons through the keyboard: Enter on a season card opens its episodes
//! band under the seasons, which at the app's window sizes starts below the fold, so the page
//! scrolls it into view (X5).

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jellyfin::models::BaseItemDto;

use crate::nav::Route;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, PageRun, fixture_json, session};

/// The series the fixtures give two seasons and three episodes.
const SERIES: &str = "129db440ee6554a42e5d8f1350516bab";

/// A mocked server with the series, its seasons and its episodes.
fn series_server() -> FixtureServer {
    let server = FixtureServer::start();
    server.json("GET", "/Items/129db440ee6554a42e5d8f1350516bab", "item_series");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Seasons", "seasons");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Episodes", "episodes");
    server
}

/// The series' page over the mocked server, on screen and ready to be driven.
fn series_harness(server: &FixtureServer) -> PageHarness {
    let session = session(server);
    let series: BaseItemDto = fixture_json("item_series");
    let name = series.name;
    PageHarness::new(session, move |cx| {
        crate::views::open(
            Route::Item {
                id: SERIES.to_owned(),
                name,
            },
            cx,
        )
    })
}

/// Whether an episode card of the open season is drawn inside the window: the band's cards
/// are laid out below the seasons, so only a scroll can bring them up.
fn episode_in_view(harness: &Harness<'static, PageRun>) -> bool {
    let window = harness.ctx.input(|input| input.viewport_rect());
    harness.query_all_by_label_contains("S01E01").any(|card| {
        let rect = card.rect();
        rect.min.y >= window.min.y && rect.max.y <= window.max.y
    })
}

/// Enter opens the season under the cursor and scrolls its episodes into view: at the app's
/// default window size the band starts below the fold, so the key would otherwise look like a
/// no-op until the next one.
#[test]
fn enter_on_a_season_brings_its_episodes_into_view() {
    let _config = ConfigDir::new("item-season-band");
    let server = series_server();
    let mut ui = series_harness(&server);
    ui.step_until("the series' seasons", |harness| {
        harness.query_by_label("Seasons").is_some()
    });

    // The first Tab only shows the cursor on the actions row; the second moves it to the
    // seasons.
    ui.press(Key::Tab);
    ui.press(Key::Tab);
    assert!(
        ui.state().handled.last().copied().unwrap_or_default(),
        "the seasons are a section of their own"
    );
    assert!(!episode_in_view(ui.harness()), "no band before a season opened");

    ui.press(Key::Enter);
    ui.step_until("the band's first episode in view", episode_in_view);
}
