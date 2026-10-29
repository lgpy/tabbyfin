//! The library's genres, tags and ratings dialog: tick any number of each. Items with any of
//! the ticked genres show, and likewise for tags and ratings; different kinds must all match.
//! Each tick reloads the grid behind the dialog. Up and Down move (wrapping round, as in a menu),
//! Space ticks, Enter or Esc closes. The values are menu rows (`widgets::popup::row`).
//!
//! The page keeps the dialog it opened and draws it every frame; [`Outcome`] says what to do
//! next. It's an `egui::Modal`, so its backdrop blocks the page, and egui keeps the keyboard
//! while it's open (`keys.rs`): the dialog reads the keys it uses itself, as U8's do.

use egui::{Align, Id, Key, Layout, Modifiers, RichText, ScrollArea, Vec2};

use crate::config::LibraryView;
use crate::views::library::{FilterValues, ValueKind};
use crate::widgets::popup::{self, Entry};

/// How tall the list may get before it scrolls, as on `main`.
const LIST_MAX_H: f32 = 420.;
/// The dialog's width, as on `main`.
const WIDTH: f32 = 380.;

/// The dialog's own state: which value the keyboard is on.
#[derive(Default)]
pub struct ValueFilters {
    /// Index into the values; the titles aren't selectable.
    highlighted: usize,
    /// Up or Down moved the highlight: bring it back on screen.
    scroll_to_highlight: bool,
}

/// What the dialog wants the page to do after drawing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still open, nothing changed.
    Stay,
    /// A value was ticked or unticked: the grid behind reloads.
    Changed,
    /// Enter, Esc, a click outside or *Done*: the page drops the dialog.
    Close,
}

impl ValueFilters {
    /// A dialog with its highlight on the first value.
    pub fn new() -> Self {
        Self {
            highlighted: 0,
            scroll_to_highlight: true,
        }
    }

    /// Draws the dialog over everything. `values` is what the library offers and `view` what's
    /// ticked now; a tick changes `view` and asks for [`Outcome::Changed`].
    pub fn show(&mut self, ctx: &egui::Context, values: &FilterValues, view: &mut LibraryView) -> Outcome {
        let lines = lines(values);
        let entries = entries(&lines);
        if entries.is_empty() {
            return Outcome::Close;
        }
        self.highlighted = self.highlighted.min(entries.len() - 1);
        let mut close = false;
        let mut ticked: Option<(ValueKind, String)> = None;
        let modal = egui::Modal::new(Id::unique("value_filters")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.heading("Genres, tags and ratings");
            ui.add_space(8.);
            // egui keeps the keyboard while the modal is open (`keys.rs`), so the dialog reads
            // the keys it uses itself.
            let (up, down, space, enter) = ui.input_mut(|input| {
                (
                    input.consume_key(Modifiers::NONE, Key::ArrowUp),
                    input.consume_key(Modifiers::NONE, Key::ArrowDown),
                    input.consume_key(Modifiers::NONE, Key::Space),
                    input.consume_key(Modifiers::NONE, Key::Enter),
                )
            });
            if enter {
                close = true;
            }
            if up || down {
                let step = if down { 1 } else { -1 };
                self.highlighted = popup::step(Some(self.highlighted), step, entries.len()).unwrap_or(0);
                self.scroll_to_highlight = true;
            }
            if space && let Some((kind, value)) = entries.get(self.highlighted) {
                ticked = Some((*kind, value.clone()));
            }

            ScrollArea::vertical().max_height(LIST_MAX_H).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
                let mut index = 0;
                let mut first = true;
                for line in &lines {
                    match line {
                        Line::Title(title) => {
                            if !first {
                                ui.add_space(12.);
                            }
                            ui.label(
                                RichText::new(*title)
                                    .font(crate::theme::text::font(ui, "sm"))
                                    .color(crate::theme::tokens_of(ui).muted_foreground),
                            );
                            ui.add_space(4.);
                        }
                        Line::Value(kind, value) => {
                            let checked = chosen(view, *kind).iter().any(|chosen| chosen == value);
                            let entry = Entry::new(value.as_str()).ticked(checked);
                            let row = popup::row(ui, &entry, true, index == self.highlighted);
                            if row.clicked() {
                                ticked = Some((*kind, value.clone()));
                            }
                            // The pointer takes the highlight, as it does in a menu.
                            if row.hovered() && pointer_moved {
                                self.highlighted = index;
                            }
                            if index == self.highlighted && self.scroll_to_highlight {
                                row.scroll_to_me(Some(Align::Center));
                                self.scroll_to_highlight = false;
                            }
                            index += 1;
                        }
                    }
                    first = false;
                }
            });

