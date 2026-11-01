//! Dialogs over a page: *Add to playlist*, *Edit playlist*, and the confirmation before
//! deleting a playlist.
//!
//! A page keeps the dialog it opened and draws it every frame; [`show`](AddToPlaylist::show)
//! returns `false` once it's done, and the page drops it. They're `egui::Modal`s, so their
//! backdrop blocks the page, and egui keeps the keyboard while one is open (`keys.rs`): each
//! dialog reads the keys it uses itself. That's also why its buttons and rows take no focus
//! (`Sense::CLICK`): Space and Enter belong to the dialog, not to whatever egui would focus.
//! The frame they share (backdrop, width, title, buttons) is `widgets::dialog`.
//!
//! Requests run as `Task`s and report as toasts; a change that worked bumps
//! `services.user_data`, so the pages showing playlists load again. `main` had
//! `views/add_to_playlist.rs`, `views/edit_playlist.rs` and `confirm_delete` in `views/item.rs`.

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2,
    WidgetInfo, pos2, vec2,
};
use egui_material_icons::icons::{ICON_ADD, ICON_CHECK, ICON_PLAYLIST_PLAY};
use jellyfin::ItemsQuery;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemSortBy};

use crate::nav::NavRequest;
use crate::task::Task;
use crate::theme::{self, text};
use crate::toasts::Toast;
use crate::views::PageCx;
use crate::widgets::button::State;
use crate::widgets::focus::{self, Place};
use crate::widgets::popup;
use crate::widgets::text::Load;
use crate::widgets::{dialog, field, label, surface};

/// How wide the dialogs are outside: `main`'s 420 points inside, give or take, with the
/// surface's padding round it.
const WIDTH: f32 = 460.;
/// The playlist list scrolls once it's this tall, as on `main`.
const LIST_MAX_H: f32 = 320.;
/// The room above and below a playlist row's text; the text size sets the rest of its height.
const ROW_PAD: f32 = 12.;
/// The room left of a row's icon, between it and the name, and right of the entry count.
const ROW_INSET: f32 = 12.;
/// The gap between playlist rows.
const ROW_GAP: f32 = 6.;
/// What deleting a playlist does, said plainly before it happens.
const DELETE_PLAYLIST_NOTE: &str =
    "The playlist is removed from the server for everyone it's shared with. What's in it stays in your libraries.";

/// The "Add to playlist" dialog: typing filters the user's playlists, Enter (or a click) adds
/// the item to the highlighted one, or makes a new playlist with the typed name.
pub struct AddToPlaylist {
    item_id: String,
    item_name: String,
    /// A movie or episode is looked for in the playlist first, since 12.2 would add it a
    /// second time. (A series or season adds its episodes; the server sorts those out.)
    check_duplicate: bool,
    playlists: Load<Vec<BaseItemDto>>,
    /// What's in the text field: part of a playlist's name, or the name of a new one.
    typed: String,
    highlighted: usize,
    /// A request is running: the field and the list wait for it.
    busy: bool,
    loading: Option<Task<jellyfin::Result<Vec<BaseItemDto>>>>,
    /// The add (or create) request while it runs, and the playlist it was for.
    working: Option<Task<(String, jellyfin::Result<Outcome>)>>,
    /// The text field asks for the keyboard on the next frame, once it exists.
    focus: bool,
    /// The highlight moved: bring it back on screen.
    scroll_to_highlight: bool,
    /// The keyboard moved the highlight last, rather than the pointer: the highlighted row
    /// gets the focus ring, so it's clear which one Enter picks.
    keyboard: bool,
}

/// How adding to a playlist went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Added,
    Created,
    AlreadyThere,
}

/// A line of the list: an existing playlist, or a new one named by what's typed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    /// `None` for a new playlist, which is made with the typed name.
    playlist_id: Option<String>,
    name: String,
    /// The playlist's entry count, when the server gave one.
    entries: Option<u32>,
}

