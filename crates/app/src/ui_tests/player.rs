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

/// A kind set to Skip is skipped as it starts, with a notice whose Undo goes back to the start of
/// the segment and doesn't skip it again.
#[test]
fn a_segment_set_to_skip_is_skipped_and_undo_goes_back() {
    use crate::config::SkipAction;
    use crate::markers::SegmentKind;

    let _config = ConfigDir::new("player-auto-skip");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();
    harness
        .state_mut()
        .now_playing_mut()
        .expect("the session")
        .skip_actions
        .set(SegmentKind::Intro, Some(SkipAction::Skip));
    harness.step();

    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 13 * 60),
        "{sent:?}"
    );
    harness.get_by_label("Skipped intro");

    // The player's position hasn't moved yet, so the segment is still the one playing: it isn't
    // skipped twice, and an automatic kind has no button.
    harness.step();
    assert!(harness.state_mut().commands().is_empty());
    assert!(harness.query_by_label("Skip intro").is_none());

    harness.get_by_label("Undo").click();
    harness.step();
    harness.step();
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 11 * 60),
        "{sent:?}"
    );
    assert!(harness.query_by_label("Undo").is_none());
    harness.step();
    assert!(harness.state_mut().commands().is_empty(), "not skipped again");
}

/// A kind set to Off has no button and isn't skipped; Enter leaves it alone.
#[test]
fn a_segment_set_to_off_is_left_alone() {
    use crate::config::SkipAction;
    use crate::markers::SegmentKind;

    let _config = ConfigDir::new("player-skip-off");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.get_by_label("Skip intro");
    harness
        .state_mut()
        .now_playing_mut()
        .expect("the session")
        .skip_actions
        .set(SegmentKind::Intro, Some(SkipAction::Off));
    harness.step();
    harness.state_mut().commands();
    assert!(harness.query_by_label("Skip intro").is_none());

    plain(&mut harness, egui::Key::Enter);
    assert!(harness.state_mut().commands().is_empty());
}

/// Credits are the up-next card's: a skip button for them only shows when there's no card.
#[test]
fn the_credits_button_gives_way_to_the_up_next_card() {
    use crate::config::SkipAction;
    use crate::markers::{Segment, SegmentKind};

    let _config = ConfigDir::new("player-credits");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let state = harness.state_mut().now_playing_mut().expect("the session");
    let at = |minutes: u64| std::time::Duration::from_secs(minutes * 60);
    state.segments = vec![Segment {
        kind: SegmentKind::Outro,
        start: at(45),
        end: at(48),
    }];
    state.position = at(46);
    // Off, as it starts out: just the card.
    assert!(state.skippable().is_none() && state.show_up_next());
    state.skip_actions.set(SegmentKind::Outro, Some(SkipAction::Ask));
    assert!(state.skippable().is_none(), "the card offers the next episode");
    state.next = None;
    assert_eq!(state.skippable().map(|s| s.end), Some(at(48)));
    // Skip jumps by itself, whether or not there's a next episode.
    state.skip_actions.set(SegmentKind::Outro, Some(SkipAction::Skip));
    assert_eq!(state.auto_skippable().map(|s| s.end), Some(at(48)));
}

/// I shows the playback info: how the item plays, the server's reasons in words, and what the
/// player reported; while it shows the page asks the player for its numbers, and it stops asking
/// when it's hidden.
#[test]
fn i_shows_the_playback_info_and_asks_the_player() {
    use crate::info::Stats;

    let _config = ConfigDir::new("player-info");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    {
        let state = harness.state_mut().now_playing_mut().expect("the session");
        state.stream.method = jellyfin::PlayMethod::Transcode;
        state.stream.transcode_reasons = vec!["VideoCodecNotSupported".into(), "ContainerBitrateExceedsLimit".into()];
        state.stream.transcode_video_codec = Some("h264".into());
        state.player_stats = Some(Stats {
            hwdec: Some("d3d11va".into()),
            size: Some((1280, 720)),
            dropped: Some(4),
            buffered: Some(9.5),
            ..Stats::default()
        });
    }
    harness.state_mut().commands();
    assert!(
        harness.query_by_label("Transcoding").is_none(),
        "hidden until asked for"
    );

    plain(&mut harness, egui::Key::I);
    harness.step();
    for text in [
        "Because",
        "The video codec isn't supported by this player",
        "The bitrate is over the streaming limit",
        "H.264",
        "1280×720",
        "d3d11va",
        "9.5 s",
    ] {
        harness.get_by_label(text);
    }
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::PollInfo, ..]),
        "the player is asked: {sent:?}"
    );

    plain(&mut harness, egui::Key::I);
    harness.step();
    assert!(harness.query_by_label("Because").is_none());
    harness.state_mut().commands();
    harness.step();
    assert!(harness.state_mut().commands().is_empty(), "no polling while hidden");
}

/// The sleep timer pauses (not stops) when it's due and holds autoplay; before that it leaves
/// playback alone.
#[test]
fn the_sleep_timer_pauses_and_holds_autoplay() {
    use std::time::{Duration, Instant};

    use crate::playback::SleepTimer;

    let _config = ConfigDir::new("player-sleep");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().now_playing_mut().expect("the session").autoplay = true;
    harness.state_mut().commands();

    let services = harness.state_mut().services_mut().expect("the services");
    services
        .playback
        .set_sleep(Some(SleepTimer::In(Instant::now() + Duration::from_secs(3600))));
    services.tick_playback();
    assert!(
        services
            .playback
            .sleep_left()
            .is_some_and(|left| left > Duration::from_secs(3500))
    );
    assert!(services.playback.now_playing().expect("the session").autoplay);
    assert!(harness.state_mut().commands().is_empty());

    let services = harness.state_mut().services_mut().expect("the services");
    services.playback.set_sleep(Some(SleepTimer::In(Instant::now())));
    services.tick_playback();
    assert_eq!(services.playback.sleep(), None);
    assert!(!services.playback.now_playing().expect("the session").autoplay);
    let sent = harness.state_mut().commands();
    assert!(matches!(sent.as_slice(), [UiCommand::SetPause(true)]), "{sent:?}");
}

/// "At end" holds back the next episode when this one ends, and is spent by it.
#[test]
fn the_sleep_timers_end_of_item_holds_the_next_episode() {
    use crate::playback::SleepTimer;

    let _config = ConfigDir::new("player-sleep-end");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let services = harness.state_mut().services_mut().expect("the services");
    let state = services.playback.now_playing_mut().expect("the session");
    state.autoplay = true;
    let session = state.session;
    assert!(state.next.is_some());

    services.playback.set_sleep(Some(SleepTimer::EndOfItem(session)));
    assert!(services.playback.take_next(session).is_none(), "held back");
    assert_eq!(services.playback.sleep(), None, "and spent");
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
