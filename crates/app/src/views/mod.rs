//! The screens: the signed-in shell and its pages, sign-in, the player, dialogs.
//!
//! A page is a plain struct implementing [`Page`], kept on the shell's stack while it's open,
//! and drawn each frame with a [`PageCx`] lending it the session's services. [`open`] makes the
//! page for a route.

pub mod dialogs;
pub mod home;
pub mod item;
pub mod libraries;
pub mod library;
pub mod login;
pub mod player;
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
    /// Where the page may paint behind the top bar. The shell sets it before the page draws;
    /// `None` outside the shell (tests, the shell's own setup).
    pub under_bar: Option<UnderBar>,
}

/// A place in the paint order under the top bar's contents and over the window's backdrop, for
/// a picture that runs up behind the bar (the item page's banner). The bar is laid out before
/// the page, so anything the page paints itself lands over the bar; the shell keeps this slot
/// for it instead.
#[derive(Clone)]
pub struct UnderBar {
    painter: egui::Painter,
    slot: egui::layers::ShapeIdx,
    /// The bar's rectangle, the room above the page's own top. The shell sets it once the bar
    /// is laid out.
    pub bar: egui::Rect,
}

impl UnderBar {
    /// Reserves the slot on `painter` (the window's), at this point of the paint order.
    pub fn reserve(painter: &egui::Painter) -> Self {
        let slot = painter.add(egui::Shape::Noop);
        Self {
            painter: painter.clone(),
            slot,
            bar: egui::Rect::NOTHING,
        }
    }

    /// Fills the slot with `shapes`, at `ui`'s opacity, so the picture fades in with its page.
    /// Clipped to the bar and `ui`'s own clip rectangle together, the part of the window the
    /// page shows plus the bar above it.
    pub fn paint(&self, ui: &egui::Ui, shapes: Vec<egui::Shape>) {
        let clip = self.bar.union(ui.clip_rect());
        let mut painter = self.painter.with_clip_rect(clip);
        painter.multiply_opacity(ui.opacity());
        painter.set(self.slot, egui::Shape::Vec(shapes));
    }
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

    /// For a page the top bar's search box filters instead of opening the results page
    /// (Settings): what it's filtered by now, which the box shows while the page is on top.
    /// `None`, the default, leaves the box to the results page.
    fn own_search(&self) -> Option<&str> {
        None
    }

    /// The top bar's search box changed over a page with [`Page::own_search`]: its text as
    /// typed, empty once cleared.
    fn set_own_search(&mut self, _text: &str) {}

    /// Enter or Down in the search box: the results page takes the keyboard. Returns whether
    /// it did, i.e. whether there is something to put the cursor on; otherwise the box keeps
    /// the keyboard, as a search still running or with no results does.
    ///
    /// The results page has the box's keys around it too: the shell focuses the box whenever
    /// the results come back on top (Back from one of their pages), and an Up or Shift+Tab the
    /// results page leaves unused at its first row goes back into the box rather than the top
    /// bar.
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
            newest_first,
        } => Box::new(library::LibraryPage::new(
            cx,
            library::Source::Folder {
                id,
                collection_type,
                newest_first,
            },
            name,
        )),
        Route::Item { id, name } => {
            let marks_seen = cx.services.refused_marks.sequence();
            Box::new(item::ItemPage::new(id, name, marks_seen))
        }
        Route::Person { id, name } => Box::new(library::LibraryPage::new(cx, library::Source::Person { id }, name)),
        Route::Libraries => Box::new(libraries::LibrariesPage::default()),
        Route::Search => Box::new(search::SearchPage::default()),
        Route::Settings => Box::new(settings::SettingsPage::new()),
    }
}
