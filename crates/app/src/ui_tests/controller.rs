//! The player page while casting: the controller (`views::player::controller`) driven against the
//! fake Chromecast (`cast::fake`). Its buttons and sliders go to the TV as the player's own
//! controls would, and the track rows open the phone's sheets.

use cast::Device;
use cast::fake::FakeDevice;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jellyfin::models::BaseItemDto;

use super::{ConfigDir, FixtureServer, block_on, fixture_json, session};
use crate::playback::PlayOptions;
use crate::services::{AppEvent, Services};
use crate::views::PageCx;
use crate::views::shell::Shell;

/// The movie the fixtures give, whose item page the test server answers.
/// The test phone, as `ui_tests/touch.rs` has it.
const PHONE: egui::Vec2 = egui::vec2(369., 760.);

/// The signed-in shell with the fake device on the network, carrying out the player's events as
/// `App` does, at a desktop's or a phone's size.
struct Run {
    session: Option<crate::session::Session>,
    services: Option<Services>,
    shell: Option<Shell>,
    device: FakeDevice,
    /// Kept for the session's requests.
    _server: FixtureServer,
}

impl Run {
    fn new() -> Self {
        let server = FixtureServer::start();
        server.json("GET", "/Items/d265861eae31cee1d44b13adac305eac", "item_movie");
        Self {
            session: Some(session(&server)),
            services: None,
            shell: None,
            device: block_on(FakeDevice::start()).expect("the fake device starts"),
            _server: server,
        }
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.services.is_none() {
            let session = self.session.take().expect("the session is taken once");
            let mut services = Services::new(session, &ctx);
            let addr = self.device.addr();
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
        let mut cx = PageCx {
            ctx: &ctx,
            services,
            nav: &mut nav,
            render_state: None,
            under_bar: None,
        };
        let shell = self.shell.get_or_insert_with(|| Shell::new(&mut cx));
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

    fn device(&self) -> &FakeDevice {
        &self.device
    }

    /// The commands the receiver app got, in order.
    fn commands(&self) -> Vec<String> {
        self.device
            .envelopes()
            .iter()
            .filter_map(|envelope| envelope["command"].as_str().map(str::to_owned))
            .collect()
    }
}

fn controller(touch: bool) -> Harness<'static, Run> {
    let size = if touch { PHONE } else { egui::vec2(1200., 800.) };
    let mut harness = super::build_harness(size, Run::new(), |ui, run| run.show(ui));
    if touch {
        crate::layout::set_touch(&harness.ctx, true);
    }
    harness.step();
    harness
}

/// Starts a cast to the fake TV and plays the fixture movie on it, until the controller shows it
/// playing.
fn casting(harness: &mut Harness<'static, Run>) {
    harness.step();
    harness.state_mut().services().start_cast("fake-tv");
    super::step_until(harness, "the receiver to connect", |harness| {
        harness
            .state()
            .services
            .as_ref()
            .and_then(|services| services.cast.session.as_ref())
            .is_some_and(|session| session.connected)
    });
    let movie: BaseItemDto = fixture_json("item_movie");
    harness.state_mut().services().play(&movie, PlayOptions::default());
    super::step_until(harness, "the controller", |harness| {
        harness.query_by_label("Pause").is_some()
    });
}

/// Waits for the receiver app to have been sent `command`.
fn sent(harness: &mut Harness<'static, Run>, command: &str) {
    super::step_until(harness, command, |harness| {
        harness.state().commands().iter().any(|sent| sent == command)
    });
}

/// Play/pause, the 10-second skips and the TV's volume are the controller's, and each goes to the
/// fake device as the player's own did.
#[test]
fn the_controllers_controls_reach_the_tv() {
    let _config = ConfigDir::new("controller-controls");
    let mut harness = controller(false);
    casting(&mut harness);

    harness.get_by_label("Pause").click();
    sent(&mut harness, "PlayPause");

    harness.get_by_label("Back 10 s").click();
    sent(&mut harness, "Seek");
    let back = harness
        .state()
        .device()
        .envelopes()
        .into_iter()
        .find(|envelope| envelope["command"] == "Seek")
        .expect("the seek was sent");
    harness.get_by_label("Forward 10 s").click();
    super::step_until(&mut harness, "the forward seek", |harness| {
        harness
            .state()
            .device()
            .envelopes()
            .into_iter()
            .filter(|envelope| envelope["command"] == "Seek")
            .count()
            >= 2
    });
    let seeks: Vec<f64> = harness
        .state()
        .device()
        .envelopes()
        .into_iter()
        .filter(|envelope| envelope["command"] == "Seek")
        .map(|envelope| envelope["options"]["position"].as_f64().unwrap_or(0.))
        .collect();
    let first = back["options"]["position"].as_f64().unwrap_or(0.);
    assert!(
        seeks.last().is_some_and(|last| *last > first),
        "forward skips further on than back: {seeks:?}"
    );

    // A click on the volume slider's middle asks the TV for 50 %.
    harness.get_by_label("Volume").click();
    super::step_until(&mut harness, "the TV's volume", |harness| {
        harness
            .state()
            .device()
            .messages()
            .iter()
            .rev()
            .find(|message| message.payload["type"] == "SET_VOLUME")
            .is_some_and(|message| (message.payload["volume"]["level"].as_f64().unwrap_or(0.) - 0.5).abs() < 1e-6)
    });
    assert!(
        !harness
            .state()
            .commands()
            .iter()
            .any(|command| command.contains("Volume")),
        "the volume is the TV's, not a receiver command"
    );
}

/// The controller's Stop casting stops the receiver and closes the player.
#[test]
fn stop_casting_stops_the_receiver() {
    let _config = ConfigDir::new("controller-stop");
    let mut harness = controller(false);
    casting(&mut harness);
    harness.step();
    super::step_until(&mut harness, "the stop button", |harness| {
        harness.query_by_label("Stop casting").is_some()
    });
    harness.get_by_label("Stop casting").click();
    super::step_until(&mut harness, "the cast to stop", |harness| {
        !harness.state().services.as_ref().is_some_and(Services::is_casting)
    });
    super::step_until(&mut harness, "the receiver to stop", |harness| {
        harness
            .state()
            .device()
            .messages()
            .iter()
            .any(|message| message.payload["type"] == "STOP")
    });
}

/// On a phone the Audio row opens the audio choices as a sheet, and one picked there plays the
/// item again on the TV with that track.
#[test]
fn the_phones_audio_row_opens_the_audio_choices() {
    let _config = ConfigDir::new("controller-phone-audio");
    let mut harness = controller(true);
    casting(&mut harness);
    // The item from the server gives the controller its two audio tracks.
    super::step_until(&mut harness, "the audio row", |harness| {
        harness.query_by_label("Audio").is_some()
    });
    harness.get_by_label("Audio").click();
    super::step_until(&mut harness, "the audio choices", |harness| {
        harness.query_by_label("Japanese - AAC - Mono").is_some()
    });
    harness.get_by_label("Japanese - AAC - Mono").click();
    super::step_until(&mut harness, "the item to play with the other track", |harness| {
        harness
            .state()
            .device()
            .envelopes()
            .iter()
            .any(|envelope| envelope["command"] == "PlayNow" && envelope["options"]["audioStreamIndex"] == 2)
    });
}
