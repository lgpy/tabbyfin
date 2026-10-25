//! The app: restores the saved session on launch and switches between sign-in and the signed-in
//! shell. Owns what lives longer than a session (toasts, the keys taken from egui) and, while
//! signed in, the session's [`Services`].
//!
//! Ported from `main`'s `app_view.rs`: the sign-in screen, waiting for an unreachable server,
//! adding and switching accounts, signing out and an expired session. Adding an account keeps
//! the signed-in screen (and its session, playback included) alive in [`App::parked`], so
//! *Back to your account* returns to it untouched.

use jellyfin::Client;

use crate::config::{Config, SavedServer};
use crate::keys::{self, Press};
use crate::services::{AppEvent, Services};
use crate::session::{Session, client_info, describe_error};
use crate::task::Task;
use crate::toasts::{ToastAction, Toasts};
use crate::views::PageCx;
use crate::views::login::{Login, LoginEvent, Prefill};
use crate::views::shell::Shell;
use crate::{credentials, runtime, theme, window};

/// Restoring fails with the request's error, or none when there's no stored token.
type Restored = Result<Session, Option<jellyfin::Error>>;

enum Screen {
    /// Signing back in with the saved account's token. `allow_reconnect` leaves the token in
    /// place when the server is unreachable, so sign-in can try the saved session again once it
    /// answers.
    Restoring {
        task: Task<Restored>,
        allow_reconnect: bool,
    },
    /// No session: the sign-in form, or the saved accounts to continue as.
    SignIn(Box<Login>),
    SignedIn(Box<SignedIn>),
}

struct SignedIn {
    services: Services,
    shell: Shell,
}

