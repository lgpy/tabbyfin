//! The library's genres and tags popover, hung from the toolbar's button: a search box, the
//! filters chosen so far, and every genre and tag as a pill. A click on a pill includes its value,
//! a second excludes it, a third, or a right click, lets it go. Items with every included genre and tag show
//! (the two kinds must both match), and items with any excluded value are left out, whatever
//! else they have. Each click reloads the grid behind the popover.
//!
//! The search box takes the keyboard while the popover is open: typing narrows the pills, Up
//! and Down move a highlight through them, Enter cycles the highlighted one, and Esc or a click
//! outside closes it (egui's popup). The popover's open state is egui's popup memory, so
//! `keys::egui_wants_keys` leaves the keyboard to it.

use egui::{Align, Align2, Id, Key, Modifiers, RectAlign, Response, ScrollArea, Ui, pos2, vec2};
use egui_material_icons::icons::ICON_SEARCH;

use crate::config::LibraryView;
use crate::theme::{self, text};
use crate::views::library::{FilterValues, ValueKind};
use crate::widgets::pill::{self, Mark};
use crate::widgets::popup;
use crate::widgets::{field, label};

/// The popover's width.
const WIDTH: f32 = 380.;
/// How tall the pills may get before they scroll.
const LIST_MAX_H: f32 = 320.;
/// The room between pills, across and down.
const PILL_GAP: f32 = 6.;
/// The room around the popover's contents, inside the panel's own padding.
const PAD: f32 = 8.;

/// The popover's own state, kept by the page between frames.
#[derive(Default)]
pub struct ValueFilters {
    /// What the search box holds; cleared each time the popover opens.
    search: String,
    /// Index into the pills the search leaves, for the keyboard.
    highlighted: Option<usize>,
    /// Put the keyboard in the search box on the next frame.
    focus_search: bool,
    /// Up or Down moved the highlight: bring it on screen.
    scroll_to_highlight: bool,
}

impl ValueFilters {
    fn id() -> Id {
        Id::unique("library_value_filters")
    }

    pub fn is_open(ctx: &egui::Context) -> bool {
        egui::Popup::is_id_open(ctx, Self::id())
    }

    /// Opens the popover with an empty search box holding the keyboard.
    pub fn open(&mut self, ctx: &egui::Context) {
        *self = Self {
            focus_search: true,
            ..Default::default()
        };
        egui::Popup::open_id(ctx, Self::id());
        ctx.request_repaint();
    }

    /// What clicking its button does.
    pub fn toggle(&mut self, ctx: &egui::Context) {
        if Self::is_open(ctx) {
            egui::Popup::close_id(ctx, Self::id());
        } else {
            self.open(ctx);
        }
    }

