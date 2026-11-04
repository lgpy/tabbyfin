//! The user's libraries as a page of their own: the phone's bottom bar leads here, where the
//! desktop has Home's *My Media* row. Each library is a [`library_tile`] as wide as its column,
//! one column on a phone and more where the window has room.

use jellyfin::models::BaseItemDto;

use crate::layout;
use crate::nav::Route;
use crate::task::Task;
use crate::views::{Page, PageCx};
use crate::widgets::{backdrop, library_tile, state};

/// The narrowest a column of tiles gets before the page drops a column.
const COLUMN_MIN_W: f32 = 300.;
const GAP: f32 = 12.;

#[derive(Default)]
pub struct LibrariesPage {
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    views: Option<Result<Vec<BaseItemDto>, String>>,
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
                    let tile_w = (width - GAP * (columns - 1) as f32) / columns as f32;
                    // The tiles keep room under them for their shadow, which is the gap down.
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, 0.);
                    for row in views.chunks(columns) {
                        ui.horizontal(|ui| {
                            for view in row {
                                if library_tile::show_wide(ui, &mut cx.services.images, view, tile_w, false).clicked() {
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
