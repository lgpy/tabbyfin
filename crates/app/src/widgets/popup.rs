//! The app's popup menu: a panel hung from a button, or from the pointer for a right-click, with
//! rows to pick from. Every list-style popup is one of these (an item's More and context menus,
//! the version pickers, the account menu), so they look and behave alike:
//!
//! - the pointer and the keyboard share one highlight, a filled row. A menu opened from the
//!   keyboard starts on its first row; one opened with a click starts on none, until the pointer
//!   or an arrow picks one;
//! - Up and Down move it, wrapping round, Home and End jump to the ends, Enter or Space picks;
//! - Esc, Backspace, Alt+Left and a click outside close it; Ctrl/Cmd+F closes it and tells the
//!   caller to open the search box;
//! - Tab and the horizontal arrows stay in it, so egui's focus pass can't hand the keyboard to a
//!   widget behind it (Enter would then press that as well).
//!
//! Whether a menu is open is egui's popup memory's, so `keys::egui_wants_keys` leaves the
//! keyboard to egui, and so to the menu, while it is. The highlight is kept in egui's temp data
//! under the menu's id: pages keep no menu state of their own. A menu nobody draws any more (its
//! page went, its card scrolled away) is closed by egui at the end of the pass.
//!
//! The rows are painted rather than egui buttons, so none of them takes egui's focus (the menu
//! reads its keys itself), and each tells AccessKit what it is. Lists that aren't menus (a
//! dialog's playlists, the player's tracks) draw the same rows with [`row`] and move their
//! highlight with [`step`], so they look and move like a menu.

use std::borrow::Cow;

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Id, Key, Layout, Margin, Modifiers, PopupAnchor, Rect, RectAlign, Response,
    Sense, SetOpenCommand, Stroke, TextFormat, Ui, WidgetInfo, pos2, vec2,
};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::ICON_CHECK;

use crate::keys;
use crate::motion;
use crate::theme::{self, text};

/// A menu's width unless the caller sets one.
pub const WIDTH: f32 = 220.;
/// The room between the panel's edge and its rows. A row's corners are the panel's radius less
/// this, so they sit concentric inside the panel's.
const PAD: f32 = 4.;
/// A row's height at the default text size; larger text makes the rows taller.
const ROW_H: f32 = 34.;
/// The room left and right of a row's contents, and between its icon and label.
const ROW_INSET: f32 = 10.;
/// The icon or check slot before the labels: kept on every row once one row has an icon, so the
/// labels line up.
const SLOT_W: f32 = 18.;
/// The room between the panel and what it hangs from.
const GAP: f32 = 6.;
/// How far the panel travels out of its button as it opens, and how long it takes.
const SLIDE: f32 = 4.;
const OPEN: f64 = 0.120;
/// Past this height the rows scroll.
const MAX_H: f32 = 420.;

/// How a menu was opened, which decides where its highlight starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    Pointer,
    Keyboard,
}

/// A row of a [`PopupMenu`].
pub struct Entry<'a> {
    label: Cow<'a, str>,
    icon: Option<MaterialIcon>,
    tint: Option<Color32>,
    checked: Option<bool>,
    ticked: Option<bool>,
    toggled: Option<bool>,
    detail: Option<Cow<'a, str>>,
    danger: bool,
    separated: bool,
}

impl<'a> Entry<'a> {
    pub fn new(label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            tint: None,
            checked: None,
            ticked: None,
            toggled: None,
            detail: None,
            danger: false,
            separated: false,
        }
    }

    /// An icon before the label.
    pub fn icon(mut self, icon: MaterialIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The icon in a colour of its own (a watched tick's green, a favorite's red).
    pub fn tint(mut self, color: Color32) -> Self {
        self.tint = Some(color);
        self
    }

    /// One of a set of choices, with a check before the chosen one: a version, an account.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// One of any number ticked at once, with a check while it's ticked: a genre to filter by.
    pub fn ticked(mut self, ticked: bool) -> Self {
        self.ticked = Some(ticked);
        self
    }

    /// A state turned on or off by picking it (Watched, Favorite), which its icon shows.
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }

    /// Muted text at the row's end.
    pub fn detail(mut self, detail: impl Into<Cow<'a, str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Something that can't be taken back, in the danger colour.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// A line above the row, setting it apart from the ones before.
    pub fn separated(mut self) -> Self {
        self.separated = true;
        self
    }

    fn has_slot(&self) -> bool {
        self.icon.is_some() || self.checked.is_some() || self.ticked.is_some()
    }

    fn has_check(&self) -> bool {
        self.checked == Some(true) || self.ticked == Some(true)
    }
}

