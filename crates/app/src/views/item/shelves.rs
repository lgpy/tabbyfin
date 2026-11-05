//! The item page's shelves under its children: chapters, cast & crew, extras and "More like
//! this". Each returns its rectangle, for the cursor scrolling it into view.

use egui::{Rect, Ui};

use super::{
    Asked, BIG_CARD_GAP, BIG_CARD_SCALE, CHAPTERS, COMPACT_CARD_GAP, COMPACT_CARD_SCALE, COMPACT_FACE, EXTRAS,
    ItemData, PEOPLE, SHELF_TITLE_GAP, SIMILAR, chapters,
};
use crate::browse::Cursor;
use crate::images::ImageCache;
use crate::nav::Route;
use crate::widgets::text::CARD_GAP;
use crate::widgets::{card, menu, shelf};

/// The chapters shelf: their image, name and start, or short text tiles when none of them has an
/// image. Chosen, the item plays from there with the version picked below its title.
pub(super) fn chapters_shelf(
    ui: &mut Ui,
    data: &ItemData,
    moved: bool,
    cursor: &mut Cursor,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let chapters = chapters(&data.item);
    let any_image = chapters.iter().any(|chapter| chapter.image_tag.is_some());
    shelf::shelf_with_gaps(ui, "chapters", "Chapters", CARD_GAP, SHELF_TITLE_GAP, 0., |ui| {
        for (index, chapter) in chapters.iter().enumerate() {
            let is_selected = cursor.is_selected(CHAPTERS, index);
            let response = if any_image {
                card::chapter(ui, images, &data.item.id, index, chapter, is_selected)
            } else {
                card::chapter_without_image(ui, index, chapter, is_selected)
            };
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
    // A phone shows round faces, more of them across.
    let compact = crate::layout::compact(ui.ctx());
    let gap = if compact { COMPACT_CARD_GAP / 2. } else { BIG_CARD_GAP };
    shelf::shelf_with_gaps(ui, "people", "Cast & crew", gap, SHELF_TITLE_GAP, 0., |ui| {
        for (index, person) in people.iter().enumerate() {
            let is_selected = cursor.is_selected(PEOPLE, index);
            let response = if compact {
                card::person_round(ui, images, person, COMPACT_FACE, is_selected)
            } else {
                card::person(ui, images, person, BIG_CARD_SCALE, is_selected)
            };
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
    // A phone's posters are smaller, so three and a bit show across.
    let (scale, gap) = match crate::layout::compact(ui.ctx()) {
        true => (COMPACT_CARD_SCALE, COMPACT_CARD_GAP),
        false => (BIG_CARD_SCALE, BIG_CARD_GAP),
    };
    shelf::shelf_with_gaps(ui, "similar", "More like this", gap, SHELF_TITLE_GAP, 0., |ui| {
        for (index, item) in similar.iter().enumerate() {
            let is_selected = cursor.is_selected(SIMILAR, index);
            let response = card::poster_scaled(ui, images, item, scale, is_selected);
            if is_selected && moved {
                response.scroll_to_me(None);
            }
            if response.hovered() {
                asked.hovered = Some(Route::for_item(item));
            }
            if response.clicked() {
                asked.open = Some(Route::for_item(item));
            }
            if let Some(action) = menu::item_menu(&response, item) {
                asked.menu = Some((action, item.clone()));
            }
        }
    })
}
