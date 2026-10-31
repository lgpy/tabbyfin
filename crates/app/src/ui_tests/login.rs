//! The sign-in form's steps against a mocked server: the server address, then the credentials
//! (typed into the fields as a user would), the error a refused sign-in shows, and the saved
//! accounts the form opens with. The answers are the captured fixtures.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use jellyfin::models::AuthenticationResult;

use crate::config::SavedServer;
use crate::ui_tests::{FixtureServer, TOKEN, build_harness, fixture_json, step_until};
use crate::views::login::{Login, LoginEvent, Prefill};

/// How a test makes its [`Login`]: the form is made on the first frame, since it needs the
/// harness's context.
type MakeLogin = Box<dyn FnOnce(&egui::Context) -> Login>;

/// The form, making its `Login` on the first frame and giving the test whatever it asks the
/// app to do.
struct LoginRun {
    make: Option<MakeLogin>,
    login: Option<Login>,
    event: Option<LoginEvent>,
}

impl LoginRun {
    fn new(make: impl FnOnce(&egui::Context) -> Login + 'static) -> Self {
        Self {
            make: Some(Box::new(make)),
            login: None,
            event: None,
        }
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let login = match &mut self.login {
            Some(login) => login,
            None => {
                let make = self.make.take().expect("the form is made once");
                self.login.insert(make(&ctx))
            }
        };
        login.tick(&ctx);
        login.show(ui, &ctx);
        if let Some(event) = login.take_event() {
            self.event = Some(event);
        }
    }

    /// The form itself, for what the app changes between frames (a parked account going away,
    /// the saved accounts after a *Forget*): those have no widget to reach through the tree.
    fn login_mut(&mut self) -> &mut Login {
        self.login.as_mut().expect("the form is made")
    }
}

fn login_harness(make: impl FnOnce(&egui::Context) -> Login + 'static) -> egui_kittest::Harness<'static, LoginRun> {
    build_harness(egui::vec2(800., 700.), LoginRun::new(make), |ui, state| state.show(ui))
}

/// A form taller than its window: eight saved accounts at 150 % text, 800×400.
fn tall_login_harness() -> egui_kittest::Harness<'static, LoginRun> {
    let accounts = (0..8)
        .map(|index| SavedServer {
            url: "http://localhost:8096/".into(),
            server_id: "a93d3c61bce64f4d8e8456ddf14de713".into(),
            user_id: format!("user-{index}"),
            username: format!("user{index}"),
            server_name: Some("tabbyfin dev".into()),
        })
        .collect();
    egui_kittest::Harness::builder()
        .with_size(egui::vec2(800., 400.))
        .allow_missing_glyphs()
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            egui_material_icons::initialize(ctx);
            crate::theme::apply(
                ctx,
                &crate::config::Config {
                    text_size: Some(150),
                    ..Default::default()
                },
            );
        })
        .build_ui_state(
            |ui, state: &mut LoginRun| state.show(ui),
            LoginRun::new(move |ctx| Login::new(ctx, "ui-test-device".into(), accounts, false, None)),
        )
}

/// Server address, then the credentials, then a session: each step draws the next, and the
/// username and password reach the sign-in request as they were typed.
#[test]
fn sign_in_steps_from_the_address_to_a_session() {
    let server = FixtureServer::start();
    server.json("GET", "/System/Info/Public", "public_info");
    server.json_for_body(
        "POST",
        "/Users/AuthenticateByName",
        "authenticate",
        serde_json::json!({"Username": "dev", "Pw": "dev"}),
    );
    let mut ui = login_harness(|ctx| Login::new(ctx, "ui-test-device".into(), Vec::new(), false, None));

    // The address field has the keyboard when the form opens.
    ui.get_by_role(Role::TextInput).type_text(&server.uri());
    ui.step();
    ui.get_by_label("Connect").click();
    ui.step();
    step_until(&mut ui, "the credentials step", |harness| {
        harness.query_by_label("Username").is_some()
    });
    // What the server said about itself, above the fields; the address step is gone.
    assert!(ui.query_by_label("Sign in to tabbyfin dev").is_some());
    assert!(ui.query_by_label("Jellyfin 12.2.0 at 127.0.0.1").is_some());
    assert!(ui.query_by_label("Connect").is_none());

    // The username field takes the keyboard next; the password is one step on from it.
    ui.step();
    ui.get_by_role(Role::TextInput).type_text("dev");
    ui.step();
    // The mascot watches the username being typed, and shuts its eyes for the password.
    assert!(ui.state_mut().login_mut().watch().look_at.is_some());
    ui.get_by_role(Role::PasswordInput).focus();
    ui.step();
    ui.get_by_role(Role::PasswordInput).type_text("dev");
    ui.step();
    let watch = ui.state_mut().login_mut().watch();
    assert!(watch.eyes_shut && watch.look_at.is_none(), "{watch:?}");
    ui.get_by_label("Sign in").click();
    ui.step();
    step_until(&mut ui, "the sign-in", |harness| harness.state().event.is_some());

    let auth: AuthenticationResult = fixture_json("authenticate");
    match ui.state().event.as_ref().expect("a sign-in event") {
        LoginEvent::LoggedIn(session) => {
            assert_eq!(session.user.name, "dev");
            assert_eq!(session.user.id, auth.user.id);
            assert_eq!(session.server.id, auth.server_id);
            assert_eq!(session.client.token(), Some(TOKEN));
        }
        _ => panic!("the form asked the app to do something other than sign in"),
    }
}