            ui.add_space(8.);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Done").clicked() {
                    close = true;
                }
            });
        });

        let changed = ticked.is_some();
        if let Some((kind, value)) = ticked {
            let chosen = chosen_mut(view, kind);
            match chosen.iter().position(|chosen| *chosen == value) {
                Some(index) => {
                    chosen.remove(index);
                }
                None => chosen.push(value),
            }
        }
        if changed {
            // The tick wins over a close in the same frame, so the grid reloads before the
            // dialog goes.
            return Outcome::Changed;
        }
        if close || modal.should_close() {
            return Outcome::Close;
        }
        Outcome::Stay
    }
}

/// A line of the dialog: a group's title, or a value to tick.
enum Line {
    Title(&'static str),
    Value(ValueKind, String),
}

fn lines(values: &FilterValues) -> Vec<Line> {
    let mut lines = Vec::new();
    for (kind, title, list) in [
        (ValueKind::Genre, "Genres", &values.genres),
        (ValueKind::Tag, "Tags", &values.tags),
        (ValueKind::Rating, "Ratings", &values.ratings),
    ] {
        if !list.is_empty() {
            lines.push(Line::Title(title));
            lines.extend(list.iter().map(|value| Line::Value(kind, value.clone())));
        }
    }
    lines
}

/// The values only, which the keyboard moves through.
fn entries(lines: &[Line]) -> Vec<(ValueKind, String)> {
    lines
        .iter()
        .filter_map(|line| match line {
            Line::Value(kind, value) => Some((*kind, value.clone())),
            Line::Title(_) => None,
        })
        .collect()
}

fn chosen(view: &LibraryView, kind: ValueKind) -> &[String] {
    match kind {
        ValueKind::Genre => &view.genres,
        ValueKind::Tag => &view.tags,
        ValueKind::Rating => &view.ratings,
    }
}

fn chosen_mut(view: &mut LibraryView, kind: ValueKind) -> &mut Vec<String> {
    match kind {
        ValueKind::Genre => &mut view.genres,
        ValueKind::Tag => &mut view.tags,
        ValueKind::Rating => &mut view.ratings,
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, entries, lines};
    use crate::views::library::{FilterValues, ValueKind};

    fn values() -> FilterValues {
        FilterValues {
            genres: vec!["Comedy".to_owned(), "Drama".to_owned()],
            tags: Vec::new(),
            ratings: vec!["PG".to_owned()],
        }
    }

    #[test]
    fn the_dialog_lists_each_kind_with_values_and_skips_empty_ones() {
        let lines = lines(&values());
        let titles: Vec<&str> = lines
            .iter()
            .filter_map(|line| match line {
                Line::Title(title) => Some(*title),
                Line::Value(..) => None,
            })
            .collect();
        // No tags in the library, so no Tags title.
        assert_eq!(titles, ["Genres", "Ratings"]);
        assert_eq!(
            entries(&lines),
            [
                (ValueKind::Genre, "Comedy".to_owned()),
                (ValueKind::Genre, "Drama".to_owned()),
                (ValueKind::Rating, "PG".to_owned()),
            ]
        );
    }

    #[test]
    fn no_values_means_no_lines() {
        let lines = lines(&FilterValues::default());
        assert!(lines.is_empty());
        assert!(entries(&lines).is_empty());
    }
}
