//! Commands on one item, shared by its page, its cards and its rows: marking it watched or
//! favorite, playing a series or season from its next episode, and the context menu with
//! those and playing it.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Icon, IconName, WindowExt};
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind};
use jellyfin::{Client, Ticks};

use crate::changes::Changes;
use crate::components::timestamp;
use crate::playback::{PlayOptions, PlaybackManager};
use crate::router::Navigator;
use crate::runtime;
use crate::views::add_to_playlist;

#[derive(Clone, Copy)]
pub enum Mark {
    Played(bool),
    Favorite(bool),
}

impl Mark {
    /// Whether `item` already shows this.
    pub fn shown_on(self, item: &BaseItemDto) -> bool {
        match self {
            Mark::Played(played) => is_played(item) == played,
            Mark::Favorite(favorite) => is_favorite(item) == favorite,
        }
    }

    /// Shows this on `item`, ahead of the server. The rest of its user data (a played item's
    /// resume point) comes with the reload that follows.
    pub fn apply(self, item: &mut BaseItemDto) {
        let data = item.user_data.get_or_insert_default();
        match self {
            Mark::Played(played) => data.played = played,
            Mark::Favorite(favorite) => data.is_favorite = favorite,
        }
    }

    pub fn undone(self) -> Mark {
        match self {
            Mark::Played(played) => Mark::Played(!played),
            Mark::Favorite(favorite) => Mark::Favorite(!favorite),
        }
    }
}

/// Sets watched or favorite state on the server, then refreshes the pages showing it. A failure
/// shows an error; the task says whether it worked, for a page that showed the change already.
pub fn mark(
    client: Client,
    nav: Navigator,
    item_id: String,
    mark: Mark,
    window: &mut Window,
    cx: &mut App,
) -> Task<bool> {
    window.spawn(cx, async move |cx| {
        let result = runtime::spawn(async move {
            match mark {
                Mark::Played(true) => client.mark_played(&item_id).await,
                Mark::Played(false) => client.mark_unplayed(&item_id).await,
                Mark::Favorite(true) => client.mark_favorite(&item_id).await,
                Mark::Favorite(false) => client.unmark_favorite(&item_id).await,
            }
        })
        .await;
        cx.update(|window, cx| match result {
            Ok(()) => {
                Changes::notify_user_data(cx);
                true
            }
            Err(err) => {
                let message = nav.handle_error(&err, cx);
                window.push_notification(Notification::error(message), cx);
                false
            }
        })
        .unwrap_or(false)
    })
}

pub fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

pub fn is_favorite(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.is_favorite)
}

/// Lucide's circle-check filled in, with the check cut out of it.
const CIRCLE_CHECK_FILLED: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <mask id="check"><rect width="24" height="24" fill="white"/>
    <path d="m16 9-5.5 5.5L8 12" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
  </mask>
  <circle cx="12" cy="12" r="11" fill="currentColor" mask="url(#check)"/>
</svg>"#;

/// Lucide's heart filled in.
const HEART_FILLED: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
  <path d="M2 9.5a5.5 5.5 0 0 1 9.591-3.676.56.56 0 0 0 .818 0A5.49 5.49 0 0 1 22 9.5c0 2.29-1.5 4-3 5.5l-5.492 5.313a2 2 0 0 1-3 .019L5 15c-1.5-1.5-3-3.2-3-5.5"/>
</svg>"#;

/// Watched's icon: a check, filled in green once it's watched.
pub fn played_icon(played: bool, cx: &App) -> Icon {
    if played {
        Icon::default().data(CIRCLE_CHECK_FILLED).text_color(cx.theme().success)
    } else {
        Icon::new(IconName::Check)
    }
}

/// Favorite's icon: a heart, filled in red once it's a favorite.
pub fn favorite_icon(favorite: bool, cx: &App) -> Icon {
    if favorite {
        Icon::default().data(HEART_FILLED).text_color(cx.theme().red)
    } else {
        Icon::new(Lucide::Heart)
    }
}

/// Where playback would resume, if it's partway through.
pub fn resume_point(item: &BaseItemDto) -> Option<Ticks> {
    item.user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0)
}

pub fn is_playable(item: &BaseItemDto) -> bool {
    matches!(
        item.kind,
        BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
    )
}

/// The episode a series' or season's Play starts, out of its episodes in order: the first
/// that isn't watched (resumed if it's partway), or the first if they all are. Specials
/// (season 0) only count when there's nothing else, as with Jellyfin's web client.
pub fn first_unwatched(episodes: &[BaseItemDto]) -> Option<&BaseItemDto> {
    let regular: Vec<_> = episodes.iter().filter(|e| e.parent_index_number != Some(0)).collect();
    let candidates = if regular.is_empty() {
        episodes.iter().collect()
    } else {
        regular
    };
    candidates
        .iter()
        .find(|e| !is_played(e))
        .or(candidates.first())
        .copied()
}

/// All of a series' episodes in order, or a season's.
pub async fn episodes_of(client: &Client, item: &BaseItemDto) -> jellyfin::Result<Vec<BaseItemDto>> {
    Ok(match item.kind {
        BaseItemKind::Series => client.episodes(&item.id, None, &[]).await?.items,
        BaseItemKind::Season => {
            let series_id = item.series_id.as_deref().unwrap_or_default();
            client.episodes(series_id, Some(&item.id), &[]).await?.items
        }
        _ => Vec::new(),
    })
}