pub struct App {
    device_id: String,
    /// The current account: signed in, being restored, or the one sign-in is for.
    saved: Option<SavedServer>,
    /// `None` only while a screen is swapped for another.
    screen: Option<Screen>,
    /// The signed-in session kept while another account is being added, to go back to. It keeps
    /// running (playback, the websocket) while the sign-in form is up.
    parked: Option<Box<SignedIn>>,
    toasts: Toasts,
    /// Keys `raw_input_hook` took from egui for the app (`keys.rs`), routed in the next `ui`.
    keys: Vec<Press>,
    placement: window::Placement,
    #[cfg(debug_assertions)]
    debug_keys: Option<crate::debug::KeyScript>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, config: Config) -> Self {
        let ctx = &cc.egui_ctx;
        if let Some(render_state) = &cc.wgpu_render_state {
            let adapter = render_state.adapter.get_info();
            let import = egui_wgpu::wgpu::Features::VULKAN_EXTERNAL_MEMORY_FD;
            tracing::info!(
                backend = ?adapter.backend,
                adapter = %adapter.name,
                vulkan_memory_import = render_state.device.features().contains(import),
                "graphics"
            );
        }
        egui_material_icons::initialize(ctx);
        theme::apply(ctx, &config);
        let screen = match &config.server {
            Some(saved) => Screen::Restoring {
                task: restore(ctx, saved.clone(), config.device_id.clone()),
                allow_reconnect: true,
            },
            None => Screen::SignIn(Box::new(Login::new(
                ctx,
                config.device_id.clone(),
                config.accounts.clone(),
                false,
                None,
            ))),
        };
        Self {
            placement: window::Placement::new(&config),
            device_id: config.device_id,
            saved: config.server,
            screen: Some(screen),
            parked: None,
            toasts: Toasts::default(),
            keys: Vec::new(),
            #[cfg(debug_assertions)]
            debug_keys: crate::debug::KeyScript::from_env(),
        }
    }

    /// The restore finished: into the shell, or to sign-in with what went wrong.
    fn finish_restore(&mut self, ctx: &egui::Context, frame: &eframe::Frame, result: Restored, allow_reconnect: bool) {
        match result {
            Ok(session) => {
                tracing::info!(user = %session.user.name, "restored session");
                self.remember(&session);
                self.show_signed_in(ctx, frame, session);
            }
            Err(err) => {
                if let Some(err) = &err {
                    tracing::warn!("couldn't restore session: {err}");
                }
                let unauthorized = matches!(&err, Some(jellyfin::Error::Unauthorized));
                if unauthorized {
                    self.forget_token();
                }
                let unreachable = matches!(&err, Some(jellyfin::Error::Http(_) | jellyfin::Error::ServerStarting));
                let prefill = self.saved.as_ref().map(|saved| Prefill {
                    url: saved.url.clone(),
                    username: saved.username.clone(),
                    error: err.as_ref().map(describe_error),
                    resume_saved_session: allow_reconnect && unreachable,
                });
                self.show_login(ctx, prefill);
            }
        }
    }

    /// Signs back in with the stored token. Falls back to the sign-in screen, pre-filled with
    /// the saved server and username, if the token is missing, revoked or the server is down.
    /// With `allow_reconnect`, an unreachable server leaves the token in place and the sign-in
    /// screen tries the saved session again once the server answers.
    fn restore(&mut self, ctx: &egui::Context, saved: SavedServer, allow_reconnect: bool) {
        self.saved = Some(saved.clone());
        self.screen = Some(Screen::Restoring {
            task: restore(ctx, saved, self.device_id.clone()),
            allow_reconnect,
        });
        ctx.request_repaint();
    }

    /// The sign-in screen, listing the saved accounts. With a parked session (adding an
    /// account), it can be cancelled back to it.
    fn show_login(&mut self, ctx: &egui::Context, prefill: Option<Prefill>) {
        let accounts = Config::load().accounts;
        let can_cancel = self.parked.is_some();
        let login = Login::new(ctx, self.device_id.clone(), accounts, can_cancel, prefill);
        self.screen = Some(Screen::SignIn(Box::new(login)));
        ctx.request_repaint();
    }

    fn show_signed_in(&mut self, ctx: &egui::Context, frame: &eframe::Frame, session: Session) {
        self.screen = Some(Screen::SignedIn(Box::new(signed_in(ctx, frame, session))));
        ctx.request_repaint();
    }

    /// Makes the session's account the current one in the config, first in the list.
    fn remember(&mut self, session: &Session) {
        let saved = SavedServer {
            url: session.client.base_url().to_string(),
            server_id: session.server.id.clone().unwrap_or_default(),
            user_id: session.user.id.clone(),
            username: session.user.name.clone(),
            server_name: session.server.server_name.clone(),
        };
        // Re-read the file so settings changed elsewhere aren't overwritten.
        let mut config = Config::load();
        config.use_account(saved.clone());
        if let Err(err) = config.save() {
            tracing::error!("failed to save config: {err:#}");
        }
        self.saved = Some(saved);
    }

    /// The session in the signed-in screen, if there is one.
    fn active_session(&self) -> Option<Session> {
        match self.screen.as_ref() {
            Some(Screen::SignedIn(signed_in)) => Some(signed_in.services.session.clone()),
            _ => None,
        }
    }

    /// Ends what the current session (and a parked one) is doing on the server, before another
    /// account takes over. Dropping the screen ends the rest of it.
    fn end_session(&mut self) {
        if let Some(Screen::SignedIn(signed_in)) = self.screen.as_mut() {
            signed_in.services.end();
        }
        if let Some(parked) = &mut self.parked {
            parked.services.end();
        }
        self.parked = None;
    }

    /// Signs the current account out for good: revokes its token and drops it from the saved
    /// accounts. The sign-in screen then lists the others.
    fn sign_out(&mut self, ctx: &egui::Context) {
        let Some(session) = self.active_session() else {
            return;
        };
        if let Some(Screen::SignedIn(signed_in)) = self.screen.as_mut() {
            signed_in.services.end();
        }
        // Revoke the token server-side; nothing to do if that fails, we forget it either way.
        let client = session.client.clone();
        runtime::detach(async move {
            if let Err(err) = client.logout().await {
                tracing::warn!("logout request failed: {err}");
            }
        });
        let Some(saved) = self.saved.take() else {
            self.show_login(ctx, None);
            return;
        };
        self.forget_account(&saved);
        // With no other account, offer the same server and user again.
        let prefill = Config::load().accounts.is_empty().then(|| Prefill {
            url: saved.url.clone(),
            username: saved.username.clone(),
            error: None,
            resume_saved_session: false,
        });
        self.show_login(ctx, prefill);
    }

    /// Deletes an account's token and drops it from the saved accounts. A token that's still
    /// stored is revoked on the server too (best effort).
    fn forget_account(&mut self, account: &SavedServer) {
        let key = account.credential_account();
        let mut config = Config::load();
        config.forget_account(&key);
        if let Err(err) = config.save() {
            tracing::error!("failed to save config: {err:#}");
        }
        if self
            .saved
            .as_ref()
            .is_some_and(|saved| saved.credential_account() == key)
        {
            // Nothing is signed in as it any more; a stale `saved` would prefill a dead account.
            self.saved = None;
        }
        let (url, device_id) = (account.url.clone(), self.device_id.clone());
        runtime::detach(async move {
            let token = tokio::task::spawn_blocking(move || {
                let token = credentials::load(&key);
                credentials::delete(&key);
                token
            })
            .await
            .ok()
            .flatten();
            let Some(token) = token else { return };
            if let Ok(client) = Client::new(&url, client_info(&device_id))
                && let Err(err) = client.with_token(token).logout().await
            {
                tracing::debug!("couldn't revoke a forgotten account's token: {err}");
            }
        });
    }

    /// The server rejected the session's token: back to sign-in for the same account.
    fn session_expired(&mut self, ctx: &egui::Context) {
        if !matches!(self.screen, Some(Screen::SignedIn(_))) {
            return;
        }
        tracing::warn!("session expired");
        if let Some(Screen::SignedIn(signed_in)) = self.screen.as_mut() {
            signed_in.services.end();
        }
        self.forget_token();
        let prefill = self.saved.as_ref().map(|saved| Prefill {
            url: saved.url.clone(),
            username: saved.username.clone(),
            error: Some(describe_error(&jellyfin::Error::Unauthorized)),
            resume_saved_session: false,
        });
        self.show_login(ctx, prefill);
    }

    /// Deletes the current account's stored token (off the UI thread: the keychain blocks),
    /// keeping the account in the list for signing in again.
    fn forget_token(&self) {
        if let Some(saved) = &self.saved {
            let account = saved.credential_account();
            runtime::detach(async move {
                let _ = tokio::task::spawn_blocking(move || credentials::delete(&account)).await;
            });
        }
    }

    /// Signs in to another server or user, keeping this session to come back to.
    fn add_account(&mut self, ctx: &egui::Context) {
        match self.screen.take() {
            Some(Screen::SignedIn(signed_in)) => {
                tracing::info!("adding an account");
                self.parked = Some(signed_in);
                self.show_login(ctx, None);
            }
            other => self.screen = other,
        }
    }

    /// Back to the session that was signed in before "Add account".
    fn unpark(&mut self, ctx: &egui::Context) {
        let Some(parked) = self.parked.take() else {
            return;
        };
        tracing::info!(user = %parked.services.session.user.name, "back to the signed-in account");
        let key = parked.services.session.account_key();
        self.saved = Config::load()
            .accounts
            .into_iter()
            .find(|account| account.credential_account() == key);
        self.screen = Some(Screen::SignedIn(parked));
        ctx.request_repaint();
    }

    /// Ends the current session (and a parked one) and signs in to `account` with its saved
    /// token.
    fn switch_account(&mut self, ctx: &egui::Context, account: SavedServer) {
        tracing::info!(account = %account.label(), "switching account");
        self.end_session();
        self.screen = None;
        self.restore(ctx, account, false);
    }

    fn parked_matches(&self, account: &SavedServer) -> bool {
        self.parked
            .as_ref()
            .is_some_and(|parked| parked.services.session.account_key() == account.credential_account())
    }

    fn on_logged_in(&mut self, ctx: &egui::Context, frame: &eframe::Frame, session: Session) {
        tracing::info!(user = %session.user.name, "signed in");
        if self.parked.is_some() {
            self.end_session();
        }
        if let Some(token) = session.client.token().map(str::to_owned) {
            let account = session.account_key();
            runtime::detach(async move {
                match tokio::task::spawn_blocking(move || credentials::store(&account, &token)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::error!("failed to store access token: {err:#}"),
                    Err(err) => tracing::error!("failed to store access token: {err}"),
                }
            });
        }
        self.remember(&session);
        self.show_signed_in(ctx, frame, session);
    }

    fn on_login_event(&mut self, ctx: &egui::Context, frame: &eframe::Frame, event: LoginEvent) {
        match event {
            LoginEvent::LoggedIn(session) => self.on_logged_in(ctx, frame, *session),
            LoginEvent::ServerReachable => {
                if let Some(saved) = self.saved.clone() {
                    self.restore(ctx, saved, false);
                }
            }
            // The account signed in before "Add account" just goes back to that session.
            LoginEvent::UseAccount(account) if self.parked_matches(&account) => self.unpark(ctx),
            LoginEvent::UseAccount(account) => self.switch_account(ctx, account),
            LoginEvent::ForgetAccount(account) => {
                let was_current = self
                    .saved
                    .as_ref()
                    .is_some_and(|saved| saved.credential_account() == account.credential_account());
                self.forget_account(&account);
                if was_current && let Some(Screen::SignIn(login)) = self.screen.as_mut() {
                    login.stop_resuming();
                }
                // Removing the account waiting behind the form leaves nothing to go back to:
                // end its session and drop it, and stop offering *Back to your account*.
                if self.parked_matches(&account) {
                    if let Some(parked) = self.parked.as_mut() {
                        parked.services.end();
                    }
                    self.parked = None;
                    if let Some(Screen::SignIn(login)) = self.screen.as_mut() {
                        login.set_can_cancel(false);
                    }
                }
                let accounts = Config::load().accounts;
                if let Some(Screen::SignIn(login)) = self.screen.as_mut() {
                    login.set_accounts(accounts);
                }
            }
            LoginEvent::Cancel => self.unpark(ctx),
        }
    }

    /// The scopes whose keys the app takes from egui this frame (`keys.rs`).
    fn key_scopes(&self) -> Vec<keys::Scope> {
        match self.screen.as_ref() {
            Some(Screen::SignedIn(signed_in)) => signed_in.shell.scopes(),
            Some(Screen::Restoring { .. } | Screen::SignIn(_)) | None => Vec::new(),
        }
    }

    /// What signed-in code asked the app to do this frame, after the shell drew.
    fn handle_events(&mut self, ctx: &egui::Context) {
        let events = match self.screen.as_mut() {
            Some(Screen::SignedIn(signed_in)) => std::mem::take(&mut signed_in.services.events),
            _ => return,
        };
        for event in events {
            match event {
                AppEvent::Toast(toast) => self.toasts.push(toast),
                // The screen changes; what's left of this frame's events would belong to the
                // session that just ended.
                AppEvent::SessionExpired => {
                    self.session_expired(ctx);
                    break;
                }
                AppEvent::SignOut => {
                    self.sign_out(ctx);
                    break;
                }
                AppEvent::AddAccount => {
                    self.add_account(ctx);
                    break;
                }
                AppEvent::SwitchAccount(account) => {
                    self.switch_account(ctx, account);
                    break;
                }
            }
        }
    }
}

