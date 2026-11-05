//! The phone's layout (`layout::Size::Compact`), driven headless with touch turned on for the
//! harness's context: the shell's bars, Settings' sections, the library grid filling its rows, the item page's
//! stacked header, and a long press opening a card's menu.

use egui::accesskit::{Role, Toggled};
use egui::{Event, Pos2, TouchDeviceId, TouchId, TouchPhase};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use jellyfin::models::CollectionType;

use crate::config::Config;
use crate::nav::Route;
use crate::services::Services;
use crate::session::Session;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, build_harness, session, step_until};
use crate::views::PageCx;
use crate::views::shell::Shell;

/// The test phone in points: 1200 × 2664 pixels at 3.25 per point, less its system bars.
const PHONE: egui::Vec2 = egui::vec2(369., 760.);

/// A config with the update check off, so the shell asks GitHub for nothing.
fn config_dir(name: &str) -> ConfigDir {
    let dir = ConfigDir::new(name);
    Config {
        check_for_updates: Some(false),
        ..Config::default()
    }
    .save()
    .expect("the test's config saves");
    dir
}

/// The signed-in shell, as the app runs it, in a phone-sized harness with touch.
struct ShellRun {
    session: Option<Session>,
    services: Option<Services>,
    shell: Option<Shell>,
}

impl ShellRun {
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut nav = Vec::new();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            self.services = Some(Services::new(session, &ctx));
        }
        let services = self.services.as_mut().expect("the services are made");
        services.tick(&ctx);
        let mut cx = PageCx {
            ctx: &ctx,
            services,
            nav: &mut nav,
            render_state: None,
            under_bar: None,
        };
        let shell = self.shell.get_or_insert_with(|| Shell::new(&mut cx));
        shell.show(ui, Vec::new(), &mut cx);
    }
}

fn phone_shell(server: &FixtureServer) -> Harness<'static, ShellRun> {
    let run = ShellRun {
        session: Some(session(server)),
        services: None,
        shell: None,
    };
    let mut harness = build_harness(PHONE, run, |ui, run| run.show(ui));
    crate::layout::set_touch(&harness.ctx, true);
    harness.step();
    harness
}

fn phone_page(server: &FixtureServer, route: Route) -> PageHarness {
    let mut ui = PageHarness::new(session(server), move |cx| crate::views::open(route, cx));
    crate::layout::set_touch(&ui.harness().ctx, true);
    ui.harness_mut().set_size(PHONE);
    ui.step();
    ui
}

fn home_server() -> FixtureServer {
    let server = FixtureServer::start();
    server.json("GET", "/UserViews", "user_views");
    server.json("GET", "/UserItems/Resume", "resume");
    server.json("GET", "/Shows/NextUp", "next_up");
    server.json("GET", "/Items/Latest", "latest");
    server.json("GET", "/Items", "items_movies");
    server.json("GET", "/Items/Filters", "item_filters");
    server
}

fn movies_route() -> Route {
    Route::Library {
        id: "ui-test-movies".into(),
        name: "Movies".into(),
        collection_type: Some(CollectionType::Movies),
        newest_first: false,
    }
}

