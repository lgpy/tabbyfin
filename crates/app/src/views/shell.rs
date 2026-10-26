//! The signed-in frame: a top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position. For now the bar has Back, the page's title and the account menu S1 needs; U1
//! brings the rest of `main`'s shell (logo, search box, SyncPlay, Settings, the account
//! button's initial, the keyboard in the bar).

use egui_material_icons::icons::{ICON_ACCOUNT_CIRCLE, ICON_ARROW_BACK};

use crate::config::{Config, SavedServer};
use crate::keys::{Action, Press, Scope};
use crate::nav::{NavRequest, Route};
use crate::services::AppEvent;
use crate::task::Task;
use crate::toasts::{Toast, ToastAction};
use crate::update::{self, Update};
use crate::views::{self, Page, PageCx};

pub struct Shell {
    /// Never empty: Home is at the bottom.
    stack: Vec<Box<dyn Page>>,
    update_check: Option<Task<Option<Update>>>,
    /// The saved accounts, read from the config when the account menu opens, as on `main`.
    accounts: Vec<SavedServer>,
}

impl Shell {
    pub fn new(cx: &mut PageCx) -> Self {
        // `mut` is for the debug build's `debug_start` below.
        #[cfg_attr(not(debug_assertions), allow(unused_mut))]
        let mut shell = Self {
            stack: vec![views::open(Route::Home, cx)],
            update_check: Some(Task::spawn(cx.ctx, update::check())),
            accounts: Vec::new(),
        };
        #[cfg(debug_assertions)]
        shell.debug_start(cx);
        shell
    }

    /// The scopes whose keys the app takes from egui (`keys.rs`): the top page's, then the
    /// shell's.
    pub fn scopes(&self) -> Vec<Scope> {
        self.top().scope().into_iter().chain([Scope::Shell]).collect()
    }

    /// Routes this frame's keys, then draws the bar and the top page, then applies the pages'
    /// navigation requests.
    pub fn show(&mut self, ui: &mut egui::Ui, keys: Vec<Press>, cx: &mut PageCx) {
        for press in keys {
            self.on_key(press, cx);
        }
        self.notify_update(cx);
        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui, cx));
        egui::CentralPanel::default().show(ui, |ui| self.top_mut().show(ui, cx));
        self.navigate(cx);
    }

    /// A key the app took, in the routing order: the page, then the shell's own. (The top bar
    /// comes first once U1 brings its keyboard.)
    fn on_key(&mut self, press: Press, cx: &mut PageCx) {
        let page = self.top_mut();
        if let Some(action) = page.scope().and_then(|scope| press.action(scope))
            && page.on_key(action, cx)
        {
            return;
        }
        if let Some(Action::Back) = press.action(Scope::Shell) {
            cx.nav.push(NavRequest::Back);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        ui.horizontal(|ui| {
            if self.stack.len() > 1 && ui.button(ICON_ARROW_BACK).on_hover_text("Back").clicked() {
                cx.nav.push(NavRequest::Back);
            }
            ui.heading(self.top().title());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.account_menu(ui, cx);
            });
        });
    }

    /// The account menu: the saved accounts to switch to (the current one checked), adding one,
    /// and signing out. The app carries the choice out after the frame. U1 turns the button
    /// into the user's initial and gives the bar its keyboard; the choices stay.
    fn account_menu(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        let session = &cx.services.session;
        let account = match &session.server.server_name {
            Some(server) => format!("{} on {server}", session.user.name),
            None => session.user.name.clone(),
        };
        let button = ui.button(ICON_ACCOUNT_CIRCLE).on_hover_text(account);
        if button.clicked() {
            self.accounts = Config::load().accounts;
        }
        egui::Popup::menu(&button).show(|ui| {
            let current = cx.services.session.account_key();
            for account in self.accounts.clone() {
                let checked = account.credential_account() == current;
                if ui.selectable_label(checked, account.label()).clicked() {
                    // Choosing the account already signed in just closes the menu, as on
                    // `main`: switching to it would end the session and lose the page stack.
                    if !checked {
                        cx.services.events.push(AppEvent::SwitchAccount(account));
                    }
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Add account…").clicked() {
                cx.services.events.push(AppEvent::AddAccount);
                ui.close();
            }
            if ui.button("Sign out").clicked() {
                cx.services.events.push(AppEvent::SignOut);
                ui.close();
            }
        });
    }

    fn navigate(&mut self, cx: &mut PageCx) {
        if cx.nav.is_empty() {
            return;
        }
        for request in std::mem::take(cx.nav) {
            match request {
                NavRequest::Push(route) => self.push(route, cx),
                NavRequest::Back => self.back(),
            }
        }
        // The new top page draws on the next frame.
        cx.ctx.request_repaint();
    }

    fn push(&mut self, route: Route, cx: &mut PageCx) {
        let page = views::open(route, cx);
        self.stack.push(page);
    }

    /// Closes the top page, unless it's Home.
    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn top(&self) -> &dyn Page {
        self.stack.last().expect("the page stack is never empty").as_ref()
    }

    fn top_mut(&mut self) -> &mut dyn Page {
        self.stack.last_mut().expect("the page stack is never empty").as_mut()
    }

    /// When the update check finds a newer release, says so with a link to it.
    fn notify_update(&mut self, cx: &mut PageCx) {
        let Some(result) = self.update_check.as_mut().and_then(Task::poll) else {
            return;
        };
        self.update_check = None;
        if let Some(update) = result {
            let text = format!(
                "Version {} is out (this is {}).",
                update.version,
                env!("CARGO_PKG_VERSION")
            );
            let toast = Toast::info(text)
                .title("Update available")
                .action("Download", ToastAction::OpenUrl(update.url));
            cx.services.toast(toast);
        }
    }

    /// Dev aid: `TABBYFIN_ROUTE` opens a page on top of Home (`debug::Start`).
    #[cfg(debug_assertions)]
    fn debug_start(&mut self, cx: &mut PageCx) {
        use crate::debug::Start;

        match Start::from_env() {
            Some(Start::Open(route)) => {
                tracing::info!("TABBYFIN_ROUTE: opening {route:?}");
                self.push(route, cx);
            }
            Some(Start::Search(term)) => {
                tracing::info!("TABBYFIN_ROUTE: typing {term:?} into the search box arrives with U1");
                self.push(Route::Search, cx);
            }
            Some(Start::Play { id, .. }) => {
                tracing::info!("TABBYFIN_ROUTE: playing arrives with P1; opening the item");
                let name = "Item".to_owned();
                self.push(Route::Item { id, name }, cx);
            }
            None => {}
        }
    }
}
