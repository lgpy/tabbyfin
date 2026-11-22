//! The subtitle search (`views/subtitle_search.rs`): its states over a mocked server, what a
//! download hands back, and who is offered it: the player's Subtitles tab and the item page's
//! More menu, only for users whose policy allows subtitle management.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use jellyfin::models::{BaseItemDto, MediaSourceInfo, UserPolicy};

use crate::nav::Route;
use crate::playback::{SubtitleChoice, UiCommand};
use crate::ui_tests::touch::PlayerRun;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, build_harness, fixture_json, session};
use crate::views::subtitle_search::{Finished, SubtitleSearch};
use crate::views::{Page, PageCx};

const MOVIE: &str = "d265861eae31cee1d44b13adac305eac";
const SEARCH: &str = "/Items/m1/RemoteSearch/Subtitles/eng";

/// A page that only holds the dialog, and notes how it finished.
struct Host {
    dialog: Option<SubtitleSearch>,
    finished: Rc<RefCell<Vec<Finished>>>,
}

impl Page for Host {
    fn title(&self) -> String {
        "Dialog".to_owned()
    }

    fn show(&mut self, _ui: &mut egui::Ui, cx: &mut PageCx) {
        let Some(dialog) = &mut self.dialog else { return };
        match dialog.show(cx) {
            Finished::Open => {}
            done => {
                self.finished.borrow_mut().push(done);
                self.dialog = None;
            }
        }
    }
}

fn movie_item(id: &str) -> BaseItemDto {
    serde_json::from_value(serde_json::json!({ "Id": id, "Name": "Big Buck Bunny", "Type": "Movie" })).expect("item")
}

fn server() -> FixtureServer {
    let server = FixtureServer::start();
    server.json("GET", "/Localization/Cultures", "cultures");
    server
}

fn search_dialog(server: &FixtureServer) -> (PageHarness, Rc<RefCell<Vec<Finished>>>) {
    let finished = Rc::new(RefCell::new(Vec::new()));
    let seen = finished.clone();
    let ui = PageHarness::new(session(server), move |cx| {
        Box::new(Host {
            dialog: Some(SubtitleSearch::new(cx, &movie_item("m1"))),
            finished: seen,
        })
    });
    (ui, finished)
}

fn results() -> serde_json::Value {
    serde_json::json!([
        { "Id": "a", "Name": "Big.Buck.Bunny.720p", "ProviderName": "Open Subtitles", "Format": "srt",
          "DownloadCount": 40, "IsHashMatch": false },
        { "Id": "b/1", "Name": "Big.Buck.Bunny.1080p", "ProviderName": "Open Subtitles", "Format": "ass",
          "DownloadCount": 1200, "IsHashMatch": true },
        { "Id": "c", "Name": "bunny", "ProviderName": null, "Format": null, "CommunityRating": 7.5 },
    ])
}

/// Each result is a row with its provider, format and popularity, and a hash match, which is
/// made for this very file, comes first and says so.
#[test]
fn results_are_rows_with_their_details_and_exact_matches_first() {
    let _config = ConfigDir::new("subtitle-search-rows");
    let server = server();
    server.json_value("GET", SEARCH, &results());
    let (mut ui, _) = search_dialog(&server);
    ui.step_until("the results", |ui| {
        ui.query_by_label_contains("Big.Buck.Bunny.1080p").is_some()
    });

    let rows: Vec<String> = ui
        .harness()
        .query_all_by_role(Role::Button)
        .filter_map(|row| row.accesskit_node().label())
        .filter(|label| label.contains("Bunny") || label.starts_with("bunny"))
        .map(|label| label.to_string())
        .collect();
    assert_eq!(
        rows,
        [
            "Big.Buck.Bunny.1080p, Exact match · Open Subtitles · ASS · 1,200 downloads",
            "Big.Buck.Bunny.720p, Open Subtitles · SRT · 40 downloads",
            "bunny, Rated 7.5",
        ]
    );
    // The language starts as the interface's, named by the server's list.
    let language = ui.harness().get_by_role(Role::ComboBox);
    assert_eq!(language.accesskit_node().value().as_deref(), Some("English"));
}

/// The language picker takes the keyboard when it has focus: Enter opens its list rather than
/// downloading the highlighted result, and Down moves in the list, not the results.
#[test]
fn the_language_picker_is_reachable_and_operable_by_keyboard() {
    let _config = ConfigDir::new("subtitle-search-language-keys");
    let server = server();
    server.json_value("GET", SEARCH, &results());
    let (mut ui, _) = search_dialog(&server);
    ui.step_until("the results", |ui| {
        ui.query_by_label_contains("Big.Buck.Bunny.1080p").is_some()
    });
    // Tab from the dialog's start reaches the picker first.
    ui.harness_mut().key_press(egui::Key::Tab);
    ui.step();
    ui.step();
    ui.harness_mut().key_press(egui::Key::Enter);
    ui.step();
    ui.step();
    assert_eq!(
        server.requests("POST", "/Items/m1/RemoteSearch/Subtitles/b%2F1"),
        0,
        "Enter on the picker didn't download a result"
    );
    assert!(
        ui.harness().query_all_by_role(Role::MenuItemRadio).count() > 1,
        "Enter opened the language list"
    );
}

