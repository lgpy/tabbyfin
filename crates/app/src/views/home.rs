//! Home. For now it lists the user's libraries: the exemplar page that shows the patterns every
//! page follows. U3 turns it into the real Home (its shelves and cards), drawing them in the
//! order [`home_sections`] gives.
//!
//! - Data loads through a [`Task`] polled in `show`, never awaited on the UI thread, and the
//!   page shows what it has until new data arrives.
//! - It loads again when `services.user_data` moves on (watch data changed elsewhere), checked
//!   in `show`, so a page under others catches up when it's back on top.
//! - Clicks and Enter open pages through `cx.nav`; Esc going back is the shell's.
//! - The browse cursor: [`Browse`] for the keys, [`browse::reveal`] and
//!   [`browse::hide_on_click`] while drawing.

use std::collections::HashMap;

use jellyfin::models::BaseItemDto;

use crate::browse::{self, Browse, Cursor, Position, Section};
use crate::keys::{Action, Scope};
use crate::nav::{NavRequest, Route};
use crate::task::Task;
use crate::views::{Page, PageCx};
use crate::widgets::text::Load;

/// The Home sections of the web client's Settings → Home that this app has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSection {
    MyMedia,
    Resume,
    NextUp,
    Latest,
}

/// The web client's section for each of its ten Home slots when the user hasn't picked one.
pub const DEFAULT_SECTIONS: [&str; 10] = [
    "smalllibrarytiles",
    "resume",
    "resumeaudio",
    "resumebook",
    "livetv",
    "nextup",
    "latestmedia",
    "none",
    "none",
    "none",
];

/// The sections in the user's order, from the web client's `homesection0`…`homesection9`
/// preferences. Sections this app doesn't have (music, books, live TV) are skipped, and a
/// section listed twice keeps its first place.
pub fn home_sections(prefs: &HashMap<String, String>) -> Vec<HomeSection> {
    let mut sections = Vec::new();
    for (slot, default) in DEFAULT_SECTIONS.iter().enumerate() {
        let value = prefs
            .get(&format!("homesection{slot}"))
            .map_or(*default, String::as_str);
        let section = match value {
            "smalllibrarytiles" | "librarybuttons" => HomeSection::MyMedia,
            "resume" => HomeSection::Resume,
            "nextup" => HomeSection::NextUp,
            "latestmedia" => HomeSection::Latest,
            _ => continue,
        };
        if !sections.contains(&section) {
            sections.push(section);
        }
    }
    sections
}

#[derive(Default)]
pub struct HomePage {
    libraries: Load<Vec<BaseItemDto>>,
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    /// The `services.user_data` the libraries were loaded at; `None` before the first load or
    /// after Retry.
    loaded_at: Option<u64>,
    cursor: Cursor,
}

impl HomePage {
    /// Fetches the libraries again. What's on screen stays up until they arrive; a load still
    /// running is replaced, which cancels it.
    fn reload(&mut self, cx: &mut PageCx) {
        self.loaded_at = Some(cx.services.user_data);
        let client = cx.services.session.client.clone();
        self.loading = Some(Task::spawn(cx.ctx, async move {
            client.user_views().await.map(|views| views.items)
        }));
    }

    fn open(item: &BaseItemDto, cx: &mut PageCx) {
        cx.nav.push(NavRequest::Push(Route::for_item(item)));
    }
}

impl Page for HomePage {
    fn title(&self) -> &str {
        "Home"
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Browse)
    }

    fn on_key(&mut self, action: Action, cx: &mut PageCx) -> bool {
        browse::on_key(self, action, cx)
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        if self.loaded_at != Some(cx.services.user_data) {
            self.reload(cx);
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.libraries = match result {
                Ok(libraries) => Load::Loaded(libraries),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        browse::hide_on_click(&mut self.cursor, ui);

        match &self.libraries {
            Load::Loading => {
                ui.centered_and_justified(|ui| ui.spinner());
            }
            Load::Failed(message) => {
                let retry = ui
                    .vertical_centered(|ui| {
                        ui.label(message);
                        ui.button("Retry").clicked()
                    })
                    .inner;
                if retry {
                    self.libraries = Load::Loading;
                    self.loaded_at = None;
                }
            }
            Load::Loaded(libraries) if libraries.is_empty() => {
                ui.centered_and_justified(|ui| ui.label("Your server has no libraries yet."));
            }
            Load::Loaded(libraries) => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, library) in libraries.iter().enumerate() {
                        let selected = self.cursor.is_selected(0, index);
                        let response = ui.add(egui::Button::selectable(selected, &library.name));
                        if selected {
                            browse::reveal(&mut self.cursor, &response);
                        }
                        if response.clicked() {
                            Self::open(library, cx);
                        }
                    }
                });
            }
        }
    }
}

impl Browse for HomePage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// One column: every arrow key moves along the list.
    fn sections(&self) -> Vec<Section> {
        match &self.libraries {
            Load::Loaded(libraries) => vec![Section::grid(libraries.len(), 1)],
            _ => Vec::new(),
        }
    }

    fn activate(&mut self, position: Position, cx: &mut PageCx) {
        if let Load::Loaded(libraries) = &self.libraries
            && let Some(library) = libraries.get(position.index)
        {
            Self::open(library, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{HomeSection, home_sections};

    #[test]
    fn default_home_sections() {
        let sections = home_sections(&HashMap::new());
        assert_eq!(
            sections,
            [
                HomeSection::MyMedia,
                HomeSection::Resume,
                HomeSection::NextUp,
                HomeSection::Latest
            ]
        );
    }

    #[test]
    fn the_users_order_with_hidden_and_unknown_sections() {
        let prefs: HashMap<String, String> = [
            ("homesection0", "nextup"),
            ("homesection1", "livetv"),
            ("homesection2", "librarybuttons"),
            ("homesection3", "none"),
            ("homesection4", "nextup"),
            ("homesection5", "none"),
            ("homesection6", "none"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        // Slots 7–9 default to none: no Continue Watching and no Latest.
        assert_eq!(home_sections(&prefs), [HomeSection::NextUp, HomeSection::MyMedia]);
    }
}