impl AddToPlaylist {
    /// Opens the dialog on `item`: the playlists load as it shows.
    pub fn new(cx: &mut PageCx, item: &BaseItemDto) -> Self {
        let client = cx.services.session.client.clone();
        let loading = Some(Task::spawn(cx.ctx, async move {
            let query = ItemsQuery {
                include_item_types: vec![BaseItemKind::Playlist],
                recursive: true,
                sort_by: vec![ItemSortBy::SortName],
                ..Default::default()
            };
            Ok(client.items(&query).await?.items)
        }));
        Self {
            item_id: item.id.clone(),
            item_name: item.name.clone(),
            check_duplicate: matches!(
                item.kind,
                BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
            ),
            playlists: Load::Loading,
            typed: String::new(),
            highlighted: 0,
            busy: false,
            loading,
            working: None,
            focus: true,
            scroll_to_highlight: false,
            keyboard: true,
        }
    }

    /// Draws the dialog: `false` once it should be dropped (added, or closed).
    pub fn show(&mut self, cx: &mut PageCx) -> bool {
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.playlists = match result {
                Ok(playlists) => Load::Loaded(playlists),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        if let Some((name, result)) = self.working.as_mut().and_then(Task::poll) {
            self.working = None;
            self.busy = false;
            match result {
                Ok(outcome) => {
                    let toast = match outcome {
                        Outcome::Added => Toast::success(format!("Added to {name}.")),
                        Outcome::Created => Toast::success(format!("Made the playlist {name}.")),
                        Outcome::AlreadyThere => Toast::info(format!("{} is already in {name}.", self.item_name)),
                    };
                    cx.services.toast(toast);
                    // Open playlist pages reload with the new entry.
                    cx.services.user_data += 1;
                    return false;
                }
                // Someone else's playlist, shared for viewing only.
                Err(jellyfin::Error::Forbidden) => {
                    let message = format!("You can't change {name}; it belongs to someone else.");
                    cx.services.toast(Toast::error(message));
                }
                Err(err) => {
                    let message = cx.services.handle_error(&err);
                    cx.services.toast(Toast::error(message));
                }
            }
        }

        let ctx = cx.ctx;
        let mut chosen = None;
        let id = egui::Id::unique("add_to_playlist");
        // What to type goes under the title rather than in the field, which has the keyboard
        // from the start and so never shows a hint (`field::single_line`).
        let subtitle = "Pick one of your playlists, or type a name to make a new one.";
        let modal = dialog::show(ctx, id, WIDTH, "Add to playlist", Some(subtitle), |ui| {
            // Enter picks the highlighted line, as `main`'s dialog's Confirm did; the field
            // never sees it, or it would only give up the focus.
            let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            let field = ui.add(field::single_line(ui, id.with("field"), &mut self.typed, "").interactive(!self.busy));
            if std::mem::take(&mut self.focus) {
                field.request_focus();
            }
            if field.changed() {
                // A different filter: back to the top line.
                self.highlighted = 0;
                self.scroll_to_highlight = true;
                self.keyboard = true;
            }

            // Up and Down move the highlight, wrapping round as a menu's does; the field has
            // nothing to move to on one line, and its own focus filter keeps egui from moving
            // focus with them (`TextEdit` locks the vertical arrows).
            let rows = self.rows();
            self.highlighted = self.highlighted.min(rows.len().saturating_sub(1));
            let (up, down) = ui.input_mut(|input| {
                (
                    input.consume_key(Modifiers::NONE, Key::ArrowUp),
                    input.consume_key(Modifiers::NONE, Key::ArrowDown),
                )
            });
            if !rows.is_empty() && !self.busy {
                if up || down {
                    let step = if down { 1 } else { -1 };
                    self.highlighted = popup::step(Some(self.highlighted), step, rows.len()).unwrap_or(0);
                    self.scroll_to_highlight = true;
                    self.keyboard = true;
                }
                if enter {
                    chosen = Some(self.highlighted);
                }
            }
            ui.add_space(surface::GAP_FIELD);
            self.list(ui, &rows, &mut chosen);
        });

        if let Some(index) = chosen {
            self.choose(index, cx);
        }
        !modal.should_close()
    }

    /// The list under the field: a status line, or the rows, scrolling once it's long. Its
    /// height is worked out from the rows', since a `ScrollArea` given the modal's leftover
    /// height would keep it at the first frame's.
    fn list(&mut self, ui: &mut Ui, rows: &[Row], chosen: &mut Option<usize>) {
        let row_height = row_height(ui);
        let height = (rows.len() as f32 * (row_height + ROW_GAP) - ROW_GAP)
            .min(LIST_MAX_H)
            .max(row_height);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                egui::ScrollArea::vertical().show(ui, |ui| self.lines(ui, rows, chosen));
            },
        );
    }

    /// The list's contents, in the space [`Self::list`] gives it.
    fn lines(&mut self, ui: &mut Ui, rows: &[Row], chosen: &mut Option<usize>) {
        match &self.playlists {
            Load::Loading => {
                ui.label(label::muted(ui, "Loading playlists…"));
            }
            Load::Failed(error) => {
                ui.label(label::muted(ui, error.clone()));
            }
            Load::Loaded(_) if rows.is_empty() => {
                let typed = !self.typed.trim().is_empty();
                let line = if typed {
                    "No playlist has that name."
                } else {
                    "No playlists yet. Type a name to make one."
                };
                ui.label(label::muted(ui, line));
            }
            Load::Loaded(_) => {
                let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
                for (index, row) in rows.iter().enumerate() {
                    let highlighted = index == self.highlighted;
                    let response = playlist_row(ui, row, highlighted, highlighted && self.keyboard);
                    // The pointer takes the highlight, as it does in a menu, so Enter picks the
                    // row under it.
                    if response.hovered() && pointer_moved {
                        self.highlighted = index;
                        self.keyboard = false;
                    }
                    if highlighted && std::mem::take(&mut self.scroll_to_highlight) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() && !self.busy {
                        *chosen = Some(index);
                    }
                }
            }
        }
    }

    /// The list's lines for what's typed, or nothing before the playlists arrived.
    fn rows(&self) -> Vec<Row> {
        match &self.playlists {
            Load::Loaded(playlists) => rows(playlists, &self.typed),
            _ => Vec::new(),
        }
    }

    /// Adds the item to the line at `index`, or makes that playlist and puts the item in it.
    fn choose(&mut self, index: usize, cx: &mut PageCx) {
        if self.busy {
            return;
        }
        let Some(row) = self.rows().get(index).cloned() else {
            return;
        };
        self.busy = true;
        let client = cx.services.session.client.clone();
        let item_id = self.item_id.clone();
        let check_duplicate = self.check_duplicate;
        self.working = Some(Task::spawn(cx.ctx, async move {
            let name = row.name;
            let ids = [item_id.clone()];
            let outcome: jellyfin::Result<Outcome> = async {
                let Some(playlist_id) = row.playlist_id else {
                    client.create_playlist(&name, &ids).await?;
                    return Ok(Outcome::Created);
                };
                if check_duplicate {
                    let entries = client.playlist_items(&playlist_id, &[]).await?;
                    if entries.items.iter().any(|entry| entry.id == item_id) {
                        return Ok(Outcome::AlreadyThere);
                    }
                }
                client.add_to_playlist(&playlist_id, &ids).await?;
                Ok(Outcome::Added)
            }
            .await;
            (name, outcome)
        }));
    }
}

