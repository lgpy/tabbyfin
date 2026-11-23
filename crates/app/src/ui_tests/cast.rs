//! Casting against the fake Chromecast (`cast::fake`), a TLS server on loopback that speaks the
//! device's namespaces and records what the receiver app is sent: the redirects of playing and
//! the controls, the credentials every message carries, and what the receiver's reports become.

use std::time::{Duration, Instant};

use cast::Device;
use cast::fake::FakeDevice;
use egui::accesskit::Role;
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jellyfin::Ticks;
use jellyfin::models::BaseItemDto;
use serde_json::Value;

use super::{ConfigDir, FixtureServer, TIMEOUT, TOKEN, block_on, fixture_json, session};
use crate::config::Config;
use crate::keys::{self, Press, Scope};
use crate::playback::{PlayOptions, SubtitleChoice, UiCommand};
use crate::services::{AppEvent, Services};

/// Signed-in services with the fake device found on the network, and the device.
struct Casting {
    services: Services,
    device: FakeDevice,
    ctx: egui::Context,
    _server: FixtureServer,
}

impl Casting {
    fn new() -> Self {
        Self::with_running_app(None)
    }

    /// Signed-in services with the fake device found; `running_app` is what its ad says it runs
    /// (`rs`), which is where a reconnect offer comes from.
    fn with_running_app(running_app: Option<&str>) -> Self {
        let server = FixtureServer::start();
        let ctx = egui::Context::default();
        super::assert_config_dir();
        let mut services = Services::new(session(&server), &ctx);
        let device = block_on(FakeDevice::start()).expect("the fake device starts");
        services.cast.add_device(Device {
            id: "fake-tv".into(),
            name: "Living room TV".into(),
            model: "Chromecast".into(),
            address: device.addr().ip(),
            port: device.addr().port(),
            running_app: running_app.map(str::to_owned),
        });
        services.tick(&ctx);
        Self {
            services,
            device,
            ctx,
            _server: server,
        }
    }

    /// Ticks the services, as frames would, until `done`.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let start = Instant::now();
        loop {
            self.services.tick(&self.ctx);
            if done(self) {
                return;
            }
            assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Ticks the services for `period`, whatever happens.
    fn tick_for(&mut self, period: std::time::Duration) {
        let start = Instant::now();
        while start.elapsed() < period {
            self.services.tick(&self.ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Which reconnect attempt is on its way, while the connection is being retried.
    fn reconnecting(&self) -> Option<u32> {
        self.services
            .cast
            .session
            .as_ref()
            .and_then(|session| session.reconnecting)
    }

    fn connected(&self) -> bool {
        self.services
            .cast
            .session
            .as_ref()
            .is_some_and(|session| session.connected)
    }

    /// The item the TV is reported to play.
    fn item_id(&self) -> Option<String> {
        self.services.cast.casting().and_then(|casting| casting.item_id.clone())
    }

    fn connect(&mut self) {
        self.services.start_cast("fake-tv");
        self.until("the receiver to connect", |this| this.connected());
    }

    /// The commands the receiver app got, in order.
    fn commands(&self) -> Vec<String> {
        self.device
            .envelopes()
            .iter()
            .filter_map(|envelope| envelope["command"].as_str().map(str::to_owned))
            .collect()
    }

    /// Waits for the receiver app to get `command`, and returns its envelope.
    fn wait_for(&mut self, command: &str) -> Value {
        self.until(command, |this| this.commands().iter().any(|sent| sent == command));
        self.device
            .envelopes()
            .into_iter()
            .rev()
            .find(|envelope| envelope["command"] == command)
            .expect("the command was sent")
    }

    fn events(&mut self) -> Vec<AppEvent> {
        std::mem::take(&mut self.services.events)
    }
}

fn movie() -> BaseItemDto {
    fixture_json("item_movie")
}

#[test]
fn casting_launches_the_receiver_and_identifies_with_the_sessions_credentials() {
    let _config = ConfigDir::new("cast-identify");
    let mut casting = Casting::new();
    assert!(casting.services.cast.has_devices());
    casting.connect();
    let identify = casting.wait_for("Identify");
    assert_eq!(identify["accessToken"], TOKEN);
    assert_eq!(identify["receiverName"], "Living room TV");
    assert_eq!(identify["serverId"], "a93d3c61bce64f4d8e8456ddf14de713");
    assert_eq!(identify["serverVersion"], "12.2.0");
    assert_eq!(identify["deviceId"], "ui-tests");
    // The test server is on loopback, which means nothing to a TV: the server's own address
    // goes instead.
    assert_eq!(identify["serverAddress"], "http://172.18.0.2:8096");
    assert!(identify.get("maxBitrate").is_none());
    assert_eq!(Config::load().cast_device.as_deref(), Some("fake-tv"));
}

#[test]
fn playing_while_casting_goes_to_the_tv() {
    let _config = ConfigDir::new("cast-play");
    let mut casting = Casting::new();
    casting.connect();
    casting.events();
    let movie = movie();
    casting.services.play(
        &movie,
        PlayOptions {
            start: Some(Ticks(600_000_000)),
            audio_stream_index: Some(1),
            subtitle: SubtitleChoice::Off,
            ..Default::default()
        },
    );
    assert!(!casting.services.playback.is_active(), "nothing plays here");
    assert!(
        casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackStarted)),
        "the player opens"
    );
    let play = casting.wait_for("PlayNow");
    let options = &play["options"];
    assert_eq!(options["items"][0]["Id"], movie.id.as_str());
    assert_eq!(options["items"][0]["MediaType"], "Video");
    assert_eq!(options["startPositionTicks"], 600_000_000);
    assert_eq!(options["audioStreamIndex"], 1);
    assert_eq!(options["subtitleStreamIndex"], -1);
    // The fake answers with progress, which the casting state follows.
    casting.until("the receiver's progress", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| casting.playing && casting.item_id.is_some())
    });
}

