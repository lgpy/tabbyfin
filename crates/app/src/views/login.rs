//! Sign-in: a saved account, or a server address, then username and password or Quick Connect.
//! Ported from `main`'s `views/login.rs`.
//!
//! The form never awaits a request: the running operation is a [`Task`], replaced or dropped
//! when the user tries something else, which cancels it. While one runs, it reports what it
//! can't return until the end — the server still starting up, the Quick Connect code — over a
//! [`task::channel`]. The app turns the form's [`LoginEvent`]s into screens; nothing here touches
//! the config or the credential store.

use std::time::{Duration, Instant};

use egui_material_icons::icons::{ICON_ARROW_BACK, ICON_VISIBILITY, ICON_VISIBILITY_OFF};
use jellyfin::models::PublicSystemInfo;
use jellyfin::{Client, ClientInfo, Error, MIN_SUPPORTED_VERSION};

use crate::config::SavedServer;
use crate::session::{self, Session, describe_error};
use crate::task::{self, Deadline, Receiver, Sender, Task};
use crate::theme;
use crate::theme::text;
use crate::widgets::avatar_row::AvatarRow;
use crate::widgets::mascot::{self, Watch};
use crate::widgets::{self, button, field, surface};

/// How long to keep retrying while the server reports that it is starting up.
const STARTUP_WAIT: Duration = Duration::from_secs(120);
const QUICK_CONNECT_POLL: Duration = Duration::from_secs(3);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// The form's width; the window centres it.
const FORM_WIDTH: f32 = 400.0;
/// The mascot's square over the form.
const MASCOT_SIZE: f32 = 160.0;
/// The room between the mascot's square and the panel: its tentacles end a little above the
/// square's bottom, so it hangs clear of the panel.
const MASCOT_GAP: f32 = 16.0;
/// The glow behind the mascot: how far it reaches, and how far below the mascot's middle it's
/// centred, so the light pools around its body and onto the panel's top.
const GLOW_RADIUS: f32 = 360.0;
const GLOW_DROP: f32 = 30.0;

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

impl Field {
    /// The field's own widget id, so the keyboard stays in it while the rows above change (an
    /// error, the starting-up status, the saved accounts): egui's automatic ids shift with the
    /// widgets made before them, and egui drops the focus of an id that no longer draws, which
    /// left the user's next keystroke with nothing. `main`'s inputs were entities and kept their
    /// identity the same way.
    fn id(self) -> egui::Id {
        egui::Id::unique(match self {
            Self::Server => "login_server",
            Self::Username => "login_username",
            Self::Password => "login_password",
        })
    }
}

