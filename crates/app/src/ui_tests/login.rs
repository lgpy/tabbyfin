//! The sign-in form's steps against a mocked server: the server address, then the credentials
//! (typed into the fields as a user would), the error a refused sign-in shows, and the saved
//! accounts the form opens with. The answers are the captured fixtures.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
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
}

fn login_harness(make: impl FnOnce(&egui::Context) -> Login + 'static) -> egui_kittest::Harness<'static, LoginRun> {
    build_harness(egui::vec2(800., 700.), LoginRun::new(make), |ui, state| state.show(ui))
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
    assert!(ui.query_by_label("tabbyfin dev · Jellyfin 12.2.0").is_some());
    assert!(ui.query_by_label("Connect").is_none());

    // The username field takes the keyboard next; the password is one step on from it.
    ui.step();
    ui.get_by_role(Role::TextInput).type_text("dev");
    ui.step();
    ui.get_by_role(Role::PasswordInput).focus();
    ui.step();
    ui.get_by_role(Role::PasswordInput).type_text("dev");
    ui.step();
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
    assert!(ui.state().event.is_none(), "a failed sign-in reached the app");
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

    assert!(ui.query_by_label("Saved accounts").is_some());
    ui.get_by_label(&account.label()).click();
    ui.step();
    match ui.state().event.as_ref() {
        Some(LoginEvent::UseAccount(chosen)) => assert_eq!(chosen, &account),
        _ => panic!("choosing a saved account didn't ask to use it"),
    }
}
