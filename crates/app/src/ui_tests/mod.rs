//! UI tests: the real pages and screens driven through `egui_kittest`, which runs egui without
//! a window and without a GPU and exposes every widget to the test through AccessKit.
//!
//! A test drives the code the app runs: keys go through [`keys::take`] and the page's
//! `Page::on_key` as the shell routes them, the page draws into the harness's frame, and
//! `Services::tick` runs before each frame as `App::logic` does. Server responses are the
//! captured fixtures (`crates/jellyfin/tests/fixtures/12.2`) served by `wiremock`, as
//! `crates/jellyfin/tests/http.rs` does. There are no snapshots: what's checked is behaviour
//! (which card the cursor lands on, which route a key opens, what a step saves to the config),
//! so the tests run anywhere a `cargo test` does.
//!
//! Tests that build services or pages touch the config, and the config directory is one global
//! (`TABBYFIN_CONFIG_DIR`): they take a [`ConfigDir`] first, which serialises them and gives
//! each a temporary directory of its own.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use egui::{Modifiers, Pos2};
use egui_kittest::{Harness, HarnessBuilder};
use jellyfin::Client;
use jellyfin::models::{AuthenticationResult, PublicSystemInfo};
use serde::de::DeserializeOwned;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::Config;
use crate::keys::{self, Action, Press, Scope};
use crate::nav::NavRequest;
use crate::runtime;
use crate::services::Services;
use crate::session::{self, Session};
use crate::views::{Page, PageCx};

pub mod dialogs;
pub mod item;
pub mod library;
pub mod login;
pub mod menus;
#[cfg(feature = "mockup")]
pub mod mockups;
pub mod settings;
pub mod shelf;
pub mod states;
pub mod theme;
pub mod toasts;
pub mod widgets;

/// A disposable mockup, a file outside the tree that `.agents/skills/ui-mockup/render.py` points
/// `TABBYFIN_MOCKUP_SRC` at; it's compiled here so it can use the app's own theme and widgets.
#[cfg(feature = "mockup")]
mod scratch {
    include!(env!("TABBYFIN_MOCKUP_SRC"));
}

/// How long a test waits for a server answer or a page before it fails.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The fixtures' server version: 12.2, the one the app is developed against.
const FIXTURES: &str = "12.2";

/// The token [`session`] signs in with, as `authenticate.json` has it.
pub const TOKEN: &str = "00000000000000000000000000000000";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../jellyfin/tests/fixtures")
        .join(FIXTURES)
        .join(format!("{name}.json"))
}

/// A captured server response, as text.
pub fn fixture_text(name: &str) -> String {
    let path = fixture_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// A captured server response, parsed into the client's model.
pub fn fixture_json<T: DeserializeOwned>(name: &str) -> T {
    serde_json::from_str(&fixture_text(name)).unwrap_or_else(|err| panic!("{name}.json: {err}"))
}

/// Waits for `future` without needing a runtime context of the test's own: it runs on the
/// app's tokio runtime, as the UI's work does, and the test thread waits for the answer.
fn block_on<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (send, receive) = std::sync::mpsc::channel();
    runtime::detach(async move {
        let _ = send.send(future.await);
    });
    receive.recv().expect("the runtime answered")
}

/// The one config directory tests may use at a time. Held for the whole test: while it lives,
/// `TABBYFIN_CONFIG_DIR` points at its temporary directory, and on drop the directory goes.
///
/// Declare it *before* the harness: the services and pages read the config again as they're
/// dropped (the volume is saved with playback's end).
pub struct ConfigDir {
    _serial: MutexGuard<'static, ()>,
    dir: PathBuf,
}

static CONFIG_DIR: Mutex<()> = Mutex::new(());