    /// Draws the popover hung from `anchor` while it's open. `values` is what the library offers
    /// and `view` what's chosen; a click changes `view` and returns true.
    pub fn show(&mut self, anchor: &Response, values: &FilterValues, view: &mut LibraryView) -> bool {
        let mut cycled: Option<(ValueKind, String, bool)> = None;
        popup::panel(Self::id(), anchor, RectAlign::BOTTOM_START, WIDTH, |ui| {
            // A frame, not a `horizontal`, which would cap the scroll area's room at one row.
            egui::Frame::new()
                .inner_margin(egui::Margin::same(PAD as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    cycled = self.contents(ui, values, view);
                });
        });
        let Some((kind, value, release)) = cycled else {
            return false;
        };
        if release {
            set_off(view, kind, &value);
        } else {
            cycle(view, kind, &value);
        }
        // The chosen pills above the list come and go: the panel sizes itself again next frame.
        anchor.ctx.request_repaint();
        true
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        values: &FilterValues,
        view: &LibraryView,
    ) -> Option<(ValueKind, String, bool)> {
        let mut cycled = None;
        let shown = matching(values, &self.search);

        // The search box keeps the keyboard; the popover reads the keys it moves the pills with.
        let (up, down, enter) = ui.input_mut(|input| {
            (
                input.consume_key(Modifiers::NONE, Key::ArrowUp),
                input.consume_key(Modifiers::NONE, Key::ArrowDown),
                input.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if up || down {
            self.highlighted = popup::step(self.highlighted, if down { 1 } else { -1 }, shown.len());
            self.scroll_to_highlight = true;
        }
        if enter && let Some((kind, value)) = self.highlighted.and_then(|index| shown.get(index)) {
            cycled = Some((*kind, value.clone(), false));
        }

        if self.search_box(ui) {
            // A different search: the highlight starts over.
            self.highlighted = None;
        }
        self.highlighted = self.highlighted.filter(|&index| index < shown.len());

        let chosen = chosen(view);
        if !chosen.is_empty() {
            ui.add_space(10.);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(PILL_GAP, PILL_GAP);
                for (kind, value, mark) in &chosen {
                    let response = pill::filter(ui, value, *mark, false);
                    if response.clicked() || response.secondary_clicked() {
                        cycled = Some((*kind, value.clone(), response.secondary_clicked()));
                    }
                }
            });
        }

        ui.add_space(12.);
        ScrollArea::vertical().max_height(LIST_MAX_H).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            if shown.is_empty() {
                let message = format!("No genres or tags match \u{201c}{}\u{201d}.", self.search.trim());
                ui.label(label::muted(ui, message));
                return;
            }
            let mut index = 0;
            for (kind, title) in [(ValueKind::Genre, "Genres"), (ValueKind::Tag, "Tags")] {
                let group: Vec<&String> = shown
                    .iter()
                    .filter(|(k, _)| *k == kind)
                    .map(|(_, value)| value)
                    .collect();
                if group.is_empty() {
                    continue;
                }
                if index > 0 {
                    ui.add_space(12.);
                }
                ui.label(label::overline(ui, title));
                ui.add_space(6.);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(PILL_GAP, PILL_GAP);
                    for value in group {
                        let highlighted = self.highlighted == Some(index);
                        let response = pill::filter(ui, value, mark(view, kind, value), highlighted);
                        if response.clicked() || response.secondary_clicked() {
                            cycled = Some((kind, value.clone(), response.secondary_clicked()));
                            self.highlighted = Some(index);
                        }
                        if highlighted && self.scroll_to_highlight {
                            response.scroll_to_me(Some(Align::Center));
                            self.scroll_to_highlight = false;
                        }
                        index += 1;
                    }
                });
            }
        });

        ui.add_space(10.);
        ui.label(label::note(
            ui,
            "Click once to include, twice to exclude, right-click to clear.",
        ));
        cycled
    }

    /// The search box: the cozy field, with its icon inside the left padding as the top bar's
    /// and Settings' have it. Returns whether its text changed.
    fn search_box(&mut self, ui: &mut Ui) -> bool {
        let icon = text::icon(ui, ICON_SEARCH.font_family());
        let inset = (12. + icon.size + 6.).round() as i8;
        let edit = field::single_line(ui, Self::id().with("search"), &mut self.search, "Find a genre or tag");
        let response = ui.add(edit.margin(egui::Margin {
            left: inset,
            right: 12,
            top: 9,
            bottom: 9,
        }));
        if std::mem::take(&mut self.focus_search) {
            response.request_focus();
        }
        let rect = response.rect;
        ui.painter().text(
            pos2(rect.left() + 12. + icon.size / 2., rect.center().y),
            Align2::CENTER_CENTER,
            ICON_SEARCH.codepoint,
            icon,
            theme::tokens_of(ui).ink_muted,
        );
        response.changed()
    }
}

/// What the toolbar's button says: what's chosen, or what it's for.
pub fn summary(view: &LibraryView) -> String {
    let chosen = chosen(view);
    match chosen.as_slice() {
        [] => "Genres and tags".to_owned(),
        [one] => describe(one),
        [one, two] => format!("{}, {}", describe(one), describe(two)),
        many => format!("{} filters", many.len()),
    }
}

fn describe((_, value, mark): &(ValueKind, String, Mark)) -> String {
    match mark {
        Mark::Excluded => format!("Not {value}"),
        _ => value.clone(),
    }
}

/// The chosen values, included before excluded, genres before tags within each.
fn chosen(view: &LibraryView) -> Vec<(ValueKind, String, Mark)> {
    let lists = [
        (ValueKind::Genre, &view.genres, Mark::Included),
        (ValueKind::Tag, &view.tags, Mark::Included),
        (ValueKind::Genre, &view.exclude_genres, Mark::Excluded),
        (ValueKind::Tag, &view.exclude_tags, Mark::Excluded),
    ];
    lists
        .into_iter()
        .flat_map(|(kind, list, mark)| list.iter().map(move |value| (kind, value.clone(), mark)))
        .collect()
}