#[test]
fn tracks_and_versions_play_the_item_again() {
    let _config = ConfigDir::new("cast-tracks");
    let mut casting = Casting::new();
    casting.connect();
    let movie = movie();
    casting.services.play(
        &movie,
        PlayOptions {
            media_source_id: Some("source-1".into()),
            audio_stream_index: Some(1),
            subtitle: SubtitleChoice::Off,
            ..Default::default()
        },
    );
    casting.until("the receiver's progress", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| casting.playing && casting.item_id.is_some())
    });
    let plays = |this: &Casting| -> Vec<Value> {
        this.device
            .envelopes()
            .into_iter()
            .filter(|envelope| envelope["command"] == "PlayNow")
            .collect()
    };

    // Another audio track: the same version, from where it is, in whole ticks.
    casting.services.change_audio(2);
    casting.until("the audio reload", |this| plays(this).len() == 2);
    let audio = &plays(&casting)[1]["options"];
    assert_eq!(audio["items"][0]["Id"], movie.id.as_str());
    assert_eq!(audio["mediaSourceId"], "source-1");
    assert_eq!(audio["audioStreamIndex"], 2);
    assert_eq!(audio["subtitleStreamIndex"], -1);
    assert!(audio["startPositionTicks"].is_i64(), "{}", audio["startPositionTicks"]);
    assert!(!casting.commands().iter().any(|command| command.starts_with("Set")));

    // Another version: its own tracks, the server's choice.
    casting.services.reload(PlayOptions {
        media_source_id: Some("source-2".into()),
        ..Default::default()
    });
    casting.until("the version reload", |this| plays(this).len() == 3);
    let version = &plays(&casting)[2]["options"];
    assert_eq!(version["mediaSourceId"], "source-2");
    assert!(version["audioStreamIndex"].is_null());
    assert!(version["subtitleStreamIndex"].is_null());

    // Each reload stops first and plays once the receiver reported that stop, and the player
    // stays open through it.
    let commands = casting.commands();
    let after_first_play: Vec<&str> = commands
        .iter()
        .skip_while(|command| *command != "PlayNow")
        .skip(1)
        .map(String::as_str)
        .collect();
    assert_eq!(after_first_play, ["Stop", "PlayNow", "Stop", "PlayNow"]);
    assert!(
        !casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackEnded))
    );
}

#[test]
fn the_end_of_an_item_closes_the_player() {
    let _config = ConfigDir::new("cast-finished");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| casting.playing && casting.item_id.is_some())
    });
    // The receiver's own start, then what a Chromecast sent at the end of a movie: a stop with
    // empty ids.
    casting.device.send_app_event(serde_json::json!({
        "type": "playbackstart",
        "data": {"ItemId": "d265861eae31cee1d44b13adac305eac", "NowPlayingItem": {"Id": "d265861eae31cee1d44b13adac305eac"}}
    }));
    casting.until("the receiver's start", |this| {
        this.services
            .cast
            .session
            .as_ref()
            .is_some_and(|session| !session.starting)
    });
    casting.events();
    casting.device.send_app_event(serde_json::json!({
        "type": "playbackstop",
        "data": {
            "ItemId": "",
            "NowPlayingItem": {"Id": "", "RunTimeTicks": 600_230_000},
            "PlayState": {"ItemId": "", "MediaSourceId": "", "PositionTicks": 600_001_580, "IsPaused": false}
        }
    }));
    casting.until("the player to close", |this| {
        this.services
            .events
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackEnded))
    });
    let casting_state = casting.services.cast.casting().expect("still connected");
    assert!(!casting_state.playing);
    assert_eq!(casting_state.item_id, None);
}

#[test]
fn wheeling_the_volume_keeps_the_cast() {
    let _config = ConfigDir::new("cast-wheel");
    let mut casting = Casting::new();
    casting.connect();
    // A flick of the wheel: many steps at once. Only the last level matters.
    for step in 1..=20 {
        casting.services.control(UiCommand::SetVolume(f64::from(step)));
    }
    casting.until("the TV's volume", |this| {
        this.device
            .messages()
            .iter()
            .rev()
            .find(|message| message.payload["type"] == "SET_VOLUME")
            .is_some_and(|message| (message.payload["volume"]["level"].as_f64().unwrap_or(0.) - 0.2).abs() < 1e-6)
    });
    let sent = casting
        .device
        .messages()
        .iter()
        .filter(|message| message.payload["type"] == "SET_VOLUME")
        .count();
    assert!(
        sent < 20,
        "the steps queued behind a request are coalesced ({sent} sent)"
    );
    casting.services.control(UiCommand::TogglePause);
    casting.wait_for("PlayPause");
    assert!(casting.services.is_casting());
}

#[test]
fn the_controls_go_to_the_receiver_while_casting() {
    let _config = ConfigDir::new("cast-controls");
    let mut casting = Casting::new();
    casting.connect();
    let services = &mut casting.services;
    services.control(UiCommand::TogglePause);
    services.control(UiCommand::SetPause(true));
    services.control(UiCommand::SetPause(false));
    services.control(UiCommand::SeekTo(std::time::Duration::from_secs(90)));
    // Not sent: the receiver has no control for speed, and other tracks play the item again,
    // which nothing is yet.
    services.control(UiCommand::SetSpeed(1.5));
    services.change_audio(2);
    services.change_subtitle(SubtitleChoice::Stream(3));
    services.play_next();
    services.play_previous();
    services.enqueue(vec![movie()], true);
    services.control(UiCommand::Stop);
    casting.wait_for("Stop");
    let expected = [
        "PlayPause",
        "Pause",
        "Unpause",
        "Seek",
        "NextTrack",
        "PreviousTrack",
        "PlayNow",
        "Stop",
    ];
    let commands = casting.commands();
    let after_identify: Vec<&str> = commands.iter().skip(1).map(String::as_str).collect();
    // Nothing played yet, so *Play next* plays now.
    assert_eq!(after_identify, expected);
    let seek = casting.wait_for("Seek");
    assert_eq!(seek["options"]["position"], 90.);
}

#[test]
fn volume_is_the_tvs() {
    let _config = ConfigDir::new("cast-volume");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.control(UiCommand::SetVolume(30.));
    casting.until("the TV's volume", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| (casting.volume - 30.).abs() < 0.5)
    });
    assert!(
        !casting.commands().iter().any(|command| command.contains("Volume")),
        "volume isn't a receiver command"
    );
}

#[test]
fn stopping_the_cast_stops_the_receiver_and_closes_the_player() {
    let _config = ConfigDir::new("cast-stop");
    let mut casting = Casting::new();
    casting.connect();
    casting.events();
    casting.services.stop_cast();
    assert!(!casting.services.is_casting());
    assert!(
        casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackEnded))
    );
    casting.until("the receiver to stop", |this| {
        this.device
            .messages()
            .iter()
            .any(|message| message.payload["type"] == "STOP")
    });
}