/// The "Edit playlist" dialog: rename a playlist, and share it with everyone on the server or
/// keep it to yourself. Down moves from the name to the sharing switch (Space, Left or Right
/// flip it), Up goes back, and Enter saves only what changed.
pub struct EditPlaylist {
    playlist_id: String,
    /// The name the dialog opened with, to send it only if it changed.
    name: String,
    typed: String,
    /// Shared with everyone on the server; read from the server when the dialog opens.
    public: Load<bool>,
    /// Where the dialog's own keyboard is.
    field: Field,
    busy: bool,
    loading: Option<Task<jellyfin::Result<bool>>>,
    /// The save request while it runs.
    saving: Option<Task<jellyfin::Result<()>>>,
    /// The text field asks for the keyboard on the next frame.
    focus: bool,
}

/// Which of the dialog's two controls the keyboard is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Sharing,
}

impl EditPlaylist {
    /// Opens the dialog on `playlist`: who can see it is fetched as it shows.
    pub fn new(cx: &mut PageCx, playlist: &BaseItemDto) -> Self {
        let client = cx.services.session.client.clone();
        let playlist_id = playlist.id.clone();
        let loading = Some(Task::spawn(cx.ctx, async move {
            client.playlist_is_public(&playlist_id).await
        }));
        Self {
            playlist_id: playlist.id.clone(),
            name: playlist.name.clone(),
            typed: playlist.name.clone(),
            public: Load::Loading,
            field: Field::Name,
            busy: false,
            loading,
            saving: None,
            focus: true,
        }
    }