/// What happened in a menu this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuResponse {
    /// The index of the entry picked; the menu has closed.
    pub chosen: Option<usize>,
    /// Ctrl/Cmd+F closed the menu: the caller opens the search box.
    pub search: bool,
}

/// What a menu remembers between frames, in egui's temp data.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    highlight: Option<usize>,
    /// The pointer put the highlight where it is, so it goes when the pointer leaves the rows.
    by_pointer: bool,
}

/// A popup menu under `id`, opened with [`PopupMenu::open`] or [`PopupMenu::toggle`] (or
/// [`PopupMenu::opening_at_pointer`] for a right-click) and drawn every frame with
/// [`PopupMenu::show`], which does nothing while it's closed.
pub struct PopupMenu {
    id: Id,
    align: RectAlign,
    width: f32,
    at_pointer: bool,
    opening: bool,
}

impl PopupMenu {
    pub fn new(id: Id) -> Self {
        Self {
            id,
            align: RectAlign::BOTTOM_START,
            width: WIDTH,
            at_pointer: false,
            opening: false,
        }
    }

    /// Where the panel goes against what it hangs from, when it fits there; egui tries the
    /// other sides when it doesn't.
    pub fn align(mut self, align: RectAlign) -> Self {
        self.align = align;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// A context menu: it hangs from where the pointer was when it opened, and opens this frame
    /// when `open` (the anchor was right-clicked).
    pub fn opening_at_pointer(mut self, open: bool) -> Self {
        self.at_pointer = true;
        self.opening = open;
        self
    }

    pub fn is_open(ctx: &egui::Context, id: Id) -> bool {
        egui::Popup::is_id_open(ctx, id)
    }

    pub fn open(ctx: &egui::Context, id: Id, opened: Opened) {
        reset(ctx, id, opened);
        egui::Popup::open_id(ctx, id);
        ctx.request_repaint();
    }

    pub fn close(ctx: &egui::Context, id: Id) {
        egui::Popup::close_id(ctx, id);
    }

    /// Opens the menu, or closes it when it's open: what clicking its button does.
    pub fn toggle(ctx: &egui::Context, id: Id, opened: Opened) {
        if Self::is_open(ctx, id) {
            Self::close(ctx, id);
        } else {
            Self::open(ctx, id, opened);
        }
    }

    /// Draws the menu hung from `anchor` while it's open, with `entries`, and reads its keys.
    pub fn show(self, anchor: &Response, entries: &[Entry]) -> MenuResponse {
        let ctx = anchor.ctx.clone();
        let id = self.id;
        let mut out = MenuResponse::default();
        if self.opening {
            reset(&ctx, id, Opened::Pointer);
        } else if !Self::is_open(&ctx, id) {
            return out;
        }
        let mut state = ctx.data(|data| data.get_temp::<State>(state_id(id)).unwrap_or_default());

        // The menu's keys. egui gives them to nothing else while a popup is open, but would still
        // move its focus behind the menu with Tab or an arrow.
        let take = |modifiers, key| keys::take_exact(&ctx, modifiers, key);
        if take(Modifiers::NONE, Key::Tab)
            | take(Modifiers::SHIFT, Key::Tab)
            | take(Modifiers::NONE, Key::ArrowLeft)
            | take(Modifiers::NONE, Key::ArrowRight)
        {
            cancel_focus(&ctx);
        }
        if take(Modifiers::NONE, Key::Backspace) | take(Modifiers::ALT, Key::ArrowLeft) {
            Self::close(&ctx, id);
            return out;
        }
        if take(Modifiers::COMMAND, Key::F) {
            Self::close(&ctx, id);
            out.search = true;
            return out;
        }
        let mut moved = false;
        let down = take(Modifiers::NONE, Key::ArrowDown);
        let up = take(Modifiers::NONE, Key::ArrowUp);
        if down | up {
            state.highlight = step(state.highlight, if down { 1 } else { -1 }, entries.len());
            moved = true;
        }
        if take(Modifiers::NONE, Key::Home) {
            state.highlight = (!entries.is_empty()).then_some(0);
            moved = true;
        }
        if take(Modifiers::NONE, Key::End) {
            state.highlight = entries.len().checked_sub(1);
            moved = true;
        }
        if moved {
            state.by_pointer = false;
            cancel_focus(&ctx);
        }
        if (take(Modifiers::NONE, Key::Enter) | take(Modifiers::NONE, Key::Space))
            && let Some(index) = state.highlight.filter(|&index| index < entries.len())
        {
            out.chosen = Some(index);
        }

        // Opening, the panel fades in and moves out of what it hangs from.
        let t = if theme::animations(&ctx) {
            let now = ctx.input(|input| input.time);
            let t = motion::progress(motion::appear(&ctx, id), now, 0., OPEN);
            if t < 1. {
                ctx.request_repaint();
            }
            t
        } else {
            1.
        };

        let tokens = theme::tokens(&ctx.global_style().visuals);
        let frame = egui::Frame::new()
            .fill(tokens.popover)
            .stroke(Stroke::new(1., tokens.border))
            .corner_radius(CornerRadius::same(tokens.radius_lg))
            .inner_margin(Margin::same(PAD as i8))
            .shadow(egui::epaint::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(if tokens.dark { 140 } else { 36 }),
            });
        let anchor_at = if self.at_pointer {
            PopupAnchor::PointerFixed
        } else {
            PopupAnchor::from(anchor)
        };
        let open_memory = if self.opening {
            Some(SetOpenCommand::Bool(true))
        } else {
            None
        };
        let slot = entries.iter().any(Entry::has_slot);
        egui::Popup::new(id, ctx.clone(), anchor_at, anchor.layer_id)
            .kind(egui::PopupKind::Popup)
            .open_memory(open_memory)
            .align(self.align)
            .gap(GAP - SLIDE * (1. - t))
            .width(self.width)
            .layout(Layout::top_down_justified(Align::Min))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(frame)
            .show(|ui| {
                ui.set_opacity(t);
                let inner = self.width - 2. * PAD;
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = 0.;
                egui::ScrollArea::vertical().max_height(MAX_H).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.;
                    let pointer_moved = ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO);
                    let mut hovered = None;
                    for (index, entry) in entries.iter().enumerate() {
                        if entry.separated && index > 0 {
                            separator(ui);
                        }
                        let height = row_height(ui);
                        let (rect, response) = ui.allocate_exact_size(vec2(inner, height), Sense::CLICK);
                        if response.hovered() {
                            hovered = Some(index);
                            if pointer_moved {
                                state.highlight = Some(index);
                                state.by_pointer = true;
                            }
                        }
                        let highlighted = state.highlight == Some(index);
                        if highlighted && moved {
                            ui.scroll_to_rect(rect, None);
                        }
                        paint_row(ui, rect, entry, slot, highlighted);
                        response.widget_info(|| row_info(entry, true));
                        if response.clicked() {
                            out.chosen = Some(index);
                        }
                    }
                    if hovered.is_none() && pointer_moved && state.by_pointer {
                        state.highlight = None;
                    }
                });
            });

        if out.chosen.is_some() {
            Self::close(&ctx, id);
        }
        ctx.data_mut(|data| data.insert_temp(state_id(id), state));
        out
    }
}

