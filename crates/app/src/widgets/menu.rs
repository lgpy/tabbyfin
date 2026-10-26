//! An item's context menu: play or resume it, play a series or season from its next episode,
//! mark it watched or favorite, add it to a playlist. The rules (what's offered for which kind,
//! what's checked) are [`crate::item_menu`]'s.
//!
//! The menu returns what was chosen instead of carrying it out: marking an item is a
//! [`crate::services::Services::mark`] call the page makes, while playing it and the playlist
//! dialog belong to the player (P1) and the dialogs (U8), so the pages that own those carry
//! [`MenuAction`]s out. `main`'s menu did all of it itself, through GPUI globals.
//!
//! egui's menus don't take keys, so the menu is pointer-only for now, like the account menu
//! (S1).

use egui::text::LayoutJob;
use egui::{Button, Color32, FontId, Response, RichText, TextFormat, TextStyle, Ui, WidgetText};
use egui_material_icons::icons::{ICON_CHECK, ICON_CHECK_CIRCLE, ICON_FAVORITE, ICON_PLAY_ARROW, ICON_PLAYLIST_ADD};

use jellyfin::models::{BaseItemDto, BaseItemKind};

use crate::item_menu::{Mark, can_add_to_playlist, can_mark, is_favorite, is_playable, is_played, resume_point};
use crate::widgets::text::timestamp;

/// What a choice in [`item_menu`] asks for.
#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    /// Play the item, resuming at `start` if it's partway through.
    Play { start: Option<jellyfin::Ticks> },
    /// Play a series or season from its next episode: the page fetches the episodes
    /// (`item_menu::episodes_of`, `item_menu::first_unwatched`) and plays that one.
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

/// Watched's icon: a check, filled in green once it's watched.
pub fn played_icon(played: bool) -> RichText {
    if played {
        ICON_CHECK_CIRCLE.rich_text().color(SUCCESS)
    } else {
        ICON_CHECK.outlined().rich_text()
    }
}

/// Favorite's icon: a heart, filled in red once it's a favorite.
pub fn favorite_icon(favorite: bool) -> RichText {
    if favorite {
        ICON_FAVORITE.rich_text().color(RED)
    } else {
        ICON_FAVORITE.outlined().rich_text()
    }
}

/// A menu entry with `icon` before its label, as `main`'s menu items had.
fn icon_entry(ui: &mut Ui, icon: egui_material_icons::MaterialIcon, label: &str) -> Response {
    let body = TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.append(
        icon.codepoint,
        0.,
        TextFormat {
            font_id: FontId::new(body.size + 2., icon.font_family()),
            color,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {label}"),
        0.,
        TextFormat {
            font_id: body,
            color,
            ..Default::default()
        },
    );
    ui.add(Button::new(WidgetText::LayoutJob(job.into())))
}

/// `main`'s green and red for the filled watched and favorite icons (gpui-kit's theme colours);
/// R1 brings the theme's own tokens.
const SUCCESS: Color32 = Color32::from_rgb(0x2e, 0xc4, 0x6e);
const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
