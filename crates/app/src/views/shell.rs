//! The signed-in frame: a top bar and a navigation stack of pages.
//!
//! Pages stay alive while they're on the stack, so going back keeps their data and scroll
//! position. For now the bar has Back and the page's title; U1 brings the rest of `main`'s
//! shell (logo, search box, SyncPlay, Settings, the account menu, the keyboard in the bar).

use egui_material_icons::icons::ICON_ARROW_BACK;

use crate::keys::{Action, Press, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::toasts::{Toast, ToastAction};
use crate::update::{self, Update};
use crate::views::{self, Page, PageCx};

pub struct Shell {
    /// Never empty: Home is at the bottom.
    stack: Vec<Box<dyn Page>>,
    update_check: Option<Task<Option<Update>>>,
}

impl Shell {
    pub fn new(cx: &mut PageCx) -> Self {
        // `mut` is for the debug build's `debug_start` below.
        #[cfg_attr(not(debug_assertions), allow(unused_mut))]
        let mut shell = Self {
            stack: vec![views::open(Route::Home, cx)],
            update_check: Some(Task::spawn(cx.ctx, update::check())),
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
            Some(Start::Open(route)) => self.push(route, cx),
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