fn state_id(id: Id) -> Id {
    id.with("popup_menu_state")
}

/// Puts the highlight where a menu opened `opened` starts.
fn reset(ctx: &egui::Context, id: Id, opened: Opened) {
    let state = State {
        highlight: (opened == Opened::Keyboard).then_some(0),
        by_pointer: false,
    };
    ctx.data_mut(|data| data.insert_temp(state_id(id), state));
}

/// One of a menu's rows in a list of the caller's own, `ui`'s width, filled while `highlighted`
/// or under the pointer. It takes no focus: the list reads its keys itself. `slot` keeps the
/// icon's room on a row without one, as a menu does once any of its rows has one.
pub fn row(ui: &mut Ui, entry: &Entry, slot: bool, highlighted: bool) -> Response {
    let height = row_height(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::CLICK);
    paint_row(
        ui,
        rect,
        entry,
        slot || entry.has_slot(),
        highlighted || response.hovered(),
    );
    response.widget_info(|| row_info(entry, false));
    response
}

/// A row's height, taller with larger text.
pub fn row_height(ui: &Ui) -> f32 {
    ROW_H.max(text::font(ui, "sm").size + 20.)
}

/// Up and Down wrap round. In a menu opened with a click, Down starts at the top and Up at the
/// bottom.
pub fn step(current: Option<usize>, step: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match current {
        Some(index) => (index as i32 + step).rem_euclid(len as i32) as usize,
        None if step > 0 => 0,
        None => len - 1,
    })
}

