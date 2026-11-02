//! The item page's shelves under its children: chapters, cast & crew, extras and "More like
//! this". Each returns its rectangle, for the cursor scrolling it into view.

use egui::{Rect, Ui};

use super::{
    Asked, BIG_CARD_GAP, BIG_CARD_SCALE, CHAPTERS, EXTRAS, ItemData, PEOPLE, SHELF_TITLE_GAP, SIMILAR, chapters,
};
use crate::browse::Cursor;
use crate::images::ImageCache;
use crate::nav::Route;
use crate::widgets::text::CARD_GAP;
use crate::widgets::{card, menu, shelf};

/// The chapters shelf: their image, name and start. Chosen, the item plays from there with the
/// version picked below its title.
pub(super) fn chapters_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let chapters = chapters(&data.item);
    shelf::shelf_with_gaps(ui, "chapters", "Chapters", CARD_GAP, SHELF_TITLE_GAP, 0., |ui| {
        for (index, chapter) in chapters.iter().enumerate() {
            let is_selected = cursor.is_selected(CHAPTERS, index);
            let response = card::chapter(ui, images, &data.item.id, index, chapter, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.play_from = Some(chapter.start_position_ticks);
            }
        }
    })
}

/// The people shelf: each one's photo or initials, role and job, opening their page.
pub(super) fn people_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let people = &data.item.people;
    shelf::shelf_with_gaps(ui, "people", "Cast & crew", BIG_CARD_GAP, SHELF_TITLE_GAP, 0., |ui| {
        for (index, person) in people.iter().enumerate() {
            let is_selected = cursor.is_selected(PEOPLE, index);
            let response = card::person(ui, images, person, BIG_CARD_SCALE, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                asked.open = Some(Route::Person {
                    id: person.id.clone(),
                    name: person.name.clone(),
                });
            }
        }
    })
}

/// Trailers and extras, played from the start when chosen.
pub(super) fn extras_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let extras = &data.extras;
    shelf::shelf_with_gaps(
        ui,
        "extras",
        "Extras",
        crate::widgets::text::CARD_GAP,
        SHELF_TITLE_GAP,
        // The page already insets its content.
        0.,
        |ui| {
            for (index, extra) in extras.iter().enumerate() {
                let is_selected = cursor.is_selected(EXTRAS, index);
                let response = card::landscape(ui, images, extra, is_selected);
                if is_selected && moved {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    asked.extra = Some(extra.clone());
                }
            }
        },
    )
}

/// "More like this": larger cards, opening their pages, with the item menu.
pub(super) fn similar_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let similar = &data.similar;
    shelf::shelf_with_gaps(
        ui,
        "similar",
        "More like this",
        BIG_CARD_GAP,
        SHELF_TITLE_GAP,
        0.,
        |ui| {
            for (index, item) in similar.iter().enumerate() {
                let is_selected = cursor.is_selected(SIMILAR, index);
                let response = card::poster_scaled(ui, images, item, BIG_CARD_SCALE, is_selected);
                if is_selected && moved {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    asked.open = Some(Route::for_item(item));
                }
                if let Some(action) = menu::item_menu(&response, item) {
                    asked.menu = Some((action, item.clone()));
                }
            }
        },
    )
}