/// The bottom bar's place named `name`, which is selected while it's open.
fn place_selected(harness: &Harness<'static, ShellRun>, name: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Tab, name)
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

fn tap(harness: &mut Harness<'static, ShellRun>, name: &str) {
    harness.get_by_role_and_label(Role::Tab, name).click();
    for _ in 0..3 {
        harness.step();
    }
}

/// The bottom bar leads to its two places, each right above Home, and shows which is open; the
/// top bar has Search, SyncPlay and the account, but no Settings, and Home leaves *My Media* to
/// the bottom bar.
#[test]
fn the_bottom_bar_opens_its_places_above_home() {
    let _config = config_dir("touch-bottom-bar");
    let server = home_server();
    let mut harness = phone_shell(&server);
    step_until(&mut harness, "Home", |harness| {
        harness.query_by_label("Continue Watching").is_some()
    });
    assert!(
        harness.query_by_label("My Media").is_none(),
        "no libraries shelf on a phone"
    );
    assert!(place_selected(&harness, "Home"));
    assert!(harness.query_by_role_and_label(Role::Tab, "Search").is_none());
    assert!(harness.query_by_role_and_label(Role::Tab, "Settings").is_none());
    assert!(
        harness.query_by_role_and_label(Role::Button, "Settings").is_none(),
        "no Settings button in a phone's top bar"
    );
    assert!(harness.query_by_role_and_label(Role::Button, "Search").is_some());

    tap(&mut harness, "Libraries");
    step_until(&mut harness, "the Libraries page", |harness| {
        harness.query_by_role_and_label(Role::Button, "Movies").is_some()
    });
    assert!(place_selected(&harness, "Libraries"));
    assert!(!place_selected(&harness, "Home"));

    // A library opened from there is still under Libraries, and Libraries again goes back to
    // the list.
    harness.get_by_role_and_label(Role::Button, "Movies").click();
    step_until(&mut harness, "the library", |harness| {
        harness.query_by_label("Back").is_some()
            && harness.query_by_label("Movies").is_some()
            && harness.query_by_role_and_label(Role::Button, "Movies").is_none()
    });
    assert!(place_selected(&harness, "Libraries"));
    tap(&mut harness, "Libraries");
    step_until(&mut harness, "the list again", |harness| {
        harness.query_by_role_and_label(Role::Button, "Movies").is_some()
    });

    tap(&mut harness, "Home");
    assert!(place_selected(&harness, "Home"));
    assert!(
        harness.query_by_label("Back").is_none(),
        "Home is at the bottom of the stack"
    );
}

/// The top bar's Search button opens the results with the keyboard in the box, and emptying the
/// box keeps the page open.
#[test]
fn search_opens_from_the_top_bar() {
    let _config = config_dir("touch-search");
    let server = home_server();
    let mut harness = phone_shell(&server);
    harness.get_by_role_and_label(Role::Button, "Search").click();
    for _ in 0..3 {
        harness.step();
    }
    let focused = harness.ctx.memory(|memory| memory.focused());
    assert_eq!(
        focused,
        Some(egui::Id::unique("top_bar_search")),
        "the box has the keyboard"
    );
    assert!(harness.query_by_label("Type in the search box to search.").is_some());
    assert!(harness.query_by_label("Back").is_some(), "Back closes the results");
}

/// Settings is in the account menu, a list of its sections, each opening on a page of its own.
#[test]
fn settings_opens_from_the_account_menu_as_a_list_of_sections() {
    let _config = config_dir("touch-settings");
    let server = home_server();
    let account = session(&server);
    let label = format!(
        "{} on {}",
        account.user.name,
        account
            .server
            .server_name
            .as_deref()
            .expect("the fixture names its server")
    );
    let mut harness = phone_shell(&server);
    harness.get_by_role_and_label(Role::Button, &label).click();
    step_until(&mut harness, "the account menu", |harness| {
        harness.query_by_label("Sign out").is_some()
    });
    harness.get_by_label("Settings").click();
    step_until(&mut harness, "the sections", |harness| {
        harness.query_by_role_and_label(Role::Button, "Playback").is_some()
    });
    assert!(
        harness
            .query_all_by_label("Play the next episode automatically")
            .next()
            .is_none(),
        "the list has the sections, not their settings"
    );

    harness.get_by_role_and_label(Role::Button, "Playback").click();
    step_until(&mut harness, "the Playback page", |harness| {
        harness
            .query_all_by_label("Play the next episode automatically")
            .next()
            .is_some()
    });
    assert!(
        harness.query_all_by_label("Theme").next().is_none(),
        "only Playback's settings"
    );

    harness.get_by_label("Back").click();
    step_until(&mut harness, "the sections again", |harness| {
        harness.query_by_role_and_label(Role::Button, "Appearance").is_some()
    });
}

/// The grid makes its cards as wide as three fill the phone's row beside the letter bar, rather
/// than one 150-point card a row.
#[test]
fn a_phones_library_grid_fills_its_rows() {
    let _config = ConfigDir::new("touch-grid");
    let server = home_server();
    let mut ui = phone_page(&server, movies_route());
    ui.step_until("the grid", |harness| harness.query_by_label("2 items").is_some());
    let first = ui
        .harness()
        .get_by_role_and_label(Role::Button, "Second Movie (2025)")
        .rect();
    let second = ui
        .harness()
        .get_by_role_and_label(Role::Button, "Test Movie (2024)")
        .rect();
    assert!((first.top() - second.top()).abs() < 1., "both on the first row");
    let width = first.width();
    assert!((80.0..100.).contains(&width), "a third of the row each, not {width}");
}

/// A phone's toolbar has the sort and a Filters button, which opens the filters in a dialog: a
/// choice there applies at once and leaves the dialog open, the button lights up while
/// anything is filtered, and *Done* closes it.
#[test]
fn a_phones_filters_open_in_a_dialog() {
    let _config = ConfigDir::new("touch-filters");
    let server = home_server();
    let mut ui = phone_page(&server, movies_route());
    ui.step_until("the grid", |harness| harness.query_by_label("2 items").is_some());
    assert!(
        ui.harness()
            .query_by_role_and_label(Role::ComboBox, "Progress")
            .is_none(),
        "no filters in the toolbar"
    );
    ui.harness().get_by_label("Sort");
    let filters_on = |ui: &PageHarness| {
        ui.harness()
            .get_by_role_and_label(Role::Button, "Filters")
            .accesskit_node()
            .toggled()
    };
    assert_eq!(filters_on(&ui), Some(Toggled::False));

    ui.harness().get_by_role_and_label(Role::Button, "Filters").click();
    ui.step_until("the dialog", |harness| {
        harness.query_by_role_and_label(Role::ComboBox, "Progress").is_some()
    });
    ui.harness().get_by_role_and_label(Role::ComboBox, "Progress").click();
    ui.step_until("the progress menu", |harness| {
        harness.query_by_label("Unwatched").is_some()
    });
    ui.harness().get_by_label("Unwatched").click();
    ui.step_until("the choice", |harness| {
        harness
            .get_by_role_and_label(Role::ComboBox, "Progress")
            .accesskit_node()
            .value()
            .as_deref()
            == Some("Unwatched")
    });
    assert_eq!(filters_on(&ui), Some(Toggled::True), "lit while filtered");

    ui.harness().get_by_label("Done").click();
    ui.step_until("the dialog to close", |harness| {
        harness.query_by_role_and_label(Role::ComboBox, "Progress").is_none()
    });
}

/// The phone's genres and tags are a page of the filters dialog: the row leads there without
/// the keyboard coming up, a tap on a pill includes it and the row says so, Back returns to the
/// filters and *Clear* lets the genres and tags go.
#[test]
fn a_phones_genres_and_tags_are_a_page_of_the_filters() {
    let _config = ConfigDir::new("touch-genres-page");
    let server = home_server();
    let mut ui = phone_page(&server, movies_route());
    ui.step_until("the grid", |harness| harness.query_by_label("2 items").is_some());
    ui.harness().get_by_role_and_label(Role::Button, "Filters").click();
    ui.step_until("the genres and tags row", |harness| {
        harness
            .query_by_role_and_label(Role::Button, "Genres and tags: Genres and tags")
            .is_some()
    });
    ui.harness()
        .get_by_role_and_label(Role::Button, "Genres and tags: Genres and tags")
        .click();
    ui.step_until("the page", |harness| {
        harness.query_by_role_and_label(Role::Button, "Comedy").is_some()
    });
    // A new modal's first frame only sizes it: tap where it settles.
    for _ in 0..3 {
        ui.step();
    }
    assert!(
        ui.harness()
            .query_by_role_and_label(Role::ComboBox, "Progress")
            .is_none(),
        "the page in place of the filters"
    );
    assert_eq!(
        ui.harness().ctx.memory(|memory| memory.focused()),
        None,
        "the search box waits for a tap"
    );

    ui.harness().get_by_role_and_label(Role::Button, "Comedy").click();
    ui.step_until("Comedy included", |harness| {
        harness.query_by_label("1 chosen \u{b7} 2 items").is_some()
            || harness.query_all_by_label("Comedy, included").next().is_some()
    });

    ui.harness().get_by_label("Back").click();
    ui.step_until("the filters again", |harness| {
        harness
            .query_by_role_and_label(Role::Button, "Genres and tags: Comedy")
            .is_some()
    });

    ui.harness()
        .get_by_role_and_label(Role::Button, "Genres and tags: Comedy")
        .click();
    ui.step_until("the page again", |harness| harness.query_by_label("Clear").is_some());
    ui.harness().get_by_label("Clear").click();
    ui.step_until("nothing chosen", |harness| {
        harness.query_all_by_label("Comedy, included").next().is_none()
    });
}

/// A long press on a card opens its menu, as a right click does with a mouse, and doesn't open
/// the card.
#[test]
fn a_long_press_on_a_card_opens_its_menu() {
    let _config = ConfigDir::new("touch-long-press");
    let server = home_server();
    let mut ui = phone_page(&server, movies_route());
    ui.step_until("the grid", |harness| harness.query_by_label("2 items").is_some());
    let at = ui
        .harness()
        .get_by_role_and_label(Role::Button, "Test Movie (2024)")
        .rect()
        .center();
    press(ui.harness_mut(), at, true);
    ui.step_until("the card's menu", |harness| harness.query_by_label("Watched").is_some());
    press(ui.harness_mut(), at, false);
    ui.step();
    assert!(ui.state().nav.is_empty(), "the press didn't open the card");
    assert!(ui.harness().query_by_label("Watched").is_some(), "the menu stays open");
}

/// A finger down at `pos` (or up again), as Android reports it: a touch, and the pointer egui
/// makes of it.
fn press(harness: &mut Harness<'static, crate::ui_tests::PageRun>, pos: Pos2, down: bool) {
    let input = harness.input_mut();
    input.events.push(Event::Touch {
        device_id: TouchDeviceId(0),
        id: TouchId(0),
        phase: if down { TouchPhase::Start } else { TouchPhase::End },
        pos,
        force: None,
    });
    if down {
        input.events.push(Event::PointerMoved(pos));
    }
    input.events.push(Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: down,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

/// The movie the fixtures give.
const MOVIE: &str = "d265861eae31cee1d44b13adac305eac";

fn movie_page(server: &FixtureServer) -> PageHarness {
    phone_page(
        server,
        Route::Item {
            id: MOVIE.into(),
            name: "Test Movie (2024)".into(),
        },
    )
}

/// An item page on a phone is laid out like a card: the title centred under the poster, Play
/// as wide as the page, and the other actions as round buttons named under them.
#[test]
fn a_phones_item_header_is_centred() {
    let _config = ConfigDir::new("touch-item");
    let server = FixtureServer::start();
    server.json("GET", "/Items/d265861eae31cee1d44b13adac305eac", "item_movie");
    let mut ui = movie_page(&server);
    ui.step_until("the item", |harness| harness.query_by_label("More").is_some());
    let play = ui.harness().get_by_label_contains("Play").rect();
    // The page's 16 either side, inside the harness's own 8.
    let across = PHONE.x - 2. * 8. - 2. * 16.;
    assert!(
        (play.width() - across).abs() < 1.,
        "Play across the page, not {}",
        play.width()
    );
    let middle = PHONE.x / 2.;
    let title = ui.harness().get_by_label("Test Movie (2024)").rect();
    assert!((title.center().x - middle).abs() < 2., "the title centred");
    for name in ["Watched", "Favorite", "More"] {
        let action = ui.harness().get_by_role_and_label(Role::Button, name).rect();
        assert!(action.top() > play.bottom(), "{name} under Play");
        assert!(action.width() >= crate::layout::TARGET, "{name} a finger wide");
    }
}

/// A long synopsis shows its first lines with *More*, which opens the rest and becomes *Less*.
#[test]
fn a_phones_synopsis_opens_with_more() {
    let _config = ConfigDir::new("touch-overview");
    let server = FixtureServer::start();
    let mut movie: serde_json::Value = crate::ui_tests::fixture_json("item_movie");
    movie["Overview"] = "A long synopsis that goes on and on. ".repeat(12).into();
    server.json_value("GET", "/Items/d265861eae31cee1d44b13adac305eac", &movie);
    let mut ui = movie_page(&server);
    ui.step_until("the synopsis", |harness| {
        harness.query_by_label_contains("A long synopsis").is_some()
    });
    let closed = ui.harness().get_by_label_contains("A long synopsis").rect().height();
    ui.harness().get_by_label("Show more").click();
    ui.step_until("the whole synopsis", |harness| {
        harness.query_by_label("Show less").is_some()
    });
    let open = ui.harness().get_by_label_contains("A long synopsis").rect().height();
    assert!(open > closed * 1.5, "the synopsis grew from {closed} to {open}");
}

/// A phone's series has its seasons on a shelf, the next episode's season open from the start
/// with its episodes under the shelf; a tap on another season opens that one, and a tap on the
/// open one leaves it open.
#[test]
fn a_phones_series_picks_its_season_from_a_shelf() {
    let _config = ConfigDir::new("touch-seasons");
    let server = FixtureServer::start();
    server.json("GET", "/Items/129db440ee6554a42e5d8f1350516bab", "item_series");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Seasons", "seasons");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Episodes", "episodes");
    let mut ui = phone_page(
        &server,
        Route::Item {
            id: "129db440ee6554a42e5d8f1350516bab".into(),
            name: "Test Show".into(),
        },
    );
    let open = |harness: &Harness<'static, crate::ui_tests::PageRun>, name: &str| {
        harness
            .query_by_role_and_label(Role::Button, name)
            .is_some_and(|season| season.accesskit_node().toggled() == Some(Toggled::True))
    };
    ui.step_until("Season 1 open with its episodes", |harness| {
        open(harness, "Season 1") && harness.query_all_by_label_contains("S01E01").next().is_some()
    });
    assert!(!open(ui.harness(), "Season 2"));

    ui.harness().get_by_role_and_label(Role::Button, "Season 2").click();
    ui.step_until("Season 2 open", |harness| open(harness, "Season 2"));
    assert!(!open(ui.harness(), "Season 1"));

    ui.harness().get_by_role_and_label(Role::Button, "Season 2").click();
    for _ in 0..5 {
        ui.step();
    }
    assert!(open(ui.harness(), "Season 2"), "the open season stays open");
}

/// A phone in landscape, less the cutout at its left edge.
const LANDSCAPE: egui::Vec2 = egui::vec2(760., 369.);
const CUTOUT: f32 = 32.;

/// The player page over a session with no player behind it ([`crate::playback::PlaybackManager::
/// start_for_test`]): an episode with a previous and a next one, chapters, and an intro playing.
pub struct PlayerRun {
    session: Option<Session>,
    services: Option<Services>,
    player: Option<crate::views::player::PlayerPage>,
    /// What the page sent the session.
    pub sent: Option<tokio::sync::mpsc::UnboundedReceiver<crate::playback::UiCommand>>,
}

impl PlayerRun {
    pub fn new(server: &FixtureServer) -> Self {
        Self {
            session: Some(session(server)),
            services: None,
            player: None,
            sent: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        use std::time::Duration;

        use crate::markers::{Chapter, Segment, SegmentKind};

        let ctx = ui.ctx().clone();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            let mut services = Services::new(session, &ctx);
            let episodes: serde_json::Value = crate::ui_tests::fixture_json("episodes");
            let item = |index: usize| -> jellyfin::models::BaseItemDto {
                serde_json::from_value(episodes["Items"][index].clone()).expect("an episode")
            };
            let movie: jellyfin::models::BaseItemDto = crate::ui_tests::fixture_json("item_movie");
            let source = movie.media_sources[0].clone();
            self.sent = Some(services.playback.start_for_test(item(1), source));
            let state = services.playback.now_playing_mut().expect("the session");
            state.previous = Some(item(0));
            state.next = Some(item(2));
            state.chapters = ["Cold open", "The wreck", "Night boat"]
                .iter()
                .enumerate()
                .map(|(index, name)| Chapter {
                    start: Duration::from_secs(index as u64 * 15 * 60),
                    name: Some((*name).to_owned()),
                    image_tag: None,
                    index,
                })
                .collect();
            state.segments = vec![Segment {
                kind: SegmentKind::Intro,
                start: Duration::from_secs(11 * 60),
                end: Duration::from_secs(13 * 60),
            }];
            self.services = Some(services);
        }
        let services = self.services.as_mut().expect("the services are made");
        let mut nav = Vec::new();
        let mut cx = PageCx {
            ctx: &ctx,
            services,
            nav: &mut nav,
            render_state: None,
            under_bar: None,
        };
        self.player
            .get_or_insert_with(crate::views::player::PlayerPage::new)
            .show(ui, &mut cx);
    }

    /// The commands the page has sent since this was last asked.
    fn commands(&mut self) -> Vec<crate::playback::UiCommand> {
        let sent = self.sent.as_mut().expect("the session started");
        std::iter::from_fn(|| sent.try_recv().ok()).collect()
    }
}

/// Opens `run`'s settings sheet once its player is up, for a mockup.
#[cfg(feature = "mockup")]
pub fn open_sheet(run: &mut PlayerRun) {
    if let Some(player) = &mut run.player {
        player.open_settings_sheet();
    }
}

/// The player on a phone in landscape with a cutout at its left edge, controls up.
fn phone_player(server: &FixtureServer) -> Harness<'static, PlayerRun> {
    let mut harness = build_harness(LANDSCAPE, PlayerRun::new(server), |ui, run| run.show(ui));
    crate::layout::set_touch(&harness.ctx, true);
    harness.input_mut().safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
        left: CUTOUT,
        ..Default::default()
    }));
    for _ in 0..3 {
        harness.step();
    }
    harness
}