    /// Draws the dialog: `false` once it should be dropped (saved, or closed).
    pub fn show(&mut self, cx: &mut PageCx) -> bool {
        if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
            self.loading = None;
            self.public = match result {
                Ok(public) => Load::Loaded(public),
                Err(err) => Load::Failed(describe(&err, cx)),
            };
        }
        if let Some(result) = self.saving.as_mut().and_then(Task::poll) {
            self.saving = None;
            self.busy = false;
            match result {
                Ok(()) => {
                    // The playlist's page and library grid show the new name.
                    cx.services.user_data += 1;
                    return false;
                }
                Err(err) => {
                    let message = describe(&err, cx);
                    cx.services.toast(Toast::error(message));
                }
            }
        }

        let ctx = cx.ctx;
        let mut save = false;
        let id = egui::Id::unique("edit_playlist");
        let modal = dialog::show(ctx, id, WIDTH, "Edit playlist", None, |ui| {
            // Enter saves, as `main`'s dialog's Confirm did; the field never sees it.
            let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            if !self.busy {
                self.keys(ui);
            }

            field::label(ui, "Name");
            let field = ui.add(field::single_line(ui, id.with("name"), &mut self.typed, "").interactive(!self.busy));
            if std::mem::take(&mut self.focus) {
                field.request_focus();
            }
            // A click into the name takes the keyboard back from the sharing switch.
            if field.gained_focus() {
                self.field = Field::Name;
            }
            ui.add_space(surface::GAP_FIELD);
            match &self.public {
                Load::Loading => {
                    ui.label(label::muted(ui, "Checking who can see it…"));
                }
                Load::Failed(message) => {
                    ui.label(label::muted(ui, message.clone()));
                }
                Load::Loaded(public) => {
                    let on = *public;
                    if switch_row(ui, on, self.field == Field::Sharing).clicked() && !self.busy {
                        self.field = Field::Sharing;
                        let_keyboard_go(ui);
                        self.public = Load::Loaded(!on);
                    }
                }
            }
            dialog::buttons(ui, |ui| {
                let state = if self.busy {
                    State::Loading
                } else if self.typed.trim().is_empty() {
                    State::Disabled
                } else {
                    State::Ready
                };
                if dialog::main(ui, "Save", state).clicked() {
                    save = true;
                }
                dialog::cancel(ui);
            });
            if enter && !self.busy && !self.typed.trim().is_empty() {
                save = true;
            }
        });