/// Plays a series or season from [`first_unwatched`], resuming it if it's partway.
pub fn play_up_next(client: Client, nav: Navigator, item: BaseItemDto, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let result = runtime::spawn(async move { episodes_of(&client, &item).await }).await;
            cx.update(|window, cx| match result {
                Ok(episodes) => match first_unwatched(&episodes) {
                    Some(episode) => {
                        let options = PlayOptions {
                            start: resume_point(episode),
                            ..Default::default()
                        };
                        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(episode, options, cx));
                    }
                    None => window.push_notification(Notification::info("Nothing here to play."), cx),
                },
                Err(err) => {
                    let message = nav.handle_error(&err, cx);
                    window.push_notification(Notification::error(message), cx);
                }
            })
            .ok();
        })
        .detach();
}

/// Movies, episodes and what holds episodes can go in a playlist.
pub fn can_add_to_playlist(item: &BaseItemDto) -> bool {
    is_playable(item) || matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season)
}

/// What has watched and favorite state worth setting from a card.
fn can_mark(item: &BaseItemDto) -> bool {
    can_add_to_playlist(item) || item.kind == BaseItemKind::BoxSet
}

/// Adds a right-click menu for `item` to a card or row: play or resume it, watched,
/// favorite and adding it to a playlist. Kinds with none of those (libraries, people) get no
/// menu. Everything in it is also on the item's page.
pub fn with_item_menu<E>(element: E, item: &BaseItemDto, client: &Client, nav: &Navigator) -> AnyElement
where
    E: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
{
    if !can_mark(item) {
        return element.into_any_element();
    }
    let (item, client, nav) = (item.clone(), client.clone(), nav.clone());
    element
        .context_menu(move |menu, _, _| item_menu(menu, &item, &client, &nav))
        .into_any_element()
}

fn item_menu(mut menu: PopupMenu, item: &BaseItemDto, client: &Client, nav: &Navigator) -> PopupMenu {
    if is_playable(item) {
        let play = |start: Option<Ticks>| {
            let item = item.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let options = PlayOptions {
                    start,
                    ..Default::default()
                };
                PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
            }
        };
        menu = match resume_point(item) {
            Some(position) => menu
                .item(
                    PopupMenuItem::new(format!("Resume from {}", timestamp(position.into())))
                        .icon(IconName::Play)
                        .on_click(play(Some(position))),
                )
                .item(PopupMenuItem::new("Play from beginning").on_click(play(None))),
            None => menu.item(PopupMenuItem::new("Play").icon(IconName::Play).on_click(play(None))),
        };
        menu = menu.separator();
    } else if matches!(item.kind, BaseItemKind::Series | BaseItemKind::Season) {
        // Which episode, and whether it resumes, takes a request; the page's button shows it.
        let (client, nav, item) = (client.clone(), nav.clone(), item.clone());
        menu = menu
            .item(
                PopupMenuItem::new("Play")
                    .icon(IconName::Play)
                    .on_click(move |_, window, cx| play_up_next(client.clone(), nav.clone(), item.clone(), window, cx)),
            )
            .separator();
    }
    let marker = |change: Mark| {
        let (client, nav, id) = (client.clone(), nav.clone(), item.id.clone());
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            mark(client.clone(), nav.clone(), id.clone(), change, window, cx).detach()
        }
    };
    let (played, favorite) = (is_played(item), is_favorite(item));
    // Checked like the page's toggles, with the same names.
    menu = menu
        .item(
            PopupMenuItem::new("Watched")
                .checked(played)
                .on_click(marker(Mark::Played(!played))),
        )
        .item(
            PopupMenuItem::new("Favorite")
                .checked(favorite)
                .on_click(marker(Mark::Favorite(!favorite))),
        );
    if can_add_to_playlist(item) {
        let (client, nav, item) = (client.clone(), nav.clone(), item.clone());
        menu = menu
            .separator()
            .item(
                PopupMenuItem::new("Add to playlist…")
                    .icon(Lucide::ListPlus)
                    .on_click(move |_, window, cx| {
                        // After the menu has closed and handed focus back, so the dialog keeps it.
                        let (client, nav, item) = (client.clone(), nav.clone(), item.clone());
                        window.defer(cx, move |window, cx| {
                            add_to_playlist::open(client, nav, &item, window, cx)
                        });
                    }),
            );
    }
    menu
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::first_unwatched;

    fn episode(id: &str, season: i32, played: bool) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Name": id,
            "Type": "Episode",
            "ParentIndexNumber": season,
            "UserData": { "Played": played, "PlaybackPositionTicks": 0, "IsFavorite": false },
        }))
        .expect("episode")
    }

    fn pick(episodes: &[BaseItemDto]) -> Option<&str> {
        first_unwatched(episodes).map(|e| e.id.as_str())
    }

    #[test]
    fn plays_the_first_unwatched_episode() {
        let episodes = [episode("a", 1, true), episode("b", 1, false), episode("c", 2, false)];
        assert_eq!(pick(&episodes), Some("b"));
    }

    #[test]
    fn starts_over_when_everything_is_watched() {
        let episodes = [episode("a", 1, true), episode("b", 1, true)];
        assert_eq!(pick(&episodes), Some("a"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn skips_specials_unless_there_is_nothing_else() {
        let episodes = [
            episode("special", 0, false),
            episode("a", 1, true),
            episode("b", 1, false),
        ];
        assert_eq!(pick(&episodes), Some("b"));
        let watched = [episode("special", 0, false), episode("a", 1, true)];
        assert_eq!(pick(&watched), Some("a"));
        assert_eq!(pick(&[episode("special", 0, false)]), Some("special"));
    }
}
