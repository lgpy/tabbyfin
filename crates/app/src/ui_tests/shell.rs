//! The signed-in shell's top bar through the keyboard, on a desktop window: Up from a page's
//! first row moving the keyboard into the bar, the arrows walking along it and Enter pressing
//! what they leave it on, and the ways back out of it and out of a page it opened.
//!
//! What the bar highlights is painted rather than exposed to AccessKit, so a test checks what a
//! press does: the box taking the keyboard, Settings opening, the account menu listing the
//! account, and the page's own cursor still where it was.

use egui::accesskit::Role;
use egui::{Id, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use jellyfin::models::{AuthenticationResult, PublicSystemInfo};

use crate::config::{Config, SavedServer};
use crate::keys::{self, Press, Scope};
use crate::services::Services;
use crate::session::Session;
use crate::ui_tests::{ConfigDir, FixtureServer, block_on, build_harness, fixture_json, session, step_until};
use crate::views::PageCx;
use crate::views::shell::Shell;

/// The window the bar is driven in: a desktop, wide enough for the whole bar.
const WINDOW: egui::Vec2 = egui::vec2(1200., 800.);

/// The search box's id: the keyboard is in the box while egui's memory says it has the focus.
const SEARCH_ID: &str = "top_bar_search";

/// The signed-in shell as the app runs it, carrying the keys the app took into the frame.
struct ShellRun {
    session: Option<Session>,
    services: Option<Services>,
    shell: Option<Shell>,
    /// The presses [`keys::take`] took from egui, for the next frame.
    keys: Vec<Press>,
}

impl ShellRun {
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            self.services = Some(Services::new(session, &ctx));
        }
        let services = self.services.as_mut().expect("the services are made");
        services.tick(&ctx);
        let mut nav = Vec::new();
        let mut cx = PageCx {
            ctx: &ctx,
            services,
            nav: &mut nav,
            render_state: None,
            under_bar: None,
        };
        let shell = self.shell.get_or_insert_with(|| Shell::new(&mut cx));
        let presses = std::mem::take(&mut self.keys);
        shell.show(ui, presses, &mut cx);
    }
}

/// The shell in a harness, with keys routed as `App::raw_input_hook` and the shell route them.
struct Bar {
    harness: Harness<'static, ShellRun>,
}

impl Bar {
    fn new(server: &FixtureServer) -> Self {
        let run = ShellRun {
            session: Some(session(server)),
            services: None,
            shell: None,
            keys: Vec::new(),
        };
        let mut harness = build_harness(WINDOW, run, |ui, run| run.show(ui));
        harness.step();
        Self { harness }
    }

    fn press(&mut self, key: Key) {
        self.press_modifiers(Modifiers::NONE, key);
    }

    /// A key press with its release, taken from egui's input the way `App::raw_input_hook` takes
    /// them when no widget, menu or popup has the keyboard.
    fn press_modifiers(&mut self, modifiers: Modifiers, key: Key) {
        let ctx = self.harness.ctx.clone();
        let scopes: Vec<Scope> = self
            .harness
            .state()
            .shell
            .as_ref()
            .map(Shell::scopes)
            .unwrap_or_else(|| vec![Scope::Shell]);
        {
            let input = self.harness.input_mut();
            for pressed in [true, false] {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                });
            }
        }
        let presses = keys::take(&ctx, self.harness.input_mut(), &scopes);
        self.harness.state_mut().keys.extend(presses);
        self.harness.step();
    }

    /// Types `text` into whatever has the keyboard, as a keyboard does: a text field takes it.
    fn type_text(&mut self, text: &str) {
        self.harness.input_mut().events.push(egui::Event::Text(text.to_owned()));
        self.harness.step();
    }

    /// What egui's memory says has the keyboard.
    fn focused(&self) -> Option<Id> {
        self.harness.ctx.memory(|memory| memory.focused())
    }

    fn step(&mut self) {
        self.harness.step();
    }

    #[track_caller]
    fn step_until(&mut self, what: &str, done: impl FnMut(&Harness<'static, ShellRun>) -> bool) {
        step_until(&mut self.harness, what, done);
    }
}