/// The receiver the connection launched is stopped, though the services that asked for it are
/// gone before the launch finished: the `Stop` they queued is acted on when it's up.
fn assert_stopped_while_connecting(name: &str, leave: impl FnOnce(&mut Services)) {
    let _config = ConfigDir::new(name);
    let mut casting = Casting::new();
    // A slow launch, so there's time to leave while it's still connecting.
    casting.device.set_launch_delay(Duration::from_millis(300));
    casting.services.start_cast("fake-tv");
    assert!(!casting.connected(), "still connecting");
    leave(&mut casting.services);
    assert!(!casting.services.is_casting());
    casting.until("the receiver to be stopped", |this| {
        this.device
            .messages()
            .iter()
            .any(|message| message.payload["type"] == "STOP")
    });
    assert!(!casting.device.is_running(), "the TV is back on its idle screen");
}

#[test]
fn stopping_while_connecting_stops_the_receiver() {
    assert_stopped_while_connecting("cast-stop-connecting", Services::stop_cast);
}

#[test]
fn signing_out_while_connecting_stops_the_receiver() {
    assert_stopped_while_connecting("cast-signout-connecting", Services::end);
}

#[test]
fn disconnecting_while_connecting_leaves_the_receiver_alone() {
    let _config = ConfigDir::new("cast-disconnect-connecting");
    let mut casting = Casting::new();
    casting.device.set_launch_delay(Duration::from_millis(300));
    casting.services.start_cast("fake-tv");
    casting.services.disconnect_cast();
    casting.until("the connection to close", |this| this.device.disconnects() > 0);
    assert!(
        !casting
            .device
            .messages()
            .iter()
            .any(|message| message.payload["type"] == "STOP"),
        "only stopping stops the receiver"
    );
    assert!(casting.device.is_running());
}

/// A second item, for the tests that play one while another is going.
fn other_movie() -> BaseItemDto {
    BaseItemDto {
        id: "other-item".into(),
        name: "Another movie".into(),
        ..movie()
    }
}

/// The ids of the items the receiver was told to play, in order.
fn played(casting: &Casting) -> Vec<String> {
    casting
        .device
        .envelopes()
        .iter()
        .filter(|envelope| envelope["command"] == "PlayNow")
        .filter_map(|envelope| envelope["options"]["items"][0]["Id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_play_made_while_reconnecting_reaches_the_tv_once_its_back() {
    let _config = ConfigDir::new("cast-play-reconnecting");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.device.close();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    // The connection's task throws away commands while it waits to retry, so the play is
    // kept here and goes out with the rejoin.
    casting.services.play(&other_movie(), PlayOptions::default());
    casting.until("the other item to reach the TV", |this| played(this).len() == 2);
    assert_eq!(played(&casting)[1], "other-item");
    casting.until("the TV's report of it", |this| {
        this.item_id().as_deref() == Some("other-item")
    });
    // Once: the replay isn't repeated by later reconnects.
    assert_eq!(played(&casting).len(), 2);
}

#[test]
fn a_play_lost_to_a_dropped_connection_isnt_waited_for() {
    let _config = ConfigDir::new("cast-play-lost");
    let mut casting = Casting::new();
    casting.connect();
    let movie = movie();
    casting.services.play(&movie, PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    // The play is made as the connection drops, before the app has heard of it: it never
    // reaches the TV, which goes on with the first item.
    casting.device.close();
    casting.services.play(&other_movie(), PlayOptions::default());
    let starting = |this: &Casting| {
        this.services
            .cast
            .session
            .as_ref()
            .is_some_and(|session| session.starting)
    };
    assert!(starting(&casting), "the player waits for it");
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    assert!(
        !starting(&casting),
        "nothing is starting once the connection is known to be down"
    );
    casting.until("the cast to be back", |this| {
        this.connected() && this.reconnecting().is_none()
    });
    casting.until("the TV's own item", |this| {
        this.item_id().as_deref() == Some(movie.id.as_str())
    });
    // So when it ends, the player closes rather than staying open on the item that never came.
    casting.events();
    casting.device.send_app_event(serde_json::json!({
        "type": "playbackstop",
        "data": {"ItemId": "", "NowPlayingItem": {"Id": ""}, "PlayState": {"PositionTicks": 0}}
    }));
    casting.until("the player to close", |this| {
        this.services
            .events
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackEnded))
    });
}

/// What the cast session has queued to hand over, as the item's id and the id the request plays.
fn handover(casting: &Casting) -> Option<(String, String, Option<i64>)> {
    let session = casting.services.cast.session.as_ref()?;
    let (item, request) = session.handover.as_ref()?;
    Some((
        item.id.clone(),
        request.items[0].id.clone(),
        request.start_position_ticks,
    ))
}

/// Local playback of `item`, with no player behind it.
fn play_here(casting: &mut Casting, item: &BaseItemDto) {
    let source = item.media_sources[0].clone();
    let _commands = casting.services.playback.start_for_test(item.clone(), source);
}

#[test]
fn a_play_made_while_connecting_replaces_the_handover() {
    let _config = ConfigDir::new("cast-handover-replaced");
    let mut casting = Casting::new();
    let first = movie();
    play_here(&mut casting, &first);
    casting.device.set_launch_delay(Duration::from_millis(300));
    casting.services.start_cast("fake-tv");
    assert_eq!(
        handover(&casting).map(|(item, ..)| item),
        Some(first.id.clone()),
        "what plays here is on its way to the TV"
    );
    assert!(!casting.connected());
    casting.events();

    // The user plays another item before the TV is up.
    casting.services.play(
        &other_movie(),
        PlayOptions {
            start: Some(Ticks(600_000_000)),
            ..Default::default()
        },
    );
    assert_eq!(
        handover(&casting),
        Some(("other-item".into(), "other-item".into(), Some(600_000_000))),
        "the handover is the new play, not what played here"
    );
    assert!(
        casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackStarted)),
        "the player stays up for it"
    );
    let name = |this: &Casting| this.services.cast.casting().map(|casting| casting.name.clone());
    assert_eq!(name(&casting).as_deref(), Some("Another movie"));

    // Connected, but the local session hasn't reported its stop: nothing goes to the TV, and
    // in particular not the first item at its position, which would play over the new one.
    casting.connect();
    casting.tick_for(Duration::from_millis(200));
    assert!(!played(&casting).contains(&first.id));
    assert_eq!(handover(&casting).map(|(item, ..)| item).as_deref(), Some("other-item"));
}