/// The genres, then the tags, whose names contain `search` (ignoring case).
fn matching(values: &FilterValues, search: &str) -> Vec<(ValueKind, String)> {
    let search = search.trim().to_lowercase();
    [(ValueKind::Genre, &values.genres), (ValueKind::Tag, &values.tags)]
        .into_iter()
        .flat_map(|(kind, list)| list.iter().map(move |value| (kind, value)))
        .filter(|(_, value)| value.to_lowercase().contains(&search))
        .map(|(kind, value)| (kind, value.clone()))
        .collect()
}

fn lists(view: &LibraryView, kind: ValueKind) -> (&Vec<String>, &Vec<String>) {
    match kind {
        ValueKind::Genre => (&view.genres, &view.exclude_genres),
        ValueKind::Tag => (&view.tags, &view.exclude_tags),
    }
}

fn lists_mut(view: &mut LibraryView, kind: ValueKind) -> (&mut Vec<String>, &mut Vec<String>) {
    match kind {
        ValueKind::Genre => (&mut view.genres, &mut view.exclude_genres),
        ValueKind::Tag => (&mut view.tags, &mut view.exclude_tags),
    }
}

pub fn mark(view: &LibraryView, kind: ValueKind, value: &str) -> Mark {
    let (included, excluded) = lists(view, kind);
    if included.iter().any(|v| v == value) {
        Mark::Included
    } else if excluded.iter().any(|v| v == value) {
        Mark::Excluded
    } else {
        Mark::Off
    }
}

/// Off, included, excluded, and off again.
pub fn cycle(view: &mut LibraryView, kind: ValueKind, value: &str) {
    let now = mark(view, kind, value);
    let (included, excluded) = lists_mut(view, kind);
    included.retain(|v| v != value);
    excluded.retain(|v| v != value);
    match now {
        Mark::Off => included.push(value.to_owned()),
        Mark::Included => excluded.push(value.to_owned()),
        Mark::Excluded => {}
    }
}

/// Lets a value go, whichever way it was chosen: a right click on its pill.
fn set_off(view: &mut LibraryView, kind: ValueKind, value: &str) {
    let (included, excluded) = lists_mut(view, kind);
    included.retain(|v| v != value);
    excluded.retain(|v| v != value);
}

#[cfg(test)]
mod tests {
    use super::{Mark, cycle, mark, matching, summary};
    use crate::config::LibraryView;
    use crate::views::library::{FilterValues, ValueKind};

    #[test]
    fn a_pill_cycles_through_include_exclude_and_off() {
        let mut view = LibraryView::default();
        assert_eq!(mark(&view, ValueKind::Genre, "Drama"), Mark::Off);
        cycle(&mut view, ValueKind::Genre, "Drama");
        assert_eq!(view.genres, ["Drama"]);
        cycle(&mut view, ValueKind::Genre, "Drama");
        assert!(view.genres.is_empty());
        assert_eq!(view.exclude_genres, ["Drama"]);
        cycle(&mut view, ValueKind::Genre, "Drama");
        assert_eq!(mark(&view, ValueKind::Genre, "Drama"), Mark::Off);
        assert!(view.exclude_genres.is_empty());
        // The kinds are apart: a tag of the same name is its own.
        cycle(&mut view, ValueKind::Tag, "Drama");
        assert_eq!(mark(&view, ValueKind::Genre, "Drama"), Mark::Off);
        assert_eq!(mark(&view, ValueKind::Tag, "Drama"), Mark::Included);
    }

    #[test]
    fn the_search_narrows_both_kinds() {
        let values = FilterValues {
            genres: vec!["Comedy".to_owned(), "Documentary".to_owned()],
            tags: vec!["test pattern".to_owned(), "multi-version".to_owned()],
            ratings: Vec::new(),
        };
        assert_eq!(matching(&values, "").len(), 4);
        assert_eq!(
            matching(&values, " PATT "),
            [(ValueKind::Tag, "test pattern".to_owned())]
        );
        assert_eq!(matching(&values, "do"), [(ValueKind::Genre, "Documentary".to_owned())]);
        assert!(matching(&values, "western").is_empty());
    }

    #[test]
    fn the_button_says_whats_chosen() {
        let mut view = LibraryView::default();
        assert_eq!(summary(&view), "Genres and tags");
        view.genres.push("Comedy".to_owned());
        view.exclude_tags.push("4K".to_owned());
        assert_eq!(summary(&view), "Comedy, Not 4K");
        view.tags.push("Kids".to_owned());
        assert_eq!(summary(&view), "3 filters");
    }
}
