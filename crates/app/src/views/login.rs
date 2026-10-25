//! Sign-in: a saved account, or a server address, then username and password or Quick Connect.
//! Ported from `main`'s `views/login.rs`.
//!
//! The form never awaits a request: the running operation is a [`Task`], replaced or dropped
//! when the user tries something else, which cancels it. While one runs, it reports what it
//! can't return until the end — the server still starting up, the Quick Connect code — over a
//! [`task::channel`]. The app turns the form's [`LoginEvent`]s into screens; nothing here touches
//! the config or the credential store.

use std::time::{Duration, Instant};

use egui_material_icons::icons::{ICON_ARROW_BACK, ICON_CLOSE, ICON_VISIBILITY, ICON_VISIBILITY_OFF};
use jellyfin::models::PublicSystemInfo;
use jellyfin::{Client, ClientInfo, Error, MIN_SUPPORTED_VERSION};

use crate::config::SavedServer;
use crate::session::{self, Session, describe_error};
use crate::task::{self, Deadline, Receiver, Sender, Task};

/// How long to keep retrying while the server reports that it is starting up.
const STARTUP_WAIT: Duration = Duration::from_secs(120);
const QUICK_CONNECT_POLL: Duration = Duration::from_secs(3);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// The form's width; the window centres it.
const FORM_WIDTH: f32 = 380.0;

pub enum LoginEvent {
    LoggedIn(Box<Session>),
    /// The server answered, and the form was opened with `resume_saved_session`.
    ServerReachable,
    /// Sign in to a saved account with its stored token.
    UseAccount(SavedServer),
    /// Remove a saved account.
    ForgetAccount(SavedServer),
    /// Go back to the session that was signed in (adding an account).
    Cancel,
}

/// Values to start the form with, e.g. after sign-out or an expired session.
pub struct Prefill {
    pub url: String,
    pub username: String,
    pub error: Option<String>,
    /// A saved session couldn't be restored because the server was unreachable. Once the
    /// server answers, emit [`LoginEvent::ServerReachable`] instead of asking for credentials.
    pub resume_saved_session: bool,
}

/// What the running operation ends with.
enum Outcome {
    /// The server answered, with its version checked: the client to sign in with, and what it
    /// says about itself.
    Connected(Box<Result<(Client, PublicSystemInfo), String>>),
    /// The server accepted the credentials.
    LoggedIn(Box<Session>),
    /// A request failed, with the message for the user.
    Failed(String),
}

/// What the running operation reports before it ends.
enum Progress {
    /// While a starting server keeps answering "still starting"; cleared by the caller when
    /// the operation ends.
    Status(String),
    /// The code to show in another Jellyfin app.
    QuickConnectCode(String),
}

/// The field to focus when the form draws next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Server,
    Username,
    Password,
}

pub struct Login {
    device_id: String,
    /// Saved accounts to continue as, most recently used first.
    accounts: Vec<SavedServer>,
    /// Another account is signed in and waiting (adding an account).
    can_cancel: bool,
    resume_saved_session: bool,

    url: String,
    username: String,
    password: String,
    /// The password field shows its text (the eye toggle).
    show_password: bool,
    /// Set when the form moves to another step, so the next field the user needs has focus.
    focus: Option<Field>,

    /// Set once the server address has been verified.
    server: Option<(Client, PublicSystemInfo)>,
    quick_connect_code: Option<String>,
    busy: bool,
    status: Option<String>,
    error: Option<String>,

    /// The in-flight operation. Replacing or clearing it cancels the previous one.
    task: Option<Task<Outcome>>,
    /// What the in-flight operation reports while it runs. Each request gets a fresh channel and
    /// cancelling drops it, so a cancelled request's late messages have nowhere to land.
    progress: Option<Receiver<Progress>>,
    /// Reconnect attempt while waiting for an unreachable server (`resume_saved_session`).
    retry: Deadline,
    /// What the form wants the app to do; taken by the app after the frame.
    event: Option<LoginEvent>,
}

