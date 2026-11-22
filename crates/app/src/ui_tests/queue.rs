//! The local play queue: what the item menu and remote control put in it, and its order.

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use jellyfin::models::BaseItemDto;

use crate::keys::{self, Scope};
use crate::playback::title;
use crate::ui_tests::touch::PlayerRun;
use crate::ui_tests::{ConfigDir, FixtureServer, build_harness, fixture_json};

/// An episode of the fixtures, by its place in their list.
fn episode(index: usize) -> BaseItemDto {
    let episodes: serde_json::Value = fixture_json("episodes");
    serde_json::from_value(episodes["Items"][index].clone()).expect("an episode")
}

/// A movie of the fixtures, for a queue with more than the episodes.
fn movie() -> BaseItemDto {
    fixture_json("item_movie")
}

/// A session playing the second episode, with no queue.
fn playing() -> (FixtureServer, egui_kittest::Harness<'static, PlayerRun>) {
    let server = FixtureServer::start();
    let mut harness = build_harness(egui::vec2(1280., 720.), PlayerRun::new(&server), |ui, run| run.show(ui));
    for _ in 0..3 {
        harness.step();
    }
    (server, harness)
}

fn queued(harness: &mut egui_kittest::Harness<'static, PlayerRun>) -> Vec<String> {
    harness
        .state_mut()
        .services()
        .playback
        .queued()
        .map(|item| item.id.clone())
        .collect()
}

/// Play next lands right after the playing item, ahead of what's queued; add lands last.
#[test]
fn play_next_goes_first_and_add_goes_last() {
    let _config = ConfigDir::new("queue-order");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    let services = harness.state_mut().services();

    services.enqueue(vec![episode(2)], false);
    services.enqueue(vec![movie()], false);
    services.enqueue(vec![episode(0)], true);
    assert_eq!(queued(&mut harness), [id(0), id(2), movie().id]);
    // The up-next card and autoplay follow the queue.
    let next = harness.state().now_playing().and_then(|state| state.next.clone());
    assert_eq!(next.map(|item| item.id), Some(id(0)));
}

/// Several items play next in their own order, ahead of the rest.
#[test]
fn several_items_play_next_in_order() {
    let _config = ConfigDir::new("queue-next-several");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    let services = harness.state_mut().services();

    services.enqueue(vec![movie()], false);
    services.enqueue(vec![episode(0), episode(2)], true);
    assert_eq!(queued(&mut harness), [id(0), id(2), movie().id]);
}

/// What the queue's remove and move do to the up-next card: it follows the front of the queue,
/// and falls back to the following episode when the queue is empty.
#[test]
fn the_up_next_card_follows_the_queue() {
    let _config = ConfigDir::new("queue-up-next");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    let next = |harness: &egui_kittest::Harness<'static, PlayerRun>| {
        harness
            .state()
            .now_playing()
            .and_then(|state| state.next.clone())
            .map(|item| item.id)
    };
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(0), episode(2)], false);
    assert_eq!(next(&harness), Some(id(0)));

    let services = harness.state_mut().services();
    assert_eq!(services.queue_move(0, 1), Some(1));
    assert_eq!(queued(&mut harness), [id(2), id(0)]);
    assert_eq!(next(&harness), Some(id(2)));

    let services = harness.state_mut().services();
    services.queue_remove(0);
    assert_eq!(next(&harness), Some(id(0)));
    let services = harness.state_mut().services();
    services.queue_remove(0);
    assert!(queued(&mut harness).is_empty());
    assert_eq!(next(&harness), None);
}

/// A key press taken from egui as `App::raw_input_hook` takes it, and handed to the player as the
/// shell routes it: while the panel is open egui keeps the keys, so they stay in its input.
fn press(harness: &mut Harness<'static, PlayerRun>, modifiers: Modifiers, key: egui::Key) {
    let ctx = harness.ctx.clone();
    {
        let input = harness.input_mut();
        for pressed in [true, false] {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
    }
    let presses = keys::take(&ctx, harness.input_mut(), &[Scope::Player, Scope::Shell]);
    harness.state_mut().presses(presses);
    harness.step();
}

fn plain(harness: &mut Harness<'static, PlayerRun>, key: egui::Key) {
    press(harness, Modifiers::NONE, key);
    // The panel takes a frame to size itself and another to draw.
    harness.step();
    harness.step();
}

/// The panel's rows, top to bottom.
fn rows(harness: &Harness<'static, PlayerRun>) -> Vec<String> {
    harness
        .query_all_by_role(Role::MenuItem)
        .filter_map(|node| node.accesskit_node().label())
        .collect()
}

/// Q opens the panel over the player: the playing item marked, then what's queued.
#[test]
fn q_opens_the_panel_with_what_plays_and_what_is_next() {
    let _config = ConfigDir::new("queue-panel");
    let (_server, mut harness) = playing();
    harness.state_mut().services().enqueue(vec![episode(2), movie()], false);
    assert!(rows(&harness).is_empty());

    plain(&mut harness, egui::Key::Q);
    assert_eq!(
        rows(&harness),
        [
            format!("{}, playing", title(&episode(1))),
            title(&episode(2)),
            title(&movie())
        ]
    );

    // Q again, and Esc, close it.
    plain(&mut harness, egui::Key::Q);
    assert!(rows(&harness).is_empty(), "Q closes the panel");
    plain(&mut harness, egui::Key::Q);
    assert!(!rows(&harness).is_empty());
    plain(&mut harness, egui::Key::Escape);
    assert!(rows(&harness).is_empty(), "Esc closes the panel");
}

/// With the panel open, Down walks the queued entries, Alt+Up and Alt+Down move the highlighted
/// one (the highlight going with it) and Delete removes it.
#[test]
fn the_panel_keys_move_and_remove_entries() {
    let _config = ConfigDir::new("queue-keys");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    harness
        .state_mut()
        .services()
        .enqueue(vec![episode(0), episode(2), movie()], false);

    // The keyboard opens it on the first entry; Down goes to the second.
    plain(&mut harness, egui::Key::Q);
    plain(&mut harness, egui::Key::ArrowDown);
    press(&mut harness, Modifiers::ALT, egui::Key::ArrowUp);
    assert_eq!(queued(&mut harness), [id(2), id(0), movie().id]);

    // The highlight went with the entry: moving it again puts it back.
    press(&mut harness, Modifiers::ALT, egui::Key::ArrowDown);
    assert_eq!(queued(&mut harness), [id(0), id(2), movie().id]);
    // Not past the end.
    press(&mut harness, Modifiers::ALT, egui::Key::ArrowDown);
    press(&mut harness, Modifiers::ALT, egui::Key::ArrowDown);
    assert_eq!(queued(&mut harness), [id(0), movie().id, id(2)]);

    plain(&mut harness, egui::Key::Delete);
    assert_eq!(queued(&mut harness), [id(0), movie().id]);
    // The page's own keys stayed out of it: nothing was seeked or turned down meanwhile.
    assert!(harness.state_mut().commands().is_empty());
    assert_eq!(
        rows(&harness),
        [
            format!("{}, playing", title(&episode(1))),
            title(&episode(0)),
            title(&movie())
        ]
    );
}
