//! Spoiler protection: with it on, an episode that hasn't been watched shows its blurhash
//! placeholder where its thumbnail would be, and hides its overview, wherever a card or row
//! lists it (season lists, Next Up, search results). The episode's own page shows everything.
//!
//! The setting is the app's, kept in egui's context data so the widgets that draw a picture
//! can ask without every page passing it down; Settings and the services set it.

use egui::Id;
use jellyfin::models::{BaseItemDto, BaseItemKind};

use crate::item_menu::{is_played, resume_point};

fn key() -> Id {
    Id::unique("spoiler_protection")
}

/// Turns the protection on or off.
pub fn set(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|data| data.insert_temp(key(), on));
}

/// Whether the picture and summary of `item` should stay hidden: the protection is on and it
/// is an episode not yet watched. One that's been started has been seen in part, so it shows.
pub fn hides(ctx: &egui::Context, item: &BaseItemDto) -> bool {
    is_spoiler(item) && ctx.data(|data| data.get_temp::<bool>(key())).unwrap_or(false)
}

/// An episode that hasn't been watched or started.
pub fn is_spoiler(item: &BaseItemDto) -> bool {
    item.kind == BaseItemKind::Episode && !is_played(item) && resume_point(item).is_none()
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::{hides, is_spoiler, set};

    fn item(kind: &str, played: bool, ticks: i64) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({
            "Id": "x",
            "Name": "x",
            "Type": kind,
            "UserData": { "Played": played, "PlaybackPositionTicks": ticks },
        }))
        .expect("an item")
    }

    #[test]
    fn only_unwatched_unstarted_episodes_are_spoilers() {
        assert!(is_spoiler(&item("Episode", false, 0)));
        assert!(!is_spoiler(&item("Episode", true, 0)));
        assert!(!is_spoiler(&item("Episode", false, 600_000_000)));
        assert!(!is_spoiler(&item("Movie", false, 0)));
    }

    #[test]
    fn nothing_hides_until_the_setting_is_on() {
        let ctx = egui::Context::default();
        let episode = item("Episode", false, 0);
        assert!(!hides(&ctx, &episode));
        set(&ctx, true);
        assert!(hides(&ctx, &episode));
        set(&ctx, false);
        assert!(!hides(&ctx, &episode));
    }
}
