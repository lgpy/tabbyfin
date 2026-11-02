//! A one-line row of a list: the SyncPlay queue's entries and the Add to playlist dialog's lines.
//! Something at the left, a truncated title, an optional muted detail at the right, and round
//! ↑ ↓ ✕ handles while the row is active.
//!
//! A row is either [`sunk`](ListRow::sunk) on its own (`inset`, `radius_control`), or plain inside
//! a [`block`], one inset that holds the whole list and keeps a long one quiet. Either way it is
//! `inset_hover` while active, under the pointer or a list's cursor. Rows are painted and
//! click-only, as the lists they're in keep the keyboard themselves.

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, CornerRadius, Frame, Id, Margin, Response, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_CLOSE, ICON_PLAY_ARROW};

use crate::theme::{self, text};
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};
use crate::widgets::round_button::{self, RoundButton};

/// A handle's diameter and its icon: one size wherever a list has them, which still leaves 3
/// points above and below in a compact row.
const HANDLE_SIZE: f32 = 30.;
const HANDLE_ICON: f32 = 18.;
const HANDLE_GAP: f32 = 2.;
/// How wide [`handles`] draws its three buttons.
pub const HANDLES_W: f32 = 3. * HANDLE_SIZE + 2. * HANDLE_GAP;

/// The room inside a row at its ends, and between its parts.
const INSET: f32 = 12.;
/// The room a number or play mark takes, wide enough for three digits.
const SLOT: f32 = 36.;
const PLAYING_ICON: f32 = 18.;
/// The room between a row's handles and its ends.
const HANDLES_INSET: f32 = 4.;
/// A regular row's room above and below its title.
const REGULAR_PAD: f32 = 12.;
/// A compact row's height at the default text size, and the room round the title past it.
const COMPACT_H: f32 = 36.;
const COMPACT_PAD: f32 = 11.;
/// The room between a [`block`] and its rows.
const BLOCK_INSET: i8 = 4;

/// A regular row's height at the text size in use, for a list that sizes itself to its rows.
pub fn regular_height(ui: &Ui) -> f32 {
    let line = ui.fonts_mut(|fonts| fonts.row_height(&text::font(ui, "base")));
    line + 2. * REGULAR_PAD
}

/// A handle a row's owner acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Up,
    Down,
    Remove,
}

/// Which of an entry's moves are possible: none past either end of the list.
#[derive(Clone, Copy, Debug)]
pub struct Handles {
    pub up: bool,
    pub down: bool,
}

/// Up, Down and Remove in a line ending at `right`, centred on `y`, for a row that is active.
/// `remove` names the last one, since what it removes from differs. Returns the one clicked.
pub fn handles(ui: &mut Ui, right: f32, y: f32, id: Id, moves: Handles, remove: &str) -> Option<Handle> {
    let mut pressed = None;
    let buttons = [
        (Handle::Up, ICON_ARROW_UPWARD, "Move up (Alt+Up)", moves.up),
        (Handle::Down, ICON_ARROW_DOWNWARD, "Move down (Alt+Down)", moves.down),
        (Handle::Remove, ICON_CLOSE, remove, true),
    ];
    let mut x = right - HANDLES_W + HANDLE_SIZE / 2.;
    for (which, icon, label, enabled) in buttons {
        let button = RoundButton::icon(icon.outlined(), label)
            .size(HANDLE_SIZE, HANDLE_ICON)
            .enabled(enabled)
            .focus(Focus::Skip)
            .show_at(ui, pos2(x, y), id.with(label));
        if button.clicked() {
            pressed = Some(which);
        }
        x += HANDLE_SIZE + HANDLE_GAP;
    }
    pressed
}

/// The one inset round a list of plain rows. A row's corners are concentric with it.
pub fn block<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = theme::tokens_of(ui);
    Frame::new()
        .fill(tokens.inset)
        .stroke(Stroke::new(1., tokens.surface_border))
        .corner_radius(CornerRadius::same(tokens.radius_control))
        .inner_margin(Margin::same(BLOCK_INSET))
        .show(ui, add)
        .inner
}

/// What sits at a row's left end.
#[derive(Clone, Copy)]
pub enum Leading {
    None,
    Icon(MaterialIcon, Color32),
    /// A place in a list, in `xs` and `ink_muted`.
    Number(usize),
    /// The `brand` play mark on what's playing, where its number would be.
    Playing,
}

pub struct ListRow<'a> {
    title: &'a str,
    leading: Leading,
    detail: Option<&'a str>,
    handles: Option<(Handles, &'a str)>,
    sunk: bool,
    compact: bool,
    highlighted: bool,
    ringed: bool,
    hover: bool,
    role: Role,
    label: Option<&'a str>,
}