impl Login {
    pub fn new(
        ctx: &egui::Context,
        device_id: String,
        accounts: Vec<SavedServer>,
        can_cancel: bool,
        prefill: Option<Prefill>,
    ) -> Self {
        let (url, username, error, resume_saved_session) = match prefill {
            Some(prefill) => (
                prefill.url,
                prefill.username,
                prefill.error,
                prefill.resume_saved_session,
            ),
            None => (String::new(), String::new(), None, false),
        };
        let has_url = !url.is_empty();
        let mut login = Self {
            device_id,
            accounts,
            can_cancel,
            resume_saved_session,
            url,
            username,
            password: String::new(),
            show_password: false,
            focus: if has_url { None } else { Some(Field::Server) },
            server: None,
            quick_connect_code: None,
            busy: false,
            status: None,
            error,
            task: None,
            progress: None,
            retry: Deadline::default(),
            event: None,
        };
        if has_url {
            login.connect(ctx);
        }
        login
    }

    /// What the form asked the app to do, once.
    pub fn take_event(&mut self) -> Option<LoginEvent> {
        self.event.take()
    }

    pub fn set_accounts(&mut self, accounts: Vec<SavedServer>) {
        self.accounts = accounts;
    }

    /// Stops waiting to restore a saved session, when that account was removed: nothing would
    /// be left to sign back in to once the server answers.
    pub fn stop_resuming(&mut self) {
        self.resume_saved_session = false;
        self.retry.cancel();
        self.status = None;
    }

    /// Whether the form can go back to a parked session; false once that session is gone.
    pub fn set_can_cancel(&mut self, can_cancel: bool) {
        self.can_cancel = can_cancel;
    }

