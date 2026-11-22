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

    // A full volume to start with: down moves it, and up goes on into the boost (volume-max is
    // 150 on the desktop), since the test's volume is still 100 when it's pressed.
    plain(&mut harness, egui::Key::ArrowDown);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetVolume(level)] if *level == 95.),
        "{sent:?}"
    );

    plain(&mut harness, egui::Key::ArrowUp);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetVolume(level)] if *level == 105.),
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

/// After Undo the segment isn't skipped again by a restart of the same item (a reconnect, a
/// track switch on a transcode), only for another item.
#[test]
fn undo_is_remembered_across_a_restart_of_the_item_but_not_for_another_item() {
    use crate::config::SkipAction;
    use crate::markers::SegmentKind;

    let _config = ConfigDir::new("player-undo-restart");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let (item, source, next, segments) = {
        let state = harness.state_mut().now_playing_mut().expect("the session");
        state.skip_actions.set(SegmentKind::Intro, Some(SkipAction::Skip));
        (
            state.item.clone(),
            state.source.clone(),
            state.next.clone().expect("a next episode"),
            state.segments.clone(),
        )
    };
    harness.step();
    harness.get_by_label("Undo").click();
    harness.step();
    harness.step();
    harness.state_mut().commands();

    // The same item again, in a new session: back inside the intro, and left alone.
    let restart = |harness: &mut Harness<'static, PlayerRun>, item: &jellyfin::models::BaseItemDto| {
        let services = harness.state_mut().services_mut().expect("the services");
        let sent = services.playback.start_for_test(item.clone(), source.clone());
        let state = services.playback.now_playing_mut().expect("the new session");
        state.skip_actions.set(SegmentKind::Intro, Some(SkipAction::Skip));
        state.segments = segments.clone();
        harness.state_mut().sent = Some(sent);
        harness.step();
        harness.step();
        harness.state_mut().commands()
    };
    let sent = restart(&mut harness, &item);
    assert!(sent.is_empty(), "not skipped again: {sent:?}");

    // Another item's intro is skipped as usual.
    let sent = restart(&mut harness, &next);
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 13 * 60),
        "{sent:?}"
    );
}

/// A session that's still loading, or hasn't reported where it is, doesn't skip an intro that
/// starts at zero: it may be resuming at 40:00 and would seek away from it.
#[test]
fn a_resuming_session_is_not_skipped_before_it_reports_its_position() {
    use std::time::Duration;

    use player::PlayerEvent;

    use crate::config::SkipAction;
    use crate::markers::{Segment, SegmentKind};

    let _config = ConfigDir::new("player-skip-resume");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    {
        let state = harness.state_mut().now_playing_mut().expect("the session");
        state.skip_actions.set(SegmentKind::Intro, Some(SkipAction::Skip));
        state.segments = vec![Segment {
            kind: SegmentKind::Intro,
            start: Duration::ZERO,
            end: Duration::from_secs(90),
        }];
        // As a new session starts, resuming at 40:00 with the intro's details already in.
        state.loading = true;
        state.position_known = false;
        state.position = Duration::ZERO;
    }
    harness.state_mut().commands();
    harness.step();
    assert!(harness.state_mut().commands().is_empty(), "loading");

    let services = harness.state_mut().services_mut().expect("the services");
    services.player_event_for_test(PlayerEvent::FileLoaded);
    harness.step();
    assert!(harness.state_mut().commands().is_empty(), "loaded, but no position yet");

    // The first position is the resume point: past the intro, nothing to skip.
    let services = harness.state_mut().services_mut().expect("the services");
    services.player_event_for_test(PlayerEvent::Position(Duration::from_secs(40 * 60)));
    harness.step();
    assert!(harness.state_mut().commands().is_empty(), "resumed past the intro");

    // From the start, the intro is skipped once the position says so.
    let services = harness.state_mut().services_mut().expect("the services");
    services.player_event_for_test(PlayerEvent::Position(Duration::from_secs(1)));
    harness.step();
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SeekTo(at)] if at.as_secs() == 90),
        "{sent:?}"
    );
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
    let item = state.item.clone();
    assert!(state.next.is_some());

    services
        .playback
        .set_sleep(Some(SleepTimer::EndOfItem(item.id.clone())));
    assert_eq!(services.end_for_test(), None, "held back");
    assert_eq!(services.playback.sleep(), None, "and spent");
}

