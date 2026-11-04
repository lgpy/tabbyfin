//! The phone's layout (`layout::Size::Compact`), driven headless with touch turned on for the
//! harness's context: the shell's bottom bar, the library grid filling its rows, the item page's
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

/// The bottom bar leads to its four places, each right above Home, and shows which is open;
/// Settings' own buttons aren't in the top bar, which has only Back, the title and the account.
#[test]
fn the_bottom_bar_opens_its_places_above_home() {
    let _config = config_dir("touch-bottom-bar");
    let server = home_server();
    let mut harness = phone_shell(&server);
    step_until(&mut harness, "Home", |harness| {
        harness.query_by_label("My Media").is_some()
    });
    assert!(place_selected(&harness, "Home"));
    assert!(
        harness.query_by_role_and_label(Role::Button, "Settings").is_none(),
        "no Settings button in a phone's top bar"
    );

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

    // Settings takes Libraries' place, with the search box in the bar instead of its title.
    tap(&mut harness, "Settings");
    assert!(place_selected(&harness, "Settings"));
    assert!(harness.query_by_label("Appearance").is_some());

    tap(&mut harness, "Home");
    assert!(place_selected(&harness, "Home"));
    assert!(
        harness.query_by_label("Back").is_none(),
        "Home is at the bottom of the stack"
    );
}

/// Search opens with the keyboard in the box, and emptying the box keeps the page open.
#[test]
fn search_is_a_place_with_its_box_in_the_top_bar() {
    let _config = config_dir("touch-search");
    let server = home_server();
    let mut harness = phone_shell(&server);
    tap(&mut harness, "Search");
    assert!(place_selected(&harness, "Search"));
    let focused = harness.ctx.memory(|memory| memory.focused());
    assert_eq!(
        focused,
        Some(egui::Id::unique("top_bar_search")),
        "the box has the keyboard"
    );
    assert!(harness.query_by_label("Type in the search box to search.").is_some());
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

/// An item page on a phone stacks its header: the title across the page under the poster, and
/// Play as wide as the page.
#[test]
fn a_phones_item_header_stacks() {
    let _config = ConfigDir::new("touch-item");
    let server = FixtureServer::start();
    server.json("GET", "/Items/d265861eae31cee1d44b13adac305eac", "item_movie");
    let mut ui = phone_page(
        &server,
        Route::Item {
            id: "d265861eae31cee1d44b13adac305eac".into(),
            name: "Test Movie (2024)".into(),
        },
    );
    ui.step_until("the item", |harness| harness.query_by_label("More").is_some());
    let play = ui.harness().get_by_label_contains("Play").rect();
    // The page's 16 either side, inside the harness's own 8.
    let across = PHONE.x - 2. * 8. - 2. * 16.;
    assert!(
        (play.width() - across).abs() < 1.,
        "Play across the page, not {}",
        play.width()
    );
}