    /// Everything that happens without drawing: the reconnect timer and the running request's
    /// progress and result. Runs from `App::logic`, which eframe also calls while the window
    /// is hidden, so an approval or a server coming back still signs in when minimised.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.retry.fire(ctx) {
            self.connect(ctx);
        }
        // Before the outcome, so a code sent just before it can't outlive the step.
        if let Some(receiver) = &mut self.progress {
            for progress in receiver.drain() {
                match progress {
                    Progress::Status(status) => self.status = Some(status),
                    Progress::QuickConnectCode(code) => self.quick_connect_code = Some(code),
                }
            }
        }
        if let Some(outcome) = self.task.as_mut().and_then(Task::poll) {
            self.task = None;
            self.finish(outcome);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(FORM_WIDTH, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.form(ui, ctx),
                );
            });
        });
    }

    fn form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("Tabbyfin").heading());
        ui.label(egui::RichText::new("Sign in to your Jellyfin server").weak());
        ui.add_space(12.0);
        if let Some(error) = self.error.clone() {
            error_box(ui, &error);
            ui.add_space(8.0);
        }
        if self.can_cancel {
            if ui.button((ICON_ARROW_BACK, "Back to your account")).clicked() {
                self.event = Some(LoginEvent::Cancel);
            }
            ui.add_space(4.0);
        }
        if let Some(status) = self.status.clone() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
            });
            ui.add_space(4.0);
        }
        if let Some(code) = self.quick_connect_code.clone() {
            self.quick_connect_step(ui, &code);
        } else if let Some((_, info)) = self.server.clone() {
            self.credentials_step(ui, &info, ctx);
        } else {
            self.server_step(ui, ctx);
        }
    }

    fn on_connect(&mut self, ctx: &egui::Context) {
        self.error = None;
        // The user's attempt replaces a pending reconnect; a failure arms it again while the
        // saved session is still being waited for.
        self.retry.cancel();
        self.connect(ctx);
    }

    /// Verifies the server address and moves on to the credentials step.
    fn connect(&mut self, ctx: &egui::Context) {
        let input = self.url.trim().to_string();
        if input.is_empty() {
            self.error = Some("Enter your server's address.".into());
            return;
        }
        let candidates = candidates(&input);
        let info = session::client_info(&self.device_id);
        self.busy = true;
        let progress = self.begin_request(ctx);
        self.task = Some(Task::spawn(ctx, async move {
            Outcome::Connected(Box::new(probe(candidates, info, progress).await))
        }));
    }

    fn sign_in(&mut self, ctx: &egui::Context) {
        let Some((client, info)) = self.server.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        let username = self.username.trim().to_string();
        let password = self.password.clone();
        if username.is_empty() {
            self.error = Some("Enter your username.".into());
            return;
        }

        self.busy = true;
        self.error = None;
        let _ = self.begin_request(ctx);
        self.task = Some(Task::spawn(ctx, async move {
            match client.authenticate_by_name(&username, &password).await {
                Ok(auth) => Outcome::LoggedIn(Box::new(Session {
                    client: client.with_token(auth.access_token.as_str()),
                    user: auth.user,
                    server: info,
                })),
                Err(Error::Unauthorized) => Outcome::Failed("Wrong username or password.".into()),
                Err(err) => Outcome::Failed(describe_error(&err)),
            }
        }));
    }

    fn start_quick_connect(&mut self, ctx: &egui::Context) {
        let Some((client, info)) = self.server.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.status = None;
        let progress = self.begin_request(ctx);
        self.task = Some(Task::spawn(ctx, async move {
            match client.quick_connect_enabled().await {
                Ok(true) => {}
                Ok(false) => return Outcome::Failed("Quick Connect is turned off on this server.".into()),
                Err(err) => return Outcome::Failed(describe_error(&err)),
            }
            let qc = match client.quick_connect_initiate().await {
                Ok(qc) => qc,
                Err(err) => return Outcome::Failed(describe_error(&err)),
            };
            let _ = progress.send(Progress::QuickConnectCode(qc.code.clone()));
            loop {
                tokio::time::sleep(QUICK_CONNECT_POLL).await;
                match client.quick_connect_state(&qc.secret).await {
                    Ok(state) if state.authenticated => break,
                    Ok(_) => {}
                    Err(Error::NotFound) => {
                        return Outcome::Failed("The Quick Connect code expired. Try again.".into());
                    }
                    Err(err) => return Outcome::Failed(describe_error(&err)),
                }
            }
            match client.authenticate_with_quick_connect(&qc.secret).await {
                Ok(auth) => Outcome::LoggedIn(Box::new(Session {
                    client: client.with_token(auth.access_token.as_str()),
                    user: auth.user,
                    server: info,
                })),
                Err(err) => Outcome::Failed(describe_error(&err)),
            }
        }));
    }

    /// What the finished operation means for the form.
    fn finish(&mut self, outcome: Outcome) {
        self.busy = false;
        self.status = None;
        match outcome {
            Outcome::Connected(result) => match *result {
                Ok((client, info)) => {
                    if self.resume_saved_session {
                        self.resume_saved_session = false;
                        self.event = Some(LoginEvent::ServerReachable);
                    } else {
                        // Keep any error passed in via `Prefill` (e.g. "session expired");
                        // user-triggered connects clear it in `on_connect`.
                        self.server = Some((client, info));
                        self.focus = Some(if self.username.trim().is_empty() {
                            Field::Username
                        } else {
                            Field::Password
                        });
                    }
                }
                Err(message) => {
                    self.error = Some(message);
                    // Waiting to restore a saved session: keep trying until the server is back.
                    if self.resume_saved_session {
                        self.status =
                            Some("You'll be signed back in automatically as soon as the server answers.".into());
                        self.retry.set(RECONNECT_INTERVAL);
                    }
                }
            },
            Outcome::LoggedIn(session) => {
                self.quick_connect_code = None;
                // A click raised this frame (*Back to your account*) wins over the late result.
                self.event.get_or_insert(LoginEvent::LoggedIn(session));
            }
            Outcome::Failed(message) => {
                // A failed sign-in keeps the password field focused, as `main`'s input did;
                // a failed Quick Connect leaves the step and asks for a new attempt.
                let from_quick_connect = self.quick_connect_code.take().is_some();
                self.error = Some(message);
                if self.server.is_some() && !from_quick_connect {
                    self.focus = Some(Field::Password);
                }
            }
        }
    }

    /// Starts a request: replaces the task slot (cancelling the previous request) and gives the
    /// new one its own progress channel.
    fn begin_request(&mut self, ctx: &egui::Context) -> Sender<Progress> {
        let (sender, progress) = task::channel(ctx);
        self.progress = Some(progress);
        sender
    }

    fn cancel(&mut self) {
        self.task = None;
        self.progress = None;
        self.busy = false;
        self.status = None;
        self.quick_connect_code = None;
    }

    fn change_server(&mut self) {
        self.cancel();
        self.retry.cancel();
        self.resume_saved_session = false;
        self.server = None;
        self.error = None;
        self.focus = Some(Field::Server);
    }

    fn server_step(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.accounts.is_empty() {
            ui.label(egui::RichText::new("Saved accounts").weak());
            ui.add_space(4.0);
            for account in self.accounts.clone() {
                ui.horizontal(|ui| {
                    let width = ui.available_width() - 32.0;
                    let use_it = egui::Button::new(account.label()).min_size(egui::vec2(width, 26.0));
                    if ui.add_enabled(!self.busy, use_it).clicked() {
                        self.event = Some(LoginEvent::UseAccount(account.clone()));
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(ICON_CLOSE))
                        .on_hover_text("Remove this account")
                        .clicked()
                    {
                        self.event = Some(LoginEvent::ForgetAccount(account.clone()));
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Or sign in to another").weak());
            ui.add_space(4.0);
        }

        ui.label("Server address");
        let address = ui.add(
            egui::TextEdit::singleline(&mut self.url)
                .hint_text("jellyfin.example.com or 192.168.1.10:8096")
                .desired_width(f32::INFINITY)
                .interactive(!self.busy),
        );
        if self.focus == Some(Field::Server) {
            address.request_focus();
            self.focus = None;
        }
        let entered = address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let connect = ui.add_enabled(!self.busy, egui::Button::new("Connect"));
            if self.busy {
                ui.spinner();
            }
            if connect.clicked() || entered {
                self.on_connect(ctx);
            }
        });
    }

    fn credentials_step(&mut self, ui: &mut egui::Ui, info: &PublicSystemInfo, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} · Jellyfin {}",
                    info.server_name.as_deref().unwrap_or("Jellyfin"),
                    info.version.as_deref().unwrap_or("?")
                ))
                .weak(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Change").clicked() {
                    self.change_server();
                }
            });
        });
        ui.add_space(8.0);

        ui.label("Username");
        let username = ui.add(
            egui::TextEdit::singleline(&mut self.username)
                .desired_width(f32::INFINITY)
                .interactive(!self.busy),
        );
        if self.focus == Some(Field::Username) {
            username.request_focus();
            self.focus = None;
        }
        let username_entered = username.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.add_space(4.0);

        ui.label("Password");
        let mut password_entered = false;
        ui.horizontal(|ui| {
            let password = ui.add(
                egui::TextEdit::singleline(&mut self.password)
                    .password(!self.show_password)
                    .desired_width(ui.available_width() - 40.0)
                    .interactive(!self.busy),
            );
            if self.focus == Some(Field::Password) {
                password.request_focus();
                self.focus = None;
            }
            password_entered = password.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let icon = if self.show_password {
                ICON_VISIBILITY_OFF
            } else {
                ICON_VISIBILITY
            };
            let tooltip = if self.show_password {
                "Hide password"
            } else {
                "Show password"
            };
            let toggle = ui
                .add_enabled(!self.busy, egui::Button::selectable(self.show_password, icon))
                .on_hover_text(tooltip);
            if toggle.clicked() {
                self.show_password = !self.show_password;
            }
        });
        if username_entered {
            self.focus = Some(Field::Password);
            ui.ctx().request_repaint();
        }
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let sign_in = ui.add_enabled(!self.busy, egui::Button::new("Sign in"));
            if self.busy {
                ui.spinner();
            }
            let quick_connect = ui.add_enabled(!self.busy, egui::Button::new("Use Quick Connect"));
            if quick_connect.clicked() {
                self.start_quick_connect(ctx);
            }
            if sign_in.clicked() || password_entered {
                self.sign_in(ctx);
            }
        });
    }

    fn quick_connect_step(&mut self, ui: &mut egui::Ui, code: &str) {
        ui.label("Enter this code in another Jellyfin app where you're signed in, under Quick Connect:");
        ui.add_space(6.0);
        ui.label(egui::RichText::new(code).size(32.0).strong());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Waiting for approval");
        });
        ui.add_space(8.0);
        if ui.button("Cancel").clicked() {
            self.cancel();
        }
    }
}

