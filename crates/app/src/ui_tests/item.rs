//! The item page's seasons through the keyboard: Enter on a season row opens its episodes under
//! the seasons, which at the app's window sizes start below the fold, so the page scrolls them
//! into view (X5). An episode's series line, which opens the series. And C11's cozy pieces: the
//! actions' names and toggles, season rows, and a playlist's handles on the active row.

use egui::Key;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
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

/// A movie's actions: playing it is one button, Watched and Favorite are toggles, and More is a
/// plain button (C11's quiet icon buttons name themselves to AccessKit).
#[test]
fn the_actions_name_themselves_and_the_toggles_say_their_state() {
    let _config = ConfigDir::new("item-actions");
    let server = FixtureServer::start();
    let movie: serde_json::Value = fixture_json("item_movie");
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
    ui.step_until("the movie's actions", |harness| {
        harness.query_by_label("More").is_some()
    });

    let harness = ui.harness();
    for name in ["Watched", "Favorite"] {
        let toggle = harness.get_by_role_and_label(Role::Button, name);
        assert!(
            toggle.accesskit_node().toggled().is_some(),
            "{name:?} reads as a toggle"
        );
    }
    let more = harness.get_by_role_and_label(Role::Button, "More");
    assert_eq!(more.accesskit_node().toggled(), None);
}

/// A series' seasons are rows: clicking one marks it open and lists its episodes under the
/// seasons, as rows too.
#[test]
fn a_season_row_lists_its_episodes() {
    let _config = ConfigDir::new("item-season-rows");
    let server = series_server();
    let mut ui = series_harness(&server);
    ui.step_until("the series' seasons", |harness| {
        harness.query_by_label("Seasons").is_some()
    });
    let season = ui.harness().get_by_role_and_label(Role::Button, "Season 1");
    assert_eq!(season.accesskit_node().toggled(), Some(Toggled::False));
    let at = season.rect().center();

    ui.click_at(at);
    ui.step_until("the season's episodes", |harness| {
        harness.query_all_by_role(Role::Button).any(|row| {
            row.accesskit_node()
                .label()
                .is_some_and(|label| label.contains("S01E01"))
        })
    });
    let season = ui.harness().get_by_role_and_label(Role::Button, "Season 1");
    assert_eq!(
        season.accesskit_node().toggled(),
        Some(Toggled::True),
        "the open season is marked"
    );
}

/// The playlist the tests make up from the fixtures' episodes.
const PLAYLIST: &str = "5a1e7c0ffee0000000000000000000aa";

/// A playlist's ↑ ↓ ✕ show only on the row the cursor (or the pointer) is on, and Up is
/// disabled on the first row.
#[test]
fn a_playlists_handles_show_on_the_active_row() {
    let _config = ConfigDir::new("item-playlist-handles");
    let server = FixtureServer::start();
    let playlist = serde_json::json!({ "Id": PLAYLIST, "Name": "Watchlist", "Type": "Playlist" });
    server.json_value("GET", "/Items/5a1e7c0ffee0000000000000000000aa", &playlist);
    let mut entries: serde_json::Value = fixture_json("episodes");
    for entry in entries["Items"].as_array_mut().expect("episodes") {
        entry["PlaylistItemId"] = entry["Id"].clone();
    }
    server.json_value("GET", "/Playlists/5a1e7c0ffee0000000000000000000aa/Items", &entries);
    let mut ui = PageHarness::new(session(&server), move |cx| {
        crate::views::open(
            Route::Item {
                id: PLAYLIST.to_owned(),
                name: "Watchlist".to_owned(),
            },
            cx,
        )
    });
    ui.step_until("the playlist's rows", |harness| {
        harness.query_by_label("Playlist").is_some()
    });
    let removes =
        |harness: &Harness<'static, PageRun>| harness.query_all_by_label("Remove from playlist (Delete)").count();
    assert_eq!(removes(ui.harness()), 0, "no handles before a row is active");

    // The first Tab shows the cursor on the actions; the second moves it to the first entry.
    ui.press(Key::Tab);
    ui.press(Key::Tab);
    ui.step();
    assert_eq!(removes(ui.harness()), 1, "the cursor's row has its handles");
    let up = ui.harness().get_by_label("Move up (Alt+Up)");
    assert!(up.accesskit_node().is_disabled(), "the first entry can't move up");

    ui.press(Key::ArrowDown);
    ui.step();
    assert_eq!(removes(ui.harness()), 1, "the handles moved with the cursor");
    let up = ui.harness().get_by_label("Move up (Alt+Up)");
    assert!(!up.accesskit_node().is_disabled());
}
