//! Root view: restores the saved session on launch and switches between sign-in and the app,
//! and between saved accounts (servers and users).

use std::time::Duration;

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;
use jellyfin::Client;

use crate::config::{Config, SavedServer};
use crate::session::{Session, client_info, describe_error};
use crate::views::login::{LoginEvent, LoginView, Prefill};
use crate::views::shell::{ShellEvent, ShellView};
use crate::{credentials, runtime, theme, window};

/// How long after the last window bounds change its size and position are saved. Dragging
/// produces a stream of changes; only the end of the drag matters.
const SAVE_PLACEMENT_AFTER: Duration = Duration::from_millis(500);

enum Screen {
    Restoring,
    Login(Entity<LoginView>),
    Home(Entity<ShellView>),
}

pub struct AppView {
    device_id: String,
    /// The current account: signed in, being restored, or the one the sign-in form is for.
    saved: Option<SavedServer>,
    screen: Screen,
    /// The signed-in session kept while another account is being added, to go back to.
    parked: Option<(Entity<ShellView>, Session)>,
    _screen_subscription: Option<Subscription>,
    /// The placement last seen. `placement_save` is the pending write; replacing it restarts the
    /// timer, so only the last change of a drag is saved.
    last_placement: window::Placement,
    placement_save: Option<Task<()>>,
    /// `last_placement` changed since it was last saved; closing the window saves it right away.
    placement_unsaved: bool,
    _appearance: Subscription,
    _text_size: Subscription,
}