/// A result without details is named by its name alone, not with a trailing comma.
#[test]
fn a_result_without_details_has_a_plain_name() {
    let _config = ConfigDir::new("subtitle-search-bare-row");
    let server = server();
    server.json_value(
        "GET",
        SEARCH,
        &serde_json::json!([{ "Id": "x", "Name": "Bare.Result" }]),
    );
    let (mut ui, _) = search_dialog(&server);
    ui.step_until("the result", |ui| ui.query_by_label("Bare.Result").is_some());
}

/// A search that finds nothing says so in the language searched, and hints at the server having
/// no provider, which it can't tell from an empty answer.
#[test]
fn an_empty_search_says_nothing_was_found() {
    let _config = ConfigDir::new("subtitle-search-empty");
    let server = server();
    server.json_value("GET", SEARCH, &serde_json::json!([]));
    let (mut ui, _) = search_dialog(&server);
    ui.step_until("the answer", |ui| {
        ui.query_by_label_contains("No subtitles found in English").is_some()
    });
    assert!(ui.harness().query_by_label_contains("subtitle provider").is_some());
}

/// While the answer is on its way the dialog says it's searching; a refusal and a failing
/// server each get their own line.
#[test]
fn the_dialog_shows_that_it_is_searching_and_what_went_wrong() {
    let _config = ConfigDir::new("subtitle-search-errors");
    let server = server();
    server.json_delayed(
        "GET",
        SEARCH,
        &serde_json::json!([]),
        std::time::Duration::from_millis(1500),
    );
    let (mut ui, _) = search_dialog(&server);
    ui.step();
    assert!(ui.harness().query_by_label("Searching…").is_some());
    ui.step_until("the answer", |ui| ui.query_by_label("Searching…").is_none());

    let server = self::server();
    server.status("GET", SEARCH, 403);
    let (mut ui, _) = search_dialog(&server);
    ui.step_until("the refusal", |ui| {
        ui.query_by_label("Your account isn't allowed to search for subtitles.")
            .is_some()
    });

    let broken = self::server();
    broken.status("GET", SEARCH, 500);
    let (mut ui, _) = search_dialog(&broken);
    ui.step_until("the failure", |ui| ui.query_by_label("Searching…").is_none());
    assert!(ui.harness().query_by_label_contains("No subtitles found").is_none());
    assert!(ui.harness().query_by_label_contains("Big.Buck").is_none());
}

/// A click on a result has the server download it, then reads the item again until the new
/// stream is in it, and hands the item's versions back as it closes.
#[test]
fn picking_a_result_downloads_it_and_hands_back_the_new_streams() {
    let _config = ConfigDir::new("subtitle-search-download");
    let server = server();
    server.json_value("GET", SEARCH, &results());
    server.status("POST", "/Items/m1/RemoteSearch/Subtitles/b%2F1", 204);
    let item = |externals: usize| {
        serde_json::json!({
            "Id": "m1", "Name": "Big Buck Bunny", "Type": "Movie",
            "MediaSources": [{
                "Id": "s1",
                "MediaStreams": (0..externals)
                    .map(|index| serde_json::json!({
                        "Index": index, "Type": "Subtitle", "IsExternal": true, "Path": format!("m.{index}.srt"),
                    }))
                    .collect::<Vec<_>>(),
            }],
        })
    };
    // Read before the download, and once more before the stream shows up.
    server.json_once("GET", "/Items/m1", &item(0));
    server.json_once("GET", "/Items/m1", &item(0));
    server.json_value("GET", "/Items/m1", &item(1));
    let (mut ui, finished) = search_dialog(&server);
    ui.step_until("the results", |ui| {
        ui.query_by_label_contains("Big.Buck.Bunny.1080p").is_some()
    });

    let row = ui
        .harness()
        .get_by_label_contains("Big.Buck.Bunny.1080p")
        .rect()
        .center();
    ui.click_at(row);
    ui.step_until("the download to finish", |_| !finished.borrow().is_empty());

    assert_eq!(server.requests("POST", "/Items/m1/RemoteSearch/Subtitles/b%2F1"), 1);
    let finished = finished.borrow();
    let Some(Finished::Added(sources)) = finished.first() else {
        panic!("not added: {finished:?}");
    };
    assert_eq!(sources[0].media_streams.len(), 1, "the item as the server has it now");
}

fn player_with(server: &FixtureServer, allowed: bool) -> Harness<'static, PlayerRun> {
    let run = PlayerRun::new(server);
    let run = if allowed { run.with_subtitle_management() } else { run };
    let mut harness = build_harness(egui::vec2(1280., 720.), run, |ui, run| run.show(ui));
    for _ in 0..3 {
        harness.step();
    }
    harness
}