        if save {
            self.save(cx);
        }
        !modal.should_close()
    }

    /// The dialog's own keys: Down to the switch, Up back to the name, Space, Left and Right to
    /// flip it. The name keeps the keys it needs itself, or its cursor couldn't be moved.
    fn keys(&mut self, ui: &mut Ui) {
        match self.field {
            Field::Name => {
                let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
                if down && matches!(self.public, Load::Loaded(_)) {
                    self.field = Field::Sharing;
                    let_keyboard_go(ui);
                }
            }
            Field::Sharing => {
                let (up, left, right, space) = ui.input_mut(|input| {
                    (
                        input.consume_key(Modifiers::NONE, Key::ArrowUp),
                        input.consume_key(Modifiers::NONE, Key::ArrowLeft),
                        input.consume_key(Modifiers::NONE, Key::ArrowRight),
                        input.consume_key(Modifiers::NONE, Key::Space),
                    )
                });
                if up {
                    self.field = Field::Name;
                    self.focus = true;
                }
                let flip = space || left || right;
                if flip && let Load::Loaded(public) = &mut self.public {
                    if left {
                        *public = false;
                    } else if right {
                        *public = true;
                    } else {
                        *public = !*public;
                    }
                }
            }
        }
    }

    /// Saves only what changed; sharing stays as it is if it couldn't be read.
    fn save(&mut self, cx: &mut PageCx) {
        if self.busy {
            return;
        }
        let typed = self.typed.trim().to_owned();
        if typed.is_empty() {
            return;
        }
        let name = (typed != self.name).then_some(typed);
        let public = match self.public {
            Load::Loaded(public) => Some(public),
            _ => None,
        };
        self.busy = true;
        let client = cx.services.session.client.clone();
        let playlist_id = self.playlist_id.clone();
        self.saving = Some(Task::spawn(cx.ctx, async move {
            client.update_playlist(&playlist_id, name.as_deref(), public).await
        }));
    }
}

/// The confirmation before deleting a playlist: its name, what deleting means, and *Delete* or
/// *Cancel*.
pub struct DeletePlaylist {
    playlist_id: String,
    name: String,
    busy: bool,
    /// The delete request while it runs.
    deleting: Option<Task<jellyfin::Result<()>>>,
}

impl DeletePlaylist {
    /// Opens the confirmation on `playlist`.
    pub fn new(playlist: &BaseItemDto) -> Self {
        Self {
            playlist_id: playlist.id.clone(),
            name: playlist.name.clone(),
            busy: false,
            deleting: None,
        }
    }

    /// Draws the confirmation: `false` once it should be dropped (deleted, or closed).
    pub fn show(&mut self, cx: &mut PageCx) -> bool {
        if let Some(result) = self.deleting.as_mut().and_then(Task::poll) {
            self.deleting = None;
            self.busy = false;
            match result {
                Ok(()) => {
                    cx.services.toast(Toast::success(format!("Deleted {}.", self.name)));
                    cx.services.user_data += 1;
                    // Back to the page before the playlist's, which is gone.
                    cx.nav.push(NavRequest::Back);
                    return false;
                }
                Err(jellyfin::Error::Forbidden) => {
                    let message = format!("{} belongs to someone else; it can't be deleted.", self.name);
                    cx.services.toast(Toast::error(message));
                }
                Err(err) => {
                    let message = cx.services.handle_error(&err);
                    cx.services.toast(Toast::error(message));
                }
            }
        }

        let ctx = cx.ctx;
        let mut delete = false;
        let title = format!("Delete “{}”?", self.name);
        let modal = dialog::show(ctx, egui::Id::unique("delete_playlist"), WIDTH, &title, None, |ui| {
            ui.label(DELETE_PLAYLIST_NOTE);
            dialog::buttons(ui, |ui| {
                let state = if self.busy { State::Loading } else { State::Ready };
                if dialog::danger(ui, "Delete", state).clicked() {
                    delete = true;
                }
                dialog::cancel(ui);
            });
            // Enter is the confirmation's Delete, as the alert's OK was on `main`.
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) && !self.busy {
                delete = true;
            }
        });

        if delete {
            self.delete(cx);
        }
        !modal.should_close()
    }

    fn delete(&mut self, cx: &mut PageCx) {
        if self.busy {
            return;
        }
        self.busy = true;
        let client = cx.services.session.client.clone();
        let playlist_id = self.playlist_id.clone();
        self.deleting = Some(Task::spawn(
            cx.ctx,
            async move { client.delete_item(&playlist_id).await },
        ));
    }
}

