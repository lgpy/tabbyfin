use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType};

use crate::views::shell::ShellView;

#[derive(Debug, Clone)]
pub enum Route {
    Home,
    /// A library or folder-like item, shown as a grid.
    Library {
        id: String,
        name: String,
        collection_type: Option<CollectionType>,
    },
    /// A detail page: movie, series, season or episode.
    Item { id: String, name: String },
}

impl Route {
    pub fn for_item(item: &BaseItemDto) -> Self {
        match item.kind {
            BaseItemKind::CollectionFolder
            | BaseItemKind::UserView
            | BaseItemKind::Folder
            | BaseItemKind::BoxSet
            | BaseItemKind::Playlist => Route::Library {
                id: item.id.clone(),
                name: item.name.clone(),
                collection_type: item.collection_type,
            },
            _ => Route::Item {
                id: item.id.clone(),
                name: item.name.clone(),
            },
        }
    }
}

/// Handle pages use to navigate. Cheap to clone into click handlers.
#[derive(Clone)]
pub struct Navigator(WeakEntity<ShellView>);

impl Navigator {
    pub fn new(shell: WeakEntity<ShellView>) -> Self {
        Self(shell)
    }

    pub fn push(&self, route: Route, window: &mut Window, cx: &mut App) {
        self.0.update(cx, |shell, cx| shell.push(route, window, cx)).ok();
    }

    pub fn open(&self, item: &BaseItemDto, window: &mut Window, cx: &mut App) {
        self.push(Route::for_item(item), window, cx);
    }
}
