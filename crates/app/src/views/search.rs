//! Search results: Movies, Shows and Episodes as the search box changes. U4 fills it.
//!
//! Until then the page holds the term the top bar's search box typed, so the shell's search
//! behaviour (typing opens the page, clearing the box closes it, Enter or Down into the
//! results) can be used; there are no results to move onto yet, so `enter_results` keeps the
//! keyboard in the box as a search with nothing found would.

use crate::views::{Page, PageCx};
use crate::widgets::state;

#[derive(Default)]
pub struct SearchPage {
    /// What the box holds, trimmed. Empty until the shell says otherwise.
    term: String,
}

impl Page for SearchPage {
    fn title(&self) -> &str {
        "Search"
    }

    fn set_search_term(&mut self, term: &str, _cx: &mut PageCx) {
        self.term = term.to_owned();
    }

    fn show(&mut self, ui: &mut egui::Ui, _cx: &mut PageCx) {
        if self.term.is_empty() {
            state::message(ui, "Type in the search box to search.");
        } else {
            state::message(ui, format!("Search results for “{}” arrive with U4.", self.term));
        }
    }
}
