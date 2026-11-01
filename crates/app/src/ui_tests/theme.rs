//! The app's colours as every page gets them: what `theme::apply` leaves egui with, in both
//! themes, now that the whole app draws in the cozy palette (C1 in `docs/ROADMAP.md`), and the
//! hairline a page draws under the top bar once it scrolls (C2).

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::nav::Route;
use crate::theme::Tokens;
use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, PageRun, session};

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

/// Whether the last frame drew [`backdrop::scroll_edge`]'s hairline: a horizontal line in
/// `surface_border` across most of the window (the page's top edge; nothing else on the page
/// draws one).
///
/// [`backdrop::scroll_edge`]: crate::widgets::backdrop::scroll_edge
fn hairline(harness: &Harness<'static, PageRun>) -> bool {
    let border = crate::theme::tokens(&harness.ctx.global_style().visuals).surface_border;
    let width = harness.ctx.content_rect().width();
    harness.output().shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::LineSegment { points: [a, b], stroke } => {
            a.y == b.y && (b.x - a.x).abs() > width * 0.5 && stroke.color == border
        }
        _ => false,
    })
}

/// The top bar has no fill of its own, so a page marks its top edge once it scrolls under the
/// bar, and not while it's at its top, where there's nothing under the bar to separate.
#[test]
fn a_page_draws_its_top_edge_only_once_it_scrolls() {
    let _config = ConfigDir::new("theme-scroll-edge");
    let server = FixtureServer::start();
    let id = "129db440ee6554a42e5d8f1350516bab";
    server.json("GET", "/Items/129db440ee6554a42e5d8f1350516bab", "item_series");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Seasons", "seasons");
    server.json("GET", "/Shows/129db440ee6554a42e5d8f1350516bab/Episodes", "episodes");
    let mut ui = PageHarness::new(session(&server), move |cx| {
        crate::views::open(
            Route::Item {
                id: id.to_owned(),
                name: "Test Show".into(),
            },
            cx,
        )
    });
    ui.step_until("the series' seasons", |harness| {
        harness.query_by_label("Seasons").is_some()
    });
    ui.step();
    assert!(!hairline(ui.harness()), "an edge on a page at its top");

    let middle = ui.harness().ctx.content_rect().center();
    ui.harness_mut().hover_at(middle);
    ui.harness_mut().input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0., -200.),
        phase: TouchPhase::Move,
        source: MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    ui.step_until("the page's edge once it scrolled", hairline);
}