/// With the timer holding the end of an item, nothing plays after it: not the item again with
/// repeat one, nor the queue started over with repeat all, and the queue stays as it was.
#[test]
fn the_sleep_timers_end_of_item_holds_repeat_too() {
    use jellyfin::RepeatMode;

    use crate::playback::SleepTimer;

    for (name, mode) in [("one", RepeatMode::RepeatOne), ("all", RepeatMode::RepeatAll)] {
        let _config = ConfigDir::new(&format!("player-sleep-repeat-{name}"));
        let server = FixtureServer::start();
        let mut harness = desktop_player(&server);
        let services = harness.state_mut().services_mut().expect("the services");
        let item = services.playback.now_playing().expect("the session").item.clone();
        services.set_repeat(mode);
        services
            .playback
            .set_sleep(Some(SleepTimer::EndOfItem(item.id.clone())));
        assert_eq!(services.end_for_test(), None, "repeat {name}: nothing plays after it");
        assert!(
            services
                .events
                .iter()
                .any(|event| matches!(event, crate::services::AppEvent::PlaybackEnded)),
            "repeat {name}: playback ends"
        );
        assert_eq!(services.playback.sleep(), None, "repeat {name}: and the timer is spent");
        assert_eq!(services.playback.modes().0, mode);
        assert!(services.playback.now_playing().is_none());
    }
}

/// A restart of the same item (a dropped connection, another audio track on a transcode, a
/// version) keeps the end-of-item timer; another item starting clears it.
#[test]
fn the_sleep_timers_end_of_item_survives_a_restart_of_its_item() {
    use crate::playback::{PlayOptions, SleepTimer};

    let _config = ConfigDir::new("player-sleep-restart");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let services = harness.state_mut().services_mut().expect("the services");
    let state = services.playback.now_playing_mut().expect("the session");
    state.autoplay = true;
    let (item, source) = (state.item.clone(), state.source.clone());
    let next = state.next.clone().expect("a next episode");
    services
        .playback
        .set_sleep(Some(SleepTimer::EndOfItem(item.id.clone())));

    // The same item again, as a restart does it.
    services.reload(PlayOptions::default());
    drop(services.playback.start_for_test(item.clone(), source.clone()));
    assert!(services.playback.sleep().is_some(), "kept for its item");
    let state = services.playback.now_playing_mut().expect("the new session");
    state.autoplay = true;
    state.next = Some(next.clone());
    assert_eq!(
        services.end_for_test(),
        None,
        "so nothing plays after the restarted session"
    );
    assert_eq!(services.playback.sleep(), None);

    // Another item starting takes it away.
    services
        .playback
        .set_sleep(Some(SleepTimer::EndOfItem(item.id.clone())));
    services.play(&next, PlayOptions::default());
    assert_eq!(services.playback.sleep(), None, "not for another item");
}

/// A timed timer that comes due while no session is live (a reconnect waits, the next item
/// loads) stays due, and pauses once a session is up; the notice shows when it paused.
#[test]
fn a_sleep_timer_due_between_sessions_pauses_the_next_one() {
    use std::time::Instant;

    use crate::playback::{PlayOptions, SleepTimer};

    let _config = ConfigDir::new("player-sleep-between");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let services = harness.state_mut().services_mut().expect("the services");
    let state = services.playback.now_playing().expect("the session");
    let (item, source) = (state.item.clone(), state.source.clone());
    services.reload(PlayOptions::default());
    services.events.clear();
    services.playback.set_sleep(Some(SleepTimer::In(Instant::now())));

    services.tick_playback();
    assert!(services.playback.sleep().is_some(), "still due");
    assert!(
        !services
            .events
            .iter()
            .any(|event| matches!(event, crate::services::AppEvent::Toast(_))),
        "nothing was paused, so nothing is announced"
    );

    let mut sent = services.playback.start_for_test(item, source);
    services.tick_playback();
    assert_eq!(services.playback.sleep(), None);
    assert!(matches!(sent.try_recv(), Ok(UiCommand::SetPause(true))));
    assert!(
        services
            .events
            .iter()
            .any(|event| matches!(event, crate::services::AppEvent::Toast(_))),
        "announced once paused"
    );
}

/// A SyncPlay group's pause is every member's, so a timer running when a group is joined is
/// dropped, without pausing the group.
#[test]
fn a_sleep_timer_is_cancelled_in_a_syncplay_group() {
    use std::time::Instant;

    use crate::playback::SleepTimer;

    let _config = ConfigDir::new("player-sleep-group");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();
    let services = harness.state_mut().services_mut().expect("the services");
    let item = services.playback.now_playing().expect("the session").item.clone();
    for timer in [SleepTimer::In(Instant::now()), SleepTimer::EndOfItem(item.id)] {
        services.playback.set_sleep(Some(timer));
        services.playback.now_playing_mut().expect("the session").group_item = Some("queue-item".into());
        services.tick_playback();
        assert_eq!(services.playback.sleep(), None);
        services.playback.now_playing_mut().expect("the session").group_item = None;
    }
    assert!(harness.state_mut().commands().is_empty(), "no pause for the group");
}