/// A mocked server with the fixtures Home's shelves ask for, and the movies a library or the
/// results page lists.
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

/// The shell on Home, its shelves loaded and the cursor away.
fn home(server: &FixtureServer) -> Bar {
    let mut bar = Bar::new(server);
    bar.step_until("Home's shelves", |harness| harness.query_by_label("My Media").is_some());
    bar
}

/// Up past Home's first row has nowhere to go on the page, so the shell takes the keyboard for
/// the top bar, which starts on Search: Enter puts it in the search box, and Esc gives it back
/// to the page.
#[test]
fn up_from_homes_first_row_takes_the_keyboard_into_the_bar() {
    let _config = config_dir("shell-up-to-bar");
    let server = home_server();
    let mut bar = home(&server);

    // The first key only shows the cursor; it starts on the first row's first tile.
    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    bar.press(Key::Enter);
    assert_eq!(
        bar.focused(),
        Some(Id::unique(SEARCH_ID)),
        "the bar's first item is the search box's"
    );

    bar.press(Key::Escape);
    assert_eq!(bar.focused(), None, "Esc gives the keyboard back to the page");
}

/// The arrows move along the bar and stop at either end: Left on the first item stays there,
/// and Right then Left comes back to it, which Enter shows by opening the box.
#[test]
fn the_arrows_walk_the_bar_and_stop_at_its_ends() {
    let _config = config_dir("shell-arrows");
    let server = home_server();
    let mut bar = home(&server);

    // The first key only shows the cursor, where it was: the first tile of Home's first row.
    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    bar.press(Key::ArrowLeft);
    bar.press(Key::Enter);
    assert_eq!(
        bar.focused(),
        Some(Id::unique(SEARCH_ID)),
        "Left at the bar's first item kept the keyboard on it"
    );
    bar.press(Key::Escape);

    // Right leaves Search for SyncPlay; Left comes back, where Enter opens the box again rather
    // than the SyncPlay panel.
    bar.press(Key::ArrowUp);
    bar.press(Key::ArrowRight);
    bar.press(Key::ArrowLeft);
    bar.press(Key::Enter);
    assert_eq!(bar.focused(), Some(Id::unique(SEARCH_ID)), "Left came back to Search");
}

/// Down in the bar gives the keyboard back to the page with its cursor where it was: the tile
/// the cursor was on, not the first one, is the one Enter opens.
#[test]
fn down_in_the_bar_returns_to_the_page_with_the_cursor_in_place() {
    let _config = config_dir("shell-down-to-page");
    let server = home_server();
    let mut bar = home(&server);

    bar.press(Key::Enter);
    bar.press(Key::ArrowRight);
    bar.press(Key::ArrowUp);
    bar.press(Key::ArrowDown);

    // The second tile is Movies; the first is Collections.
    bar.press(Key::Enter);
    bar.step_until("the Movies library", |harness| {
        harness.query_by_role_and_label(Role::Label, "Movies").is_some()
    });
    assert!(
        bar.harness
            .query_by_role_and_label(Role::Label, "Collections")
            .is_none(),
        "the cursor moved off the tile it was on"
    );
}

/// Settings is the bar's own item, and the page it opens has no cursor of its own: Esc goes
/// back to the page the keyboard came from.
#[test]
fn enter_on_settings_opens_settings_and_escape_comes_back() {
    let _config = config_dir("shell-settings");
    let server = home_server();
    let mut bar = home(&server);

    // Up into the bar on Search, then Right twice: SyncPlay, then Settings.
    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    bar.press(Key::ArrowRight);
    bar.press(Key::ArrowRight);
    bar.press(Key::Enter);
    bar.step_until("Settings", |harness| {
        harness.query_by_role_and_label(Role::Button, "Playback").is_some()
    });

    bar.press(Key::Escape);
    bar.step_until("Home again", |harness| {
        harness.query_by_role_and_label(Role::Button, "Playback").is_none()
            && harness.query_by_label("My Media").is_some()
    });
}