/// The controls keep clear of the cutout, as far in from both sides so they stay centred on the
/// picture, with play/pause in the middle between the skips and the episodes.
#[test]
fn a_phones_player_keeps_its_controls_in_the_safe_area() {
    let _config = ConfigDir::new("touch-player-safe");
    let server = FixtureServer::start();
    let harness = phone_player(&server);
    let window = harness.ctx.viewport_rect();
    let rect = |name: &str| harness.get_by_label(name).rect();
    let left = rect("Back").left() - window.left();
    let right = window.right() - rect("Settings").right();
    assert!(left >= CUTOUT, "Back clear of the cutout ({left})");
    assert!(
        (left - right).abs() < 1.,
        "as far in from both sides: {left} and {right}"
    );
    let seek = rect("Seek");
    assert!(seek.left() >= window.left() + CUTOUT && seek.right() <= window.right() - CUTOUT);
    let middle = window.center().x;
    let play = rect("Pause");
    assert!((play.center().x - middle).abs() < 1., "play/pause in the middle");
    let order = ["Previous episode", "Back 10 s", "Pause", "Forward 10 s", "Next episode"];
    let xs: Vec<f32> = order.iter().map(|name| rect(name).center().x).collect();
    assert!(xs.windows(2).all(|pair| pair[0] < pair[1]), "left to right: {order:?}");
    for name in ["Chapters", "Subtitles", "Settings"] {
        assert!(rect(name).top() < play.top(), "{name} along the top");
    }
}