/// A fresh install's form takes the keyboard by itself: the very first thing typed at launch
/// lands in the address field, with no click and no Tab before it, and each step on from it
/// (the username, then the password, Enter taking each one to the next as `main`'s did) is
/// focused before anything is typed into it. The sign-in the mock only matches with the typed
/// credentials is what proves they arrived.
#[test]
fn the_form_takes_the_keyboard_at_launch() {
    let server = FixtureServer::start();
    server.json("GET", "/System/Info/Public", "public_info");
    server.json_for_body(
        "POST",
        "/Users/AuthenticateByName",
        "authenticate",
        serde_json::json!({"Username": "dev", "Pw": "dev"}),
    );
    let mut ui = login_harness(|ctx| Login::new(ctx, "ui-test-device".into(), Vec::new(), false, None));

    // The address field has the keyboard on the form's first frame, before anything was clicked.
    assert!(
        ui.get_by_role(Role::TextInput).is_focused(),
        "the address field doesn't have the keyboard at launch"
    );
    ui.get_by_role(Role::TextInput).type_text(&server.uri());
    ui.step();
    assert_eq!(
        ui.get_by_role(Role::TextInput).value().as_deref(),
        Some(server.uri().as_str()),
        "typing at launch didn't reach the address field"
    );

    // Enter connects, as on `main`, and the username field is focused for the next thing typed.
    ui.key_press(egui::Key::Enter);
    ui.step();
    step_until(&mut ui, "the credentials step", |harness| {
        harness.query_by_label("Username").is_some()
    });
    assert!(
        ui.get_by_role(Role::TextInput).is_focused(),
        "the username field didn't take the keyboard"
    );
    ui.get_by_role(Role::TextInput).type_text("dev");
    ui.step();
    assert_eq!(ui.get_by_role(Role::TextInput).value().as_deref(), Some("dev"));

    // Enter moves on to the password field, which is focused too; Enter there signs in.
    ui.key_press(egui::Key::Enter);
    ui.step();
    assert!(
        ui.get_by_role(Role::PasswordInput).is_focused(),
        "Enter in the username didn't move the keyboard to the password"
    );
    ui.get_by_role(Role::PasswordInput).type_text("dev");
    ui.step();
    ui.key_press(egui::Key::Enter);
    ui.step();
    step_until(&mut ui, "the sign-in", |harness| harness.state().event.is_some());

    match ui.state().event.as_ref().expect("a sign-in event") {
        LoginEvent::LoggedIn(session) => {
            assert_eq!(session.user.name, "dev");
            assert_eq!(session.client.token(), Some(TOKEN));
        }
        _ => panic!("the form asked the app to do something other than sign in"),
    }
}

