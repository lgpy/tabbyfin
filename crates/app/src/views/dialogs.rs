//! Dialogs over a page: *Add to playlist*, *Edit playlist*, and the confirmation before
//! deleting a playlist.
//!
//! A page keeps the dialog it opened and draws it every frame; [`show`](AddToPlaylist::show)
//! returns `false` once it's done, and the page drops it. They're `egui::Modal`s, so their
//! backdrop blocks the page, and egui keeps the keyboard while one is open (`keys.rs`): each
//! dialog reads the keys it uses itself. That's also why its buttons and rows take no focus
//! (`Sense::CLICK`): Space and Enter belong to the dialog, not to whatever egui would focus.
//!
//! Requests run as `Task`s and report as toasts; a change that worked bumps
//! `services.user_data`, so the pages showing playlists load again. `main` had
//! `views/add_to_playlist.rs`, `views/edit_playlist.rs` and `confirm_delete` in `views/item.rs`.

use egui::{
    Align2, Button, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextEdit, TextStyle, Ui, Vec2, pos2, vec2,
};
use egui_material_icons::icons::{ICON_ADD, ICON_CHECK, ICON_PLAYLIST_PLAY};
use jellyfin::ItemsQuery;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemSortBy};

use crate::nav::NavRequest;
use crate::task::Task;
use crate::theme;
use crate::toasts::Toast;
use crate::views::PageCx;
use crate::widgets::popup::{self, Entry};
use crate::widgets::text::Load;

/// How wide the dialogs are, as `main`'s were (420 points).
const WIDTH: f32 = 420.;
/// The playlist list scrolls once it's this tall, as on `main`.
const LIST_MAX_H: f32 = 320.;
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
    /// A row's height, measured as the rows are drawn: the list's height is worked out from it,
    /// since a `ScrollArea` given the modal's leftover height would keep it at the first frame's.
    row_height: f32,
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
            row_height: 26.,
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
        // `main`'s dialogs dimmed the page under them with the theme's `overlay`.
        let backdrop = theme::tokens(&ctx.style_of(ctx.theme()).visuals).overlay;
        let modal = egui::Modal::new(egui::Id::unique("add_to_playlist"))
            .backdrop_color(backdrop)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.heading("Add to playlist");
                ui.add_space(8.);
                // Enter picks the highlighted line, as `main`'s dialog's Confirm did; the field
                // never sees it, or it would only give up the focus.
                let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                let field = ui.add(
                    TextEdit::singleline(&mut self.typed)
                        .interactive(!self.busy)
                        .hint_text("Find a playlist or name a new one")
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus) {
                    field.request_focus();
                }
                if field.changed() {
                    // A different filter: back to the top line.
                    self.highlighted = 0;
                    self.scroll_to_highlight = true;
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
                    }
                    if enter {
                        chosen = Some(self.highlighted);
                    }
                }
                ui.add_space(4.);
                self.list(ui, &rows, &mut chosen);
            });

        if let Some(index) = chosen {
            self.choose(index, cx);
        }
        !modal.should_close()
    }

    /// The list under the field: a status line, or the rows, scrolling once it's long.
    fn list(&mut self, ui: &mut Ui, rows: &[Row], chosen: &mut Option<usize>) {
        let gap = ui.spacing().item_spacing.y;
        let height = (rows.len() as f32 * (self.row_height + gap))
            .min(LIST_MAX_H)
            .max(self.row_height);
        let mut measured = self.row_height;
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.lines(ui, rows, chosen, &mut measured));
            },
        );
        self.row_height = measured;
    }

    /// The list's contents, in the space [`Self::list`] gives it.
    fn lines(&mut self, ui: &mut Ui, rows: &[Row], chosen: &mut Option<usize>, measured: &mut f32) {
        match &self.playlists {
            Load::Loading => muted(ui, "Loading playlists…"),
            Load::Failed(error) => muted(ui, error.clone()),
            Load::Loaded(_) if rows.is_empty() => {
                let typed = !self.typed.trim().is_empty();
                muted(
                    ui,
                    if typed {
                        "No playlist has that name."
                    } else {
                        "No playlists yet. Type a name to make one."
                    },
                );
            }
            Load::Loaded(_) => {
                let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
                for (index, row) in rows.iter().enumerate() {
                    let highlighted = index == self.highlighted;
                    let response = popup::row(ui, &entry(row), true, highlighted);
                    *measured = response.rect.height();
                    // The pointer takes the highlight, as it does in a menu, so Enter picks the
                    // row under it.
                    if response.hovered() && pointer_moved {
                        self.highlighted = index;
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
        let backdrop = theme::tokens(&ctx.style_of(ctx.theme()).visuals).overlay;
        let modal = egui::Modal::new(egui::Id::unique("edit_playlist"))
            .backdrop_color(backdrop)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.heading("Edit playlist");
                ui.add_space(8.);
                // Enter saves, as `main`'s dialog's Confirm did; the field never sees it.
                let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
                if !self.busy {
                    self.keys(ui);
                }

                muted(ui, "Name");
                let field = ui.add(
                    TextEdit::singleline(&mut self.typed)
                        .interactive(!self.busy)
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus) {
                    field.request_focus();
                }
                // A click into the name takes the keyboard back from the sharing switch.
                if field.gained_focus() {
                    self.field = Field::Name;
                }
                ui.add_space(8.);
                match &self.public {
                    Load::Loading => muted(ui, "Checking who can see it…"),
                    Load::Failed(message) => muted(ui, message.clone()),
                    Load::Loaded(public) => {
                        let on = *public;
                        if switch_row(ui, on, self.field == Field::Sharing).clicked() && !self.busy {
                            self.field = Field::Sharing;
                            let_keyboard_go(ui);
                            self.public = Load::Loaded(!on);
                        }
                    }
                }
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_save = !self.busy && !self.typed.trim().is_empty();
                        if ui
                            .add_enabled(can_save, Button::new("Save").sense(Sense::CLICK))
                            .clicked()
                        {
                            save = true;
                        }
                        if ui.add(Button::new("Cancel").sense(Sense::CLICK)).clicked() {
                            ui.close();
                        }
                    });
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
        let backdrop = theme::tokens(&ctx.style_of(ctx.theme()).visuals).overlay;
        let modal = egui::Modal::new(egui::Id::unique("delete_playlist"))
            .backdrop_color(backdrop)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.heading(format!("Delete “{}”?", self.name));
                ui.add_space(8.);
                ui.label(DELETE_PLAYLIST_NOTE);
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, Button::new("Delete").sense(Sense::CLICK))
                            .clicked()
                        {
                            delete = true;
                        }
                        if ui.add(Button::new("Cancel").sense(Sense::CLICK)).clicked() {
                            ui.close();
                        }
                    });
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

