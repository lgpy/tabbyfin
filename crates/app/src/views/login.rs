//! Sign-in flow: a saved account, or a server address, then username/password or Quick Connect.

use std::time::{Duration, Instant};

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::PublicSystemInfo;
use jellyfin::{Client, ClientInfo, Error, MIN_SUPPORTED_VERSION};

use crate::config::SavedServer;
use crate::runtime;
use crate::session::{self, Session, describe_error};

/// How long to keep retrying while the server reports that it is starting up.
const STARTUP_WAIT: Duration = Duration::from_secs(120);
const QUICK_CONNECT_POLL: Duration = Duration::from_secs(3);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);

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

pub struct LoginView {
    device_id: String,
    /// Saved accounts to continue as, most recently used first.
    accounts: Vec<SavedServer>,
    /// Another account is signed in and waiting (adding an account).
    can_cancel: bool,
    resume_saved_session: bool,
    server_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    /// Set once the server address has been verified.
    server: Option<(Client, PublicSystemInfo)>,
    quick_connect_code: Option<SharedString>,
    busy: bool,
    status: Option<SharedString>,
    error: Option<SharedString>,
    /// The in-flight operation. Replacing or clearing it cancels the previous one.
    task: Option<Task<()>>,
    /// Reconnect attempt while waiting for an unreachable server (`resume_saved_session`).
    retry: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoginEvent> for LoginView {}

impl LoginView {
    pub fn new(
        device_id: String,
        prefill: Option<Prefill>,
        accounts: Vec<SavedServer>,
        can_cancel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (url, username, error, resume_saved_session) = match prefill {
            Some(p) => (p.url, p.username, p.error, p.resume_saved_session),
            None => (String::new(), String::new(), None, false),
        };
        let has_url = !url.is_empty();

        let server_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("jellyfin.example.com or 192.168.1.10:8096")
                .default_value(url)
        });
        let username_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Username")
                .default_value(username)
        });
        let password_input = cx.new(|cx| InputState::new(window, cx).placeholder("Password").masked(true));

        let _subscriptions = vec![
            cx.subscribe_in(&server_input, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.on_connect(window, cx);
                }
            }),
            cx.subscribe_in(&username_input, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.password_input.update(cx, |input, cx| input.focus(window, cx));
                }
            }),
            cx.subscribe_in(&password_input, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.sign_in(window, cx);
                }
            }),
        ];

        let mut this = Self {
            device_id,
            accounts,
            can_cancel,
            resume_saved_session,
            server_input,
            username_input,
            password_input,
            server: None,
            quick_connect_code: None,
            busy: false,
            status: None,
            error: error.map(Into::into),
            task: None,
            retry: None,
            _subscriptions,
        };
        if has_url {
            this.connect(window, cx);
        } else {
            this.server_input.update(cx, |input, cx| input.focus(window, cx));
        }
        this
    }

    fn on_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.connect(window, cx);
    }

    /// Verifies the server address and moves on to the credentials step.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.server_input.read(cx).value().trim().to_string();
        if input.is_empty() {
            self.error = Some("Enter your server's address.".into());
            cx.notify();
            return;
        }
        // Without a scheme, prefer HTTPS and fall back to plain HTTP.
        let candidates = if input.contains("://") {
            vec![input]
        } else {
            vec![format!("https://{input}"), format!("http://{input}")]
        };
        let info = session::client_info(&self.device_id);

        self.busy = true;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = probe(candidates, info, &this, cx).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.status = None;
                match result {
                    Ok(_) if this.resume_saved_session => {
                        this.resume_saved_session = false;
                        cx.emit(LoginEvent::ServerReachable);
                    }
                    Ok(server) => {
                        // Keep any error passed in via `Prefill` (e.g. "session expired");
                        // user-triggered connects clear it in `on_connect`.
                        this.server = Some(server);
                        let focus = if this.username_input.read(cx).value().is_empty() {
                            &this.username_input
                        } else {
                            &this.password_input
                        };
                        focus.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(message) => {
                        this.error = Some(message.into());
                        // Waiting to restore a saved session: keep trying until the server is back.
                        if this.resume_saved_session {
                            this.status =
                                Some("You'll be signed back in automatically as soon as the server answers.".into());
                            this.retry = Some(cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor().timer(RECONNECT_INTERVAL).await;
                                this.update_in(cx, |this, window, cx| this.connect(window, cx)).ok();
                            }));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((client, info)) = self.server.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        let username = self.username_input.read(cx).value().trim().to_string();
        let password = self.password_input.read(cx).value().to_string();
        if username.is_empty() {
            self.error = Some("Enter your username.".into());
            cx.notify();
            return;
        }

        self.busy = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let c = client.clone();
            let result = runtime::spawn(async move { c.authenticate_by_name(&username, &password).await }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(auth) => cx.emit(LoginEvent::LoggedIn(Box::new(Session {
                        client: client.with_token(auth.access_token.as_str()),
                        user: auth.user,
                        server: info,
                    }))),
                    Err(Error::Unauthorized) => this.error = Some("Wrong username or password.".into()),
                    Err(err) => this.error = Some(describe_error(&err).into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn start_quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((client, info)) = self.server.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let fail = |this: &WeakEntity<Self>, cx: &mut AsyncWindowContext, message: String| {
                this.update(cx, |this, cx| {
                    this.busy = false;
                    this.quick_connect_code = None;
                    this.error = Some(message.into());
                    cx.notify();
                })
                .ok();
            };

            let c = client.clone();
            let started = runtime::spawn(async move {
                if !c.quick_connect_enabled().await? {
                    return Ok(None);
                }
                c.quick_connect_initiate().await.map(Some)
            })
            .await;
            let qc = match started {
                Ok(Some(qc)) => qc,
                Ok(None) => return fail(&this, cx, "Quick Connect is turned off on this server.".into()),
                Err(err) => return fail(&this, cx, describe_error(&err)),
            };
            this.update(cx, |this, cx| {
                this.busy = false;
                this.quick_connect_code = Some(qc.code.clone().into());
                cx.notify();
            })
            .ok();

            loop {
                cx.background_executor().timer(QUICK_CONNECT_POLL).await;
                let (c, secret) = (client.clone(), qc.secret.clone());
                match runtime::spawn(async move { c.quick_connect_state(&secret).await }).await {
                    Ok(state) if state.authenticated => break,
                    Ok(_) => {}
                    Err(Error::NotFound) => {
                        return fail(&this, cx, "The Quick Connect code expired. Try again.".into());
                    }
                    Err(err) => return fail(&this, cx, describe_error(&err)),
                }
            }

            let (c, secret) = (client.clone(), qc.secret.clone());
            match runtime::spawn(async move { c.authenticate_with_quick_connect(&secret).await }).await {
                Ok(auth) => {
                    this.update(cx, |this, cx| {
                        this.quick_connect_code = None;
                        cx.emit(LoginEvent::LoggedIn(Box::new(Session {
                            client: client.with_token(auth.access_token.as_str()),
                            user: auth.user,
                            server: info,
                        })));
                        cx.notify();
                    })
                    .ok();
                }
                Err(err) => fail(&this, cx, describe_error(&err)),
            }
        }));
    }

    pub fn set_accounts(&mut self, accounts: Vec<SavedServer>, cx: &mut Context<Self>) {
        self.accounts = accounts;
        cx.notify();
    }

    /// The saved accounts, each to continue as or remove.
    fn render_accounts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.accounts.is_empty() {
            return None;
        }
        let rows = self.accounts.iter().enumerate().map(|(i, account)| {
            let (use_it, forget) = (account.clone(), account.clone());
            h_flex()
                .gap_1()
                .child(
                    Button::new(("use-account", i))
                        .outline()
                        .flex_1()
                        .justify_start()
                        .label(account.label())
                        .disabled(self.busy)
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(LoginEvent::UseAccount(use_it.clone())))),
                )
                .child(
                    Button::new(("forget-account", i))
                        .ghost()
                        .icon(IconName::Close)
                        .tooltip("Remove this account")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(LoginEvent::ForgetAccount(forget.clone())))),
                )
        });
        Some(
            v_flex()
                .gap_2()
                .child(field_label("Saved accounts", cx))
                .children(rows)
                .child(div().pt_2().child(field_label("Or sign in to another", cx)))
                .into_any_element(),
        )
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.busy = false;
        self.status = None;
        self.quick_connect_code = None;
        cx.notify();
    }

    fn change_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx);
        self.retry = None;
        self.resume_saved_session = false;
        self.server = None;
        self.error = None;
        self.server_input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn render_server_step(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .children(self.render_accounts(cx))
            .child(field_label("Server address", cx))
            .child(Input::new(&self.server_input).disabled(self.busy))
            .child(
                Button::new("connect")
                    .primary()
                    .label("Connect")
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.on_connect(window, cx))),
            )
            .into_any_element()
    }

    fn render_credentials_step(&self, info: &PublicSystemInfo, cx: &mut Context<Self>) -> AnyElement {
        let server = format!(
            "{} · Jellyfin {}",
            info.server_name.as_deref().unwrap_or("Jellyfin"),
            info.version.as_deref().unwrap_or("?")
        );
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(server))
                    .child(
                        Button::new("change-server")
                            .ghost()
                            .label("Change")
                            .on_click(cx.listener(|this, _, window, cx| this.change_server(window, cx))),
                    ),
            )
            .child(field_label("Username", cx))
            .child(Input::new(&self.username_input).disabled(self.busy))
            .child(field_label("Password", cx))
            .child(Input::new(&self.password_input).mask_toggle().disabled(self.busy))
            .child(
                Button::new("sign-in")
                    .primary()
                    .label("Sign in")
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.sign_in(window, cx))),
            )
            .child(
                Button::new("quick-connect")
                    .outline()
                    .label("Use Quick Connect")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.start_quick_connect(window, cx))),
            )
            .into_any_element()
    }

    fn render_quick_connect(&self, code: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Enter this code in another Jellyfin app where you're signed in, under Quick Connect:"),
            )
            .child(div().text_3xl().font_weight(FontWeight::BOLD).child(code.clone()))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(Spinner::new())
                    .child("Waiting for approval"),
            )
            .child(
                Button::new("cancel-quick-connect")
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .into_any_element()
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = if let Some(code) = &self.quick_connect_code {
            self.render_quick_connect(code, cx)
        } else if let Some((_, info)) = &self.server {
            self.render_credentials_step(info, cx)
        } else {
            self.render_server_step(cx)
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w(px(380.))
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child("jelly-gpui"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Sign in to your Jellyfin server"),
                            ),
                    )
                    .when_some(self.error.clone(), |el, error| {
                        el.child(Alert::error("login-error", error))
                    })
                    .when(self.can_cancel, |el| {
                        el.child(
                            h_flex().child(
                                Button::new("cancel-add-account")
                                    .ghost()
                                    .icon(IconName::ArrowLeft)
                                    .label("Back to your account")
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(LoginEvent::Cancel))),
                            ),
                        )
                    })
                    .when_some(self.status.clone(), |el, status| {
                        el.child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(Spinner::new())
                                .child(status),
                        )
                    })
                    .child(step),
            )
    }
}

fn field_label(text: &'static str, cx: &App) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text)
}

/// Tries each candidate URL in turn and returns the first supported Jellyfin server.
/// Waits (and reports status) while a server is still starting up.
async fn probe(
    candidates: Vec<String>,
    info: ClientInfo,
    this: &WeakEntity<LoginView>,
    cx: &mut AsyncWindowContext,
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
            let c = client.clone();
            match runtime::spawn(async move { c.public_info().await }).await {
                Ok(server) => {
                    let Some(version) = server.server_version() else {
                        return Err("The server didn't report its version.".into());
                    };
                    if !version.is_supported() {
                        return Err(format!(
                            "This server runs Jellyfin {version}. jelly-gpui needs {MIN_SUPPORTED_VERSION} or newer."
                        ));
                    }
                    tracing::info!(url = %client.base_url(), %version, "connected to server");
                    return Ok((client, server));
                }
                Err(Error::ServerStarting) if started.elapsed() < STARTUP_WAIT => {
                    this.update(cx, |this, cx| {
                        this.status = Some("The server is starting up. Waiting…".into());
                        cx.notify();
                    })
                    .ok();
                    cx.background_executor().timer(Duration::from_secs(2)).await;
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
