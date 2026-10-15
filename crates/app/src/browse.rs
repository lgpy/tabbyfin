//! Keyboard navigation for browsing pages: a cursor that moves through sections of cards
//! (shelves, grids and lists) with the arrow keys, Tab between sections and Enter to open.
//!
//! The cursor stays hidden until a key is pressed, and hides again on a click, so mouse users
//! never see it.

use gpui_kit::*;

actions!(
    browse,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        NextSection,
        PrevSection,
        Activate
    ]
);

pub const KEY_CONTEXT: &str = "Browse";

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("tab", NextSection, context),
        KeyBinding::new("shift-tab", PrevSection, context),
        KeyBinding::new("enter", Activate, context),
    ]
}

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
}

impl Cursor {
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
        Some(next)
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

/// Scrolls `handle` by the least amount that brings its child `ix` into view, leaving
/// `margin` around it. Uses the layout from the last frame.
pub fn reveal(handle: &ScrollHandle, ix: usize, margin: Pixels) {
    let Some(child) = handle.bounds_for_item(ix) else {
        return;
    };
    let viewport = handle.bounds();
    let max = handle.max_offset();
    let mut offset = handle.offset();
    // Child bounds are laid out without the scroll offset.
    let axis = |offset: Pixels, start: Pixels, len: Pixels, view_start: Pixels, view_len: Pixels, max: Pixels| {
        let start = start - view_start;
        let end = start + len;
        let visible_start = -offset;
        let next = if len + margin * 2. > view_len || start - margin < visible_start {
            -(start - margin)
        } else if end + margin > visible_start + view_len {
            -(end + margin - view_len)
        } else {
            offset
        };
        next.clamp(-max, px(0.))
    };
    offset.x = axis(
        offset.x,
        child.origin.x,
        child.size.width,
        viewport.origin.x,
        viewport.size.width,
        max.x,
    );
    offset.y = axis(
        offset.y,
        child.origin.y,
        child.size.height,
        viewport.origin.y,
        viewport.size.height,
        max.y,
    );
    handle.set_offset(offset);
}

/// A page with a [`Cursor`]. [`on_browse_actions`] wires up the keys.
pub trait Browse: Sized + 'static {
    fn cursor(&mut self) -> &mut Cursor;

    /// The current layout, in the order sections appear on the page.
    fn sections(&self, window: &Window, cx: &App) -> Vec<Section>;

    /// Scrolls the selected card into view.
    fn reveal(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>);

    /// Enter on a card.
    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>);

    /// A key that would move the cursor past the first or last section.
    fn at_edge(&mut self, _mv: Move, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Left or Right on a control that takes them itself (a picker or a switch), instead of
    /// moving the cursor. Returns whether it did.
    fn adjust(&mut self, _position: Position, _forward: bool, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        false
    }

    fn navigate(&mut self, mv: Move, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(position), Move::Left | Move::Right) = (self.cursor().selected(), mv)
            && self.adjust(position, mv == Move::Right, window, cx)
        {
            cx.notify();
            return;
        }
        let sections = self.sections(window, cx);
        match self.cursor().apply(mv, &sections) {
            Some(position) => {
                self.reveal(position, window, cx);
                cx.notify();
            }
            None => self.at_edge(mv, window, cx),
        }
    }
}

/// Adds the cursor's key handlers to a page's root element, and hides the cursor on a click.
pub fn on_browse_actions<T: Browse, E: InteractiveElement>(element: E, cx: &mut Context<T>) -> E {
    let mv = |mv: Move| move |this: &mut T, window: &mut Window, cx: &mut Context<T>| this.navigate(mv, window, cx);
    let (left, right, up, down, next, prev) = (
        mv(Move::Left),
        mv(Move::Right),
        mv(Move::Up),
        mv(Move::Down),
        mv(Move::NextSection),
        mv(Move::PrevSection),
    );
    element
        .key_context(KEY_CONTEXT)
        .on_action(cx.listener(move |this, _: &MoveLeft, window, cx| left(this, window, cx)))
        .on_action(cx.listener(move |this, _: &MoveRight, window, cx| right(this, window, cx)))
        .on_action(cx.listener(move |this, _: &MoveUp, window, cx| up(this, window, cx)))
        .on_action(cx.listener(move |this, _: &MoveDown, window, cx| down(this, window, cx)))
        .on_action(cx.listener(move |this, _: &NextSection, window, cx| next(this, window, cx)))
        .on_action(cx.listener(move |this, _: &PrevSection, window, cx| prev(this, window, cx)))
        .on_action(cx.listener(|this, _: &Activate, window, cx| {
            match this.cursor().selected() {
                Some(position) => this.activate(position, window, cx),
                // Enter with the cursor hidden shows it, like an arrow key.
                None => this.navigate(Move::Left, window, cx),
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if this.cursor().hide() {
                    cx.notify();
                }
            }),
        )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob would shadow `#[test]`.
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
}