/// Signs back in with the stored token: reads it from the keychain, then checks it with the
/// server, which also says who it belongs to.
fn restore(ctx: &egui::Context, saved: SavedServer, device_id: String) -> Task<Restored> {
    Task::spawn(ctx, async move {
        let account = saved.credential_account();
        let token = tokio::task::spawn_blocking(move || credentials::load(&account))
            .await
            .ok()
            .flatten();
        let Some(token) = token else {
            return Err(None);
        };
        let client = Client::new(&saved.url, client_info(&device_id))
            .map_err(Some)?
            .with_token(token);
        let server = client.public_info().await.map_err(Some)?;
        let user = client.current_user().await.map_err(Some)?;
        Ok(Session { client, user, server })
    })
}

fn signed_in(ctx: &egui::Context, frame: &eframe::Frame, session: Session) -> SignedIn {
    let mut services = Services::new(session);
    let mut nav = Vec::new();
    let shell = Shell::new(&mut PageCx {
        ctx,
        services: &mut services,
        nav: &mut nav,
        render_state: frame.wgpu_render_state(),
    });
    SignedIn { services, shell }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(debug_assertions)]
        if let Some(script) = &mut self.debug_keys {
            script.inject(raw_input);
        }
        let scopes = self.key_scopes();
        self.keys.extend(keys::take(ctx, raw_input, &scopes));
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.placement.tick(ctx);

        // A finished restore swaps the screen for the sign-in form or the shell. This runs
        // here, not in `ui`, because eframe only calls `ui` while the window is visible.
        let restored = match self.screen.as_mut() {
            Some(Screen::Restoring { task, allow_reconnect }) => task.poll().map(|result| (result, *allow_reconnect)),
            _ => None,
        };
        if let Some((result, allow_reconnect)) = restored {
            self.finish_restore(ctx, frame, result, allow_reconnect);
        }

        match self.screen.as_mut() {
            Some(Screen::SignedIn(signed_in)) => signed_in.services.tick(ctx),
            Some(Screen::SignIn(login)) => login.tick(ctx),
            Some(Screen::Restoring { .. }) | None => {}
        }

        // The form's requests finish here too, so an approval or a returning server signs in
        // even with the window hidden.
        if let Some(event) = match self.screen.as_mut() {
            Some(Screen::SignIn(login)) => login.take_event(),
            _ => None,
        } {
            self.on_login_event(ctx, frame, event);
        }

        if let Some(parked) = &mut self.parked {
            parked.services.tick(ctx);
            // The parked screen isn't drawn. Its toasts still go up, its screen-changing
            // events have nothing to act on and are dropped, and everything else waits in the
            // queue for the shell, which handles it after *Back to your account*. Event kinds
            // added later default to kept.
            let mut kept = Vec::new();
            for event in std::mem::take(&mut parked.services.events) {
                // The catch-all is unreachable today on purpose: an event kind added later
                // must stay queued for the shell rather than be dropped here.
                #[allow(unreachable_patterns)]
                match event {
                    AppEvent::Toast(toast) => self.toasts.push(toast),
                    AppEvent::SessionExpired
                    | AppEvent::SignOut
                    | AppEvent::SwitchAccount(_)
                    | AppEvent::AddAccount => {}
                    other => kept.push(other),
                }
            }
            parked.services.events = kept;
        }

        #[cfg(debug_assertions)]
        if let Some(due) = self.debug_keys.as_ref().and_then(|script| script.next_due()) {
            ctx.request_repaint_after(due.saturating_duration_since(std::time::Instant::now()));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let keys = std::mem::take(&mut self.keys);
        match self.screen.as_mut() {
            Some(Screen::SignedIn(signed_in)) => {
                let SignedIn { services, shell } = signed_in.as_mut();
                let mut nav = Vec::new();
                let mut cx = PageCx {
                    ctx: &ctx,
                    services,
                    nav: &mut nav,
                    render_state: frame.wgpu_render_state(),
                };
                shell.show(ui, keys, &mut cx);
            }
            Some(Screen::SignIn(login)) => {
                egui::CentralPanel::default().show(ui, |ui| login.show(ui, &ctx));
            }
            Some(Screen::Restoring { .. }) | None => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| ui.spinner());
                });
            }
        }

        self.handle_events(&ctx);
        if let Some(action) = self.toasts.show(&ctx) {
            match action {
                ToastAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            }
        }
    }

    fn on_exit(&mut self) {
        self.placement.save_now();
    }
}
