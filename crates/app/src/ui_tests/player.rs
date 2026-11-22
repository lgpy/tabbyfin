//! The desktop player page driven by its keys: the bindings of `keys::Scope::Player`, taken from
//! egui and handled by the page as the shell routes them, over a session with no player behind it
//! whose commands the test reads back.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::keys::{self, Scope};
use crate::playback::UiCommand;
use crate::ui_tests::touch::PlayerRun;
use crate::ui_tests::{ConfigDir, FixtureServer, build_harness};

/// A desktop window: no touch, so the player draws its desktop controls and reads the mouse.
const DESKTOP: egui::Vec2 = egui::vec2(1280., 720.);

/// The player over a session playing an episode 12 minutes in, with the desktop's layout.
fn desktop_player(server: &FixtureServer) -> Harness<'static, PlayerRun> {
    let mut harness = build_harness(DESKTOP, PlayerRun::new(server), |ui, run| run.show(ui));
    for _ in 0..3 {
        harness.step();
    }
    harness
}

/// A key press with its release, taken from egui as `App::raw_input_hook` takes it and handed to
/// the page before the frame draws, as the shell hands over the player's keys.
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

/// [`press`] without modifiers.
fn plain(harness: &mut Harness<'static, PlayerRun>, key: egui::Key) {
    press(harness, Modifiers::NONE, key);
}

/// Space asks the session to pause, and then to play again.
#[test]
fn space_pauses_and_resumes() {
    let _config = ConfigDir::new("player-space");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::Space);
    let sent = harness.state_mut().commands();
    assert!(matches!(sent.as_slice(), [UiCommand::TogglePause]), "{sent:?}");

    plain(&mut harness, egui::Key::Space);
    let sent = harness.state_mut().commands();
    assert!(matches!(sent.as_slice(), [UiCommand::TogglePause]), "{sent:?}");
}

/// Left and Right seek ten seconds back and forward.
#[test]
fn the_arrow_keys_seek_ten_seconds() {
    let _config = ConfigDir::new("player-seek");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::ArrowLeft);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekBy(by)] if *by == -10.),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::ArrowRight);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekBy(by)] if *by == 10.),
        "{sent:?}"
    );
}

/// Down and Up step the volume by five, and M mutes.
#[test]
fn the_volume_keys_are_down_and_up_and_m_mutes() {
    let _config = ConfigDir::new("player-volume");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    // A full volume to start with: down moves it, and up asks for full again.
    plain(&mut harness, egui::Key::ArrowDown);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetVolume(level)] if *level == 95.),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::ArrowUp);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetVolume(level)] if *level == 100.),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::M);
    let sent = harness.state_mut().commands();
    assert!(matches!(sent.as_slice(), [UiCommand::ToggleMute]), "{sent:?}");
}

/// PgDn seeks to the next chapter and PgUp to the current one's start.
#[test]
fn the_page_keys_jump_between_chapters() {
    let _config = ConfigDir::new("player-chapters");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    // Twelve minutes in, the next chapter is the one at fifteen.
    plain(&mut harness, egui::Key::PageDown);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 15 * 60),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::PageUp);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 0),
        "{sent:?}"
    );
}

/// Enter and S skip the intro playing, to its end.
#[test]
fn enter_and_s_skip_the_intro() {
    let _config = ConfigDir::new("player-skip");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::Enter);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 13 * 60),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::S);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 13 * 60),
        "{sent:?}"
    );
}

/// The bracket keys step the speed up and down through the menu's speeds.
#[test]
fn the_brackets_step_the_speed() {
    let _config = ConfigDir::new("player-speed");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::CloseBracket);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSpeed(speed)] if *speed == 1.25),
        "{sent:?}"
    );
    assert_eq!(
        harness.state().now_playing().expect("the session").adjustments.speed,
        1.25
    );

    plain(&mut harness, egui::Key::OpenBracket);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSpeed(speed)] if *speed == 1.),
        "{sent:?}"
    );
    assert_eq!(
        harness.state().now_playing().expect("the session").adjustments.speed,
        1.
    );
}

