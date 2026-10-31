//! The app's colours as every page gets them: what `theme::apply` leaves egui with, in both
//! themes, now that the whole app draws in the cozy palette (C1 in `docs/ROADMAP.md`).

use egui_kittest::kittest::Queryable;

use crate::theme::Tokens;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, session};

/// A page's background, text, fields, menus and focus ring come from the cozy tokens, in the
/// light theme and the dark one, without the page asking for them.
#[test]
fn pages_draw_in_the_cozy_palette() {
    let _config = ConfigDir::new("theme-cozy-palette");
    let server = FixtureServer::start();
    let ui = PageHarness::new(session(&server), |cx| crate::views::open(crate::nav::Route::Search, cx));
    let ctx = &ui.harness().ctx;
    for (theme, tokens) in [(egui::Theme::Light, Tokens::LIGHT), (egui::Theme::Dark, Tokens::DARK)] {
        let visuals = ctx.style_of(theme).visuals.clone();
        assert_eq!(visuals.panel_fill, tokens.backdrop_top, "{theme:?} page background");
        assert_eq!(visuals.window_fill, tokens.surface, "{theme:?} menus and dialogs");
        assert_eq!(visuals.text_color(), tokens.ink, "{theme:?} text");
        assert_eq!(visuals.weak_text_color(), tokens.ink_muted, "{theme:?} secondary text");
        assert_eq!(visuals.text_edit_bg_color(), tokens.inset, "{theme:?} text fields");
        assert_eq!(visuals.selection.stroke.color, tokens.brand, "{theme:?} focus ring");
        assert_eq!(
            visuals.widgets.inactive.corner_radius.nw, tokens.radius_control,
            "{theme:?} control radius"
        );
    }
    // The page drew in it: the search page's hint is on screen.
    assert!(ui.harness().query_by_label_contains("search").is_some());
}
