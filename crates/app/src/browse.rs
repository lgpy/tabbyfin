//! Keyboard navigation for browsing pages: a cursor that moves through sections of cards
//! (shelves, grids and lists) with the arrow keys, Tab between sections and Enter to open.
//!
//! The cursor stays hidden until a key is pressed, and hides again on a click, so mouse users
//! never see it.
//!
//! A page implements [`Browse`] and passes its keys to [`on_key`] (its `Page::on_key`). While
//! drawing, it marks the card at [`Cursor::is_selected`] and calls [`reveal`] for it, and calls
//! [`hide_on_click`].

use crate::keys::Action;
use crate::views::PageCx;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    NextSection,
    PrevSection,
}

/// One section's layout: `len` items in rows of `columns`. A shelf is a single row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    pub len: usize,
    pub columns: usize,
}

impl Section {
    pub fn row(len: usize) -> Self {
        Self {
            len,
            columns: len.max(1),
        }
    }

    pub fn grid(len: usize, columns: usize) -> Self {
        Self {
            len,
            columns: columns.max(1),
        }
    }

    fn last_row(&self) -> usize {
        self.len.saturating_sub(1) / self.columns
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub section: usize,
    pub index: usize,
}

#[derive(Default)]
pub struct Cursor {
    position: Position,
    visible: bool,
    /// A key moved the cursor (or showed it) since the selected card last scrolled into view.
    reveal: bool,
}

impl Cursor {
    /// A hidden cursor that shows at `position` on the first key.
    pub fn starting_at(position: Position) -> Self {
        Self {
            position,
            ..Self::default()
        }
    }

    /// The highlighted card, if the cursor is showing.
    pub fn selected(&self) -> Option<Position> {
        self.visible.then_some(self.position)
    }

    pub fn is_selected(&self, section: usize, index: usize) -> bool {
        self.selected() == Some(Position { section, index })
    }

    /// Hides the cursor. Returns whether it was showing.
    pub fn hide(&mut self) -> bool {
        std::mem::take(&mut self.visible)
    }

    /// Shows the cursor at `position` (e.g. returning from the search box).
    pub fn show_at(&mut self, position: Position) {
        self.position = position;
        self.visible = true;
        self.reveal = true;
    }

    /// Applies a key press. The first press only shows the cursor where it was. Returns the
    /// new position, or `None` if the cursor is at an edge (or there's nothing to select).
    pub fn apply(&mut self, mv: Move, sections: &[Section]) -> Option<Position> {
        let current = clamp(self.position, sections)?;
        let next = if self.visible {
            step(current, mv, sections)?
        } else {
            current
        };
        self.position = next;
        self.visible = true;
        self.reveal = true;
        Some(next)
    }

    /// Whether the selected card should scroll into view, once after each move: scrolling
    /// it every frame would fight the mouse wheel.
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }
}

/// Keeps a position valid after the sections changed (e.g. after a reload).
fn clamp(position: Position, sections: &[Section]) -> Option<Position> {
    let section = position.section.min(sections.len().checked_sub(1)?);
    let found = (section..sections.len())
        .chain((0..section).rev())
        .find(|&s| sections[s].len > 0)?;
    let index = if found == section {
        position.index.min(sections[found].len - 1)
    } else {
        0
    };
    Some(Position { section: found, index })
}

fn step(Position { section, index }: Position, mv: Move, sections: &[Section]) -> Option<Position> {
    let current = sections[section];
    let column = index % current.columns;
    let at = |section, index| Some(Position { section, index });
    // The next non-empty section in either direction.
    let next_section = || (section + 1..sections.len()).find(|&s| sections[s].len > 0);
    let prev_section = || (0..section).rev().find(|&s| sections[s].len > 0);
    match mv {
        Move::Left => (index > 0).then(|| Position {
            section,
            index: index - 1,
        }),
        Move::Right => (index + 1 < current.len).then_some(Position {
            section,
            index: index + 1,
        }),
        // Within a grid, move a row and keep the column (the last row may be shorter).
        Move::Down if index / current.columns < current.last_row() => {
            at(section, (index + current.columns).min(current.len - 1))
        }
        Move::Up if index >= current.columns => at(section, index - current.columns),
        // Otherwise into the neighbouring section, as close to the same column as it has.
        Move::Down => {
            let next = next_section()?;
            at(next, column.min(sections[next].len - 1))
        }
        Move::Up => {
            let prev = prev_section()?;
            let target = sections[prev];
            let last_row_start = target.last_row() * target.columns;
            at(prev, (last_row_start + column).min(target.len - 1))
        }
        Move::NextSection => at(next_section()?, 0),
        Move::PrevSection => at(prev_section()?, 0),
    }
}

/// A page with a [`Cursor`]: what [`on_key`] needs to move it and act on it.
pub trait Browse {
    fn cursor(&mut self) -> &mut Cursor;

    /// The current layout, in the order sections appear on the page.
    fn sections(&self) -> Vec<Section>;

    /// Enter on a card.
    fn activate(&mut self, position: Position, cx: &mut PageCx);

    /// A key that would move the cursor past the first or last section. Returns whether the
    /// page used it. By default Up goes on to the shell, which moves into the top bar; the
    /// cursor hides meanwhile, and the next key on the page shows it where it was.
    fn at_edge(&mut self, mv: Move, _cx: &mut PageCx) -> bool {
        if mv == Move::Up {
            self.cursor().hide();
            return false;
        }
        true
    }

