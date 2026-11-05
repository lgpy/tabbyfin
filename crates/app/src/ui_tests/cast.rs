//! Casting against the fake Chromecast (`cast::fake`), a TLS server on loopback that speaks the
//! device's namespaces and records what the receiver app is sent: the redirects of playing and
//! the controls, the credentials every message carries, and what the receiver's reports become.

use std::time::Instant;

use cast::Device;
use cast::fake::FakeDevice;
use egui_kittest::kittest::Queryable;
use jellyfin::Ticks;
use jellyfin::models::BaseItemDto;
use serde_json::Value;

use super::{ConfigDir, FixtureServer, TIMEOUT, TOKEN, block_on, fixture_json, session};
use crate::config::Config;
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
        let server = FixtureServer::start();
        let ctx = egui::Context::default();
        let mut services = Services::new(session(&server), &ctx);
        let device = block_on(FakeDevice::start()).expect("the fake device starts");
        services.cast.add_device(Device {
            id: "fake-tv".into(),
            name: "Living room TV".into(),
            model: "Chromecast".into(),
            address: device.addr().ip(),
            port: device.addr().port(),
            running_app: None,
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

    fn connect(&mut self) {
        self.services.start_cast("fake-tv");
        self.until("the receiver to connect", |this| {
            this.services
                .cast
                .session
                .as_ref()
                .is_some_and(|session| session.connected)
        });
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
fn a_device_that_drops_ends_the_cast_with_a_notice() {
    let _config = ConfigDir::new("cast-drop");
    let mut casting = Casting::new();
    casting.connect();
    casting.events();
    casting.device.close();
    casting.until("the cast to end", |this| !this.services.is_casting());
    let events = casting.events();
    assert!(events.iter().any(|event| matches!(event, AppEvent::Toast(_))));
    assert!(events.iter().any(|event| matches!(event, AppEvent::PlaybackEnded)));
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

/// The signed-in shell as the app runs it, with the fake device on the network, carrying out the
/// player's events as `App` does.
struct ShellRun {
    session: Option<crate::session::Session>,
    services: Option<Services>,
    shell: Option<crate::views::shell::Shell>,
    device: Option<FakeDevice>,
    /// Kept for the session's requests.
    _server: FixtureServer,
}

impl ShellRun {
    fn new() -> Self {
        let server = FixtureServer::start();
        Self {
            session: Some(session(&server)),
            services: None,
            shell: None,
            device: Some(block_on(FakeDevice::start()).expect("the fake device starts")),
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
            let mut services = Services::new(session, &ctx);
            let addr = self.device().addr();
            services.cast.add_device(Device {
                id: "fake-tv".into(),
                name: "Living room TV".into(),
                model: "Chromecast".into(),
                address: addr.ip(),
                port: addr.port(),
                running_app: None,
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
        shell.show(ui, Vec::new(), &mut cx);
    }

    fn services(&mut self) -> &mut Services {
        self.services.as_mut().expect("the services are made")
    }
}

fn shell() -> egui_kittest::Harness<'static, ShellRun> {
    let mut harness = super::build_harness(egui::vec2(1200., 800.), ShellRun::new(), |ui, run| run.show(ui));
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

#[test]
fn the_cast_panel_lists_the_tv_and_casts_to_it() {
    use egui::accesskit::Role;

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
    harness.get_by_label("Stop casting").click();
    super::step_until(&mut harness, "the cast to stop", |harness| {
        !harness.state().services.as_ref().is_some_and(Services::is_casting)
    });
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