/// One line of the playlist list, as a menu's row: the icon, the name, and the entry count at
/// the right end.
fn entry(row: &Row) -> Entry<'static> {
    match row.playlist_id {
        Some(_) => {
            let entry = Entry::new(row.name.clone()).icon(ICON_PLAYLIST_PLAY);
            match row.entries {
                Some(count) => entry.detail(entry_count(count)),
                None => entry,
            }
        }
        None => Entry::new(format!("New playlist “{}”", row.name)).icon(ICON_ADD),
    }
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
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(3., tokens.foreground),
            StrokeKind::Inside,
        );
    }
    let tick_box = Rect::from_min_size(
        pos2(rect.left() + padding, rect.center().y - line * 0.5),
        Vec2::splat(line),
    );
    if on {
        // `main`'s checked box: the primary colour with its tick in the foreground.
        ui.painter()
            .rect_filled(tick_box, CornerRadius::same(4), tokens.primary);
        ui.painter().text(
            tick_box.center(),
            Align2::CENTER_CENTER,
            ICON_CHECK.codepoint,
            FontId::new(line * 0.8, ICON_CHECK.font_family()),
            tokens.primary_foreground,
        );
    } else {
        ui.painter().rect_stroke(
            tick_box,
            CornerRadius::same(4),
            Stroke::new(1., tokens.input),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        Pos2::new(tick_box.right() + 8., rect.center().y),
        Align2::LEFT_CENTER,
        "Everyone on this server can see it",
        TextStyle::Body.resolve(ui.style()),
        tokens.foreground,
    );
    response
}

/// A line of muted text: what the dialog is waiting for, or why there's nothing to pick.
fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).color(theme::tokens_of(ui).muted_foreground));
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