    /// Left or Right on a control that takes them itself (a picker or a switch), instead of
    /// moving the cursor. Returns whether it did.
    fn adjust(&mut self, _position: Position, _forward: bool, _cx: &mut PageCx) -> bool {
        false
    }
}

/// A browsing page's keys: its `Page::on_key`. Returns whether the key was used; the ones that
/// weren't go on to the shell.
pub fn on_key<B: Browse>(page: &mut B, action: Action, cx: &mut PageCx) -> bool {
    let mv = match action {
        Action::Move(mv) => mv,
        Action::Activate => match page.cursor().selected() {
            Some(position) => {
                page.activate(position, cx);
                return true;
            }
            // Enter with the cursor hidden shows it, like an arrow key.
            None => Move::Left,
        },
        _ => return false,
    };
    if let (Some(position), Move::Left | Move::Right) = (page.cursor().selected(), mv)
        && page.adjust(position, mv == Move::Right, cx)
    {
        return true;
    }
    let sections = page.sections();
    match page.cursor().apply(mv, &sections) {
        Some(_) => true,
        None => page.at_edge(mv, cx),
    }
}

/// Hides the cursor when the mouse is pressed on the page, so mouse users never see it. Call it
/// with the page's whole `Ui`.
pub fn hide_on_click(cursor: &mut Cursor, ui: &egui::Ui) {
    if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
        cursor.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, Move, Position, Section};

    fn pos(section: usize, index: usize) -> Position {
        Position { section, index }
    }

    /// Shows the cursor at `start`, then applies `moves`.
    fn run(sections: &[Section], start: Position, moves: &[Move]) -> Vec<Option<Position>> {
        let mut cursor = Cursor::default();
        cursor.show_at(start);
        moves.iter().map(|&mv| cursor.apply(mv, sections)).collect()
    }

    #[test]
    fn first_press_only_shows_the_cursor() {
        let mut cursor = Cursor::default();
        let sections = [Section::row(3)];
        assert_eq!(cursor.selected(), None);
        assert_eq!(cursor.apply(Move::Right, &sections), Some(pos(0, 0)));
        assert_eq!(cursor.apply(Move::Right, &sections), Some(pos(0, 1)));
        assert!(cursor.hide());
        assert_eq!(cursor.apply(Move::Right, &sections), Some(pos(0, 1)));
    }

    #[test]
    fn shelves() {
        let sections = [Section::row(3), Section::row(10), Section::row(2)];
        let moves = [
            Move::Right,
            Move::Right,
            Move::Right,
            Move::Down,
            Move::Down,
            Move::Down,
            Move::Up,
        ];
        assert_eq!(
            run(&sections, pos(0, 0), &moves),
            [
                Some(pos(0, 1)),
                Some(pos(0, 2)),
                None,
                Some(pos(1, 2)),
                // The shorter shelf clamps to its last card.
                Some(pos(2, 1)),
                None,
                Some(pos(1, 1)),
            ]
        );
        assert_eq!(run(&sections, pos(0, 0), &[Move::Left, Move::Up]), [None, None]);
    }

    #[test]
    fn grid_rows_and_short_last_row() {
        // Rows: 0..4, 4..8, 8..10.
        let sections = [Section::grid(10, 4)];
        assert_eq!(
            run(&sections, pos(0, 7), &[Move::Down, Move::Down, Move::Up, Move::Up]),
            [Some(pos(0, 9)), None, Some(pos(0, 5)), Some(pos(0, 1))]
        );
        // Left and Right wrap between rows.
        assert_eq!(run(&sections, pos(0, 3), &[Move::Right]), [Some(pos(0, 4))]);
    }

    #[test]
    fn between_grids_keeps_the_column() {
        let sections = [Section::grid(6, 4), Section::grid(9, 4)];
        // Down from the short last row of the first grid lands in the second grid's top row.
        assert_eq!(run(&sections, pos(0, 5), &[Move::Down]), [Some(pos(1, 1))]);
        // Up from the second grid's top row lands in the first grid's last row.
        assert_eq!(run(&sections, pos(1, 3), &[Move::Up]), [Some(pos(0, 5))]);
        assert_eq!(run(&sections, pos(1, 1), &[Move::Up]), [Some(pos(0, 5))]);
    }

    #[test]
    fn tab_skips_empty_sections() {
        let sections = [Section::row(2), Section::row(0), Section::grid(5, 2)];
        assert_eq!(
            run(
                &sections,
                pos(0, 1),
                &[Move::NextSection, Move::NextSection, Move::PrevSection]
            ),
            [Some(pos(2, 0)), None, Some(pos(0, 0))]
        );
        assert_eq!(run(&sections, pos(0, 1), &[Move::Down]), [Some(pos(2, 1))]);
    }

    #[test]
    fn clamps_after_the_layout_shrinks() {
        let mut cursor = Cursor::default();
        cursor.show_at(pos(2, 7));
        assert_eq!(cursor.apply(Move::Left, &[Section::row(4)]), Some(pos(0, 2)));
        // Nothing left to select.
        assert_eq!(cursor.apply(Move::Left, &[Section::row(0)]), None);
        assert_eq!(cursor.apply(Move::Left, &[]), None);
    }

    #[test]
    fn a_move_reveals_the_card_once() {
        let mut cursor = Cursor::default();
        assert!(!cursor.take_reveal());
        cursor.apply(Move::Right, &[Section::row(3)]);
        assert!(cursor.take_reveal());
        assert!(!cursor.take_reveal());
    }
}