impl AppView {
    pub fn new(config: Config, placement: window::Placement, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_bounds(window, |this, window, cx| this.remember_window_bounds(window, cx))
            .detach();
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            view.update(cx, |this, cx| this.save_placement(cx)).ok();
            true
        });
        let mut this = Self {
            device_id: config.device_id,
            saved: config.server.clone(),
            screen: Screen::Restoring,
            parked: None,
            _screen_subscription: None,
            last_placement: placement,
            placement_save: None,
            placement_unsaved: false,
            _appearance: theme::follow_system(window),
            _text_size: theme::follow_system_text_size(window, cx),
        };
        match config.server {
            Some(saved) => this.restore(saved, true, window, cx),
            None => this.show_login(None, window, cx),
        }
        #[cfg(debug_assertions)]
        replay_debug_keys(window, cx);
        this
    }

    /// Saves the window's size, position and maximized state shortly after they stop changing,
    /// so the next launch reopens the window the same way. The bounds come from `window_bounds`,
    /// the value `WindowOptions` takes back at launch.
    fn remember_window_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = window::WindowState {
            bounds: window.window_bounds().get_bounds(),
            maximized: window.is_maximized(),
            fullscreen: window.is_fullscreen(),
        };
        let placement = window::remember(self.last_placement, state);
        if placement == self.last_placement {
            return;
        }
        self.last_placement = placement;
        self.placement_unsaved = true;
        self.placement_save = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SAVE_PLACEMENT_AFTER).await;
            this.update(cx, |this, cx| this.save_placement(cx)).ok();
        }));
    }

    fn save_placement(&mut self, cx: &mut App) {
        if std::mem::take(&mut self.placement_unsaved) {
            window::save_placement(self.last_placement, cx);
        }
    }

    /// Signs back in with the stored token. Falls back to the sign-in screen, pre-filled with
    /// the saved server and username, if the token is missing, revoked or the server is down.
    /// With `allow_reconnect`, an unreachable server leaves the token in place and the sign-in
    /// screen tries the saved session again once the server answers.
    fn restore(&mut self, saved: SavedServer, allow_reconnect: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.saved = Some(saved.clone());
        self.screen = Screen::Restoring;
        cx.notify();
        let device_id = self.device_id.clone();
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
                    this.remember(&session);
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
                    let unreachable = matches!(err, Some(jellyfin::Error::Http(_) | jellyfin::Error::ServerStarting));
                    let prefill = Prefill {
                        url: saved.url.clone(),
                        username: saved.username.clone(),
                        error: err.map(|e| describe_error(&e)),
                        resume_saved_session: allow_reconnect && unreachable,
                    };
                    this.show_login(Some(prefill), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The sign-in screen, listing the saved accounts. With a parked session (adding an
    /// account), it can be cancelled back to it.
    fn show_login(&mut self, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) {
        let device_id = self.device_id.clone();
        let accounts = Config::load().accounts;
        let can_cancel = self.parked.is_some();
        let login = cx.new(|cx| LoginView::new(device_id, prefill, accounts, can_cancel, window, cx));
        self._screen_subscription = Some(cx.subscribe_in(&login, window, |this, login, ev, window, cx| {
            match ev {
                LoginEvent::LoggedIn(session) => this.on_logged_in((**session).clone(), window, cx),
                LoginEvent::ServerReachable => {
                    if let Some(saved) = this.saved.clone() {
                        this.restore(saved, false, window, cx);
                    }
                }
                // The account signed in before "Add account" just goes back to that session.
                LoginEvent::UseAccount(account)
                    if this
                        .parked
                        .as_ref()
                        .is_some_and(|(_, session)| session.account_key() == account.credential_account()) =>
                {
                    this.unpark(window, cx)
                }
                LoginEvent::UseAccount(account) => this.switch_account(account.clone(), window, cx),
                LoginEvent::ForgetAccount(account) => {
                    this.forget_account(account);
                    let accounts = Config::load().accounts;
                    login.update(cx, |login, cx| login.set_accounts(accounts, cx));
                }
                LoginEvent::Cancel => this.unpark(window, cx),
            }
        }));
        self.screen = Screen::Login(login);
        cx.notify();
    }

    fn show_home(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let home = cx.new(|cx| ShellView::new(session.clone(), window, cx));
        self.attach_home(home, session, window, cx);
    }

    fn attach_home(&mut self, home: Entity<ShellView>, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        self._screen_subscription = Some(
            cx.subscribe_in(&home, window, move |this, home, ev, window, cx| match ev {
                ShellEvent::SignOut => this.sign_out(&session, window, cx),
                ShellEvent::SessionExpired => this.session_expired(window, cx),
                ShellEvent::SwitchAccount(account) => this.switch_account(account.clone(), window, cx),
                ShellEvent::AddAccount => {
                    tracing::info!("adding an account");
                    this.parked = Some((home.clone(), session.clone()));
                    this.show_login(None, window, cx);
                }
            }),
        );
        self.screen = Screen::Home(home);
        cx.notify();
    }

    /// Back to the session that was signed in before "Add account".
    fn unpark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((home, session)) = self.parked.take() {
            tracing::info!(user = %session.user.name, "back to the signed-in account");
            self.saved = Config::load()
                .accounts
                .into_iter()
                .find(|account| account.credential_account() == session.account_key());
            self.attach_home(home, session, window, cx);
        }
    }

    /// Ends the current session (and a parked one) and signs in to `account` with its saved
    /// token.
    fn switch_account(&mut self, account: SavedServer, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!(account = %account.label(), "switching account");
        self.end_session(cx);
        self.restore(account, false, window, cx);
    }

    /// Stops playback, leaves a SyncPlay group and drops the signed-in pages, before another
    /// account takes over.
    fn end_session(&mut self, cx: &mut Context<Self>) {
        stop_activity(cx);
        self.parked = None;
    }

    fn on_logged_in(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!(user = %session.user.name, "signed in");
        if self.parked.is_some() {
            self.end_session(cx);
        }
        if let Some(token) = session.client.token().map(str::to_owned) {
            let account = session.account_key();
            cx.background_executor()
                .spawn(async move {
                    if let Err(err) = credentials::store(&account, &token) {
                        tracing::error!("failed to store access token: {err:#}");
                    }
                })
                .detach();
        }
        self.remember(&session);
        self.show_home(session, window, cx);
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

    /// Signs the current account out for good: revokes its token and drops it from the saved
    /// accounts. The sign-in screen then lists the others.
    fn sign_out(&mut self, session: &Session, window: &mut Window, cx: &mut Context<Self>) {
        stop_activity(cx);
        // Revoke the token server-side; nothing to do if that fails, we forget it either way.
        let client = session.client.clone();
        runtime::detach(async move {
            if let Err(err) = client.logout().await {
                tracing::warn!("logout request failed: {err}");
            }
        });
        let Some(saved) = self.saved.take() else {
            return self.show_login(None, window, cx);
        };
        self.forget_account(&saved);
        // With no other account, offer the same server and user again.
        let prefill = Config::load().accounts.is_empty().then(|| Prefill {
            url: saved.url.clone(),
            username: saved.username.clone(),
            error: None,
            resume_saved_session: false,
        });
        self.show_login(prefill, window, cx);
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

    fn session_expired(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.screen, Screen::Login(_)) {
            return;
        }
        tracing::warn!("session expired");
        crate::playback::PlaybackManager::global(cx).update(cx, |playback, cx| playback.stop(cx));
        self.forget_token(Some(describe_error(&jellyfin::Error::Unauthorized)), window, cx);
    }

    /// Deletes the current account's stored token, keeping the account in the list, and shows
    /// sign-in for it.
    fn forget_token(&mut self, error: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let prefill = self.saved.as_ref().map(|saved| {
            let account = saved.credential_account();
            cx.background_executor()
                .spawn(async move { credentials::delete(&account) })
                .detach();
            Prefill {
                url: saved.url.clone(),
                username: saved.username.clone(),
                error,
                resume_saved_session: false,
            }
        });
        self.show_login(prefill, window, cx);
    }
}

/// Dev aid: `JELLY_GPUI_KEYS="down right enter wait escape"` presses keys after startup, for
/// testing keyboard navigation without touching the desktop, on any screen (sign-in too). `wait`
/// pauses for a second; `down*40` presses a key 40 times, 30 ms apart (a held key, roughly).
#[cfg(debug_assertions)]
fn replay_debug_keys(window: &mut Window, cx: &mut Context<AppView>) {
    use std::time::Duration;

    let Ok(keys) = std::env::var("JELLY_GPUI_KEYS") else {
        return;
    };
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        for key in keys.split_whitespace() {
            if key == "wait" {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                continue;
            }
            let (key, times) = match key.split_once('*') {
                Some((key, times)) => (key, times.parse().unwrap_or(1)),
                None => (key, 1),
            };
            match Keystroke::parse(key) {
                Ok(keystroke) => {
                    tracing::info!("debug key: {key} x{times}");
                    for i in 0..times {
                        if i > 0 {
                            cx.background_executor().timer(Duration::from_millis(30)).await;
                        }
                        cx.update(|window, cx| window.dispatch_keystroke(keystroke.clone(), cx))
                            .ok();
                    }
                }
                Err(err) => tracing::warn!("bad debug key {key:?}: {err}"),
            }
            cx.background_executor().timer(Duration::from_millis(250)).await;
        }
    })
    .detach();
}

/// Stops what the signed-in session is doing on the server: playback, and a SyncPlay group.
fn stop_activity(cx: &mut App) {
    crate::playback::PlaybackManager::global(cx).update(cx, |playback, cx| playback.stop(cx));
    if let Some(syncplay) = crate::syncplay::SyncPlay::try_global(cx).filter(|s| s.read(cx).group.is_some()) {
        syncplay.update(cx, |syncplay, cx| syncplay.leave(cx));
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
