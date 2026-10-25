//! The app: restores the saved session on launch and switches between sign-in and the signed-in
//! shell. Owns what lives longer than a session (toasts, the keys taken from egui) and, while
//! signed in, the session's [`Services`].
//!
//! S1 brings the rest of `main`'s `app_view.rs`: the sign-in screen, waiting for an unreachable
//! server, adding and switching accounts, signing out.

use jellyfin::Client;

use crate::config::{Config, SavedServer};
use crate::keys::{self, Press};
use crate::services::{AppEvent, Services};
use crate::session::{Session, client_info, describe_error};
use crate::task::Task;
use crate::toasts::{ToastAction, Toasts};
use crate::views::PageCx;
use crate::views::shell::Shell;
use crate::{credentials, runtime, theme, window};

/// Restoring fails with the request's error, or none when there's no stored token.
type Restored = Result<Session, Option<jellyfin::Error>>;

enum Screen {
    /// Signing back in with the saved account's token.
    Restoring(Task<Restored>),
    /// No session. S1's sign-in form goes here; until then, why there's none.
    SignIn {
        message: Option<String>,
    },
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
    screen: Screen,
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
            Some(saved) => Screen::Restoring(restore(ctx, saved.clone(), config.device_id.clone())),
            None => Screen::SignIn { message: None },
        };
        Self {
            placement: window::Placement::new(&config),
            device_id: config.device_id,
            saved: config.server,
            screen,
            toasts: Toasts::default(),
            keys: Vec::new(),
            #[cfg(debug_assertions)]
            debug_keys: crate::debug::KeyScript::from_env(),
        }
    }

    /// The restore finished: into the shell, or to sign-in with what went wrong.
    fn restored(&mut self, ctx: &egui::Context, frame: &eframe::Frame, result: Restored) -> Screen {
        match result {
            Ok(session) => {
                tracing::info!(user = %session.user.name, "restored session");
                self.remember(&session);
                signed_in(ctx, frame, session)
            }
            Err(err) => {
                if let Some(err) = &err {
                    tracing::warn!("couldn't restore session: {err}");
                }
                if matches!(err, Some(jellyfin::Error::Unauthorized)) {
                    self.forget_token();
                }
                Screen::SignIn {
                    message: err.map(|err| describe_error(&err)),
                }
            }
        }
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

    fn session_expired(&mut self) -> Screen {
        tracing::warn!("session expired");
        self.forget_token();
        Screen::SignIn {
            message: Some(describe_error(&jellyfin::Error::Unauthorized)),
        }
    }

    /// The scopes whose keys the app takes from egui this frame (`keys.rs`).
    fn key_scopes(&self) -> Vec<keys::Scope> {
        match &self.screen {
            Screen::SignedIn(signed_in) => signed_in.shell.scopes(),
            Screen::Restoring(_) | Screen::SignIn { .. } => Vec::new(),
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

fn signed_in(ctx: &egui::Context, frame: &eframe::Frame, session: Session) -> Screen {
    let mut services = Services::new(session);
    let mut nav = Vec::new();
    let shell = Shell::new(&mut PageCx {
        ctx,
        services: &mut services,
        nav: &mut nav,
        render_state: frame.wgpu_render_state(),
    });
    Screen::SignedIn(Box::new(SignedIn { services, shell }))
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

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.placement.tick(ctx);
        if let Screen::SignedIn(signed_in) = &mut self.screen {
            signed_in.services.tick(ctx);
        }
        #[cfg(debug_assertions)]
        if let Some(due) = self.debug_keys.as_ref().and_then(|script| script.next_due()) {
            ctx.request_repaint_after(due.saturating_duration_since(std::time::Instant::now()));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let keys = std::mem::take(&mut self.keys);
        let mut next = None;
        match &mut self.screen {
            Screen::Restoring(task) => match task.poll() {
                Some(result) => next = Some(result),
                None => {
                    egui::CentralPanel::default().show(ui, |ui| ui.centered_and_justified(|ui| ui.spinner()));
                }
            },
            Screen::SignIn { message } => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        if let Some(message) = message {
                            ui.label(message.as_str());
                        }
                        ui.label("Sign-in arrives with S1.");
                    });
                });
            }
            Screen::SignedIn(signed_in) => {
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
        }
        if let Some(result) = next {
            self.screen = self.restored(&ctx, frame, result);
            ctx.request_repaint();
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

impl App {
    /// What signed-in code asked the app to do this frame.
    fn handle_events(&mut self, ctx: &egui::Context) {
        let Screen::SignedIn(signed_in) = &mut self.screen else {
            return;
        };
        for event in std::mem::take(&mut signed_in.services.events) {
            match event {
                AppEvent::Toast(toast) => self.toasts.push(toast),
                AppEvent::SessionExpired => {
                    self.screen = self.session_expired();
                    ctx.request_repaint();
                    return;
                }
            }
        }
    }
}