/// Cancels the focus move egui queued for a Tab or an arrow pressed while a menu is open.
fn cancel_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
}

/// A line across the panel, edge to edge, with room above and below.
fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2. * PAD + 1.), Sense::hover());
    let y = rect.center().y;
    ui.painter().hline(
        (rect.left() - PAD)..=(rect.right() + PAD),
        y,
        Stroke::new(1., theme::tokens_of(ui).border),
    );
}

fn paint_row(ui: &Ui, rect: Rect, entry: &Entry, slot: bool, highlighted: bool) {
    let tokens = theme::tokens_of(ui);
    let painter = ui.painter();
    let color = if entry.danger { tokens.danger } else { tokens.foreground };
    if highlighted {
        let fill = if entry.danger {
            tokens.danger.gamma_multiply(0.14)
        } else {
            tokens.accent
        };
        painter.rect_filled(rect, CornerRadius::same(tokens.radius_lg - PAD as u8), fill);
    }

    let mut x = rect.left() + ROW_INSET;
    if slot {
        let icon = if entry.has_check() {
            Some(ICON_CHECK)
        } else {
            entry.icon
        };
        if let Some(icon) = icon {
            painter.text(
                pos2(x + SLOT_W / 2., rect.center().y),
                Align2::CENTER_CENTER,
                icon.codepoint,
                text::icon(ui, icon.font_family()),
                entry.tint.unwrap_or(color),
            );
        }
        x += SLOT_W + ROW_INSET;
    }

    let mut right = rect.right() - ROW_INSET;
    if let Some(detail) = &entry.detail {
        let galley = painter.layout_no_wrap(detail.to_string(), text::font(ui, "xs"), tokens.muted_foreground);
        right -= galley.size().x;
        painter.galley(
            pos2(right, rect.center().y - galley.size().y / 2.),
            galley,
            tokens.muted_foreground,
        );
        right -= ROW_INSET;
    }

    // A label too long for the row ends in an ellipsis.
    let mut job = LayoutJob::single_section(
        entry.label.to_string(),
        TextFormat {
            font_id: text::font(ui, "sm"),
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width((right - x).max(0.));
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.), galley, color);
}