/// The picture choices belong to the item: a restart of it (a lost stream, another track on a
/// transcode, a version) takes them to the new player, and another item starts without.
#[test]
fn a_restart_of_the_item_keeps_the_picture_choices() {
    use std::time::Duration;

    use crate::playback::{AbLoop, Aspect, Picture, PlayOptions};

    let _config = ConfigDir::new("player-picture-restart");
    // Chooses the picture on the running session, and returns what a restart then carries.
    let choose = |harness: &mut Harness<'static, PlayerRun>| {
        let state = harness.state_mut().now_playing_mut().expect("the session");
        state.aspect = Aspect::Fill;
        state.deinterlace = true;
        state.deinterlace_chosen = true;
        state.secondary_subtitle = Some(6);
        state.ab_loop = AbLoop::Loop(Duration::from_secs(30), Duration::from_secs(60));
        Picture {
            source_id: state.source.id.clone(),
            aspect: Aspect::Fill,
            deinterlace: true,
            deinterlace_chosen: true,
            secondary_subtitle: Some(6),
            ab_loop: AbLoop::Loop(Duration::from_secs(30), Duration::from_secs(60)),
        }
    };

    // A restart, as a version switch or a track switch on a transcode makes it.
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    let kept = choose(&mut harness);
    let services = harness.state_mut().services_mut().expect("the services");
    services.reload(PlayOptions::default());
    assert_eq!(
        services.last_started_for_test().1.and_then(|o| o.picture),
        Some(kept.clone())
    );

    // A lost stream waits to reconnect, with the choices too.
    let mut harness = desktop_player(&server);
    let kept = choose(&mut harness);
    let services = harness.state_mut().services_mut().expect("the services");
    services.lose_for_test();
    assert_eq!(
        services.reconnect_options_for_test().and_then(|o| o.picture),
        Some(kept)
    );

    // Another item starts as a new one does.
    let mut harness = desktop_player(&server);
    choose(&mut harness);
    let services = harness.state_mut().services_mut().expect("the services");
    let next = services
        .playback
        .now_playing()
        .and_then(|s| s.next.clone())
        .expect("a next episode");
    services.play(&next, PlayOptions::default());
    assert_eq!(services.last_started_for_test().1.and_then(|o| o.picture), None);
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

/// A new subtitle position goes to the player as mpv's `sub-pos`, and is kept for the next item.
#[test]
fn the_subtitle_position_goes_to_the_player() {
    use crate::playback::Adjustments;

    let _config = ConfigDir::new("player-sub-pos");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    harness.state_mut().adjust(Adjustments {
        subtitle_position: 90,
        ..Adjustments::default()
    });
    harness.step();
    let sent = harness.state_mut().commands();
    assert!(
        matches!(sent.as_slice(), [UiCommand::SetSubtitlePosition(90)]),
        "{sent:?}"
    );
    assert_eq!(harness.state().now_playing().unwrap().adjustments.subtitle_position, 90);
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

/// The period and comma step a frame forward and back, only while paused.
#[test]
fn the_period_and_comma_step_frames_while_paused() {
    let _config = ConfigDir::new("player-frame-step");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    // Playing: nothing to step.
    plain(&mut harness, egui::Key::Period);
    assert!(harness.state_mut().commands().is_empty());

    harness.state_mut().now_playing_mut().unwrap().paused = true;
    plain(&mut harness, egui::Key::Period);
    plain(&mut harness, egui::Key::Comma);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(
            sent.as_slice(),
            [UiCommand::FrameStep(true), UiCommand::FrameStep(false)]
        ),
        "{sent:?}"
    );
}

/// L sets the loop's start, then its end, then clears both.
#[test]
fn l_cycles_the_ab_loop() {
    use crate::playback::AbLoop;

    let _config = ConfigDir::new("player-ab-loop");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();

    plain(&mut harness, egui::Key::L);
    assert!(matches!(harness.state().now_playing().unwrap().ab_loop, AbLoop::A(_)));
    plain(&mut harness, egui::Key::L);
    assert!(matches!(
        harness.state().now_playing().unwrap().ab_loop,
        AbLoop::Loop(..)
    ));
    plain(&mut harness, egui::Key::L);
    assert_eq!(harness.state().now_playing().unwrap().ab_loop, AbLoop::Off);
    let sent = harness.state_mut().commands();
    assert!(
        matches!(
            sent.as_slice(),
            [UiCommand::CycleAbLoop, UiCommand::CycleAbLoop, UiCommand::CycleAbLoop]
        ),
        "{sent:?}"
    );
}

/// In a SyncPlay group neither goes anywhere: a frame step or a loop's jump back would move this
/// member alone.
#[test]
fn a_syncplay_group_has_no_frame_step_or_ab_loop() {
    let _config = ConfigDir::new("player-group-steps");
    let server = FixtureServer::start();
    let mut harness = desktop_player(&server);
    harness.state_mut().commands();
    let state = harness.state_mut().now_playing_mut().unwrap();
    state.paused = true;
    state.group_item = Some("queue-item".into());

    plain(&mut harness, egui::Key::Period);
    plain(&mut harness, egui::Key::Comma);
    plain(&mut harness, egui::Key::L);
    let sent = harness.state_mut().commands();
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(
        harness.state().now_playing().unwrap().ab_loop,
        crate::playback::AbLoop::Off
    );
}