/// The middle's buttons pause and skip, and the skip pill shows over an intro.
#[test]
fn a_phones_player_buttons_pause_and_skip() {
    use crate::playback::UiCommand;

    let _config = ConfigDir::new("touch-player-buttons");
    let server = FixtureServer::start();
    let mut harness = phone_player(&server);
    harness.state_mut().commands();
    harness.get_by_label("Back 10 s").click();
    harness.step();
    harness.get_by_label("Pause").click();
    harness.step();
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekBy(by), UiCommand::TogglePause] if *by == -10.),
        "{sent:?}"
    );
    harness.get_by_label("Skip intro");
}

/// Near the end the up-next card comes up over the clock, below the middle's buttons rather than
/// over them.
#[test]
fn a_phones_up_next_card_keeps_clear_of_the_middle() {
    let _config = ConfigDir::new("touch-player-up-next");
    let server = FixtureServer::start();
    let mut harness = phone_player(&server);
    {
        let services = harness.state_mut().services.as_mut().expect("the services");
        let state = services.playback.now_playing_mut().expect("the session");
        state.position = std::time::Duration::from_secs(47 * 60 + 50);
        state.position_at = std::time::Instant::now();
    }
    for _ in 0..3 {
        harness.step();
    }
    // The buttons, and the card's padding round them.
    let card = harness
        .get_by_label("Play now")
        .rect()
        .union(harness.get_by_label("Cancel").rect())
        .expand(8.);
    for name in ["Back 10 s", "Pause", "Forward 10 s", "Next episode"] {
        let button = harness.get_by_label(name).rect();
        assert!(
            !card.intersects(button),
            "the card clear of {name}: {card:?} and {button:?}"
        );
    }
    assert!(
        card.bottom() < harness.get_by_label("Seek").rect().top(),
        "the card over the seek bar"
    );
}