/// Starting a cast while local playback is still loading has nothing to hand over; the cast
/// still starts clean, and the loading session is stopped rather than left to start beside it.
#[test]
fn a_cast_started_while_local_playback_loads_hands_nothing_over() {
    let _config = ConfigDir::new("cast-loading-local");
    let mut casting = Casting::new();
    casting.services.play(&movie(), PlayOptions::default());
    assert!(casting.services.playback.is_active(), "loading");
    assert!(casting.services.playback.now_playing().is_none());
    assert!(!casting.services.playback.stop_noted_for_test());
    casting.connect();
    assert!(
        casting.services.playback.stop_noted_for_test(),
        "the loading session was stopped, handover or not"
    );
    assert_eq!(handover(&casting), None);
    casting.tick_for(Duration::from_millis(200));
    assert_eq!(played(&casting), Vec::<String>::new());
}

/// A local session loading (as far as the services know) when the cast starts, and the TV
/// connected; the player isn't the cast's yet.
fn cast_over_a_loading_session(casting: &mut Casting) {
    casting.services.playback.start_loading_for_test(movie());
    casting.connect();
    assert!(casting.services.playback.stop_noted_for_test());
    assert!(!casting.services.cast.owns_player(), "nothing is cast yet");
    casting.events();
}

fn raised_player_ended(casting: &mut Casting) -> bool {
    casting
        .events()
        .iter()
        .any(|event| matches!(event, AppEvent::PlaybackEnded))
}

#[test]
fn the_end_of_a_stopped_local_session_leaves_the_casting_player_open() {
    let _config = ConfigDir::new("cast-local-end-after-play");
    let mut casting = Casting::new();
    cast_over_a_loading_session(&mut casting);
    // The user plays something on the TV before the stopped local session has finished ending:
    // the player page is the cast's by now.
    casting.services.play(&other_movie(), PlayOptions::default());
    casting.events();
    assert!(casting.services.cast.owns_player());
    casting.services.end_loading_for_test();
    assert!(!raised_player_ended(&mut casting), "the cast's player stays");
    // Nor does it after the TV has reported the item.
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.services.playback.start_loading_for_test(movie());
    casting.services.end_loading_for_test();
    assert!(!raised_player_ended(&mut casting), "the cast's player stays");
}

#[test]
fn the_end_of_a_stopped_local_session_closes_its_own_player_when_nothing_is_cast() {
    let _config = ConfigDir::new("cast-local-end-alone");
    let mut casting = Casting::new();
    cast_over_a_loading_session(&mut casting);
    casting.services.end_loading_for_test();
    assert!(
        raised_player_ended(&mut casting),
        "a connected TV with nothing on it doesn't hold the local player open"
    );
}

#[test]
fn an_idle_tv_has_nothing_playing() {
    let _config = ConfigDir::new("cast-idle");
    let mut casting = Casting::new();
    casting.connect();
    // The fake TV answers Identify as an idle one does: a volumechange with empty ids.
    casting.until("Identify's answer", |this| {
        this.services
            .cast
            .session
            .as_ref()
            .is_some_and(|session| session.last_event.is_some())
    });
    let session = casting.services.cast.session.as_ref().expect("casting");
    assert_eq!(session.casting.item_id, None);
    assert!(!session.casting.playing);
}

#[test]
fn a_status_without_apps_doesnt_end_the_cast() {
    let _config = ConfigDir::new("cast-status-without-apps");
    let mut casting = Casting::new();
    // The fake TV follows the launch with a volume-only status, as a real one does.
    casting.connect();
    casting.events();
    // The volume's reply comes after that status, so the cast has seen it by then.
    casting.services.control(UiCommand::SetVolume(30.));
    casting.until("the TV's volume", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| (casting.volume - 30.).abs() < 0.5)
    });
    assert!(casting.services.is_casting());
    assert!(
        !casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::Toast(_) | AppEvent::PlaybackEnded))
    );
}

#[test]
fn disconnecting_leaves_the_tv_playing() {
    let _config = ConfigDir::new("cast-disconnect");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.disconnect_cast();
    assert!(!casting.services.is_casting());
    casting.until("the connection to close", |this| this.device.disconnects() > 0);
    // A `CLOSE` would make the receiver quit; dropping the connection leaves it playing.
    assert!(
        !casting
            .device
            .messages()
            .iter()
            .any(|message| message.payload["type"] == "CLOSE" || message.payload["type"] == "STOP")
    );
    assert!(casting.device.is_running());
}

#[test]
fn a_dropped_connection_reconnects_and_keeps_playing() {
    let _config = ConfigDir::new("cast-reconnect");
    let mut casting = Casting::new();
    casting.connect();
    let movie = movie();
    casting.services.play(&movie, PlayOptions::default());
    casting.until("the receiver's progress", |this| {
        this.connected() && this.item_id().is_some()
    });
    casting.events();
    // The TV's connection drops, as a Wi-Fi hiccup would; the receiver keeps playing.
    casting.device.close();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    casting.until("the cast to be back", |this| {
        this.connected() && this.reconnecting().is_none()
    });
    assert!(casting.device.connections() >= 2, "a second connection to the TV");
    // `Identify`'s answer restored the state: the same item, still playing, from where the TV
    // is rather than from the start.
    let state = casting.services.cast.casting().expect("still casting");
    assert_eq!(state.item_id.as_deref(), Some(movie.id.as_str()));
    assert!(state.playing);
    assert!(
        state.position() >= std::time::Duration::from_secs(1),
        "the position came back rather than starting over: {:?}",
        state.position()
    );
    assert!(casting.events().is_empty(), "a reconnect isn't worth a notice");
}

#[test]
fn every_outage_gets_its_own_tries() {
    let _config = ConfigDir::new("cast-reconnect-again");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| {
        this.connected() && this.item_id().is_some()
    });
    // More drops than one outage's tries, each recovered before the next: the cast carries on.
    for _ in 0..4 {
        casting.device.close();
        casting.until("the reconnect", |this| this.reconnecting().is_some());
        casting.until("the cast to be back", |this| {
            this.connected() && this.reconnecting().is_none()
        });
    }
    assert!(casting.services.is_casting());
}

#[test]
fn a_tv_that_stays_away_gives_the_cast_up() {
    let _config = ConfigDir::new("cast-reconnect-lost");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.events();
    // Off the network for good: the retries all fail.
    casting.device.stop();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    casting.until("the cast to end", |this| !this.services.is_casting());
    let events = casting.events();
    assert!(
        events.iter().any(|event| matches!(event, AppEvent::Toast(_))),
        "the lost connection is a notice"
    );
    assert!(
        events.iter().any(|event| matches!(event, AppEvent::PlaybackEnded)),
        "the player closes with it"
    );
}

