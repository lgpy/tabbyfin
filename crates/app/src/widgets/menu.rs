//! An item's context menu: play or resume it, play a series or season from its next episode,
//! mark it watched or favorite, add it to a playlist. The rules (what's offered for which kind,
//! what's checked) are [`crate::item_menu`]'s.
//!
//! The menu returns what was chosen instead of carrying it out. [`Services::carry_out`] does
//! everything only the services can — playing, playing a series' next episode, marking it
//! watched or favorite — and hands back *Add to playlist…* for the page to open U8's dialog.
//! `main`'s menu did all of it itself, through GPUI globals.
//!
//! Each action that takes a request first ([`Services::mark`], [`Services::play_up_next`],
//! [`Services::play_shuffled`]) runs it on tokio and reports back in
//! [`Services::apply_menu_requests`], which `Services::tick` calls, so no page owns a task for
//! it.
//!
//! egui's menus don't take keys, so the context menu is pointer-only for now, like the account
//! menu (S1); the item page's own More menu draws and reads its keys itself (`views/item.rs`).

use std::collections::VecDeque;

use egui::text::LayoutJob;
use egui::{Button, Response, RichText, TextFormat, Ui, WidgetText};
use egui_material_icons::icons::{ICON_CHECK, ICON_CHECK_CIRCLE, ICON_FAVORITE, ICON_PLAY_ARROW, ICON_PLAYLIST_ADD};

use jellyfin::models::{BaseItemDto, BaseItemKind};

use crate::item_menu::{self, Mark, can_add_to_playlist, can_mark, is_favorite, is_playable, is_played, resume_point};
use crate::playback::PlayOptions;
use crate::services::{Marked, Services, Shuffled, UpNext};
use crate::theme;
use crate::theme::text;
use crate::toasts::Toast;
use crate::views::item::shuffle;
use crate::widgets::text::timestamp;

/// What a choice in [`item_menu`] asks for.
#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    /// Play the item, resuming at `start` if it's partway through.
    Play { start: Option<jellyfin::Ticks> },
    /// Play a series or season from its next episode (`item_menu::first_unwatched`).
    UpNext,
    /// Set watched or favorite state.
    Mark(Mark),
    /// Open U8's *Add to playlist* dialog.
    AddToPlaylist,
}

/// The menu worn by `response`, as `main`'s `with_item_menu` did: right-click it to open it.
/// Kinds with none of the entries (libraries, people) get no menu.
pub fn item_menu(response: &Response, item: &BaseItemDto) -> Option<MenuAction> {
    if !can_mark(item) {
        return None;
    }
    let mut chosen = None;
    response.context_menu(|ui| {
        if let Some(action) = entries(ui, item) {
            chosen = Some(action);
            ui.close();
        }
    });
    chosen
}

/// The menu's entries for `item`, in a `Ui`: what the context menu shows, and what the item
/// page's *More* menu will show (U6).
pub fn entries(ui: &mut Ui, item: &BaseItemDto) -> Option<MenuAction> {
    let mut chosen = None;
    if is_playable(item) {
        match resume_point(item) {
            Some(position) => {
                if icon_entry(
                    ui,
                    ICON_PLAY_ARROW,
                    &format!("Resume from {}", timestamp(position.into())),
                )
                .clicked()
                {
                    chosen = Some(MenuAction::Play { start: Some(position) });
                }
                if ui.button("Play from beginning").clicked() {
                    chosen = Some(MenuAction::Play { start: None });
                }
            }
            None => {
                if icon_entry(ui, ICON_PLAY_ARROW, "Play").clicked() {
                    chosen = Some(MenuAction::Play { start: None });
                }
            }
        }
        ui.separator();
    } else if matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season) {
        // Which episode, and whether it resumes, takes a request; the item page's button shows
        // it, and this asks the page to play it.
        if icon_entry(ui, ICON_PLAY_ARROW, "Play").clicked() {
            chosen = Some(MenuAction::UpNext);
        }
        ui.separator();
    }
    // Checked like the item page's toggles, with the same names.
    let mut played = is_played(item);
    if ui.checkbox(&mut played, "Watched").clicked() {
        chosen = Some(MenuAction::Mark(Mark::Played(played)));
    }
    let mut favorite = is_favorite(item);
    if ui.checkbox(&mut favorite, "Favorite").clicked() {
        chosen = Some(MenuAction::Mark(Mark::Favorite(favorite)));
    }
    if can_add_to_playlist(item) {
        ui.separator();
        if icon_entry(ui, ICON_PLAYLIST_ADD, "Add to playlist…").clicked() {
            chosen = Some(MenuAction::AddToPlaylist);
        }
    }
    chosen
}