/// A boxed line in the theme's error colour, so it reads as more than body text.
fn error_box(ui: &mut egui::Ui, message: &str) {
    let color = ui.visuals().error_fg_color;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.colored_label(color, message);
        });
}

/// The URLs to try for an address: one with a scheme; without one, HTTPS first and plain HTTP
/// as the fallback.
fn candidates(input: &str) -> Vec<String> {
    if input.contains("://") {
        vec![input.to_owned()]
    } else {
        vec![format!("https://{input}"), format!("http://{input}")]
    }
}

/// Tries each candidate URL in turn and returns the first supported Jellyfin server.
/// Waits (and reports status) while a server is still starting up.
async fn probe(
    candidates: Vec<String>,
    info: ClientInfo,
    progress: Sender<Progress>,
) -> Result<(Client, PublicSystemInfo), String> {
    let mut last_error = None;
    for url in candidates {
        let client = match Client::new(&url, info.clone()) {
            Ok(client) => client,
            Err(err) => {
                last_error = Some(err);
                continue;
            }
        };
        let started = Instant::now();
        loop {
            match client.public_info().await {
                Ok(server) => {
                    let Some(version) = server.server_version() else {
                        return Err("The server didn't report its version.".into());
                    };
                    if !version.is_supported() {
                        return Err(format!(
                            "This server runs Jellyfin {version}. Tabbyfin needs {MIN_SUPPORTED_VERSION} or newer."
                        ));
                    }
                    tracing::info!(url = %client.base_url(), %version, "connected to server");
                    return Ok((client, server));
                }
                Err(Error::ServerStarting) if started.elapsed() < STARTUP_WAIT => {
                    let _ = progress.send(Progress::Status("The server is starting up. Waiting…".into()));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(err) => {
                    tracing::debug!(%url, "probe failed: {err}");
                    last_error = Some(err);
                    break;
                }
            }
        }
    }
    Err(match last_error {
        Some(Error::Decode { .. } | Error::NotFound) => "That address doesn't look like a Jellyfin server.".into(),
        Some(err) => describe_error(&err),
        None => "Couldn't reach the server.".into(),
    })
}

#[cfg(test)]
mod tests {
    use jellyfin::Client;
    use jellyfin::models::{PublicSystemInfo, UserDto};

    use std::time::Duration;

    use super::{Field, Login, LoginEvent, Outcome, Progress, candidates};
    use crate::session::{Session, client_info};

    fn client() -> Client {
        Client::new("http://localhost:8096", client_info("s1-test")).expect("the URL parses")
    }

    fn server() -> PublicSystemInfo {
        PublicSystemInfo {
            id: Some("server".into()),
            server_name: Some("test".into()),
            version: Some("12.2.0".into()),
            product_name: None,
            local_address: None,
            startup_wizard_completed: None,
        }
    }

    fn session() -> Session {
        Session {
            client: client(),
            user: UserDto {
                id: "user".into(),
                name: "dev".into(),
                server_id: Some("server".into()),
                primary_image_tag: None,
                configuration: None,
            },
            server: server(),
        }
    }

    fn new_login() -> Login {
        Login::new(&egui::Context::default(), "s1-test".into(), Vec::new(), false, None)
    }

    #[test]
    fn candidates_follow_the_scheme_or_try_https_first() {
        assert_eq!(
            candidates("jellyfin.example.com"),
            [
                "https://jellyfin.example.com".to_owned(),
                "http://jellyfin.example.com".to_owned()
            ]
        );
        assert_eq!(
            candidates("http://192.168.1.10:8096"),
            ["http://192.168.1.10:8096".to_owned()]
        );
        assert_eq!(
            candidates("https://host/jellyfin"),
            ["https://host/jellyfin".to_owned()]
        );
    }

    #[test]
    fn a_server_answering_for_a_saved_session_asks_the_app_to_restore() {
        let mut login = new_login();
        login.resume_saved_session = true;
        login.finish(Outcome::Connected(Box::new(Ok((client(), server())))));
        assert!(matches!(login.take_event(), Some(LoginEvent::ServerReachable)));
        assert!(login.server.is_none());
        assert!(login.take_event().is_none());
    }

    #[test]
    fn a_failed_reconnect_waits_and_tries_again() {
        let mut login = new_login();
        login.resume_saved_session = true;
        login.finish(Outcome::Connected(Box::new(Err("Couldn't reach the server.".into()))));
        assert_eq!(login.error.as_deref(), Some("Couldn't reach the server."));
        assert!(
            login
                .status
                .as_deref()
                .is_some_and(|status| status.contains("signed back in automatically"))
        );
        assert!(login.retry.is_set());
        assert!(login.take_event().is_none());
    }

    #[test]
    fn a_successful_connect_focuses_the_next_missing_field() {
        let mut login = new_login();
        login.finish(Outcome::Connected(Box::new(Ok((client(), server())))));
        assert!(login.server.is_some());
        assert_eq!(login.focus, Some(Field::Username));

        let mut login = new_login();
        login.username = "dev".into();
        login.finish(Outcome::Connected(Box::new(Ok((client(), server())))));
        assert_eq!(login.focus, Some(Field::Password));
    }

    #[test]
    fn a_failed_sign_in_keeps_the_password_focused() {
        let mut login = new_login();
        login.server = Some((client(), server()));
        login.finish(Outcome::Failed("Wrong username or password.".into()));
        assert_eq!(login.error.as_deref(), Some("Wrong username or password."));
        assert_eq!(login.focus, Some(Field::Password));
    }

    #[test]
    fn a_quick_connect_failure_clears_the_code_without_stealing_focus() {
        let mut login = new_login();
        login.server = Some((client(), server()));
        login.quick_connect_code = Some("ABC123".into());
        // What the credentials step is left with: the password field lost focus to the code.
        login.focus = None;
        login.finish(Outcome::Failed("The Quick Connect code expired. Try again.".into()));
        assert!(login.quick_connect_code.is_none());
        assert_eq!(login.focus, None);
    }

    #[test]
    fn a_replaced_request_cannot_post_progress_to_the_new_one() {
        let ctx = egui::Context::default();
        let mut login = new_login();
        let old = login.begin_request(&ctx);
        let new = login.begin_request(&ctx);
        // The cancelled request's messages die with its own receiver, so a late Quick Connect
        // code can't bring that step back.
        assert!(old.send(Progress::QuickConnectCode("OLD".into())).is_err());
        assert!(new.send(Progress::QuickConnectCode("NEW".into())).is_ok());
        login.tick(&ctx);
        assert_eq!(login.quick_connect_code.as_deref(), Some("NEW"));
    }

    #[test]
    fn a_cancelled_request_cannot_post_progress() {
        let ctx = egui::Context::default();
        let mut login = new_login();
        let sender = login.begin_request(&ctx);
        // *Change* clicked between the code being sent and the next tick draining it.
        assert!(sender.send(Progress::QuickConnectCode("LATE".into())).is_ok());
        login.change_server();
        login.tick(&ctx);
        assert_eq!(login.quick_connect_code, None);
        assert!(sender.send(Progress::QuickConnectCode("LATER".into())).is_err());
    }

    #[test]
    fn connecting_by_hand_cancels_the_pending_reconnect() {
        let ctx = egui::Context::default();
        let mut login = new_login();
        login.retry.set(Duration::from_secs(5));
        // An empty address fails at once, so this only exercises the attempt itself.
        login.on_connect(&ctx);
        assert!(!login.retry.is_set());
        assert_eq!(login.error.as_deref(), Some("Enter your server's address."));
    }

    #[test]
    fn signing_in_clears_the_quick_connect_code() {
        let mut login = new_login();
        login.quick_connect_code = Some("ABC123".into());
        login.finish(Outcome::LoggedIn(Box::new(session())));
        assert!(login.quick_connect_code.is_none());
        assert!(matches!(login.take_event(), Some(LoginEvent::LoggedIn(_))));
    }
}