#[test]
fn stopping_while_reconnecting_ends_the_cast() {
    let _config = ConfigDir::new("cast-reconnect-stop");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.events();
    casting.device.stop();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    casting.services.stop_cast();
    assert!(!casting.services.is_casting());
    // Nothing goes on in the background, and no retry's notice arrives later.
    casting.tick_for(std::time::Duration::from_millis(900));
    assert!(!casting.services.is_casting());
    assert!(
        !casting.events().iter().any(|event| matches!(event, AppEvent::Toast(_))),
        "stopping isn't losing the connection"
    );
}

#[test]
fn signing_out_stops_a_reconnecting_cast() {
    let _config = ConfigDir::new("cast-reconnect-signout");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.events();
    // The TV is still there, so the cast would reconnect; signing out revokes the token the
    // receiver plays with, so it must not.
    casting.device.close();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    let connections = casting.device.connections();
    casting.services.end();
    casting.tick_for(std::time::Duration::from_millis(900));
    assert!(!casting.services.is_casting());
    assert_eq!(
        casting.device.connections(),
        connections,
        "no reconnect after signing out"
    );
    assert!(
        !casting.events().iter().any(|event| matches!(event, AppEvent::Toast(_))),
        "no notice after signing out"
    );
}

#[test]
fn another_sender_is_followed_through_the_media_status() {
    let _config = ConfigDir::new("cast-other-sender");
    let mut casting = Casting::new();
    casting.connect();
    let movie = movie();
    casting.services.play(&movie, PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    // A phone casts something else: the receiver answers it now, so all that reaches us is the
    // device's own status.
    casting.device.send_media_status("phone-item", 300., false);
    casting.until("the other sender's item", |this| {
        this.item_id().as_deref() == Some("phone-item")
    });
    let state = casting.services.cast.casting().expect("still casting");
    assert!(state.playing);
    assert!(
        state.position() >= std::time::Duration::from_secs(299),
        "the other sender's position: {:?}",
        state.position()
    );
    // It keeps following: the phone pauses, ten minutes in.
    casting.device.send_media_status("phone-item", 330., true);
    casting.until("the other sender's pause", |this| {
        this.services
            .cast
            .casting()
            .is_some_and(|casting| casting.paused && casting.position() >= std::time::Duration::from_secs(329))
    });
    // Our next command makes the receiver answer us again, and our item comes back with it.
    casting.services.control(UiCommand::TogglePause);
    casting.wait_for("PlayPause");
    casting.device.send_app_event(serde_json::json!({
        "type": "playbackprogress",
        "data": {
            "ItemId": movie.id,
            "PlayState": {"PositionTicks": 42_000_000_000i64, "IsPaused": false, "CanSeek": true},
            "NowPlayingItem": {"Id": movie.id, "Name": movie.name, "Type": "Movie", "RunTimeTicks": 1_440_000_000i64}
        }
    }));
    casting.until("our own item again", |this| {
        this.item_id().as_deref() == Some(movie.id.as_str())
    });
}

#[test]
fn casting_is_off_offline() {
    let _config = ConfigDir::new("cast-offline");
    let server = FixtureServer::start();
    let ctx = egui::Context::default();
    let mut services = Services::offline(session(&server), &ctx);
    assert!(!services.cast.is_online(), "an offline session finds no devices");
    // Even with a device listed, there's no server for the TV to play from.
    services.cast.add_device(Device {
        id: "fake-tv".into(),
        name: "Living room TV".into(),
        model: "Chromecast".into(),
        address: [127, 0, 0, 1].into(),
        port: 8009,
        running_app: Some("Jellyfin".into()),
    });
    services.tick(&ctx);
    assert!(services.cast.has_devices(), "the device is listed");
    services.start_cast("fake-tv");
    assert!(!services.is_casting(), "a cast needs the server");
}

#[test]
fn casting_is_off_in_a_syncplay_group() {
    let _config = ConfigDir::new("cast-syncplay");
    let mut casting = Casting::new();
    casting.services.syncplay.group =
        Some(serde_json::from_value(serde_json::json!({"GroupId": "g", "GroupName": "Movie night"})).expect("a group"));
    casting.services.start_cast("fake-tv");
    assert!(!casting.services.is_casting());
}

fn starting(casting: &Casting) -> bool {
    casting
        .services
        .cast
        .session
        .as_ref()
        .is_some_and(|session| session.starting)
}

#[test]
fn a_stop_while_reconnecting_cancels_the_kept_play() {
    let _config = ConfigDir::new("cast-stop-keeps-nothing");
    let mut casting = Casting::new();
    casting.connect();
    let first = movie();
    casting.services.play(&first, PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.device.close();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    // Kept for the rejoin, then not wanted any more.
    casting.services.play(&other_movie(), PlayOptions::default());
    assert!(starting(&casting));
    casting.events();
    casting.services.control(UiCommand::Stop);
    assert!(!starting(&casting), "nothing is starting any more");
    assert!(
        casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackEnded)),
        "the player closes as a stop closes it"
    );
    casting.until("the cast to be back", |this| {
        this.connected() && this.reconnecting().is_none()
    });
    // The TV still plays what it played; the stop is what reaches it, after the rejoin.
    casting.wait_for("Stop");
    casting.tick_for(Duration::from_millis(300));
    assert_eq!(
        played(&casting),
        std::slice::from_ref(&first.id),
        "the kept play wasn't sent"
    );
    assert_eq!(casting.commands(), ["Identify", "PlayNow", "Identify", "Stop"]);
}

/// A pause isn't "not that any more": the kept play still goes out.
#[test]
fn a_pause_while_reconnecting_keeps_the_play() {
    let _config = ConfigDir::new("cast-pause-keeps-play");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.device.close();
    casting.until("the reconnect", |this| this.reconnecting().is_some());
    casting.services.play(&other_movie(), PlayOptions::default());
    casting.services.control(UiCommand::SetPause(true));
    casting.until("the other item to reach the TV", |this| played(this).len() == 2);
    assert_eq!(played(&casting)[1], "other-item");
}

/// One go at a play made while the first connect is retried: the play reaches the TV whichever
/// way the retry went. Returns whether the retry was the first connect's (a second launch,
/// ending in `Update::Connected`).
fn play_during_a_retried_first_connect() -> bool {
    let mut casting = Casting::new();
    // The launch works, but the connection is gone before the app can join the receiver.
    casting.device.drop_after_next_launch();
    casting.services.start_cast("fake-tv");
    casting.until("the retry", |this| this.reconnecting().is_some());
    casting.services.play(&other_movie(), PlayOptions::default());
    assert!(starting(&casting));
    casting.until("the item to reach the TV", |this| played(this) == ["other-item"]);
    casting.until("the TV's report of it", |this| {
        this.item_id().as_deref() == Some("other-item")
    });
    assert_eq!(played(&casting).len(), 1, "once");
    let launches = casting
        .device
        .messages()
        .iter()
        .filter(|message| message.payload["type"] == "LAUNCH")
        .count();
    launches == 2
}

