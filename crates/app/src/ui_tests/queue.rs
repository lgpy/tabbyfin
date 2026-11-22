//! The local play queue: what the item menu and remote control put in it, and its order.

use jellyfin::models::BaseItemDto;

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
