//! The user's libraries as a page of their own: the phone's bottom bar leads here, where the
//! desktop has Home's *My Media* row. Each library is a [`library_banner`] as wide as its column,
//! one column on a phone and more where the window has room, with how much it holds under its
//! name once the server has counted it.

use std::collections::HashMap;

use jellyfin::ItemsQuery;
use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType};

use crate::layout;
use crate::nav::Route;
use crate::task::Task;
use crate::views::{Page, PageCx};
use crate::widgets::{backdrop, library_banner, state};

/// The narrowest a column of banners gets before the page drops a column.
const COLUMN_MIN_W: f32 = 300.;
const GAP: f32 = 12.;

#[derive(Default)]
pub struct LibrariesPage {
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    views: Option<Result<Vec<BaseItemDto>, String>>,
    /// Each library's count by its id, while it's asked for.
    counting: HashMap<String, Task<jellyfin::Result<u32>>>,
    /// Each library's count line by its id; a count the server couldn't give has none.
    counts: HashMap<String, Option<String>>,
}

impl LibrariesPage {
    /// Asks for each library's count once the libraries are in, and takes the answers.
    fn count(&mut self, cx: &mut PageCx) {
        let Some(Ok(views)) = &self.views else { return };
        for view in views {
            if self.counts.contains_key(&view.id) || self.counting.contains_key(&view.id) {
                continue;
            }
            let client = cx.services.session.client.clone();
            let query = count_query(view);
            self.counting.insert(
                view.id.clone(),
                Task::spawn(cx.ctx, async move {
                    client.items(&query).await.map(|result| result.total_record_count)
                }),
            );
        }
        let kinds: HashMap<&str, Option<CollectionType>> = views
            .iter()
            .map(|view| (view.id.as_str(), view.collection_type))
            .collect();
        self.counting.retain(|id, task| {
            let Some(result) = task.poll() else { return true };
            let line = result
                .ok()
                .map(|n| count_line(kinds.get(id.as_str()).copied().flatten(), n));
            self.counts.insert(id.clone(), line);
            false
        });
    }
}

impl Page for LibrariesPage {
    fn title(&self) -> &str {
        "Libraries"
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        if self.views.is_none() && self.loading.is_none() {
            let client = cx.services.session.client.clone();
            self.loading = Some(Task::spawn(cx.ctx, async move {
                client.user_views().await.map(|views| views.items)
            }));
        }
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.views = Some(result.map_err(|err| cx.services.handle_error(&err)));
        }
        self.count(cx);
        let views = match &self.views {
            None => return state::spinner(ui),
            Some(Err(message)) => {
                if state::error(ui, message.clone()) {
                    self.views = None;
                }
                return;
            }
            Some(Ok(views)) if views.is_empty() => return state::empty(ui, "Your server has no libraries yet."),
            Some(Ok(views)) => views,
        };
        let side = layout::side(ui.ctx());
        let mut opened = None;
        let area = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: side as i8,
                    right: side as i8,
                    top: 8,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    let width = ui.available_width();
                    let columns = (((width + GAP) / (COLUMN_MIN_W + GAP)).floor() as usize).max(1);
                    let banner_w = ((width - GAP * (columns - 1) as f32) / columns as f32).floor();
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                    for row in views.chunks(columns) {
                        ui.horizontal(|ui| {
                            for view in row {
                                let count = self.counts.get(&view.id).and_then(Option::as_deref);
                                let banner = library_banner::show(ui, &mut cx.services.images, view, count, banner_w);
                                if banner.clicked() {
                                    opened = Some(Route::for_item(view));
                                }
                            }
                        });
                    }
                });
        });
        backdrop::scroll_edge(ui, &area);
        if let Some(route) = opened {
            cx.nav.push(crate::nav::NavRequest::Push(route));
        }
    }
}

/// What counts as a library's contents: what its page lists (`library.rs`), the movies or the
/// shows in a library of either, its own entries in the rest. Only the total comes back.
fn count_query(view: &BaseItemDto) -> ItemsQuery {
    let (include_item_types, recursive) = match view.collection_type {
        Some(CollectionType::Movies) => (vec![BaseItemKind::Movie], true),
        Some(CollectionType::TvShows) => (vec![BaseItemKind::Series], true),
        _ => (Vec::new(), false),
    };
    ItemsQuery {
        parent_id: Some(view.id.clone()),
        include_item_types,
        recursive,
        limit: Some(0),
        lean: true,
        ..Default::default()
    }
}

/// "1,284 movies", by what the library holds.
fn count_line(kind: Option<CollectionType>, count: u32) -> String {
    let (one, many) = match kind {
        Some(CollectionType::Movies) => ("movie", "movies"),
        Some(CollectionType::TvShows) => ("show", "shows"),
        Some(CollectionType::BoxSets) => ("collection", "collections"),
        Some(CollectionType::Playlists) => ("playlist", "playlists"),
        _ => ("item", "items"),
    };
    let noun = if count == 1 { one } else { many };
    format!("{} {noun}", thousands(count))
}

/// `count` with a comma between each three digits.
fn thousands(count: u32) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_as_what_the_library_holds() {
        assert_eq!(count_line(Some(CollectionType::Movies), 1284), "1,284 movies");
        assert_eq!(count_line(Some(CollectionType::TvShows), 1), "1 show");
        assert_eq!(count_line(Some(CollectionType::BoxSets), 0), "0 collections");
        assert_eq!(count_line(None, 1_000_000), "1,000,000 items");
        assert_eq!(count_line(Some(CollectionType::Music), 999), "999 items");
    }
}