/// The device hangs up right behind the launch's answer, so the app usually finds the connection
/// closed before it can join the receiver: the retry is a launch again and ends in
/// `Update::Connected`. Sometimes the app wins that race, joins, and loses the connection after
/// it (a rejoin, which `a_play_made_while_reconnecting_reaches_the_tv_once_its_back` covers); the
/// test goes again until the first connect's retry has been seen.
#[test]
fn a_play_made_while_the_first_connect_is_retried_reaches_the_tv() {
    let _config = ConfigDir::new("cast-first-connect-retried");
    let seen = (0..10).any(|_| play_during_a_retried_first_connect());
    assert!(seen, "the first connect was never the one retried");
}

/// A play made while the first connect is retried is newer than a handover waiting for the TV: it wins, as a play
/// made while connecting does.
#[test]
fn a_play_made_while_the_first_connect_is_retried_replaces_the_handover() {
    let _config = ConfigDir::new("cast-kept-over-handover");
    let mut casting = Casting::new();
    casting.device.drop_after_next_launch();
    let first = movie();
    play_here(&mut casting, &first);
    casting.services.playback.start_loading_for_test(first.clone());
    casting.services.start_cast("fake-tv");
    assert!(handover(&casting).is_some());
    casting.until("the retry", |this| this.reconnecting().is_some());
    casting.services.play(&other_movie(), PlayOptions::default());
    casting.services.end_loading_for_test();
    casting.until("the item to reach the TV", |this| !played(this).is_empty());
    casting.tick_for(Duration::from_millis(300));
    assert_eq!(played(&casting), ["other-item"], "not what played here");
    assert_eq!(handover(&casting), None);
}

#[test]
fn an_enqueue_during_a_handover_follows_it() {
    let _config = ConfigDir::new("cast-enqueue-handover");
    let mut casting = Casting::new();
    let first = movie();
    play_here(&mut casting, &first);
    casting.services.playback.start_loading_for_test(first.clone());
    casting.device.set_launch_delay(Duration::from_millis(200));
    casting.services.start_cast("fake-tv");
    let at = Ticks::from(Duration::from_secs(12 * 60)).0;
    let pending = Some((first.id.clone(), first.id.clone(), Some(at)));
    assert_eq!(handover(&casting), pending);
    casting.events();

    casting.services.enqueue(vec![other_movie()], true);
    assert_eq!(handover(&casting), pending, "the item being handed over isn't replaced");
    assert!(
        !casting
            .events()
            .iter()
            .any(|event| matches!(event, AppEvent::PlaybackStarted)),
        "nothing was played"
    );
    // Connected, but the local session hasn't reported its stop: nothing goes out yet.
    casting.connect();
    casting.tick_for(Duration::from_millis(200));
    assert_eq!(casting.commands(), ["Identify"]);
    assert_eq!(handover(&casting), pending);

    casting.services.end_loading_for_test();
    casting.wait_for("PlayNext");
    assert_eq!(casting.commands(), ["Identify", "PlayNow", "PlayNext"]);
    let envelopes = casting.device.envelopes();
    assert_eq!(envelopes[1]["options"]["items"][0]["Id"], first.id.as_str());
    assert_eq!(envelopes[1]["options"]["startPositionTicks"], at);
    assert_eq!(envelopes[2]["options"]["items"][0]["Id"], "other-item");
}

#[test]
fn an_enqueue_while_a_play_is_starting_doesnt_replace_it() {
    let _config = ConfigDir::new("cast-enqueue-starting");
    let mut casting = Casting::new();
    casting.connect();
    let first = movie();
    casting.services.play(&first, PlayOptions::default());
    assert!(
        starting(&casting) && casting.item_id().is_none(),
        "sent, not reported yet"
    );
    casting.services.enqueue(vec![other_movie()], false);
    casting.wait_for("PlayLast");
    assert_eq!(played(&casting), std::slice::from_ref(&first.id));
    assert_eq!(casting.commands(), ["Identify", "PlayNow", "PlayLast"]);
}

/// The enqueue a reload made pending goes out after the reload's play, not between its stop and
/// its play (where the receiver would put it in the queue of what's being stopped).
#[test]
fn an_enqueue_during_a_reload_follows_its_play() {
    let _config = ConfigDir::new("cast-enqueue-reload");
    let mut casting = Casting::new();
    casting.connect();
    casting.services.play(&movie(), PlayOptions::default());
    casting.until("the receiver's progress", |this| this.item_id().is_some());
    casting.services.change_audio(2);
    casting.services.enqueue(vec![other_movie()], false);
    casting.until("the reload's play", |this| played(this).len() == 2);
    casting.wait_for("PlayLast");
    assert_eq!(
        casting.commands(),
        ["Identify", "PlayNow", "Stop", "PlayNow", "PlayLast"]
    );
}

/// The signed-in shell as the app runs it, with the fake device on the network, carrying out the
/// player's events as `App` does.
struct ShellRun {
    session: Option<crate::session::Session>,
    services: Option<Services>,
    shell: Option<crate::views::shell::Shell>,
    device: Option<FakeDevice>,
    /// What the device's ad says it runs (`rs`): the reconnect offer's signal.
    running_app: Option<String>,
    /// The presses [`keys::take`] took from egui, for the next frame.
    keys: Vec<Press>,
    /// Kept for the session's requests.
    _server: FixtureServer,
}

impl ShellRun {
    fn new() -> Self {
        Self::with_running_app(None)
    }

    /// The shell with the fake device on the network, its ad saying it runs `running_app`.
    fn with_running_app(running_app: Option<&str>) -> Self {
        let server = FixtureServer::start();
        server.json("GET", "/UserViews", "user_views");
        server.json("GET", "/UserItems/Resume", "resume");
        server.json("GET", "/Shows/NextUp", "next_up");
        server.json("GET", "/Items/Latest", "latest");
        server.json("GET", "/Items", "items_movies");
        server.json("GET", "/Items/Filters", "item_filters");
        Self {
            session: Some(session(&server)),
            services: None,
            shell: None,
            device: Some(block_on(FakeDevice::start()).expect("the fake device starts")),
            running_app: running_app.map(str::to_owned),
            keys: Vec::new(),
            _server: server,
        }
    }

