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

/// Repeat and shuffle go to the session, which reports them to the server with its progress, and
/// the queue keeps the playing item when it shuffles.
#[test]
fn repeat_and_shuffle_are_sent_to_the_session() {
    use jellyfin::{PlaybackOrder, RepeatMode};

    use crate::playback::UiCommand;

    let _config = ConfigDir::new("queue-modes");
    let (_server, mut harness) = playing();
    harness.state_mut().commands();
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(0), episode(2), movie()], false);
    let sent = |harness: &mut Harness<'static, PlayerRun>| harness.state_mut().commands();

    services.cycle_repeat();
    let commands = sent(&mut harness);
    assert!(
        matches!(
            commands.as_slice(),
            [UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Default)]
        ),
        "{commands:?}"
    );
    let services = harness.state_mut().services();
    services.cycle_repeat();
    services.toggle_shuffle();
    let commands = sent(&mut harness);
    assert!(
        matches!(
            commands.as_slice(),
            [
                UiCommand::SetQueueModes(RepeatMode::RepeatOne, PlaybackOrder::Default),
                UiCommand::SetQueueModes(RepeatMode::RepeatOne, PlaybackOrder::Shuffle)
            ]
        ),
        "{commands:?}"
    );

    // Shuffling mixed what was queued and kept the playing item, and the up-next card follows.
    let mut shuffled = queued(&mut harness);
    shuffled.sort();
    let mut expected = vec![episode(0).id, episode(2).id, movie().id];
    expected.sort();
    assert_eq!(shuffled, expected);
    let front = queued(&mut harness).into_iter().next();
    let state = harness.state().now_playing().expect("the session");
    assert_eq!(state.item.id, episode(1).id);
    assert_eq!(state.next.as_ref().map(|item| item.id.clone()), front);

    // Off again: the order it was in, and off is reported.
    let services = harness.state_mut().services();
    services.toggle_shuffle();
    services.cycle_repeat();
    let commands = sent(&mut harness);
    assert!(
        matches!(
            commands.as_slice(),
            [
                UiCommand::SetQueueModes(RepeatMode::RepeatOne, PlaybackOrder::Default),
                UiCommand::SetQueueModes(RepeatMode::RepeatNone, PlaybackOrder::Default)
            ]
        ),
        "{commands:?}"
    );
    assert_eq!(queued(&mut harness), [episode(0).id, episode(2).id, movie().id]);
}

/// Another client's repeat and shuffle commands set the same modes.
#[test]
fn remote_control_sets_repeat_and_shuffle() {
    use jellyfin::{GeneralCommand, PlaybackOrder, RepeatMode};

    use crate::playback::UiCommand;

    let _config = ConfigDir::new("queue-remote-modes");
    let (_server, mut harness) = playing();
    harness.state_mut().commands();
    let services = harness.state_mut().services();
    services.on_remote_command(GeneralCommand::SetRepeatMode(RepeatMode::RepeatAll));
    services.on_remote_command(GeneralCommand::SetShuffleQueue(true));
    let commands = harness.state_mut().commands();
    assert!(
        matches!(
            commands.as_slice(),
            [
                UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Default),
                UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Shuffle)
            ]
        ),
        "{commands:?}"
    );
}

/// The panel's repeat and shuffle are on the keyboard's way: with nothing queued Q starts on
/// repeat, Enter flips it, Down goes to shuffle and Enter flips that.
#[test]
fn the_panels_toggles_work_from_the_keyboard() {
    use jellyfin::{PlaybackOrder, RepeatMode};

    use crate::playback::UiCommand;

    let _config = ConfigDir::new("queue-toggle-keys");
    let (_server, mut harness) = playing();
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::Q);
    plain(&mut harness, egui::Key::Enter);
    plain(&mut harness, egui::Key::ArrowDown);
    plain(&mut harness, egui::Key::Enter);
    let commands = harness.state_mut().commands();
    assert!(
        matches!(
            commands.as_slice(),
            [
                UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Default),
                UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Shuffle)
            ]
        ),
        "{commands:?}"
    );
    // The panel stayed up for it.
    assert!(!rows(&harness).is_empty());
}

/// What an item that played to its end is followed by: the next queued one when autoplay is on,
/// the same one again with repeat one, nothing without either.
#[test]
fn the_end_of_an_item_follows_the_queue_and_the_repeat_mode() {
    use jellyfin::RepeatMode;

    let _config = ConfigDir::new("queue-end");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    harness.state_mut().services().enqueue(vec![episode(2)], false);

    // No autoplay (the setting, or nothing next) and no repeat: playback just ends.
    assert_eq!(harness.state_mut().services().end_for_test(), None);
    assert!(
        harness
            .state_mut()
            .services()
            .events
            .iter()
            .any(|event| matches!(event, crate::services::AppEvent::PlaybackEnded))
    );

    // Repeat one: the same item again, and the queue is as it was.
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(2)], false);
    services.set_repeat(RepeatMode::RepeatOne);
    assert_eq!(services.end_for_test(), Some(id(1)));
    assert_eq!(queued(&mut harness), [id(2)]);

    // Autoplay: the queued item plays and leaves the queue.
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(2)], false);
    harness.state_mut().now_playing_mut().expect("the session").autoplay = true;
    assert_eq!(harness.state_mut().services().end_for_test(), Some(id(2)));
    assert!(queued(&mut harness).is_empty());
}

