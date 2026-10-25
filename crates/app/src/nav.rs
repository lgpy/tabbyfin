//! Where pages lead: the routes the shell's page stack is made of, and the requests pages make
//! to change it.
//!
//! Pages don't hold the shell. They push a [`NavRequest`] into their `PageCx`, and the shell
//! applies it after the frame.

use jellyfin::models::{BaseItemDto, BaseItemKind, CollectionType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Home,
    /// A library or folder-like item, shown as a grid.
    Library {
        id: String,
        name: String,
        collection_type: Option<CollectionType>,
    },
    /// A detail page: movie, series, season or episode.
    Item {
        id: String,
        name: String,
    },
    /// A person's movies and shows.
    Person {
        id: String,
        name: String,
    },
    Search,
    Settings,
}

impl Route {
    pub fn for_item(item: &BaseItemDto) -> Self {
        match item.kind {
            BaseItemKind::CollectionFolder | BaseItemKind::UserView | BaseItemKind::Folder => Route::Library {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavRequest {
    /// Opens a page on top of the stack.
    Push(Route),
    /// Closes the top page, like the Back button.
    Back,
}