/// What a row did this frame. `row` covers the whole row; the handles are on top of it and take
/// the clicks on them.
pub struct ListRowResponse {
    pub row: Response,
    pub handle: Option<Handle>,
}

impl<'a> ListRow<'a> {
    /// A regular row in `base`, plain, a button to AccessKit named by its title.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            leading: Leading::None,
            detail: None,
            handles: None,
            sunk: false,
            compact: false,
            highlighted: false,
            ringed: false,
            hover: true,
            role: Role::Button,
            label: None,
        }
    }

    pub fn leading(mut self, leading: Leading) -> Self {
        self.leading = leading;
        self
    }

    /// `sm` and `ink_muted`, at the right end.
    pub fn detail(mut self, detail: Option<&'a str>) -> Self {
        self.detail = detail;
        self
    }

    /// ↑ ↓ ✕ while the row is active, with `remove` naming ✕.
    pub fn handles(mut self, handles: Handles, remove: &'a str) -> Self {
        self.handles = Some((handles, remove));
        self
    }

    /// Sunk on its own rather than plain inside a [`block`].
    pub fn sunk(mut self) -> Self {
        self.sunk = true;
        self
    }

    /// The title in `sm`, in a row 36 tall rather than one line and 24.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// The list's cursor is on it.
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// The keyboard's ring, for a cursor the keyboard put there.
    pub fn ringed(mut self, ringed: bool) -> Self {
        self.ringed = ringed;
        self
    }

    /// Leaves lighting up to `highlighted`, for a list whose highlight follows the pointer: a
    /// pointer left on one row would otherwise light it beside where the keyboard went.
    pub fn no_hover(mut self) -> Self {
        self.hover = false;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// What AccessKit reads, when it's more than the title.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Draws the row across `ui`'s width.
    pub fn show(self, ui: &mut Ui) -> ListRowResponse {
        let tokens = theme::tokens_of(ui);
        let font = text::font(ui, if self.compact { "sm" } else { "base" });
        let height = if self.compact {
            (font.size + 2. * COMPACT_PAD).max(COMPACT_H)
        } else {
            regular_height(ui)
        };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::CLICK);
        response.widget_info(|| WidgetInfo::labeled(self.role, true, self.label.unwrap_or(self.title)));

        // The pointer over a handle is over the row too, so the row asks for the pointer rather
        // than its response's hover, which the handle takes.
        let active = self.highlighted || (self.hover && ui.rect_contains_pointer(rect));
        let radius = if self.sunk {
            tokens.radius_control
        } else {
            tokens.radius_control - BLOCK_INSET as u8
        };
        if active {
            ui.painter().rect_filled(rect, radius, tokens.inset_hover);
        } else if self.sunk {
            ui.painter().rect_filled(rect, radius, tokens.inset);
        }
        if self.ringed {
            focus::ring(ui, rect, radius as f32, Place::Inside);
        }

        let y = rect.center().y;
        let slot = pos2(rect.left() + SLOT / 2., y);
        let left = match self.leading {
            Leading::None => rect.left() + INSET,
            Leading::Icon(icon, colour) => {
                let size = text::font(ui, "lg").size;
                round_button::glyph(ui, pos2(rect.left() + INSET + size / 2., y), icon, size, colour);
                rect.left() + 2. * INSET + size
            }
            Leading::Number(number) => {
                let font = text::font(ui, "xs");
                let painter = ui.painter();
                painter.text(slot, Align2::CENTER_CENTER, number.to_string(), font, tokens.ink_muted);
                rect.left() + SLOT
            }
            Leading::Playing => {
                round_button::glyph(ui, slot, ICON_PLAY_ARROW, PLAYING_ICON, tokens.brand);
                rect.left() + SLOT
            }
        };

        let mut right = rect.right() - INSET;
        let mut handle = None;
        if let Some((moves, remove)) = self.handles.filter(|_| active) {
            let end = rect.right() - HANDLES_INSET;
            handle = handles(ui, end, y, response.id, moves, remove);
            right = end - HANDLES_W - HANDLES_INSET;
        }
        if let Some(detail) = self.detail {
            let galley = ui
                .painter()
                .layout_no_wrap(detail.to_owned(), text::font(ui, "sm"), tokens.ink_muted);
            right -= galley.size().x;
            let at = pos2(right, y - galley.size().y / 2.);
            ui.painter().galley(at, galley, tokens.ink_muted);
            right -= INSET;
        }

        let mut job = LayoutJob::simple_singleline(self.title.to_owned(), font, tokens.ink);
        job.wrap = TextWrapping::truncate_at_width((right - left).max(0.));
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .galley(pos2(left, y - galley.size().y / 2.), galley, tokens.ink);
        ListRowResponse { row: response, handle }
    }
}
