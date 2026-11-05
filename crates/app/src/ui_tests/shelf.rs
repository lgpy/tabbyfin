//! A shelf's scroll feel, without a window: its arrows glide a page over, a drag follows the
//! pointer and stops with it, a drag up or down scrolls the page instead, and the wheel is the
//! page's rather than the row's.
//!
//! egui_kittest turns animations off for its snapshots, which the glide follows: the first test
//! turns them back on for its harness, as the app's default setting has them.

use egui::accesskit::Role;
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};

use crate::widgets::shelf;

/// The window one row is laid out in.
const SIZE: egui::Vec2 = vec2(600., 300.);

/// A window with one shelf of `cards` labelled cards, laid out and with its arrows (which come
/// from the frame before).
fn row(cards: usize) -> Harness<'static, ()> {
    // A step is a frame at 60 fps, so a glide takes the frames it would in the app (the
    // default quarter of a second a step would almost close it in one). The rest is what
    // `ui_tests::build_harness` sets up: the app's icons, theme and fonts.
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1. / 60.)
        .allow_missing_glyphs()
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            egui_material_icons::initialize(ctx);
            crate::theme::apply(ctx, &crate::config::Config::default());
        })
        .build_ui(move |ui| {
            shelf::shelf_with_gaps(ui, "row", "A shelf", 12., 8., shelf::PADDING, |ui| {
                for index in 0..cards {
                    ui.label(format!("card {index}"));
                }
            });
        });
    harness
        .ctx
        .global_style_mut(|style| style.animation_time = egui::Style::default().animation_time);
    harness.step();
    harness.step();
    harness
}

/// How far the row has scrolled left of where it started: the cards are laid out without the
/// row's offset, so the first one moves as it scrolls.
fn offset(harness: &Harness<'static>, start: f32) -> f32 {
    start - harness.get_by_label("card 0").rect().left()
}

/// Clicks the row's next-page arrow: drawn from the right, so it is the rightmost button. The
/// press and release are one frame, as `Harness::step` runs a frame per queued event.
fn click_next(harness: &mut Harness<'static>) {
    let at = {
        let next = harness
            .get_all_by_role(Role::Button)
            .max_by(|a, b| a.rect().center().x.total_cmp(&b.rect().center().x))
            .expect("the row's arrows");
        next.rect().center()
    };
    for event in [
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ] {
        harness.input_mut().events.push(event);
    }
    harness.step();
}

#[test]
fn an_arrow_glides_the_row_a_page_over() {
    let mut ui = row(30);
    let start = ui.get_by_label("card 0").rect().left();
    click_next(&mut ui);
    assert_eq!(offset(&ui, start), 0., "the click itself doesn't move the row");

    // The first frame of the glide is a step towards the page, not the whole page.
    ui.step();
    let first = offset(&ui, start);
    assert!(first > 0.5, "the glide moved on: {first}");

    // It eases on to the end of its page, never going back, and stops there.
    let mut last = first;
    let mut frames = 0;
    loop {
        ui.step();
        frames += 1;
        assert!(frames < 120, "the glide never stopped");
        let now = offset(&ui, start);
        assert!(now >= last - 0.01, "{last} -> {now}");
        if (now - last).abs() < 0.001 {
            break;
        }
        last = now;
    }
    let rest = offset(&ui, start);
    // A page is the row's own width (a little short of the window's), and no more.
    assert!((SIZE.x - 100. ..=SIZE.x).contains(&rest), "a page over: {rest}");
    assert!(
        first < rest / 2.,
        "the first frame is a step, not the page: {first} of {rest}"
    );
    // Nothing moves any more, so nothing asks for another frame (the tooltip under the pointer
    // would, so the pointer leaves first).
    ui.hover_at(egui::pos2(5., 5.));
    for _ in 0..20 {
        ui.step();
        if !ui.ctx.has_requested_repaint() {
            return;
        }
    }
    panic!("an idle row keeps asking for frames");
}

#[test]
fn with_animations_off_the_arrow_jumps() {
    let mut ui = row(30);
    ui.ctx.global_style_mut(|style| style.animation_time = 0.);
    let start = ui.get_by_label("card 0").rect().left();
    click_next(&mut ui);
    let jumped = offset(&ui, start);
    assert!(jumped > SIZE.x / 2., "the page was there at once: {jumped}");
    ui.step();
    assert_eq!(offset(&ui, start), jumped, "nothing glides with animations off");
}

#[test]
fn a_drag_follows_the_pointer_and_stops_with_it() {
    let mut ui = row(30);
    let start = ui.get_by_label("card 0").rect().left();
    let from = ui.get_by_label("card 1").rect().center();
    let at = |x: f32| from - vec2(x, 0.);

    // Press, then drag 100 points left in 20-point frames: the row follows 1:1.
    ui.hover_at(from);
    ui.step();
    ui.drag_at(from);
    ui.step();
    for step in 1..=5 {
        ui.hover_at(at(20. * step as f32));
        ui.step();
    }
    // The first move only gets the press past the drag threshold; from then on the row follows
    // the pointer exactly.
    let dragged = offset(&ui, start);
    assert!(dragged > 0.5, "the drag moved the row: {dragged}");
    ui.hover_at(at(120.));
    ui.step();
    let further = offset(&ui, start) - dragged;
    assert!((further - 20.).abs() < 0.5, "the row followed the pointer: {further}");

    // Let go: the row stays there, with no coasting after the release. (A drag is applied
    // after the row is laid out, so the frame that draws it is the one after the move.)
    ui.drop_at(at(120.));
    ui.step();
    let released = offset(&ui, start);
    assert!(
        released > dragged,
        "the last move reached the release frame: {released}"
    );
    for _ in 0..5 {
        ui.step();
    }
    assert!(
        (offset(&ui, start) - released).abs() < 0.5,
        "the row coasted on after the release"
    );

    // Dragging back past the start stops at it instead of rubber-banding.
    let back = from + vec2(150., 0.);
    ui.hover_at(back);
    ui.step();
    ui.drag_at(back);
    ui.step();
    for step in 1..=5 {
        ui.hover_at(back + vec2(40. * step as f32, 0.));
        ui.step();
    }
    ui.drop_at(back + vec2(200., 0.));
    ui.step();
    assert!(offset(&ui, start).abs() < 0.01, "the row stayed inside its start");
}