    fn device(&self) -> &FakeDevice {
        self.device.as_ref().expect("the device is up")
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            super::assert_config_dir();
            let mut services = Services::new(session, &ctx);
            let addr = self.device().addr();
            services.cast.add_device(Device {
                id: "fake-tv".into(),
                name: "Living room TV".into(),
                model: "Chromecast".into(),
                address: addr.ip(),
                port: addr.port(),
                running_app: self.running_app.clone(),
            });
            self.services = Some(services);
        }
        let services = self.services.as_mut().expect("the services are made");
        services.tick(&ctx);
        let mut nav = Vec::new();
        let mut cx = crate::views::PageCx {
            ctx: &ctx,
            services,
            nav: &mut nav,
            render_state: None,
            under_bar: None,
        };
        let shell = self
            .shell
            .get_or_insert_with(|| crate::views::shell::Shell::new(&mut cx));
        for event in std::mem::take(&mut cx.services.events) {
            match event {
                AppEvent::PlaybackStarted => shell.player_started(&ctx),
                AppEvent::PlaybackEnded | AppEvent::LeavePlayer => shell.player_ended(&ctx),
                _ => {}
            }
        }
        let presses = std::mem::take(&mut self.keys);
        shell.show(ui, presses, &mut cx);
    }

    fn services(&mut self) -> &mut Services {
        self.services.as_mut().expect("the services are made")
    }
}

fn shell() -> egui_kittest::Harness<'static, ShellRun> {
    shell_with(ShellRun::new())
}

fn shell_with(run: ShellRun) -> egui_kittest::Harness<'static, ShellRun> {
    let mut harness = super::build_harness(egui::vec2(1200., 800.), run, |ui, run| run.show(ui));
    harness.step();
    harness
}

/// Waits, frame by frame, for the fake device to have been sent `command`.
fn sent(harness: &mut egui_kittest::Harness<'static, ShellRun>, command: &str) {
    super::step_until(harness, command, |harness| {
        harness
            .state()
            .device()
            .envelopes()
            .iter()
            .any(|envelope| envelope["command"] == command)
    });
}

/// Presses `key` and its release, taken from egui the way `App::raw_input_hook` takes them when
/// no widget, menu or popup has the keyboard, and hands them to the shell.
fn press(harness: &mut Harness<'static, ShellRun>, key: egui::Key) {
    let ctx = harness.ctx.clone();
    let scopes = harness
        .state()
        .shell
        .as_ref()
        .map(crate::views::shell::Shell::scopes)
        .unwrap_or_else(|| vec![Scope::Shell]);
    {
        let input = harness.input_mut();
        for pressed in [true, false] {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
        }
    }
    let presses = keys::take(&ctx, harness.input_mut(), &scopes);
    harness.state_mut().keys.extend(presses);
    harness.step();
}

/// What the now-casting bar and the panel's card are called to a screen reader.
fn bar_label(movie: &BaseItemDto) -> String {
    format!("{}, on Living room TV", movie.name)
}

/// Casts the movie to the fake TV, then leaves the player for the pages: the now-casting bar is
/// there and so is the panel's card.
fn cast_and_leave(harness: &mut Harness<'static, ShellRun>) -> BaseItemDto {
    super::step_until(harness, "Home's shelves", |harness| {
        harness.query_by_label("My Media").is_some()
    });
    harness.state_mut().services().start_cast("fake-tv");
    super::step_until(harness, "the receiver to connect", |harness| {
        harness
            .state()
            .services
            .as_ref()
            .and_then(|services| services.cast.session.as_ref())
            .is_some_and(|session| session.connected)
    });
    let movie = movie();
    harness.state_mut().services().play(&movie, PlayOptions::default());
    sent(harness, "PlayNow");
    super::step_until(harness, "the casting player", |harness| {
        harness.query_by_label("Pause").is_some()
    });
    harness.get_by_label("Back").click();
    super::step_until(harness, "the now-casting bar", |harness| {
        harness.query_by_label(&bar_label(&movie)).is_some()
    });
    movie
}

#[test]
fn the_cast_panel_lists_the_tv_and_casts_to_it() {
    let _config = ConfigDir::new("cast-panel");
    let mut harness = shell();
    super::step_until(&mut harness, "the cast button", |harness| {
        harness.query_by_label("Cast to a TV").is_some()
    });
    harness.get_by_label("Cast to a TV").click();
    super::step_until(&mut harness, "the cast panel", |harness| {
        harness.query_by_label("Cast to Living room TV").is_some()
    });
    harness
        .get_by_role_and_label(Role::MenuItem, "Cast to Living room TV")
        .click();
    sent(&mut harness, "Identify");
    super::step_until(&mut harness, "the lit cast button", |harness| {
        harness.query_by_label("Casting (cast to a TV)").is_some()
    });
    // While casting the panel marks the TV in use and offers to leave or stop.
    harness.get_by_label("Casting (cast to a TV)").click();
    super::step_until(&mut harness, "the casting panel", |harness| {
        harness.query_by_label("Living room TV, casting").is_some()
    });
    assert!(
        harness.query_by_label("Disconnect, and leave it playing").is_some(),
        "leaving the TV playing is a link under Stop casting"
    );
    assert!(
        harness.query_by_label("Disconnect").is_none(),
        "the old full-width Disconnect button is gone"
    );
    harness.get_by_label("Stop casting").click();
    super::step_until(&mut harness, "the cast to stop", |harness| {
        !harness.state().services.as_ref().is_some_and(Services::is_casting)
    });
}

/// The panel's card is the TV's remote: its play/pause, its skips and its volume go to the
/// receiver, and its body goes back to the player.
#[test]
fn the_cast_panels_card_controls_the_tv() {
    let _config = ConfigDir::new("cast-panel-card");
    let mut harness = shell();
    let movie = cast_and_leave(&mut harness);
    harness.get_by_label("Casting (cast to a TV)").click();
    // The bar has a pause of its own: the card's is the upper one, in the panel.
    super::step_until(&mut harness, "the panel's card", |harness| {
        harness.query_all_by_label("Pause").count() == 2
    });
    let pause = harness
        .query_all_by_label("Pause")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("the card's pause");
    pause.click();
    sent(&mut harness, "PlayPause");

    harness.get_by_label("Forward 10 s").click();
    super::step_until(&mut harness, "the TV to seek", |harness| {
        harness.state().device().envelopes().iter().any(|envelope| {
            envelope["command"] == "Seek" && envelope["options"]["position"].as_f64().is_some_and(|at| at >= 10.)
        })
    });

    // The volume is the TV's: pressing a quarter of the way along the card's slider sets it.
    let slider = harness.get_by_role_and_label(Role::Slider, "TV volume").rect();
    let at = egui::pos2(slider.left() + slider.width() / 4., slider.center().y);
    harness.hover_at(at);
    harness.drag_at(at);
    harness.step();
    harness.drop_at(at);
    harness.step();
    super::step_until(&mut harness, "the TV's volume", |harness| {
        harness.state().device().messages().iter().any(|message| {
            message.payload["type"] == "SET_VOLUME"
                && message.payload["volume"]["level"]
                    .as_f64()
                    .is_some_and(|level| (level - 0.25).abs() < 0.05)
        })
    });

    // Its body, not its controls, goes back to the player.
    let label = bar_label(&movie);
    let card = harness
        .query_all_by_label(&label)
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("the card");
    card.click();
    super::step_until(&mut harness, "the player again", |harness| {
        harness.query_by_label("Back").is_some() && harness.query_by_label("Disconnect, and leave it playing").is_none()
    });
}