/// Z delays the subtitles by a tenth of a second and X brings them back.
#[test]
fn z_and_x_nudge_the_subtitle_delay() {
    let _config = ConfigDir::new("player-delay");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::Z);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSubtitleDelay(delay)] if (*delay + 0.1).abs() < 1e-9),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::X);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSubtitleDelay(delay)] if delay.abs() < 1e-9),
        "{sent:?}"
    );
}

/// Near the end, Esc dismisses the up-next card before it leaves the player: the first press
/// keeps playback going, the second stops it.
#[test]
fn esc_dismisses_the_up_next_card_before_leaving_the_player() {
    let _config = ConfigDir::new("player-esc");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    {
        let state = harness.state_mut().now_playing_mut().expect("the session");
        state.position = std::time::Duration::from_secs(47 * 60 + 50);
        state.position_at = std::time::Instant::now();
    }
    harness.state_mut().commands();
    assert!(harness.state().now_playing().expect("the session").show_up_next());

    plain(&mut harness, egui::Key::Escape);
    let sent = harness.state_mut().commands();
    let state = harness.state_mut().now_playing().expect("the session");
    assert!(state.next_cancelled, "the up-next card was dismissed");
    assert!(!state.show_up_next());
    assert!(
        !sent.iter().any(|command| matches!(command, UiCommand::Stop)),
        "playback went on: {sent:?}"
    );

    plain(&mut harness, egui::Key::Escape);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::Stop]),
        "the second Esc stops: {sent:?}"
    );
}

/// Shift+N asks the server for the next episode, whose playback info is the negotiation's first
/// request. The request is answered with an error, so no player is started.
#[test]
fn shift_n_plays_the_next_episode() {
    const NEXT: &str = "1934096244392b7cb140b7ca490b8ca2";
    let _config = ConfigDir::new("player-next");
    let server = FixtureServer::start();
    server.status("POST", "/Items/1934096244392b7cb140b7ca490b8ca2/PlaybackInfo", 500);
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    press(&mut harness, Modifiers::SHIFT, egui::Key::N);
    let asked = |server: &FixtureServer| server.requests("POST", &format!("/Items/{NEXT}/PlaybackInfo"));
    let start = std::time::Instant::now();
    while asked(&server) == 0 {
        assert!(
            start.elapsed() < crate::ui_tests::TIMEOUT,
            "the next episode was never asked for"
        );
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// A queue of `entries`, the first playing, as the group's server sends it.
fn group_queue(entries: &[&str]) -> jellyfin::PlayQueue {
    let playlist: Vec<_> = entries
        .iter()
        .map(|id| serde_json::json!({ "ItemId": format!("item-{id}"), "PlaylistItemId": id }))
        .collect();
    serde_json::from_value(serde_json::json!({
        "Reason": "NewPlaylist",
        "Playlist": playlist,
        "PlayingItemIndex": 0,
        "IsPlaying": true,
    }))
    .expect("a queue")
}

/// In a group the Next button follows the group's queue, not the item's neighbouring episodes:
/// there with an entry queued after the one playing (named for the direction, not an episode),
/// gone when the queue ends there, however many episodes the series has after it. Previous
/// likewise, at the first entry.
#[test]
fn in_a_group_next_and_previous_follow_the_groups_queue() {
    let _config = ConfigDir::new("player-group-skip");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    // Outside a group the series' neighbours decide.
    assert!(harness.query_by_label("Next episode").is_some());
    assert!(harness.query_by_label("Previous episode").is_some());

    let services = harness.state_mut().services_mut().expect("the services");
    services.syncplay.join_for_test(group_queue(&["a", "b"]), "a");
    assert!(services.can_skip(true) && !services.can_skip(false));
    harness.step();
    assert!(
        harness.query_by_label("Next").is_some(),
        "an entry is queued after this one"
    );
    assert!(harness.query_by_label("Next episode").is_none());
    assert!(
        harness.query_by_label("Previous").is_none(),
        "nothing is queued before it"
    );

    let services = harness.state_mut().services_mut().expect("the services");
    services.syncplay.join_for_test(group_queue(&["a"]), "a");
    harness.step();
    assert!(harness.query_by_label("Next").is_none(), "the queue ends here");
    assert!(
        harness.query_by_label("Next episode").is_none(),
        "the series' next episode isn't the group's"
    );
}