#[test]
fn a_drag_up_or_down_scrolls_the_page_and_holds_the_row() {
    // The row at the top of a page that scrolls, as on Home.
    let mut ui = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1. / 60.)
        .allow_missing_glyphs()
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            egui_material_icons::initialize(ctx);
            crate::theme::apply(ctx, &crate::config::Config::default());
        })
        .build_ui(|ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                shelf::shelf_with_gaps(ui, "row", "A shelf", 12., 8., shelf::PADDING, |ui| {
                    for index in 0..30 {
                        ui.label(format!("card {index}"));
                    }
                });
                for line in 0..60 {
                    ui.label(format!("line {line}"));
                }
            });
        });
    ui.step();
    ui.step();
    let card = |ui: &Harness<'static>| ui.get_by_label("card 0").rect().left_top();
    let start = card(&ui);
    let from = ui.get_by_label("card 1").rect().center();

    // Press on the row, then drag 100 points up in 20-point frames, wobbling a few points from
    // side to side as a finger does. The row holds still on every frame, not only at the end:
    // a row drawn off by each frame's sideways movement jitters under the finger.
    ui.hover_at(from);
    ui.step();
    ui.drag_at(from);
    ui.step();
    for step in 1..=5 {
        let wobble = if step % 2 == 0 { 4. } else { -4. };
        ui.hover_at(from + vec2(wobble, -20. * step as f32));
        ui.step();
        let moved = card(&ui) - start;
        assert!(moved.x.abs() < 0.5, "the row held still at frame {step}: {moved:?}");
    }
    ui.drop_at(from + vec2(-4., -100.));
    ui.step();
    let moved = card(&ui) - start;
    assert!(moved.y < -60., "the page scrolled up with the drag: {moved:?}");
    assert!(moved.x.abs() < 0.5, "the row held still: {moved:?}");
    // Nor does it move once the drag is over.
    ui.step();
    let moved = card(&ui) - start;
    assert!(moved.x.abs() < 0.5, "the row stayed put after the release: {moved:?}");
}

#[test]
fn a_page_pulled_past_its_top_and_flung_springs_back() {
    // A page with shelves, as Home is on a phone: with touch, its flings are `Coast`s.
    let mut ui = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1. / 60.)
        .allow_missing_glyphs()
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            egui_material_icons::initialize(ctx);
            crate::theme::apply(ctx, &crate::config::Config::default());
            crate::layout::set_touch(ctx, true);
        })
        .build_ui(|ui| {
            // egui drags a page with touch only once it has seen a touch event; the harness's
            // pointer is a mouse, so the drag is on for it here.
            let area = egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource::ALL);
            crate::smooth_scroll::page(ui, area, |ui| {
                for line in 0..80 {
                    ui.label(format!("line {line}"));
                }
            });
        });
    ui.step();
    ui.step();
    let top = |ui: &Harness<'static>| ui.get_by_label("line 0").rect().top();
    let start = top(&ui);

    // At the top, a quick pull down: the page stretches past its top, and the release flings.
    let from = ui.get_by_label("line 3").rect().center();
    ui.hover_at(from);
    ui.step();
    ui.drag_at(from);
    ui.step();
    for step in 1..=4 {
        ui.hover_at(from + vec2(0., 40. * step as f32));
        ui.step();
    }
    ui.drop_at(from + vec2(0., 160.));
    ui.step();

    // The fling doesn't carry the page further past its top, and the page springs back.
    let mut furthest = top(&ui) - start;
    for _ in 0..120 {
        ui.step();
        furthest = furthest.max(top(&ui) - start);
    }
    // The pull itself stretched it about 115 points; a coast that kept going took it over 1000.
    assert!(furthest < 160., "the fling pulled the page {furthest} past its top");
    let gap = top(&ui) - start;
    assert!(gap.abs() < 0.5, "the page stayed {gap} past its top");
}

#[test]
fn the_wheel_leaves_the_row_to_the_page() {
    let mut ui = row(30);
    let start = ui.get_by_label("card 0").rect().left();
    ui.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(-80., 0.),
        phase: TouchPhase::Move,
        source: MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    ui.step();
    ui.step();
    assert_eq!(offset(&ui, start), 0., "the wheel moved the row");
}

/// The arrows are round buttons named by what they do, the one at the row's end disabled.
#[test]
fn the_arrows_are_named_and_dim_at_their_end() {
    let mut ui = row(30);
    let disabled = |ui: &Harness<'static>, label: &str| {
        ui.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    };
    assert!(disabled(&ui, "Scroll left"), "the row starts at its start");
    assert!(!disabled(&ui, "Scroll right"));
    ui.ctx.global_style_mut(|style| style.animation_time = 0.);
    click_next(&mut ui);
    ui.step();
    ui.step();
    assert!(!disabled(&ui, "Scroll left"), "a page on, the row can go back");
}