/// The keyboard walks to the panel's card first: Enter opens the player, as a click on it does.
#[test]
fn enter_on_the_cards_now_playing_opens_the_player() {
    let _config = ConfigDir::new("cast-panel-now-playing");
    let mut harness = shell();
    cast_and_leave(&mut harness);
    // Enter shows the page's cursor, Up goes into the bar, Right along to Cast, Enter opens the
    // panel on the card.
    press(&mut harness, egui::Key::Enter);
    press(&mut harness, egui::Key::ArrowUp);
    press(&mut harness, egui::Key::ArrowRight);
    press(&mut harness, egui::Key::Enter);
    super::step_until(&mut harness, "the cast panel", |harness| {
        harness.query_by_label("Stop casting").is_some()
    });
    press(&mut harness, egui::Key::Enter);
    super::step_until(&mut harness, "the player again", |harness| {
        harness.query_by_label("Back").is_some() && harness.query_by_label("Disconnect, and leave it playing").is_none()
    });
    assert!(
        harness.state().services.as_ref().is_some_and(Services::is_casting),
        "the cast goes on"
    );
}

/// The bar floats over the pages, so they keep room under their last row: scrolled to its end,
/// Home's lowest control is above the bar rather than behind it.
#[test]
fn the_pages_keep_room_under_the_now_casting_bar() {
    let _config = ConfigDir::new("cast-bar-room");
    let mut harness = shell();
    let movie = cast_and_leave(&mut harness);
    let window = harness.ctx.content_rect();
    harness.hover_at(window.center());
    for _ in 0..80 {
        harness.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0., -300.),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    let bar = harness.get_by_label(&bar_label(&movie)).rect();
    let page_bottom = harness
        .query_all_by_role(Role::Button)
        .filter(|node| !bar.contains_rect(node.rect()))
        .map(|node| node.rect().bottom())
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        page_bottom <= bar.top(),
        "the page's last row ends at {page_bottom}, under the bar at {}",
        bar.top()
    );
}

#[test]
fn the_player_draws_the_cast_and_back_leaves_it_for_the_now_casting_bar() {
    let _config = ConfigDir::new("cast-player");
    let mut harness = shell();
    harness.step();
    harness.state_mut().services().start_cast("fake-tv");
    super::step_until(&mut harness, "the receiver to connect", |harness| {
        harness
            .state()
            .services
            .as_ref()
            .and_then(|services| services.cast.session.as_ref())
            .is_some_and(|session| session.connected)
    });
    let movie = movie();
    harness.state_mut().services().play(&movie, PlayOptions::default());
    sent(&mut harness, "PlayNow");
    super::step_until(&mut harness, "the casting player", |harness| {
        harness.query_by_label("Pause").is_some()
    });
    // The receiver's progress says it's playing; the player's pause goes to the TV.
    harness.get_by_label("Pause").click();
    sent(&mut harness, "PlayPause");
    // Back leaves the player while the cast goes on.
    harness.get_by_label("Back").click();
    let bar = format!("{}, on Living room TV", movie.name);
    super::step_until(&mut harness, "the now-casting bar", |harness| {
        harness.query_by_label(&bar).is_some()
    });
    assert!(harness.state().services.as_ref().is_some_and(Services::is_casting));
    harness.get_by_label(&bar).click();
    super::step_until(&mut harness, "the player again", |harness| {
        harness.query_by_label(&bar).is_none() && harness.query_by_label("Back").is_some()
    });
}

#[test]
fn a_tv_whose_receiver_is_running_offers_to_reconnect() {
    let _config = ConfigDir::new("cast-reconnect-offer");
    let mut harness = shell_with(ShellRun::with_running_app(Some("Jellyfin")));
    super::step_until(&mut harness, "the cast button", |harness| {
        harness.query_by_label("Cast to a TV").is_some()
    });
    harness.get_by_label("Cast to a TV").click();
    super::step_until(&mut harness, "the reconnect offer", |harness| {
        harness.query_by_label("Reconnect to Living room TV").is_some()
    });
    harness
        .get_by_role_and_label(egui::accesskit::Role::MenuItem, "Reconnect to Living room TV")
        .click();
    // Picking it up is the same launch: the receiver is already up, and `Identify` restores it.
    sent(&mut harness, "Identify");
}

#[test]
fn the_now_casting_bar_says_when_the_cast_is_reconnecting() {
    let _config = ConfigDir::new("cast-reconnect-bar");
    let mut harness = shell();
    harness.step();
    harness.state_mut().services().start_cast("fake-tv");
    super::step_until(&mut harness, "the receiver to connect", |harness| {
        harness
            .state()
            .services
            .as_ref()
            .and_then(|services| services.cast.session.as_ref())
            .is_some_and(|session| session.connected)
    });
    let movie = movie();
    harness.state_mut().services().play(&movie, PlayOptions::default());
    sent(&mut harness, "PlayNow");
    super::step_until(&mut harness, "the casting player", |harness| {
        harness.query_by_label("Pause").is_some()
    });
    harness.get_by_label("Back").click();
    super::step_until(&mut harness, "the now-casting bar", |harness| {
        harness
            .query_by_label(&format!("{}, on Living room TV", movie.name))
            .is_some()
    });
    // The connection drops: the bar says so until the TV answers again.
    harness.state().device().close();
    let reconnecting = format!("{}, reconnecting to Living room TV", movie.name);
    super::step_until(&mut harness, "the bar to say it's reconnecting", |harness| {
        harness.query_by_label(&reconnecting).is_some()
    });
    super::step_until(&mut harness, "the bar to say it's back", |harness| {
        harness
            .query_by_label(&format!("{}, on Living room TV", movie.name))
            .is_some()
    });
}