impl ConfigDir {
    pub fn new(name: &str) -> Self {
        let serial = CONFIG_DIR.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = std::env::temp_dir().join(format!("tabbyfin-ui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary config directory");
        // SAFETY: every test that reads the config holds this guard for as long as the variable
        // is set, and no background thread reads it in between.
        unsafe { std::env::set_var("TABBYFIN_CONFIG_DIR", &dir) };
        Self { _serial: serial, dir }
    }
}

impl Drop for ConfigDir {
    fn drop(&mut self) {
        // SAFETY: see `new`; the guard is still held here.
        unsafe { std::env::remove_var("TABBYFIN_CONFIG_DIR") };
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A mocked server answering with the captured fixtures. `Arc` because setting up a mock hands
/// the server to the test's runtime, and wiremock's server has no `Clone`.
pub struct FixtureServer {
    mock: std::sync::Arc<MockServer>,
}

impl FixtureServer {
    pub fn start() -> Self {
        Self {
            mock: std::sync::Arc::new(block_on(MockServer::start())),
        }
    }

    pub fn uri(&self) -> String {
        self.mock.uri()
    }

    /// Answers `route` with fixture `name`, whatever the request.
    pub fn json(&self, http_method: &'static str, route: &'static str, name: &str) {
        self.respond(
            http_method,
            route,
            ResponseTemplate::new(200).set_body_string(fixture_text(name)),
        )
    }

    /// Answers `route` with `body`: a response the fixtures only hold inside another one, such
    /// as an episode out of a season's list.
    pub fn json_value(&self, http_method: &'static str, route: &'static str, body: &serde_json::Value) {
        self.respond(
            http_method,
            route,
            ResponseTemplate::new(200).set_body_string(body.to_string()),
        )
    }

    /// Answers `route` with fixture `name`, only for a request whose JSON body has `body`'s
    /// fields: a sign-in mock that only matches the credentials the test typed.
    pub fn json_for_body(&self, http_method: &'static str, route: &'static str, name: &str, body: serde_json::Value) {
        let template = ResponseTemplate::new(200).set_body_string(fixture_text(name));
        let mock = self.mock.clone();
        block_on(async move {
            Mock::given(method(http_method))
                .and(path(route))
                .and(wiremock::matchers::body_partial_json(body))
                .respond_with(template)
                .mount(&mock)
                .await;
        });
    }

    /// Answers `route` with the file at `path`, as `content_type`: an image for a mockup, where
    /// the fixtures only carry blurhashes.
    #[cfg(feature = "mockup")]
    pub fn file(&self, http_method: &'static str, route: &'static str, path: &str, content_type: &str) {
        let body = std::fs::read(path).unwrap_or_else(|err| panic!("{path}: {err}"));
        self.respond(
            http_method,
            route,
            ResponseTemplate::new(200).set_body_raw(body, content_type),
        )
    }

    /// Answers `route` with a bare status, e.g. a 401 for a wrong password.
    pub fn status(&self, http_method: &'static str, route: &'static str, status: u16) {
        self.respond(http_method, route, ResponseTemplate::new(status))
    }

    fn respond(&self, http_method: &'static str, route: &'static str, template: ResponseTemplate) {
        let mock = self.mock.clone();
        block_on(async move {
            Mock::given(method(http_method))
                .and(path(route))
                .respond_with(template.insert_header("content-type", "application/json"))
                .mount(&mock)
                .await;
        });
    }

    /// The JSON bodies of every `http_method` request to `route` served, in order.
    pub fn bodies(&self, http_method: &str, route: &str) -> Vec<serde_json::Value> {
        let mock = self.mock.clone();
        let requests = block_on(async move { mock.received_requests().await.unwrap_or_default() });
        requests
            .iter()
            .filter(|request| request.method.as_str() == http_method && request.url.path() == route)
            .map(|request| serde_json::from_slice(&request.body).expect("a JSON body"))
            .collect()
    }

    /// The `sortBy` of every `/Items` request served, in order.
    pub fn sorts_requested(&self) -> Vec<String> {
        let mock = self.mock.clone();
        let requests = block_on(async move { mock.received_requests().await.unwrap_or_default() });
        requests
            .iter()
            .filter(|request| request.url.path() == "/Items")
            .filter_map(|request| {
                request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "sortBy")
                    .map(|(_, value)| value.into_owned())
            })
            .collect()
    }
}

/// A signed-in session on `server`: the token and user from `authenticate.json` and the
/// server's `public_info.json`, so the services see what a real sign-in gives them.
pub fn session(server: &FixtureServer) -> Session {
    let auth: AuthenticationResult = fixture_json("authenticate");
    let info: PublicSystemInfo = fixture_json("public_info");
    let client = Client::new(&server.uri(), session::client_info("ui-tests"))
        .expect("the mock server's URL")
        .with_token(TOKEN);
    Session {
        client,
        user: auth.user,
        server: info,
    }
}

/// A harness with the app's fonts and initial appearance, and no accessibility check: the app
/// names most of its controls with a label beside them rather than `labelled_by`, which the
/// check (meant for a restyle's accessibility pass) would flag on nearly every one.
///
/// Characters only the system's fonts have (the arrow in Settings' descriptions, say) are drawn
/// as boxes rather than panicked on: eframe hands the app a system font provider, which a
/// harness has no counterpart for here (and CI has no fonts to offer it).
pub fn build_harness<S>(
    size: egui::Vec2,
    state: S,
    frame: impl FnMut(&mut egui::Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    harness_builder(size).build_ui_state(frame, state)
}

/// [`build_harness`]'s settings, for a harness that needs more of them (a mockup's scale and
/// theme).
pub fn harness_builder<S>(size: egui::Vec2) -> HarnessBuilder<S> {
    Harness::builder()
        .with_size(size)
        .allow_missing_glyphs()
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            egui_material_icons::initialize(ctx);
            crate::theme::apply(ctx, &Config::default());
        })
}

/// Steps frames until `done` holds, sleeping between them so a request has time to answer.
#[track_caller]
pub fn step_until<'a, S>(harness: &mut Harness<'a, S>, what: &str, mut done: impl FnMut(&Harness<'a, S>) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A page opened into a harness, with the services it draws with.
///
/// The first frame makes the services (which need the harness's context, to wake it) and the
/// page through `open`, as `App` makes them when the session signs in.
pub struct PageRun {
    session: Option<Session>,
    open: Option<OpenPage>,
    services: Option<Services>,
    page: Option<Box<dyn Page>>,
    /// What the page asked the shell to open.
    pub nav: Vec<NavRequest>,
    /// The keys the app took from egui, waiting for this frame.
    keys: Vec<Press>,
    /// What the page answered for each key it was offered, in order: `false` is a key it left
    /// to the shell (or the next of its own).
    pub handled: Vec<bool>,
}

type OpenPage = Box<dyn FnOnce(&mut PageCx) -> Box<dyn Page>>;

impl PageRun {
    fn new(session: Session, open: impl FnOnce(&mut PageCx) -> Box<dyn Page> + 'static) -> Self {
        Self {
            session: Some(session),
            open: Some(Box::new(open)),
            services: None,
            page: None,
            nav: Vec::new(),
            keys: Vec::new(),
            handled: Vec::new(),
        }
    }

    /// One frame: the app's logic (the services' work), then the keys the shell routes, then
    /// the page.
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            self.services = Some(Services::new(session, &ctx));
            let open = self.open.take().expect("the page is opened once");
            let page = {
                let mut cx = page_cx(&ctx, &mut self.services, &mut self.nav);
                open(&mut cx)
            };
            self.page = Some(page);
        }
        self.services.as_mut().expect("the services are made").tick(&ctx);
        let presses = std::mem::take(&mut self.keys);
        let mut handled = Vec::with_capacity(presses.len());
        {
            let mut cx = page_cx(&ctx, &mut self.services, &mut self.nav);
            let page = self.page.as_mut().expect("the page is open");
            let scope = page.scope();
            for press in presses {
                let mut used = false;
                if let Some(action) = scope.and_then(|scope| press.action(scope)) {
                    used = page.on_key(action, &mut cx);
                }
                if !used && press.action(Scope::Shell) == Some(Action::Back) {
                    cx.nav.push(NavRequest::Back);
                    used = true;
                }
                handled.push(used);
            }
        }
        self.handled.extend(handled);
        let mut cx = page_cx(&ctx, &mut self.services, &mut self.nav);
        self.page.as_mut().expect("the page is open").show(ui, &mut cx);
    }
}