/// The subtitles button turns the default track off, and the same one back on.
#[test]
fn a_phones_subtitles_button_turns_them_off_and_on() {
    use crate::playback::{SubtitleChoice, UiCommand};

    let _config = ConfigDir::new("touch-player-subtitles");
    let server = FixtureServer::start();
    let mut harness = phone_player(&server);
    harness.state_mut().commands();
    let toggled = |harness: &Harness<'static, PlayerRun>| harness.get_by_label("Subtitles").accesskit_node().toggled();
    assert_eq!(toggled(&harness), Some(Toggled::True), "the default track shows");
    harness.get_by_label("Subtitles").click();
    harness.step();
    harness.step();
    assert_eq!(toggled(&harness), Some(Toggled::False));
    harness.get_by_label("Subtitles").click();
    harness.step();
    let sent = harness.state_mut().commands();
    assert!(
        matches!(
            sent.as_slice(),
            [
                UiCommand::Subtitle(SubtitleChoice::Off),
                UiCommand::Subtitle(SubtitleChoice::Stream(3))
            ]
        ),
        "{sent:?}"
    );
}

/// Settings opens a sheet at the right, the full height, with the menu's tabs; a track picked
/// there goes to the session, and Back (Escape) closes the sheet without leaving the player.
#[test]
fn a_phones_player_settings_open_as_a_side_sheet() {
    use crate::playback::UiCommand;

    let _config = ConfigDir::new("touch-player-sheet");
    let server = FixtureServer::start();
    let mut harness = phone_player(&server);
    harness.get_by_label("Settings").click();
    for _ in 0..3 {
        harness.step();
    }
    let content = harness.ctx.content_rect();
    let audio = harness.get_by_role_and_label(Role::Tab, "Audio");
    assert!(audio.rect().left() > content.center().x, "the sheet is at the right");
    audio.click();
    for _ in 0..2 {
        harness.step();
    }
    harness.state_mut().commands();
    harness.get_by_label_contains("Japanese").click();
    for _ in 0..2 {
        harness.step();
    }
    let sent = harness.state_mut().commands();
    assert!(matches!(sent.as_slice(), [UiCommand::Audio(2)]), "{sent:?}");

    harness.key_press(egui::Key::Escape);
    for _ in 0..2 {
        harness.step();
    }
    assert!(
        harness.query_by_role_and_label(Role::Tab, "Audio").is_none(),
        "the sheet closed"
    );
    let sent = harness.state_mut().commands();
    assert!(
        !sent.iter().any(|command| matches!(command, UiCommand::Stop)),
        "{sent:?}"
    );
}

/// Chapters opens its sheet, and a chapter picked there seeks to it and closes the sheet.
#[test]
fn a_phones_chapters_sheet_seeks() {
    use crate::playback::UiCommand;

    let _config = ConfigDir::new("touch-player-chapters");
    let server = FixtureServer::start();
    let mut harness = phone_player(&server);
    harness.get_by_label("Chapters").click();
    for _ in 0..3 {
        harness.step();
    }
    harness.state_mut().commands();
    harness.get_by_label_contains("Night boat").click();
    for _ in 0..2 {
        harness.step();
    }
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 30 * 60),
        "{sent:?}"
    );
    assert!(
        harness.query_by_label_contains("Night boat").is_none(),
        "the sheet closed"
    );
}
