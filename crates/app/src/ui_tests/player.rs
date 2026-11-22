//! The desktop player page driven by its keys: the bindings of `keys::Scope::Player`, taken from
//! egui and handled by the page as the shell routes them, over a session with no player behind it
//! whose commands the test reads back.

use egui::Modifiers;
use egui_kittest::Harness;

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

/// A second subtitle is the menu's pick: it goes to the session, a track can't be both the first
/// and the second, and a cast has none.
#[test]
fn a_second_subtitle_is_picked_and_cleared() {
    use crate::playback::SubtitleChoice;

    let _config = ConfigDir::new("player-second-subtitle");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();
    let state = harness.state_mut().now_playing_mut().expect("the session");
    let mut extra = state.source.media_streams.last().expect("a subtitle").clone();
    extra.index = 4;
    state.source.media_streams.push(extra);

    let services = harness.state_mut().services_mut().expect("the services");
    services.change_secondary_subtitle(Some(4));
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSecondarySubtitle(Some(4))]),
        "{sent:?}"
    );
    assert_eq!(harness.state().now_playing().unwrap().secondary_subtitle, Some(4));

    // Asking for the same again sends nothing.
    harness
        .state_mut()
        .services_mut()
        .unwrap()
        .change_secondary_subtitle(Some(4));
    assert!(harness.state_mut().commands().is_empty());

    // Making it the first track drops it as the second.
    harness
        .state_mut()
        .services_mut()
        .unwrap()
        .change_subtitle(SubtitleChoice::Stream(4));
    let sent = harness.state_mut().commands();
    assert!(
        matches!(
            sent.as_slice(),
            [
                UiCommand::SetSecondarySubtitle(None),
                UiCommand::Subtitle(SubtitleChoice::Stream(4))
            ]
        ),
        "{sent:?}"
    );
    assert_eq!(harness.state().now_playing().unwrap().secondary_subtitle, None);
}

/// The seek step setting is how far the arrow keys jump, in both directions.
#[test]
fn the_seek_step_setting_sets_how_far_the_arrows_jump() {
    let _config = ConfigDir::new("player-seek-step");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();
    harness
        .state_mut()
        .services_mut()
        .expect("the services")
        .playback
        .set_seek_step(Some(30));

    plain(&mut harness, egui::Key::ArrowRight);
    plain(&mut harness, egui::Key::ArrowLeft);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekBy(a), UiCommand::SeekBy(b)] if *a == 30. && *b == -30.),
        "{sent:?}"
    );

    // A value Settings doesn't offer falls back to ten seconds.
    let playback = &mut harness.state_mut().services_mut().unwrap().playback;
    playback.set_seek_step(Some(7));
    assert_eq!(playback.seek_step(), 10.);
    playback.set_seek_step(None);
    assert_eq!(playback.seek_step(), 10.);
}

/// A cycles the aspect modes, fit, fill, stretch, 16:9, 4:3 and round to fit, each a command to
/// the session; each mode sets every property it uses, so the last one's leftovers don't stay.
#[test]
fn a_cycles_the_aspect_modes() {
    use crate::playback::Aspect;

    let _config = ConfigDir::new("player-aspect");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    for expected in [
        Aspect::Fill,
        Aspect::Stretch,
        Aspect::Widescreen,
        Aspect::Standard,
        Aspect::Fit,
    ] {
        plain(&mut harness, egui::Key::A);
        let sent = harness.state_mut().commands();
        assert!(
            matches!(sent.as_slice(), [UiCommand::SetAspect(a)] if *a == expected),
            "{sent:?}"
        );
        assert_eq!(harness.state().now_playing().unwrap().aspect, expected);
    }

    // Not on a player that can't (Media3).
    harness.state_mut().now_playing_mut().unwrap().mpv = false;
    plain(&mut harness, egui::Key::A);
    assert!(harness.state_mut().commands().is_empty());

    let names = |aspect: Aspect| aspect.mpv_properties().map(|(name, _)| name);
    assert!(Aspect::ALL.iter().all(|&aspect| names(aspect) == names(Aspect::Fit)));
    let props: Vec<_> = Aspect::Fill.mpv_properties().into_iter().collect();
    assert_eq!(props[0], ("panscan", serde_json::json!(1.)));
    assert_eq!(
        Aspect::Stretch.mpv_properties()[1],
        ("keepaspect", serde_json::json!(false))
    );
    assert_eq!(
        Aspect::Widescreen.mpv_properties()[2],
        ("video-aspect-override", serde_json::json!("16:9"))
    );
}

/// Deinterlacing is the menu's switch, and starts on only for an interlaced stream that mpv gets
/// as it is.
#[test]
fn deinterlacing_is_a_switch_that_starts_on_for_interlaced_video() {
    let _config = ConfigDir::new("player-deinterlace");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    let services = harness.state_mut().services_mut().expect("the services");
    services.set_deinterlace(true);
    services.set_deinterlace(true);
    services.set_deinterlace(false);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(
            sent.as_slice(),
            [UiCommand::SetDeinterlace(true), UiCommand::SetDeinterlace(false)]
        ),
        "{sent:?}"
    );
}
