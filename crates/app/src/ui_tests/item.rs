//! The item page's seasons through the keyboard: Enter on a season card opens its episodes
//! band under the seasons, which at the app's window sizes starts below the fold, so the page
//! scrolls it into view (X5). And an episode's series line, which opens the series.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jellyfin::models::BaseItemDto;

use crate::nav::{NavRequest, Route};
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

/// The series' first episode, whose page is opened by its id.
const EPISODE: &str = "14401f7e4f9a2da047069ad4a3124f9c";

/// Clicking the series' name above an episode's title opens the series' page.
#[test]
fn an_episodes_series_line_opens_the_series() {
    let _config = ConfigDir::new("item-series-link");
    let server = FixtureServer::start();
    let episodes: serde_json::Value = fixture_json("episodes");
    let episode = episodes["Items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["Id"] == EPISODE))
        .expect("the episode in the fixture");
    server.json_value("GET", "/Items/14401f7e4f9a2da047069ad4a3124f9c", episode);
    let name = episode["Name"].as_str().unwrap_or_default().to_owned();
    let mut ui = PageHarness::new(session(&server), move |cx| {
        crate::views::open(
            Route::Item {
                id: EPISODE.to_owned(),
                name,
            },
            cx,
        )
    });
    ui.step_until("the episode's series line", |harness| {
        harness.query_by_label("Test Show").is_some()
    });

    let series = ui.harness().get_by_label("Test Show").rect().center();
    ui.click_at(series);
    assert_eq!(
        ui.state().nav,
        [NavRequest::Push(Route::Item {
            id: SERIES.to_owned(),
            name: "Test Show".to_owned(),
        })]
    );
}

/// The movie the fixtures give, whose page has room for tags beside its poster.
const MOVIE: &str = "d265861eae31cee1d44b13adac305eac";

/// A movie with more tags than one row holds: they wrap onto rows that stay inside the window,
/// which is narrower than the column's 820-point cap once the poster is beside it.
#[test]
fn many_tags_wrap_inside_the_window() {
    let _config = ConfigDir::new("item-tags-wrap");
    let server = FixtureServer::start();
    let mut movie: serde_json::Value = fixture_json("item_movie");
    movie["Tags"] = (0..40).map(|index| format!("tag {index}")).collect::<Vec<_>>().into();
    server.json_value("GET", "/Items/d265861eae31cee1d44b13adac305eac", &movie);
    let name = movie["Name"].as_str().unwrap_or_default().to_owned();
    let mut ui = PageHarness::new(session(&server), move |cx| {
        crate::views::open(
            Route::Item {
                id: MOVIE.to_owned(),
                name,
            },
            cx,
        )
    });
    ui.step_until("the movie's tags", |harness| harness.query_by_label("tag 0").is_some());

    let window = ui.harness().ctx.input(|input| input.viewport_rect());
    let rows: Vec<_> = (0..40)
        .map(|index| ui.harness().get_by_label(&format!("tag {index}")).rect())
        .inspect(|tag| assert!(tag.max.x <= window.max.x, "{tag:?} runs past {window:?}"))
        .map(|tag| tag.min.y as i32)
        .collect();
    assert!(rows.windows(2).any(|pair| pair[1] > pair[0]), "the tags wrapped");
}