/// *Save as playlist…* in the panel makes a playlist of what plays and what's queued, in order,
/// named for the show.
#[test]
fn the_queue_is_saved_as_a_playlist() {
    let _config = ConfigDir::new("queue-save");
    let (server, mut harness) = playing();
    server.json_value("POST", "/Playlists", &serde_json::json!({ "Id": "made" }));
    harness.state_mut().services().enqueue(vec![episode(2), movie()], false);

    plain(&mut harness, egui::Key::Q);
    harness.get_by_label("Save as playlist…").click();
    for _ in 0..4 {
        harness.step();
    }
    assert!(rows(&harness).is_empty(), "the panel closes for the dialog");
    // The name starts as the show's; Enter saves.
    plain(&mut harness, egui::Key::Enter);
    for _ in 0..50 {
        if server.requests("POST", "/Playlists") > 0 {
            break;
        }
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let bodies = server.bodies("POST", "/Playlists");
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    assert_eq!(bodies[0]["Name"], "Test Show");
    assert_eq!(
        bodies[0]["Ids"],
        serde_json::json!([episode(1).id, episode(2).id, movie().id])
    );
}

/// The keyboard reaches *Save as playlist…* at the end of the entries, and only with entries.
#[test]
fn save_is_the_last_stop_of_the_panels_keys() {
    let _config = ConfigDir::new("queue-save-keys");
    let (server, mut harness) = playing();
    server.json_value("POST", "/Playlists", &serde_json::json!({ "Id": "made" }));

    // Nothing queued: no button to save.
    plain(&mut harness, egui::Key::Q);
    assert!(harness.query_by_label("Save as playlist…").is_none());
    plain(&mut harness, egui::Key::Q);

    harness.state_mut().services().enqueue(vec![episode(2)], false);
    plain(&mut harness, egui::Key::Q);
    assert!(harness.query_by_label("Save as playlist…").is_some());
    // Up from the first entry goes round to the end: the button.
    plain(&mut harness, egui::Key::ArrowUp);
    plain(&mut harness, egui::Key::ArrowUp);
    plain(&mut harness, egui::Key::ArrowUp);
    plain(&mut harness, egui::Key::Enter);
    assert!(rows(&harness).is_empty(), "the dialog took over from the panel");
    plain(&mut harness, egui::Key::Enter);
    for _ in 0..50 {
        if server.requests("POST", "/Playlists") > 0 {
            break;
        }
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(server.requests("POST", "/Playlists"), 1);
}

/// Adding to the queue while the player waits to reconnect puts the items in the queue that's
/// there, and the interrupted item plays on when the reconnect starts.
#[test]
fn adding_while_a_reconnect_waits_keeps_the_queue_and_the_reconnect() {
    let _config = ConfigDir::new("queue-reconnect");
    let (_server, mut harness) = playing();
    let id = |index: usize| episode(index).id;
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(2)], false);
    services.lose_for_test();
    assert!(services.reconnect_pending_for_test());

    services.enqueue(vec![movie()], false);
    assert!(services.reconnect_pending_for_test(), "the reconnect goes ahead");
    assert_eq!(queued(&mut harness), [id(2), movie().id]);
    // Play next goes the same way.
    let services = harness.state_mut().services();
    services.enqueue(vec![episode(0)], true);
    assert!(services.reconnect_pending_for_test());
    assert_eq!(queued(&mut harness), [id(0), id(2), movie().id]);
}

/// Asking for something to play next or adding to the queue undoes dismissing the up-next card,
/// so the queued item plays when this one ends.
#[test]
fn queueing_after_dismissing_the_up_next_card_plays_the_queued_item() {
    let _config = ConfigDir::new("queue-after-dismissed");
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    {
        let state = services.playback.now_playing_mut().expect("the session");
        state.autoplay = true;
        state.next_cancelled = true;
    }
    services.enqueue(vec![episode(2)], false);
    assert!(!services.playback.now_playing().expect("the session").next_cancelled);
    assert_eq!(services.end_for_test(), Some(episode(2).id));
}

/// Repeat all starts over at the end of what the queue holds, a lone item included, rather than
/// moving on to the series' following episode.
#[test]
fn repeat_all_wraps_before_the_following_episode() {
    use jellyfin::RepeatMode;

    let _config = ConfigDir::new("queue-repeat-all-wrap");
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    services.set_repeat(RepeatMode::RepeatAll);
    {
        // As the details leave a mid-season episode: a following one, and nothing queued.
        let state = services.playback.now_playing_mut().expect("the session");
        state.autoplay = true;
        state.next = Some(episode(2));
    }
    assert_eq!(services.end_for_test(), Some(episode(1).id), "the lone item plays again");

    // Without repeat the following episode plays, as before.
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    {
        let state = services.playback.now_playing_mut().expect("the session");
        state.autoplay = true;
        state.next = Some(episode(2));
    }
    assert_eq!(services.end_for_test(), Some(episode(2).id));
}

/// Playing the item again with repeat one keeps its version and tracks.
#[test]
fn repeat_one_keeps_the_version_and_tracks() {
    use jellyfin::RepeatMode;

    use crate::playback::{PlayOptions, SubtitleChoice};

    let _config = ConfigDir::new("queue-repeat-one-tracks");
    let (_server, mut harness) = playing();
    let services = harness.state_mut().services();
    services.set_repeat(RepeatMode::RepeatOne);
    let source_id = {
        let state = services.playback.now_playing_mut().expect("the session");
        state.audio = Some(3);
        state.subtitle = SubtitleChoice::Stream(5);
        state.source.id.clone()
    };
    // It was started with the defaults and the user chose since; what plays again is the choice.
    assert_eq!(
        services.end_with_for_test(PlayOptions::default()),
        Some(episode(1).id)
    );
    let (_, options) = services.last_started_for_test();
    let options = options.expect("a new session");
    assert_eq!(options.media_source_id, Some(source_id));
    assert_eq!(options.audio_stream_index, Some(3));
    assert_eq!(options.subtitle, SubtitleChoice::Stream(5));
    assert!(options.start.is_none(), "from its start");
}