/// The list's lines for what's typed: every playlist whose name contains it, then a new
/// playlist named by it unless one already has that name.
fn rows(playlists: &[BaseItemDto], typed: &str) -> Vec<Row> {
    let typed = typed.trim();
    let needle = typed.to_lowercase();
    let mut rows: Vec<Row> = playlists
        .iter()
        .filter(|playlist| playlist.name.to_lowercase().contains(&needle))
        .map(|playlist| Row {
            playlist_id: Some(playlist.id.clone()),
            name: playlist.name.clone(),
            entries: playlist.child_count,
        })
        .collect();
    if !typed.is_empty()
        && !playlists
            .iter()
            .any(|playlist| playlist.name.eq_ignore_ascii_case(typed))
    {
        rows.push(Row {
            playlist_id: None,
            name: typed.to_owned(),
            entries: None,
        });
    }
    rows
}

/// A playlist row's height at the text size in use.
fn row_height(ui: &Ui) -> f32 {
    let line = ui.fonts_mut(|fonts| fonts.row_height(&text::font(ui, "base")));
    line + 2. * ROW_PAD
}

/// One line of the playlist list, sunk into the dialog like an `AvatarRow`: the icon, the name,
/// and the entry count at the right end. `inset_hover` while `highlighted`, and the focus ring
/// as well when the keyboard put the highlight there (`ring`). Painted, and click-only, since
/// the dialog keeps the keyboard.
fn playlist_row(ui: &mut Ui, row: &Row, highlighted: bool, ring: bool) -> Response {
    let tokens = theme::tokens_of(ui);
    let (name, icon) = match row.playlist_id {
        Some(_) => (row.name.clone(), ICON_PLAYLIST_PLAY),
        None => (format!("New playlist “{}”", row.name), ICON_ADD),
    };
    let detail = row.entries.map(entry_count);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height(ui)), Sense::CLICK);
    response.widget_info(|| {
        let label = match &detail {
            Some(detail) => format!("{name}, {detail}"),
            None => name.clone(),
        };
        WidgetInfo::labeled(Role::Button, true, label)
    });

    let painter = ui.painter();
    let fill = if highlighted { tokens.inset_hover } else { tokens.inset };
    painter.rect_filled(rect, tokens.radius_control, fill);
    if ring {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }

    let icon_font = FontId::new(text::font(ui, "lg").size, icon.font_family());
    let icon_color = if row.playlist_id.is_some() {
        tokens.ink_muted
    } else {
        tokens.brand
    };
    let glyph = painter.layout_no_wrap(icon.codepoint.to_owned(), icon_font, icon_color);
    // The icon font's glyphs sit low in their line, so they're centred by their bounds.
    let bounds = glyph.rows.first().map_or(glyph.rect, |row| row.rect());
    let icon_center = pos2(rect.left() + ROW_INSET + bounds.width() / 2., rect.center().y);
    let left = icon_center.x + bounds.width() / 2. + ROW_INSET;
    painter.galley(icon_center - bounds.center().to_vec2(), glyph, icon_color);

    let mut right = rect.right() - ROW_INSET;
    if let Some(detail) = detail {
        let galley = painter.layout_no_wrap(detail, text::font(ui, "sm"), tokens.ink_muted);
        right -= galley.size().x;
        painter.galley(
            pos2(right, rect.center().y - galley.size().y / 2.),
            galley,
            tokens.ink_muted,
        );
        right -= ROW_INSET;
    }

    let mut job = LayoutJob::simple_singleline(name, text::font(ui, "base"), tokens.ink);
    job.wrap = TextWrapping::truncate_at_width((right - left).max(0.));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y - galley.size().y / 2.), galley, tokens.ink);
    response
}