/// The bar's Back button pops the page it opened, as Esc does.
#[test]
fn the_back_button_pops_a_pushed_page() {
    let _config = config_dir("shell-back-button");
    let server = home_server();
    let mut bar = home(&server);

    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    bar.press(Key::ArrowRight);
    bar.press(Key::ArrowRight);
    bar.press(Key::Enter);
    bar.step_until("Settings", |harness| {
        harness.query_by_role_and_label(Role::Button, "Playback").is_some()
    });

    bar.harness.get_by_label("Back").click();
    bar.step_until("Home again", |harness| {
        harness.query_by_role_and_label(Role::Button, "Playback").is_none()
            && harness.query_by_label("My Media").is_some()
    });
}

/// Ctrl+F puts the keyboard in the bar's search box from wherever it was; typing there opens
/// the results with the term, and they're a page of their own, so Back appears.
#[test]
fn ctrl_f_puts_the_keyboard_in_the_search_box_and_typing_searches() {
    let _config = config_dir("shell-search");
    let server = home_server();
    let mut bar = home(&server);
    assert!(
        bar.harness.query_by_label("Back").is_none(),
        "Home is at the bottom of the stack"
    );

    let ctrl = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    bar.press_modifiers(ctrl, Key::F);
    bar.step();
    assert_eq!(bar.focused(), Some(Id::unique(SEARCH_ID)), "the box has the keyboard");

    bar.type_text("movie");
    bar.step_until("the results", |harness| {
        harness
            .query_by_role_and_label(Role::Button, "Second Movie (2025)")
            .is_some()
    });
    assert!(bar.harness.query_by_label("Back").is_some(), "the results are on top");
    assert!(
        bar.harness
            .query_all_by_value("movie")
            .any(|node| node.accesskit_node().role() == Role::TextInput),
        "the box holds the term"
    );
}

/// The account menu opens from the keyboard, its entries a menu's, the signed-in account listed
/// among them with the server it's on.
#[test]
fn the_account_menu_opens_from_the_keyboard_and_lists_the_account() {
    let _config = config_dir("shell-account-menu");
    let server = home_server();
    let auth: AuthenticationResult = fixture_json("authenticate");
    let info: PublicSystemInfo = fixture_json("public_info");
    Config {
        check_for_updates: Some(false),
        accounts: vec![SavedServer {
            url: server.uri(),
            server_id: info.id.clone().unwrap_or_default(),
            user_id: auth.user.id.clone(),
            username: auth.user.name.clone(),
            server_name: info.server_name.clone(),
        }],
        ..Config::default()
    }
    .save()
    .expect("the test's config saves");

    let mut bar = home(&server);
    // Up into the bar on Search, then Right three times, past SyncPlay and Settings, to the
    // account.
    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    for _ in 0..3 {
        bar.press(Key::ArrowRight);
    }
    bar.press(Key::Enter);

    // A menu row's name has its detail after a comma: the account and the server it's on.
    let row = format!(
        "{}, {}",
        auth.user.name,
        info.server_name.as_deref().expect("the fixture names its server")
    );
    bar.step_until("the account menu", |harness| {
        harness.query_by_role_and_label(Role::MenuItemRadio, &row).is_some()
    });
}

