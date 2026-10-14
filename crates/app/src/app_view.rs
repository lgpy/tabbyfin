//! Root view: restores the saved session on launch and switches between sign-in and the app.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;
use jellyfin::Client;

use crate::config::{Config, SavedServer};
use crate::session::{Session, client_info, describe_error};
use crate::views::shell::{ShellEvent, ShellView};
use crate::views::login::{LoginEvent, LoginView, Prefill};
use crate::{credentials, runtime};

enum Screen {
    Restoring,
    Login(Entity<LoginView>),
    Home(Entity<ShellView>),
}

pub struct AppView {
    config: Config,
    screen: Screen,
    _screen_subscription: Option<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let saved = config.server.clone();
        let mut this = Self {
            config,
            screen: Screen::Restoring,
            _screen_subscription: None,
        };
        match saved {
            Some(saved) => this.restore(saved, window, cx),
            None => this.show_login(None, window, cx),
        }
        this
    }

    /// Signs back in with the stored token. Falls back to the sign-in screen, pre-filled with
    /// the saved server and username, if the token is missing, revoked or the server is down.
    fn restore(&mut self, saved: SavedServer, window: &mut Window, cx: &mut Context<Self>) {
        let device_id = self.config.device_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let account = saved.credential_account();
            let token = cx
                .background_executor()
                .spawn(async move { credentials::load(&account) })
                .await;

            let result = match token {
                None => Err(None),
                Some(token) => match Client::new(&saved.url, client_info(&device_id)) {
                    Err(err) => Err(Some(err)),
                    Ok(client) => {
                        let client = client.with_token(token);
                        runtime::spawn(async move {
                            let server = client.public_info().await?;
                            let user = client.current_user().await?;
                            Ok(Session { client, user, server })
                        })
                        .await
                        .map_err(Some)
                    }
                },
            };

            this.update_in(cx, |this, window, cx| match result {
                Ok(session) => {
                    tracing::info!(user = %session.user.name, "restored session");
                    this.show_home(session, window, cx);
                }
                Err(err) => {
                    if let Some(err) = &err {
                        tracing::warn!("couldn't restore session: {err}");
                    }
                    if matches!(err, Some(jellyfin::Error::Unauthorized)) {
                        let account = saved.credential_account();
                        cx.background_executor()
                            .spawn(async move { credentials::delete(&account) })
                            .detach();
                    }
                    let prefill = Prefill {
                        url: saved.url.clone(),
                        username: saved.username.clone(),
                        error: err.map(|e| describe_error(&e)),
                    };
                    this.show_login(Some(prefill), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn show_login(&mut self, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) {
        let device_id = self.config.device_id.clone();
        let login = cx.new(|cx| LoginView::new(device_id, prefill, window, cx));
        self._screen_subscription = Some(cx.subscribe_in(&login, window, |this, _, ev, window, cx| match ev {
            LoginEvent::LoggedIn(session) => this.on_logged_in(session.clone(), window, cx),
        }));
        self.screen = Screen::Login(login);
        cx.notify();
    }

    fn show_home(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let home = cx.new(|cx| ShellView::new(session.clone(), window, cx));
        self._screen_subscription = Some(cx.subscribe_in(&home, window, move |this, _, ev, window, cx| match ev {
            ShellEvent::SignOut => this.sign_out(&session, window, cx),
        }));
        self.screen = Screen::Home(home);
        cx.notify();
    }

    fn on_logged_in(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!(user = %session.user.name, "signed in");
        let saved = SavedServer {
            url: session.client.base_url().to_string(),
            server_id: session.server.id.clone().unwrap_or_default(),
            user_id: session.user.id.clone(),
            username: session.user.name.clone(),
        };
        if let Some(token) = session.client.token().map(str::to_owned) {
            let account = saved.credential_account();
            cx.background_executor()
                .spawn(async move {
                    if let Err(err) = credentials::store(&account, &token) {
                        tracing::error!("failed to store access token: {err:#}");
                    }
                })
                .detach();
        }
        self.config.server = Some(saved);
        if let Err(err) = self.config.save() {
            tracing::error!("failed to save config: {err:#}");
        }
        self.show_home(session, window, cx);
    }

    fn sign_out(&mut self, session: &Session, window: &mut Window, cx: &mut Context<Self>) {
        crate::playback::PlaybackManager::global(cx).update(cx, |playback, _| playback.stop());
        // Revoke the token server-side; nothing to do if that fails, we forget it either way.
        let client = session.client.clone();
        runtime::detach(async move {
            if let Err(err) = client.logout().await {
                tracing::warn!("logout request failed: {err}");
            }
        });
        let prefill = self.config.server.as_ref().map(|saved| {
            let account = saved.credential_account();
            cx.background_executor()
                .spawn(async move { credentials::delete(&account) })
                .detach();
            Prefill {
                url: saved.url.clone(),
                username: saved.username.clone(),
                error: None,
            }
        });
        self.show_login(prefill, window, cx);
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.screen {
            Screen::Restoring => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .bg(cx.theme().background)
                .child(Spinner::new())
                .into_any_element(),
            Screen::Login(view) => view.clone().into_any_element(),
            Screen::Home(view) => view.clone().into_any_element(),
        }
    }
}