/// What a row is to AccessKit: a menu item in a menu, a radio button, check box or button in a
/// list of its own. Its name has the detail too: two accounts with the same name on different
/// servers differ only there.
fn row_info(entry: &Entry, in_menu: bool) -> WidgetInfo {
    let name = match &entry.detail {
        Some(detail) => format!("{}, {detail}", entry.label),
        None => entry.label.to_string(),
    };
    let (radio, check_box, item) = if in_menu {
        (Role::MenuItemRadio, Role::MenuItemCheckBox, Role::MenuItem)
    } else {
        (Role::RadioButton, Role::CheckBox, Role::Button)
    };
    match (entry.checked, entry.ticked.or(entry.toggled)) {
        (Some(checked), _) => WidgetInfo::selected(radio, true, checked, name),
        (None, Some(on)) => WidgetInfo::selected(check_box, true, on, name),
        (None, None) => WidgetInfo::labeled(item, true, name),
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers};

    use super::{Entry, MenuResponse, Opened, PopupMenu, step};

    #[test]
    fn the_highlight_wraps_round() {
        assert_eq!(step(Some(0), 1, 3), Some(1));
        assert_eq!(step(Some(2), 1, 3), Some(0));
        assert_eq!(step(Some(0), -1, 3), Some(2));
        // A menu opened with a click: Down starts at the top, Up at the bottom.
        assert_eq!(step(None, 1, 3), Some(0));
        assert_eq!(step(None, -1, 3), Some(2));
        assert_eq!(step(Some(0), 1, 0), None);
    }

    /// Runs one frame with `keys` pressed, a button and its menu of three rows in it, the menu
    /// opened before the frame as `opened` (or left closed).
    fn frame(ctx: &egui::Context, opened: Option<Opened>, keys: &[(Modifiers, Key)]) -> (MenuResponse, Vec<Event>) {
        let id = egui::Id::unique("menu");
        if let Some(opened) = opened {
            PopupMenu::open(ctx, id, opened);
        }
        let mut input = egui::RawInput::default();
        for &(modifiers, key) in keys {
            input.events.push(Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        let mut response = MenuResponse::default();
        let mut left = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            let button = ui.button("More");
            let entries = [Entry::new("One"), Entry::new("Two"), Entry::new("Three").separated()];
            response = PopupMenu::new(id).show(&button, &entries);
            left = ui.input(|input| input.events.clone());
        });
        // The deltas are the renderer's to apply; a test has none.
        output.textures_delta.clear();
        (response, left)
    }

    #[test]
    fn the_keyboard_walks_and_picks() {
        let ctx = egui::Context::default();
        // Opened from the keyboard it starts on the first row: two Downs and Enter pick the third.
        let down = (Modifiers::NONE, Key::ArrowDown);
        let (response, _) = frame(&ctx, Some(Opened::Keyboard), &[down]);
        assert_eq!(response.chosen, None);
        let (response, left) = frame(&ctx, None, &[down, (Modifiers::NONE, Key::Enter)]);
        assert_eq!(response.chosen, Some(2));
        assert!(left.is_empty());
        assert!(!PopupMenu::is_open(&ctx, egui::Id::unique("menu")));
    }

    #[test]
    fn enter_in_a_clicked_open_menu_waits_for_a_highlight() {
        let ctx = egui::Context::default();
        let (response, _) = frame(&ctx, Some(Opened::Pointer), &[(Modifiers::NONE, Key::Enter)]);
        assert_eq!(response.chosen, None);
        // Up from nothing is the last row.
        let (response, _) = frame(
            &ctx,
            None,
            &[(Modifiers::NONE, Key::ArrowUp), (Modifiers::NONE, Key::Enter)],
        );
        assert_eq!(response.chosen, Some(2));
    }

    #[test]
    fn back_keys_close_it_and_ctrl_f_asks_for_search() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("menu");
        let (_, left) = frame(&ctx, Some(Opened::Keyboard), &[(Modifiers::NONE, Key::Backspace)]);
        assert!(left.is_empty());
        assert!(!PopupMenu::is_open(&ctx, id));

        let (_, _) = frame(&ctx, Some(Opened::Keyboard), &[(Modifiers::ALT, Key::ArrowLeft)]);
        assert!(!PopupMenu::is_open(&ctx, id));

        let command = Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        };
        let (response, left) = frame(&ctx, Some(Opened::Keyboard), &[(command, Key::F)]);
        assert!(response.search);
        assert!(left.is_empty());
        assert!(!PopupMenu::is_open(&ctx, id));
    }

    #[test]
    fn rows_outside_a_menu_say_what_they_are() {
        use egui::accesskit::Role;

        let info = |entry: &Entry| super::row_info(entry, false);
        assert_eq!(info(&Entry::new("Drama").ticked(true)).role, Role::CheckBox);
        assert_eq!(info(&Entry::new("Drama").ticked(true)).selected, Some(true));
        assert_eq!(info(&Entry::new("English").checked(false)).role, Role::RadioButton);
        assert_eq!(info(&Entry::new("Watch later")).role, Role::Button);
        assert_eq!(
            super::row_info(&Entry::new("Drama").ticked(false), true).role,
            Role::MenuItemCheckBox
        );
    }

    #[test]
    fn a_closed_menu_leaves_the_keys_alone() {
        let ctx = egui::Context::default();
        let keys = [(Modifiers::NONE, Key::Tab), (Modifiers::NONE, Key::Backspace)];
        let (response, left) = frame(&ctx, None, &keys);
        assert_eq!(response, MenuResponse::default());
        assert_eq!(left.len(), 2);
    }

    #[test]
    fn tab_and_the_horizontal_arrows_stay_in_an_open_menu() {
        let ctx = egui::Context::default();
        let keys = [
            (Modifiers::NONE, Key::Tab),
            (Modifiers::SHIFT, Key::Tab),
            (Modifiers::NONE, Key::ArrowLeft),
            (Modifiers::NONE, Key::ArrowRight),
        ];
        let (_, left) = frame(&ctx, Some(Opened::Pointer), &keys);
        assert!(left.is_empty());
        assert!(PopupMenu::is_open(&ctx, egui::Id::unique("menu")));
    }
}