/// A failed connect says so above the field and puts the keyboard back in it, so the address
/// can be fixed without clicking into it first (a connect started with Enter gives the field up,
/// as a text field does).
#[test]
fn a_failed_connect_keeps_the_keyboard_in_the_address_field() {
    let server = FixtureServer::start();
    server.status("GET", "/System/Info/Public", 404);
    let mut ui = login_harness(|ctx| Login::new(ctx, "ui-test-device".into(), Vec::new(), false, None));

    ui.get_by_role(Role::TextInput).type_text(&server.uri());
    ui.step();
    ui.key_press(egui::Key::Enter);
    ui.step();
    step_until(&mut ui, "the error", |harness| {
        harness
            .query_by_label("That address doesn't look like a Jellyfin server.")
            .is_some()
    });
    ui.step();

    // Typing again goes into the field, without clicking it first.
    assert!(
        ui.get_by_role(Role::TextInput).is_focused(),
        "the keyboard didn't come back to the address field"
    );
    ui.get_by_role(Role::TextInput).type_text("0");
    ui.step();
    let expected = format!("{}0", server.uri());
    assert_eq!(
        ui.get_by_role(Role::TextInput).value().as_deref(),
        Some(expected.as_str()),
        "the field lost what was typed after the error"
    );
}

/// The keyboard stays in the address field while the rows above it change: it keeps its own
/// widget id, where egui's automatic ones come and go with the widgets made before them, and
/// egui drops the focus of an id that stops drawing (`main`'s inputs were entities and kept
/// theirs). Here the parked account behind *Add account* goes away, which takes its button
/// away from above the field.
#[test]
fn the_keyboard_stays_in_the_field_when_the_rows_above_it_change() {
    let mut ui = login_harness(|ctx| Login::new(ctx, "ui-test-device".into(), Vec::new(), true, None));
    assert!(ui.query_by_label_contains("Back to your account").is_some());
    assert!(ui.get_by_role(Role::TextInput).is_focused());

    ui.state_mut().login_mut().set_can_cancel(false);
    ui.step();
    assert!(ui.query_by_label_contains("Back to your account").is_none());
    assert!(
        ui.get_by_role(Role::TextInput).is_focused(),
        "the field lost the keyboard when the rows above it changed"
    );

    ui.get_by_role(Role::TextInput).type_text("http://localhost:8096");
    ui.step();
    assert_eq!(
        ui.get_by_role(Role::TextInput).value().as_deref(),
        Some("http://localhost:8096")
    );
}

/// A refused sign-in says so and stays on the form, with the password field ready for another
/// try (as `main`'s did); no session reaches the app.
#[test]
fn a_wrong_password_keeps_the_form_and_says_why() {
    let server = FixtureServer::start();
    server.json("GET", "/System/Info/Public", "public_info");
    server.status("POST", "/Users/AuthenticateByName", 401);
    let url = server.uri();
    let mut ui = login_harness(move |ctx| {
        Login::new(
            ctx,
            "ui-test-device".into(),
            Vec::new(),
            false,
            Some(Prefill {
                url,
                username: "dev".into(),
                error: None,
                resume_saved_session: false,
            }),
        )
    });

    // A form opened with an address connects by itself; the password is what's missing.
    step_until(&mut ui, "the credentials step", |harness| {
        harness.query_by_label("Password").is_some()
    });
    ui.step();
    ui.get_by_role(Role::PasswordInput).focus();
    ui.step();
    ui.get_by_role(Role::PasswordInput).type_text("wrong");
    ui.step();
    ui.get_by_label("Sign in").click();
    ui.step();
    step_until(&mut ui, "the error", |harness| {
        harness.query_by_label("Wrong username or password.").is_some()
    });
    // The message is a danger notice (`surface::notice`), whose icon says what kind it is.
    assert!(
        ui.query_by_role_and_label(Role::Image, "Error").is_some(),
        "the error isn't shown as a danger notice"
    );
    assert!(ui.state().event.is_none(), "a failed sign-in reached the app");
}