/// The Subtitles tab ends with *Search for subtitles…* for users who may manage subtitles, and
/// has nothing of the kind for the others.
#[test]
fn the_players_subtitle_tab_offers_the_search_only_with_the_permission() {
    let _config = ConfigDir::new("subtitle-search-player-entry");
    for (allowed, name) in [(false, "without"), (true, "with")] {
        let server = FixtureServer::start();
        let mut harness = player_with(&server, allowed);
        harness.get_by_label("Subtitles").click();
        for _ in 0..3 {
            harness.step();
        }
        assert!(
            harness.query_by_label("Off").is_some(),
            "the Subtitles tab didn't open ({name} the permission)"
        );
        assert_eq!(
            harness.query_by_label("Search for subtitles…").is_some(),
            allowed,
            "the entry {name} the permission"
        );
    }
}

/// Opening the entry shows the dialog over the player.
#[test]
fn the_players_entry_opens_the_dialog() {
    let _config = ConfigDir::new("subtitle-search-player-open");
    let server = server();
    let mut harness = player_with(&server, true);
    harness.get_by_label("Subtitles").click();
    for _ in 0..3 {
        harness.step();
    }
    harness.get_by_label("Search for subtitles…").click();
    for _ in 0..3 {
        harness.step();
    }
    assert!(
        harness.query_by_label("Search for subtitles").is_some(),
        "the dialog's title"
    );
}

/// A downloaded subtitle for what plays is loaded into the player and chosen, from the new
/// stream's URL; the ones the file had keep their place in mpv's numbering.
#[test]
fn a_downloaded_subtitle_is_added_to_the_player_and_selected() {
    let _config = ConfigDir::new("subtitle-search-add");
    let server = FixtureServer::start();
    let mut harness = player_with(&server, true);
    harness.state_mut().commands();
    let (item_id, mut source) = {
        let state = harness.state().now_playing().expect("playing");
        (state.item.id.clone(), state.source.clone())
    };
    let stream = |index: i32| -> jellyfin::models::MediaStream {
        serde_json::from_value(serde_json::json!({
            "Index": index, "Type": "Subtitle", "IsExternal": true, "Codec": "subrip",
            "DisplayTitle": "Portuguese - SUBRIP", "Language": "por", "Path": "movie.por.srt",
        }))
        .unwrap()
    };
    let next = source.media_streams.iter().map(|s| s.index).max().unwrap_or(0) + 1;
    source.media_streams.push(stream(next));
    let sources: Vec<MediaSourceInfo> = vec![source];

    assert!(harness.state_mut().add_downloaded_subtitle(&item_id, &sources));
    let state = harness.state().now_playing().expect("playing");
    assert_eq!(state.subtitle, SubtitleChoice::Stream(next));
    assert_eq!(state.stream.subtitle_files, [next]);
    let sent = harness.state_mut().commands();
    let [UiCommand::AddSubtitle(added)] = sent.as_slice() else {
        panic!("sent {sent:?}");
    };
    assert_eq!(added.index, next);
    assert!(
        added.url.path().ends_with(&format!("/Subtitles/{next}/Stream.srt")),
        "{}",
        added.url
    );
    assert_eq!(added.language.as_deref(), Some("por"));
    // Another item's download isn't taken.
    assert!(!harness.state_mut().add_downloaded_subtitle("someone else", &sources));
}

fn movie_page(server: &FixtureServer, allowed: bool) -> PageHarness {
    let movie: serde_json::Value = fixture_json("item_movie");
    server.json_value("GET", "/Items/d265861eae31cee1d44b13adac305eac", &movie);
    let name = movie["Name"].as_str().unwrap_or_default().to_owned();
    let mut session = session(server);
    session.user.policy = allowed.then_some(UserPolicy {
        enable_subtitle_management: true,
    });
    let mut ui = PageHarness::new(session, move |cx| {
        crate::views::open(
            Route::Item {
                id: MOVIE.to_owned(),
                name,
            },
            cx,
        )
    });
    ui.step_until("the movie's actions", |ui| ui.query_by_label("More").is_some());
    ui
}

/// The item page's More menu lists *Search for subtitles…* for a movie, for users who may
/// manage subtitles only.
#[test]
fn the_item_menu_offers_the_search_only_with_the_permission() {
    let _config = ConfigDir::new("subtitle-search-item-entry");
    for allowed in [false, true] {
        let server = server();
        let mut ui = movie_page(&server, allowed);
        let more = ui.harness().get_by_role_and_label(Role::Button, "More").rect().center();
        ui.click_at(more);
        ui.step_until("the menu", |ui| ui.query_by_label("Add to playlist…").is_some());
        assert_eq!(
            ui.harness().query_by_label("Search for subtitles…").is_some(),
            allowed,
            "allowed: {allowed}"
        );
    }
}