/// Watched's icon: a check, filled in green once it's watched (`main`'s `success`).
pub fn played_icon(ui: &Ui, played: bool) -> RichText {
    if played {
        ICON_CHECK_CIRCLE.rich_text().color(theme::tokens_of(ui).success)
    } else {
        ICON_CHECK.outlined().rich_text()
    }
}

/// Favorite's icon: a heart, filled in red once it's a favorite (`main`'s base red).
pub fn favorite_icon(ui: &Ui, favorite: bool) -> RichText {
    if favorite {
        ICON_FAVORITE.rich_text().color(theme::tokens_of(ui).red)
    } else {
        ICON_FAVORITE.outlined().rich_text()
    }
}

/// A menu entry with `icon` before its label, as `main`'s menu items had.
fn icon_entry(ui: &mut Ui, icon: egui_material_icons::MaterialIcon, label: &str) -> Response {
    let icon_font = text::icon(ui, icon.font_family());
    let color = theme::tokens_of(ui).foreground;
    let mut job = LayoutJob::default();
    job.append(
        icon.codepoint,
        0.,
        TextFormat {
            font_id: icon_font,
            color,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {label}"),
        0.,
        TextFormat {
            font_id: text::font(ui, "base"),
            color,
            ..Default::default()
        },
    );
    ui.add(Button::new(WidgetText::LayoutJob(job.into())))
}

/// How many refused marks the services remember. A page left off screen catches up with the
/// newest ones (it takes back each refusal once); past this many the page is showing data the
/// server has long replaced, and a successful mark reloads it anyway.
pub(crate) const REFUSED_MARKS_KEPT: usize = 64;

/// A mark the server refused, numbered in the order the refusals came in.
pub(crate) struct RefusedMark {
    pub(crate) sequence: u64,
    pub(crate) item_id: String,
    pub(crate) mark: Mark,
}

/// The recent marks the server refused. A page remembers the last sequence it applied and takes
/// the newer ones while it draws, so a refusal that came while the page was off screen (under
/// another page, behind the player, or with the window hidden) is still taken back, and only
/// once. `main` kept the page's own request open for the length of the mark, which navigation
/// dropped.
#[derive(Default)]
pub(crate) struct RefusedMarks {
    sequence: u64,
    entries: VecDeque<RefusedMark>,
}

impl RefusedMarks {
    pub(crate) fn push(&mut self, item_id: String, mark: Mark) {
        self.sequence += 1;
        self.entries.push_back(RefusedMark {
            sequence: self.sequence,
            item_id,
            mark,
        });
        while self.entries.len() > REFUSED_MARKS_KEPT {
            self.entries.pop_front();
        }
    }

    /// The sequence of the newest refusal (0 before any).
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The refusals after `sequence`, oldest first.
    pub(crate) fn after(&self, sequence: u64) -> impl Iterator<Item = &RefusedMark> {
        self.entries.iter().filter(move |entry| entry.sequence > sequence)
    }
}

/// What a menu choice needs beyond the menu: playing (its controls are in `playback.rs`),
/// marking, and the episode fetch that *Play* on a series or season comes down to.
impl Services {
    /// Carries out a choice from the item menu: playing the item, playing a series' or season's
    /// next episode, marking it watched or favorite. Everything is done here except *Add to
    /// playlist…*, which needs U8's dialog and comes back for the page to open.
    pub fn carry_out(&mut self, action: MenuAction, item: &BaseItemDto) -> Option<MenuAction> {
        match action {
            // `start` only, as `main`'s menu did: the player still offers to start over.
            MenuAction::Play { start } => {
                self.play(
                    item,
                    PlayOptions {
                        start,
                        ..Default::default()
                    },
                );
                None
            }
            MenuAction::UpNext => {
                self.play_up_next(item);
                None
            }
            MenuAction::Mark(mark) => {
                self.mark(item.id.clone(), mark);
                None
            }
            MenuAction::AddToPlaylist => Some(action),
        }
    }

    /// Plays a series or season from its next episode, resuming it if it's partway there: which
    /// episode takes a request, so it runs on tokio and [`Services::tick`] plays it.
    pub fn play_up_next(&mut self, item: &BaseItemDto) {
        let client = self.session.client.clone();
        let item = item.clone();
        let sender = self.up_next_from.clone();
        crate::runtime::detach(async move {
            let result = item_menu::episodes_of(&client, &item)
                .await
                .map(|episodes| item_menu::first_unwatched(&episodes).cloned());
            let _ = sender.send(result);
        });
    }

    /// Sets watched or favorite state on the server, from a menu or an item page's toggle. The
    /// pages that show it reload when it worked (`user_data`); a failure is a toast.
    pub fn mark(&mut self, item_id: String, mark: Mark) {
        let client = self.session.client.clone();
        let sender = self.marking.clone();
        crate::runtime::detach(async move {
            let result = match mark {
                Mark::Played(true) => client.mark_played(&item_id).await,
                Mark::Played(false) => client.mark_unplayed(&item_id).await,
                Mark::Favorite(true) => client.mark_favorite(&item_id).await,
                Mark::Favorite(false) => client.unmark_favorite(&item_id).await,
            };
            let _ = sender.send((mark, item_id, result));
        });
    }

    /// The requests those actions started that have finished, as of this frame: a mark that
    /// makes the pages reload, or the episode to play next (or why there isn't one).
    /// `Services::tick` calls it, so it runs while the window is hidden too.
    pub(crate) fn apply_menu_requests(&mut self) {
        let marked: Vec<Marked> = self.marked.drain().collect();
        for (mark, item_id, result) in marked {
            match result {
                // What's shown of it (a played item's resume point, a season's unwatched count)
                // changed too: pages load again.
                Ok(()) => {
                    tracing::debug!(item = %item_id, ?mark, "marked");
                    self.user_data += 1;
                }
                Err(err) => {
                    let message = self.handle_error(&err);
                    self.toast(Toast::error(message));
                    // The page (or menu) may have shown the change already; it takes it back
                    // when it draws, however long from now that is.
                    self.refused_marks.push(item_id, mark);
                }
            }
        }
        let next: Vec<UpNext> = self.up_next.drain().collect();
        for result in next {
            match result {
                Ok(Some(episode)) => self.play(
                    &episode,
                    PlayOptions {
                        start: item_menu::resume_point(&episode),
                        ..Default::default()
                    },
                ),
                Ok(None) => self.toast(Toast::info("Nothing here to play.")),
                Err(err) => {
                    let message = self.handle_error(&err);
                    self.toast(Toast::error(message));
                }
            }
        }
        let shuffled: Vec<Shuffled> = self.shuffles.drain().collect();
        for result in shuffled {
            match result {
                Ok(mut episodes) => {
                    shuffle(&mut episodes);
                    self.play_all(episodes);
                }
                Err(err) => {
                    let message = self.handle_error(&err);
                    self.toast(Toast::error(message));
                }
            }
        }
    }

    /// Plays all of a series' or season's episodes in random order, for the item page's
    /// *Shuffle*: which episodes takes a request, so it runs on tokio and outlives the page —
    /// going Back still starts the shuffle, as `main`'s detached task did.
    pub fn play_shuffled(&mut self, item: &BaseItemDto) {
        let client = self.session.client.clone();
        let item = item.clone();
        let sender = self.shuffle_from.clone();
        crate::runtime::detach(async move {
            let result = item_menu::episodes_of(&client, &item).await;
            let _ = sender.send(result);
        });
    }
}
