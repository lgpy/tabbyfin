//! The screens: the signed-in shell and its pages, sign-in, the player, dialogs.
//!
//! A page is a plain struct implementing [`Page`], kept on the shell's stack while it's open,
//! and drawn each frame with a [`PageCx`] lending it the session's services. [`open`] makes the
//! page for a route; pages that aren't ported yet are a [`StubPage`] naming the task that
//! brings them, and each task replaces its arm.

pub mod dialogs;
pub mod home;
pub mod item;
pub mod library;
pub mod login;
pub mod player;
#[cfg(debug_assertions)]
pub mod posters;
pub mod search;
pub mod settings;
pub mod shell;
pub mod value_filters;

use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::services::Services;

/// What a page gets each frame.
pub struct PageCx<'a> {
    pub ctx: &'a egui::Context,
    pub services: &'a mut Services,
    /// Where the page asks to open or leave pages; the shell applies them after the frame.
    pub nav: &'a mut Vec<NavRequest>,
    /// The device and texture registry egui draws with, for video. `None` in tests.
    pub render_state: Option<&'a egui_wgpu::RenderState>,
}

pub trait Page {
    /// The top bar's title while the page is on top.
    fn title(&self) -> &str;

    /// The key bindings the page uses, besides the shell's (`keys.rs`). `None` leaves Tab, the
    /// arrows, Space and Enter to egui's own focus, for a page of ordinary controls.
    fn scope(&self) -> Option<Scope> {
        None
    }

    /// A key bound in [`Page::scope`], in the routing order after the top bar. Returns whether
    /// the page used it; the ones it didn't go on to the shell.
    fn on_key(&mut self, _action: Action, _cx: &mut PageCx) -> bool {
        false
    }

    /// Text typed while the page has the keyboard and egui isn't using it (no widget focus, no
    /// menu or dialog open, the top bar's keyboard not in the bar): the library's letter jump.
    /// Returns whether the page used it. The shell offers the top page every text event of the
    /// frame, before the page draws.
    fn on_text(&mut self, _text: &str, _cx: &mut PageCx) -> bool {
        false
    }

    /// The top bar's search box typed a new term (trimmed, never empty), for the results page
    /// to search for. Only [`search::SearchPage`] uses it.
    fn set_search_term(&mut self, _term: &str, _cx: &mut PageCx) {}

    /// Enter or Down in the search box: the results page takes the keyboard. Returns whether
    /// it did, i.e. whether there is something to put the cursor on; otherwise the box keeps
    /// the keyboard, as a search still running or with no results does.
    ///
    /// The results page has the box's keys around it too: the shell focuses the box whenever
    /// the results come back on top (Back from one of their pages), and an Up or Shift+Tab the
    /// results page leaves unused at its first row goes back into the box rather than the top
    /// bar, as `main`'s search page did (`at_edge`).
    fn enter_results(&mut self, _cx: &mut PageCx) -> bool {
        false
    }

    /// Takes the page's keyboard cursor off screen, for the top bar or the search box taking
    /// the keyboard: the next Down on the page shows it where it was. Pages with a cursor
    /// (`browse::Browse`) hide theirs; pages without one do nothing.
    fn hide_cursor(&mut self) {}

    /// Draws the page and handles its input. Results of the page's tasks are polled here too.
    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx);
}

/// The page for `route`.
pub fn open(route: Route, cx: &mut PageCx) -> Box<dyn Page> {
    match route {
        Route::Home => Box::new(home::HomePage::default()),
        Route::Library {
            id,
            name,
            collection_type,
        } => Box::new(library::LibraryPage::new(
            cx,
            library::Source::Folder { id, collection_type },
            name,
        )),
        Route::Item { id, name } => {
            let marks_seen = cx.services.refused_marks.sequence();
            Box::new(item::ItemPage::new(id, name, marks_seen))
        }
        Route::Person { id, name } => Box::new(library::LibraryPage::new(cx, library::Source::Person { id }, name)),
        Route::Search => Box::new(search::SearchPage::default()),
        Route::Settings => Box::new(settings::SettingsPage::new()),
        #[cfg(debug_assertions)]
        Route::Posters { id } => Box::new(posters::PostersPage::new(id)),
    }
}

/// A page that isn't ported to egui yet.
pub struct StubPage {
    title: String,
    task: &'static str,
}

impl StubPage {
    fn new(title: String, task: &'static str) -> Self {
        Self { title, task }
    }
}

impl Page for StubPage {
    fn title(&self) -> &str {
        &self.title
    }

    fn show(&mut self, ui: &mut egui::Ui, _cx: &mut PageCx) {
        ui.centered_and_justified(|ui| {
            ui.label(format!("This page arrives with {}.", self.task));
        });
    }
}