/// What the form is waiting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Busy {
    Connecting,
    SigningIn,
    QuickConnect,
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
    /// The widget the keyboard was in last frame, so a new one is scrolled into view.
    last_focus: Option<egui::Id>,
    /// What the mascot does this frame, filled in as the fields are drawn.
    watch: Watch,

    /// Set once the server address has been verified.
    server: Option<(Client, PublicSystemInfo)>,
    quick_connect_code: Option<String>,
    /// The request in flight, so its button shows it loading while the rest of the form waits.
    busy: Option<Busy>,
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
            last_focus: None,
            watch: Watch::default(),
            server: None,
            quick_connect_code: None,
            busy: None,
            status: None,
            error,
            task: None,
            progress: None,
            retry: Deadline::default(),
            event: None,
        };
        // As on `main`, which focused the input as the form was made: asking by id focuses the
        // field the first time it draws, so a key typed at launch can't land in a form that
        // hasn't taken the keyboard yet.
        if let Some(field) = login.focus {
            ctx.memory_mut(|memory| memory.request_focus(field.id()));
        }
        if has_url {
            login.connect(ctx);
        }
        login
    }

    /// What the mascot did on the last frame.
    #[cfg(test)]
    pub fn watch(&self) -> Watch {
        self.watch
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
        // The sign-in screen is the first in the cozy look: the backdrop under everything, and
        // the look's text, fields and buttons for what's drawn on it.
        widgets::backdrop::fill(ui.painter(), ui.max_rect(), theme::tokens_of(ui));
        theme::cozy_style(ui);
        // `main`'s form sat in the middle of the window (`items_center` and `justify_center`),
        // not at the top of a page. The form's height is only known after it has been laid
        // out, so the spacer above it comes from the last frame; a step that changes height
        // asks for the frame that recentres it. A form taller than the window starts at the
        // top and the scroll area reaches the rest, instead of being cut off.
        let viewport = ui.available_height();
        // The vertical area fills the window's width, so the form's fixed-width column is
        // centred in it, and only takes the height its contents need.
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let id = egui::Id::unique("sign-in-form-height");
                let form_height: f32 = ui.data(|data| data.get_temp(id)).unwrap_or(0.);
                ui.add_space(((viewport - form_height) * 0.5).max(0.));
                let form = ui.allocate_ui_with_layout(
                    egui::vec2(FORM_WIDTH, 0.),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.form(ui, ctx),
                );
                let measured = form.response.rect.height();
                if (measured - form_height).abs() > 0.5 {
                    ui.ctx().request_repaint();
                }
                ui.data_mut(|data| data.insert_temp(id, measured));
                // Tab has to reach a field the window's edge cuts off: when the keyboard moves
                // to something in the form, scroll that widget into view. Nothing else (a toast
                // action, say) moves the form.
                let focused = ctx.memory(|memory| memory.focused());
                if focused != self.last_focus
                    && let Some(response) = focused.and_then(|id| ctx.read_response(id))
                    && form.response.rect.contains_rect(response.rect)
                {
                    ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                }
                self.last_focus = focused;
            });
        });
    }

    fn form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // The mascot's place comes first, but it's painted last: the fields tell it where to
        // look. It floats over the panel, with a gap and no shadow, in the glow behind it.
        let top = ui
            .allocate_space(egui::vec2(ui.available_width(), MASCOT_SIZE + MASCOT_GAP))
            .1;
        let mascot = egui::Rect::from_min_size(
            egui::pos2(top.center().x - MASCOT_SIZE / 2., top.top()),
            egui::Vec2::splat(MASCOT_SIZE),
        );
        let tokens = theme::tokens_of(ui);
        widgets::backdrop::glow(
            ui.painter(),
            mascot.center() + egui::vec2(0., GLOW_DROP),
            GLOW_RADIUS,
            tokens,
        );
        self.watch = Watch {
            puzzled: self.error.is_some(),
            ..Watch::default()
        };
        surface::frame(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.panel(ui, ctx);
        });
        ui.add_space(surface::GAP_GROUP);
        ui.vertical_centered(|ui| {
            // An aside to the step: the way out of a Quick Connect wait, the way back to another
            // server, or what this one needs.
            if self.quick_connect_code.is_some() {
                if button::subtle(ui, "Cancel").clicked() {
                    self.cancel();
                }
            } else if self.server.is_some() {
                if button::subtle(ui, "Use another server").clicked() {
                    self.change_server();
                }
            } else {
                muted(ui, &format!("Works with Jellyfin {MIN_SUPPORTED_VERSION} or newer"));
            }
            if self.can_cancel {
                ui.add_space(surface::GAP_ITEM);
                if ui.button((ICON_ARROW_BACK, "Back to your account")).clicked() {
                    self.event = Some(LoginEvent::Cancel);
                }
            }
        });
        mascot::watching(ui, mascot, self.watch, false);
    }

    /// The panel under the mascot: the step's title, what went wrong, and the step's fields.
    fn panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (title, subtitle) = if self.quick_connect_code.is_some() {
            (
                "Approve this sign-in".to_owned(),
                "Enter this code in Quick Connect, in a Jellyfin app you're signed in to.".to_owned(),
            )
        } else if let Some((client, info)) = &self.server {
            let host = client.base_url().host_str().unwrap_or_default();
            (
                format!("Sign in to {}", info.server_name.as_deref().unwrap_or("Jellyfin")),
                format!("Jellyfin {} at {host}", info.version.as_deref().unwrap_or("?")),
            )
        } else if !self.accounts.is_empty() {
            (
                "Welcome back".to_owned(),
                "Pick an account to carry on watching.".to_owned(),
            )
        } else {
            (
                "Welcome to Tabbyfin".to_owned(),
                "Let's find your Jellyfin server.".to_owned(),
            )
        };
        surface::heading(ui, &title, &subtitle);
        ui.add_space(surface::GAP_HEADING);
        if let Some(error) = self.error.clone() {
            surface::notice(ui, surface::Kind::Danger, &error);
            ui.add_space(surface::GAP_FIELD);
        }
        if let Some(status) = self.status.clone() {
            surface::waiting(ui, &status);
            ui.add_space(surface::GAP_FIELD);
        }
        if let Some(code) = self.quick_connect_code.clone() {
            self.quick_connect_step(ui, &code);
        } else if self.server.is_some() {
            self.credentials_step(ui, ctx);
        } else {
            self.server_step(ui, ctx);
        }
    }

    /// Points the mascot at `field`'s caret while it has the keyboard.
    fn watch_field(&mut self, field: &egui::text_edit::TextEditOutput) {
        if let Some(caret) = caret(field) {
            self.watch.look_at = Some(caret);
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
        self.busy = Some(Busy::Connecting);
        let progress = self.begin_request(ctx);
        self.task = Some(Task::spawn(ctx, async move {
            Outcome::Connected(Box::new(probe(candidates, info, progress).await))
        }));
    }

    fn sign_in(&mut self, ctx: &egui::Context) {
        let Some((client, info)) = self.server.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        let username = self.username.trim().to_string();
        let password = self.password.clone();
        if username.is_empty() {
            self.error = Some("Enter your username.".into());
            return;
        }

        self.busy = Some(Busy::SigningIn);
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
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::QuickConnect);
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
        self.busy = None;
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
                    } else {
                        // The address is what has to change (or be tried again), so the keyboard
                        // goes back to it, as a failed sign-in puts it back in the password.
                        // Connecting with Enter gave the field up, as a text field does.
                        self.focus = Some(Field::Server);
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
        self.busy = None;
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
        for account in self.accounts.clone() {
            let label = account.label();
            let row = AvatarRow {
                name: &account.username,
                title: &account.username,
                subtitle: account.server_name.as_deref().unwrap_or(&account.url),
                label: &label,
                remove: Some("Remove this account"),
            }
            .show(ui, self.busy.is_none());
            if row.row.clicked() {
                self.event = Some(LoginEvent::UseAccount(account.clone()));
            }
            if row.remove.is_some_and(|remove| remove.clicked()) {
                self.event = Some(LoginEvent::ForgetAccount(account.clone()));
            }
            ui.add_space(surface::GAP_ITEM);
        }
        if !self.accounts.is_empty() {
            ui.add_space(surface::GAP_GROUP);
        }

        field::label(
            ui,
            if self.accounts.is_empty() {
                "Server address"
            } else {
                "Or connect to another server"
            },
        );
        let address = field::single_line(
            ui,
            Field::Server.id(),
            &mut self.url,
            "jellyfin.example.com or 192.168.1.10:8096",
        )
        .interactive(self.busy.is_none())
        .show(ui);
        self.watch_field(&address);
        let address = address.response.response;
        if self.focus == Some(Field::Server) {
            address.request_focus();
            self.focus = None;
        }
        let entered = address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.add_space(surface::GAP_GROUP);
        let connect = button::brand(
            ui,
            "Connect",
            ui.available_width(),
            self.button(Busy::Connecting),
            button::Focus::Tab,
        );
        if connect.clicked() || entered {
            self.on_connect(ctx);
        }
    }

    /// The state of the button that starts `action`: loading while it runs, and disabled while
    /// anything else does.
    fn button(&self, action: Busy) -> button::State {
        match self.busy {
            None => button::State::Ready,
            Some(busy) if busy == action => button::State::Loading,
            Some(_) => button::State::Disabled,
        }
    }

    fn credentials_step(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        field::label(ui, "Username");
        let username = field::single_line(ui, Field::Username.id(), &mut self.username, "")
            .interactive(self.busy.is_none())
            .show(ui);
        self.watch_field(&username);
        let username = username.response.response;
        if self.focus == Some(Field::Username) {
            username.request_focus();
            self.focus = None;
        }
        let username_entered = username.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.add_space(surface::GAP_FIELD);

        field::label(ui, "Password");
        let password = field::with_trailing(ui, Field::Password.id(), &mut self.password, "")
            .password(!self.show_password)
            .interactive(self.busy.is_none())
            .show(ui);
        // A hidden password is none of its business: it shuts its eyes, and opens them to read
        // along once the password is shown.
        if self.show_password {
            self.watch_field(&password);
        } else if has_keyboard(&password) {
            self.watch.eyes_shut = true;
        }
        let password = password.response.response;
        if self.focus == Some(Field::Password) {
            password.request_focus();
            self.focus = None;
        }
        let password_entered = password.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let (icon, tooltip) = if self.show_password {
            (ICON_VISIBILITY_OFF, "Hide password")
        } else {
            (ICON_VISIBILITY, "Show password")
        };
        if field::trailing(ui, &password, icon, self.busy.is_none())
            .on_hover_text(tooltip)
            .clicked()
        {
            self.show_password = !self.show_password;
        }
        if username_entered {
            self.focus = Some(Field::Password);
            ui.ctx().request_repaint();
        }
        ui.add_space(surface::GAP_GROUP);

        let width = ui.available_width();
        let sign_in = button::brand(ui, "Sign in", width, self.button(Busy::SigningIn), button::Focus::Tab);
        if sign_in.clicked() || password_entered {
            self.sign_in(ctx);
        }
        ui.add_space(surface::GAP_ITEM);
        let quick_connect = self.button(Busy::QuickConnect);
        if button::quiet(ui, "Use Quick Connect", width, quick_connect, button::Focus::Tab).clicked() {
            self.start_quick_connect(ctx);
        }
    }

    /// The code to approve and the wait for it. Cancel is the aside under the panel (`form`):
    /// with nothing to weigh it against, a full-width button outweighed the code.
    fn quick_connect_step(&mut self, ui: &mut egui::Ui, code: &str) {
        // The code is what the user has to read off, so the mascot looks at it too.
        let shown = widgets::code::show(ui, code);
        self.watch.look_at = Some(shown.rect.center());
        ui.add_space(surface::GAP_GROUP);
        surface::waiting(ui, "Waiting for approval…");
    }
}

/// A line of secondary text: `text_sm` in the muted ink.
fn muted(ui: &mut egui::Ui, line: &str) {
    ui.label(
        egui::RichText::new(line)
            .font(text::font(ui, "sm"))
            .color(theme::tokens_of(ui).ink_muted),
    );
}

/// Whether `field` is the widget the keyboard goes to. `Response::has_focus` also asks for the
/// window to have the OS's focus; the mascot keeps watching the field while it's in the
/// background, as the field keeps its caret.
fn has_keyboard(field: &egui::text_edit::TextEditOutput) -> bool {
    field.response.ctx.memory(|memory| memory.focused()) == Some(field.response.id)
}

/// Where `field`'s caret is on screen, while the field has the keyboard.
fn caret(field: &egui::text_edit::TextEditOutput) -> Option<egui::Pos2> {
    if !has_keyboard(field) {
        return None;
    }
    let cursor = field.cursor_range?.primary;
    Some(field.galley.pos_from_cursor(cursor).center() + field.galley_pos.to_vec2())
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