/// Quick Connect swaps the credentials for the code to approve and a line saying it's waiting,
/// with Cancel under the panel; Cancel goes back to the credentials.
#[test]
fn quick_connect_shows_the_code_until_cancelled() {
    let server = FixtureServer::start();
    server.json("GET", "/System/Info/Public", "public_info");
    server.json_value("GET", "/QuickConnect/Enabled", &serde_json::json!(true));
    let pending = serde_json::json!({"Authenticated": false, "Secret": "secret", "Code": "482913"});
    server.json_value("POST", "/QuickConnect/Initiate", &pending);
    server.json_value("GET", "/QuickConnect/Connect", &pending);
    let url = server.uri();
    let mut ui = login_harness(move |ctx| {
        Login::new(
            ctx,
            "ui-test-device".into(),
            Vec::new(),
            false,
            Some(Prefill {
                url,
                username: String::new(),
                error: None,
                resume_saved_session: false,
            }),
        )
    });
    step_until(&mut ui, "the credentials step", |harness| {
        harness.query_by_label("Use Quick Connect").is_some()
    });
    ui.get_by_label("Use Quick Connect").click();
    ui.step();
    step_until(&mut ui, "the code", |harness| {
        harness.query_by_label("482913").is_some()
    });
    assert!(ui.query_by_label("Waiting for approval…").is_some());
    assert!(ui.query_by_label("Sign in").is_none());

    // The copy button puts the code on the clipboard and says so, once the form has recentred.
    ui.step();
    ui.get_by_label("Copy code").click();
    ui.step();
    let commands = &ui.output().platform_output.commands;
    assert!(
        commands.contains(&egui::OutputCommand::CopyText("482913".into())),
        "the code wasn't copied: {commands:?}"
    );
    ui.step();
    assert!(ui.query_by_label("Copied").is_some());

    // The shorter step recentres the form on the next frame; click Cancel where it settles.
    ui.step();
    ui.get_by_label("Cancel").click();
    ui.step();
    step_until(&mut ui, "the credentials again", |harness| {
        harness.query_by_label("Sign in").is_some()
    });
    assert!(ui.query_by_label("482913").is_none());
}

/// The address field keeps its size as it takes and gives up the keyboard (egui takes a field's
/// outline out of its whole-point margin, so a focus ring wider than the resting outline shrank
/// it), and its hint shows only while it doesn't have the keyboard.
#[test]
fn the_address_field_keeps_its_size_and_hides_its_hint_while_typed_in() {
    let mut ui = login_harness(|ctx| Login::new(ctx, "ui-test-device".into(), Vec::new(), false, None));
    ui.step();
    ui.step();
    let field = ui.get_by_role(Role::TextInput);
    assert!(field.is_focused(), "the address field takes the keyboard at launch");
    let focused = field.rect();
    // egui names a field without a hint with an empty one.
    assert_eq!(
        field.accesskit_node().placeholder().unwrap_or_default(),
        "",
        "no hint beside the caret"
    );

    // Escape gives up a text field's focus.
    ui.key_press(egui::Key::Escape);
    ui.step();
    ui.step();
    let field = ui.get_by_role(Role::TextInput);
    assert!(!field.is_focused());
    assert_eq!(field.rect(), focused, "the field changed size with the focus");
    assert_eq!(
        field.accesskit_node().placeholder(),
        Some("jellyfin.example.com or 192.168.1.10:8096")
    );
}

/// The form opens with the saved accounts to continue as, and choosing one asks the app to
/// sign in with its stored token.
#[test]
fn a_saved_account_is_offered_and_chosen() {
    let account = SavedServer {
        url: "http://127.0.0.1:8096".into(),
        server_id: "server".into(),
        user_id: "user".into(),
        username: "dev".into(),
        server_name: Some("tabbyfin dev".into()),
    };
    let listed = account.clone();
    let mut ui = login_harness(move |ctx| Login::new(ctx, "ui-test-device".into(), vec![listed], false, None));

    assert!(ui.query_by_label("Welcome back").is_some());
    ui.get_by_label(&account.label()).click();
    ui.step();
    match ui.state().event.as_ref() {
        Some(LoginEvent::UseAccount(chosen)) => assert_eq!(chosen, &account),
        _ => panic!("choosing a saved account didn't ask to use it"),
    }
}

/// A form taller than the window scrolls the field the keyboard lands in into view: the
/// address field has the focus on the first frame, as it does at launch.
#[test]
fn a_tall_form_reveals_the_focused_field() {
    let mut ui = tall_login_harness();
    ui.step();
    ui.step();
    let field = ui.get_by_role(Role::TextInput);
    assert!(field.is_focused(), "the address field takes the keyboard");
    assert!(
        field.rect().top() >= 0. && field.rect().bottom() <= 400.,
        "the focused field is on screen: {:?}",
        field.rect()
    );
    // Tab goes on to Connect and then the saved accounts; the form scrolls back up to them.
    for _ in 0..2 {
        ui.key_press(egui::Key::Tab);
        ui.step();
    }
    // The scroll target animates over the next frames.
    for _ in 0..20 {
        ui.step();
    }
    let account = ui.get_by_label("user0 on tabbyfin dev");
    assert!(account.is_focused(), "Tab reached the first saved account");
    assert!(
        account.rect().top() >= 0. && account.rect().bottom() <= 400.,
        "the newly focused account is on screen: {:?}",
        account.rect()
    );
}