/// The sharing switch: a tick box and its label, with the keyboard's ring on it while the
/// dialog's keys are there. Drawn rather than an egui widget so that it takes no focus, which
/// would take Space from the dialog.
fn switch_row(ui: &mut Ui, on: bool, selected: bool) -> Response {
    let padding = 6.;
    let line = ui.text_style_height(&TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), line + 2. * padding), Sense::CLICK);
    let tokens = theme::tokens_of(ui);
    if selected {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }
    let tick_box = Rect::from_min_size(
        pos2(rect.left() + padding, rect.center().y - line * 0.5),
        Vec2::splat(line),
    );
    if on {
        // A checked box: `brand` with its tick in the text on it.
        ui.painter().rect_filled(tick_box, CornerRadius::same(4), tokens.brand);
        ui.painter().text(
            tick_box.center(),
            Align2::CENTER_CENTER,
            ICON_CHECK.codepoint,
            FontId::new(line * 0.8, ICON_CHECK.font_family()),
            tokens.brand_foreground,
        );
    } else {
        // Sunk into the surface, outlined in the muted ink: a panel's outline colour vanishes
        // on the surface it outlines.
        ui.painter().rect(
            tick_box,
            CornerRadius::same(4),
            tokens.inset,
            Stroke::new(1.5, tokens.ink_muted),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        Pos2::new(tick_box.right() + 8., rect.center().y),
        Align2::LEFT_CENTER,
        "Everyone on this server can see it",
        TextStyle::Body.resolve(ui.style()),
        tokens.ink,
    );
    response
}

/// The sharing switch takes no widget focus, so the name must let go of the keyboard on the way
/// to it; otherwise Space would type into the name instead of flipping the switch.
fn let_keyboard_go(ui: &Ui) {
    ui.memory_mut(|memory| memory.stop_text_input());
}

fn entry_count(count: u32) -> String {
    match count {
        1 => "1 item".into(),
        n => format!("{n} items"),
    }
}

/// Another user's playlist answers 404 to changes (and 403 to some reads).
fn describe(err: &jellyfin::Error, cx: &mut PageCx) -> String {
    match err {
        jellyfin::Error::NotFound | jellyfin::Error::Forbidden => {
            "This playlist belongs to someone else; it can't be changed.".to_owned()
        }
        err => cx.services.handle_error(err),
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::{Row, rows};

    fn playlist(id: &str, name: &str, entries: u32) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": name,
            "Type": "Playlist",
            "ChildCount": entries,
        }))
        .expect("playlist")
    }

    fn lines(rows: &[Row]) -> Vec<(&str, Option<&str>, Option<u32>)> {
        rows.iter()
            .map(|row| (row.name.as_str(), row.playlist_id.as_deref(), row.entries))
            .collect()
    }

    #[test]
    fn every_playlist_until_something_is_typed() {
        let playlists = [playlist("a", "Action", 3), playlist("b", "Best of", 1)];
        assert_eq!(
            lines(&rows(&playlists, "")),
            [("Action", Some("a"), Some(3)), ("Best of", Some("b"), Some(1))]
        );
    }

    #[test]
    fn typing_filters_without_regard_to_case() {
        let playlists = [playlist("a", "Action", 3), playlist("b", "Best of", 1)];
        assert_eq!(
            lines(&rows(&playlists, "  be ")),
            [("Best of", Some("b"), Some(1)), ("be", None, None)],
            "a filter that names no playlist also offers to make one"
        );
        assert_eq!(
            lines(&rows(&playlists, "ACT")),
            [("Action", Some("a"), Some(3)), ("ACT", None, None)]
        );
    }

    #[test]
    fn a_name_nobody_has_offers_a_new_playlist() {
        let playlists = [playlist("a", "Action", 3)];
        assert_eq!(
            lines(&rows(&playlists, " Watch later ")),
            // The typed name is trimmed, as the playlist `main` made was.
            [("Watch later", None, None)]
        );
    }

    #[test]
    fn a_name_a_playlist_already_has_offers_no_second_one() {
        let playlists = [playlist("a", "Action", 3)];
        assert_eq!(lines(&rows(&playlists, "action")), [("Action", Some("a"), Some(3))]);
    }

    #[test]
    fn nothing_but_a_new_playlist_before_any_exist() {
        assert_eq!(
            lines(&rows(&[], "First")),
            [("First", None, None)],
            "typing makes the first playlist"
        );
        assert!(rows(&[], "").is_empty());
    }
}