/// Home with the account menu open from the keyboard, on a server whose Quick Connect is `on`.
fn account_menu(name: &str, quick_connect: bool) -> (ConfigDir, FixtureServer, Bar) {
    let config = config_dir(name);
    let server = home_server();
    server.json_value("GET", "/QuickConnect/Enabled", &serde_json::json!(quick_connect));
    let mut bar = home(&server);
    // Quick Connect's answer comes with the first frames.
    bar.step_until("Quick Connect's answer", |harness| {
        harness
            .state()
            .shell
            .as_ref()
            .is_some_and(|shell| shell.quick_connect_known())
    });
    bar.press(Key::Enter);
    bar.press(Key::ArrowUp);
    for _ in 0..3 {
        bar.press(Key::ArrowRight);
    }
    bar.press(Key::Enter);
    bar.step_until("the account menu", |harness| {
        harness
            .query_by_role_and_label(Role::MenuItemRadio, "Sign out")
            .is_some()
    });
    (config, server, bar)
}

/// *Authorise a device…* is in the account menu when the server has Quick Connect on.
#[test]
fn authorise_a_device_is_offered_with_quick_connect_on() {
    let (_config, _server, bar) = account_menu("shell-authorise-on", true);
    assert!(
        bar.harness
            .query_by_role_and_label(Role::MenuItemRadio, "Authorise a device…")
            .is_some()
    );
}

/// ...and left out when it's off.
#[test]
fn authorise_a_device_is_left_out_with_quick_connect_off() {
    let (_config, _server, bar) = account_menu("shell-authorise-off", false);
    assert!(
        bar.harness
            .query_by_role_and_label(Role::MenuItemRadio, "Authorise a device…")
            .is_none()
    );
}

/// *Watch history* is in the account menu whatever Quick Connect does.
#[test]
fn watch_history_is_in_the_account_menu() {
    let (_config, _server, bar) = account_menu("shell-history-entry", false);
    assert!(
        bar.harness
            .query_by_role_and_label(Role::MenuItemRadio, "Watch history")
            .is_some()
    );
}

/// The dialog takes the code, Enter sends it (without the spaces it was read out with), and the
/// device being signed in is a toast.
#[test]
fn authorising_a_device_sends_the_typed_code() {
    use crate::services::AppEvent;

    let (_config, server, mut bar) = account_menu("shell-authorise-send", true);
    server.json_value("POST", "/QuickConnect/Authorize", &serde_json::json!(true));
    bar.harness
        .get_by_role_and_label(Role::MenuItemRadio, "Authorise a device…")
        .click();
    bar.step_until("the dialog", |harness| harness.query_by_label("Code").is_some());
    bar.type_text("123 456");
    bar.harness.key_press(Key::Enter);
    bar.step_until("the request", |_| {
        server.requests("POST", "/QuickConnect/Authorize") == 1
    });
    bar.step_until("the toast", |harness| {
        harness
            .state()
            .services
            .as_ref()
            .is_some_and(|services| services.events.iter().any(|event| matches!(event, AppEvent::Toast(_))))
    });
    let request = block_on({
        let mock = server.mock.clone();
        async move { mock.received_requests().await.unwrap_or_default() }
    });
    let sent = request
        .iter()
        .find(|request| request.url.path() == "/QuickConnect/Authorize")
        .expect("the request");
    assert_eq!(sent.url.query(), Some("code=123456"));
    assert!(bar.harness.query_by_label("Code").is_none(), "the dialog closed");
}

/// A code the server doesn't know is worded under the field, and the dialog stays for another try.
#[test]
fn a_wrong_code_is_said_in_words_and_keeps_the_dialog() {
    let (_config, server, mut bar) = account_menu("shell-authorise-wrong", true);
    server.status("POST", "/QuickConnect/Authorize", 404);
    bar.harness
        .get_by_role_and_label(Role::MenuItemRadio, "Authorise a device…")
        .click();
    bar.step_until("the dialog", |harness| harness.query_by_label("Code").is_some());
    bar.type_text("000000");
    bar.harness.key_press(Key::Enter);
    bar.step_until("the explanation", |harness| {
        harness.query_by_label_contains("isn't right").is_some()
    });
    assert!(bar.harness.query_by_label("Code").is_some(), "the dialog is still open");
}