/// A [`PageCx`] over the page's own parts, so the page and its services can be borrowed
/// together (a method on `PageRun` would borrow all of it).
fn page_cx<'a>(ctx: &'a egui::Context, services: &'a mut Option<Services>, nav: &'a mut Vec<NavRequest>) -> PageCx<'a> {
    PageCx {
        ctx,
        services: services.as_mut().expect("the services are made"),
        nav,
        render_state: None,
    }
}

/// A [`PageRun`] harness: keys as the app would read them, frames as the app would run them.
pub struct PageHarness {
    harness: Harness<'static, PageRun>,
}

impl PageHarness {
    pub fn new(session: Session, open: impl FnOnce(&mut PageCx) -> Box<dyn Page> + 'static) -> Self {
        let harness = build_harness(egui::vec2(1000., 700.), PageRun::new(session, open), |ui, state| {
            state.show(ui)
        });
        Self { harness }
    }

    pub fn state(&self) -> &PageRun {
        self.harness.state()
    }

    /// The harness itself, for a test that reads what a frame drew (a node's rectangle).
    pub fn harness(&self) -> &Harness<'static, PageRun> {
        &self.harness
    }

    /// The harness to give input to directly: the pointer and the wheel, which the page's own
    /// keys don't cover.
    pub fn harness_mut(&mut self) -> &mut Harness<'static, PageRun> {
        &mut self.harness
    }

    /// A key press with its release, routed as `App::raw_input_hook` and the shell route them:
    /// [`keys::take`] removes the press from egui's input first, so egui never sees a key the
    /// page uses.
    pub fn press(&mut self, key: egui::Key) {
        self.press_modifiers(Modifiers::NONE, key);
    }

    pub fn press_modifiers(&mut self, modifiers: Modifiers, key: egui::Key) {
        let ctx = self.harness.ctx.clone();
        let scopes: Vec<Scope> = {
            let page = self.state().page.as_ref().expect("the page is open");
            page.scope().into_iter().chain([Scope::Shell]).collect()
        };
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

    pub fn step(&mut self) {
        self.harness.step();
    }

    /// Steps frames until `done` holds, for a page or a request that's still on its way.
    #[track_caller]
    pub fn step_until(&mut self, what: &str, done: impl FnMut(&Harness<'static, PageRun>) -> bool) {
        step_until(&mut self.harness, what, done);
    }

    /// A click at `pos`: hover, press, release, and the pointer off the page again.
    pub fn click_at(&mut self, pos: Pos2) {
        self.harness.hover_at(pos);
        self.harness.step();
        self.harness.drag_at(pos);
        self.harness.step();
        self.harness.drop_at(pos);
        self.harness.step();
    }
}
