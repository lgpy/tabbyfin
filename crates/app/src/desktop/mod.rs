//! What the desktop shows and does around a playing video: the display stays awake, the
//! system's media controls (Windows' media overlay, MPRIS, Now Playing) carry the title and
//! take the media keys, the window's title names the video, and on Windows the taskbar button
//! shows the position.
//!
//! The app owns one [`Desktop`] and calls [`Desktop::update`] every frame from `App::logic`,
//! with the signed-in services. Everything is read from [`NowPlaying`] and everything the OS
//! asks for goes back through [`Services::control`] (or `play_next` / `play_previous`), the
//! paths the player's own controls take, so a SyncPlay group gets the media keys too. While
//! nothing plays (or a cast plays on a TV: `NowPlaying` is empty then) nothing is registered
//! with the OS and the display may sleep.
//!
//! This file is the part that doesn't depend on the OS: what to tell the OS ([`Snapshot`],
//! [`Reporter`]), the commands it sends back ([`Command`]), and when to hold the display
//! ([`Awake`]). Each OS has one small module of its own that implements [`Inhibit`] and
//! [`Controls`]; none of it exists on Android, which keeps its own screen on and has its own
//! media session.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use jellyfin::Client;
use jellyfin::models::BaseItemKind;
use url::Url;

use crate::playback::{NowPlaying, UiCommand};
use crate::services::Services;
use crate::widgets::text::episode_label;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos"))]
mod media_controls;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(windows)]
use windows as os;

/// The app's name in the window's title and in the OS's media widgets.
const APP_NAME: &str = "Tabbyfin";
/// A position the OS is told again after this long even when nothing changed, which keeps the
/// taskbar's progress and a widget's timeline from drifting.
const REFRESH: Duration = Duration::from_secs(5);
/// How far the position may be from where it was expected before it counts as a seek.
const JUMP: Duration = Duration::from_millis(1500);

/// Something the OS asked the player to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Forward (`true`) or backward by this much.
    SeekBy(bool, Duration),
    /// Forward (`true`) or backward by the seek step Settings holds: an OS "seek" without an
    /// amount.
    SeekStep(bool),
    SeekTo(Duration),
}

/// What a [`Command`] does in the app.
#[derive(Debug)]
enum Action {
    Control(UiCommand),
    Next,
    Previous,
}

impl Command {
    /// What this does; `step` is the seek step in seconds.
    fn action(self, step: f64) -> Action {
        match self {
            Command::Play => Action::Control(UiCommand::SetPause(false)),
            Command::Pause => Action::Control(UiCommand::SetPause(true)),
            Command::Toggle => Action::Control(UiCommand::TogglePause),
            Command::Stop => Action::Control(UiCommand::Stop),
            Command::Next => Action::Next,
            Command::Previous => Action::Previous,
            Command::SeekBy(forward, by) => {
                let seconds = by.as_secs_f64();
                Action::Control(UiCommand::SeekBy(if forward { seconds } else { -seconds }))
            }
            Command::SeekStep(forward) => Action::Control(UiCommand::SeekBy(if forward { step } else { -step })),
            Command::SeekTo(to) => Action::Control(UiCommand::SeekTo(to)),
        }
    }
}

/// What the OS shows about the item: static between seeks.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// The episode's or movie's name.
    pub title: String,
    /// The series, for an episode.
    pub artist: Option<String>,
    /// The season, for an episode.
    pub album: Option<String>,
    pub duration: Option<Duration>,
    /// The item's image, as a plain URL: the server serves images without a token, so none is
    /// ever put in it (see [`artwork_url`]).
    pub artwork: Option<String>,
    /// The window's title.
    pub heading: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transport {
    Playing,
    Paused,
}

/// Playback as the OS is told: the state and where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackState {
    pub transport: Transport,
    pub position: Duration,
    pub duration: Option<Duration>,
}

/// One frame's reading of `NowPlaying`.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub info: MediaInfo,
    pub state: PlaybackState,
}

/// The artwork URL to hand the OS: the image URL without any credential. The client sends its
/// token in a header, so there is none in a URL it builds, and this keeps it that way whatever
/// is put in later: no user info, no `ApiKey` / `api_key` / `token` query, and only http(s).
pub fn artwork_url(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !matches!(
                key.as_str(),
                "apikey" | "api_key" | "token" | "accesstoken" | "access_token"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    Some(url.into())
}

impl Snapshot {
    /// What `state` says now. The position is extrapolated while it plays, as the player page's
    /// seek bar does, since mpv reports it once a second.
    pub fn of(state: &NowPlaying, client: &Client, now: Instant) -> Self {
        let item = &state.item;
        let (title, artist, album) = match item.kind {
            BaseItemKind::Episode => (
                item.name.clone(),
                item.series_name.clone().filter(|name| !name.is_empty()),
                item.season_name
                    .clone()
                    .or_else(|| item.parent_index_number.map(|number| format!("S{number}")))
                    .filter(|name| !name.is_empty()),
            ),
            _ => (item.name.clone(), None, None),
        };
        let heading = match (&artist, item.kind) {
            (Some(series), BaseItemKind::Episode) => match episode_label(item) {
                label if label.is_empty() => format!("{series} · {title}"),
                label => format!("{series} · {label} · {title}"),
            },
            _ => title.clone(),
        };
        let artwork = item
            .primary_image()
            .map(|request| request.fill_height(480))
            .and_then(|request| artwork_url(&client.image_url(&request)));
        let transport = if state.paused {
            Transport::Paused
        } else {
            Transport::Playing
        };
        let mut position = state.position;
        if transport == Transport::Playing && !state.loading {
            // At the playback speed, as `NowPlaying::position_now` carries it on.
            position += now
                .saturating_duration_since(state.position_at)
                .mul_f64(state.adjustments.speed.max(0.));
        }
        if let Some(duration) = state.duration {
            position = position.min(duration);
        }
        Snapshot {
            info: MediaInfo {
                title,
                artist,
                album,
                duration: state.duration,
                artwork,
                heading,
            },
            state: PlaybackState {
                transport,
                position,
                duration: state.duration,
            },
        }
    }
}

/// The window's title for what plays (nothing: the app's name).
pub fn window_title(info: Option<&MediaInfo>) -> String {
    match info {
        Some(info) => format!("{} – {APP_NAME}", info.heading),
        None => APP_NAME.to_owned(),
    }
}

/// What to tell the OS.
#[derive(Debug, PartialEq)]
pub enum Report {
    Metadata(MediaInfo),
    Playback(PlaybackState),
}

/// Works out what the OS has to be told, so it hears about a change once and not every frame.
#[derive(Default)]
pub struct Reporter {
    info: Option<MediaInfo>,
    /// The last state told, with when.
    told: Option<(PlaybackState, Instant)>,
}

impl Reporter {
    /// The reports a frame needs, in the order to apply them.
    pub fn next(&mut self, snapshot: &Snapshot, now: Instant) -> Vec<Report> {
        let mut reports = Vec::new();
        if self.info.as_ref() != Some(&snapshot.info) {
            self.info = Some(snapshot.info.clone());
            reports.push(Report::Metadata(snapshot.info.clone()));
        }
        let due = match &self.told {
            None => true,
            Some((told, at)) => {
                // Where the position should be by now if nothing happened.
                let expected = match told.transport {
                    Transport::Playing => told.position + now.saturating_duration_since(*at),
                    Transport::Paused => told.position,
                };
                told.transport != snapshot.state.transport
                    || told.duration != snapshot.state.duration
                    || expected.abs_diff(snapshot.state.position) > JUMP
                    || now.saturating_duration_since(*at) >= REFRESH
            }
        };
        if due {
            self.told = Some((snapshot.state, now));
            reports.push(Report::Playback(snapshot.state));
        }
        reports
    }
}

/// Holds something while it's wanted and lets go when it isn't, once per change.
pub trait Inhibit {
    fn acquire(&mut self);
    fn release(&mut self);
}

/// Keeps a display awake while video plays.
pub struct Awake<I: Inhibit> {
    inhibit: I,
    held: bool,
}

impl<I: Inhibit> Awake<I> {
    pub fn new(inhibit: I) -> Self {
        Self { inhibit, held: false }
    }

    /// Whether `playing` should keep the display awake: a video that isn't paused.
    pub fn wanted(state: Option<&PlaybackState>) -> bool {
        state.is_some_and(|state| state.transport == Transport::Playing)
    }

    pub fn set(&mut self, wanted: bool) {
        match (wanted, self.held) {
            (true, false) => self.inhibit.acquire(),
            (false, true) => self.inhibit.release(),
            _ => return,
        }
        self.held = wanted;
    }
}

impl<I: Inhibit> Drop for Awake<I> {
    fn drop(&mut self) {
        self.set(false);
    }
}

/// The OS's media controls, while something plays.
pub trait Controls {
    fn set_metadata(&mut self, info: &MediaInfo);
    fn set_playback(&mut self, state: &PlaybackState);
}

/// What the OS side is handed to start: where its commands go, and a way to wake the UI when
/// one arrives (they come from another thread).
pub struct Link {
    pub commands: mpsc::Sender<Command>,
    pub ctx: egui::Context,
}

impl Link {
    /// Called from the OS's threads.
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            self.ctx.request_repaint();
        }
    }
}

pub struct Desktop {
    awake: Awake<os::Inhibitor>,
    controls: Option<Box<dyn Controls>>,
    /// Starting the controls failed during this playback: it isn't tried again until the next.
    controls_failed: bool,
    reporter: Reporter,
    link: Link,
    commands: mpsc::Receiver<Command>,
    title: String,
}

impl Desktop {
    pub fn new(ctx: &egui::Context) -> Self {
        let (commands_tx, commands) = mpsc::channel();
        Self {
            awake: Awake::new(os::Inhibitor::new()),
            controls: None,
            controls_failed: false,
            reporter: Reporter::default(),
            link: Link {
                commands: commands_tx,
                ctx: ctx.clone(),
            },
            commands,
            title: APP_NAME.to_owned(),
        }
    }

    /// From `App::logic`, every frame: carries out what the OS asked, then tells it what plays.
    pub fn update(&mut self, ctx: &egui::Context, frame: &eframe::Frame, services: Option<&mut Services>) {
        let mut services = services;
        while let Ok(command) = self.commands.try_recv() {
            let Some(services) = services.as_deref_mut() else {
                continue;
            };
            tracing::debug!(?command, "media control");
            match command.action(services.playback.seek_step()) {
                Action::Control(command) => services.control(command),
                Action::Next => services.play_next(),
                Action::Previous => services.play_previous(),
            }
        }
        let now = Instant::now();
        let snapshot = services.as_deref().and_then(|services| {
            let state = services.playback.now_playing()?;
            Some(Snapshot::of(state, &services.session.client, now))
        });
        self.awake
            .set(Awake::<os::Inhibitor>::wanted(snapshot.as_ref().map(|s| &s.state)));

        let title = window_title(snapshot.as_ref().map(|s| &s.info));
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        let Some(snapshot) = snapshot else {
            // Nothing plays: nothing stays registered with the OS.
            self.controls = None;
            self.controls_failed = false;
            self.reporter = Reporter::default();
            return;
        };
        if self.controls.is_none() && !self.controls_failed {
            self.controls = os::controls(frame, &self.link);
            self.controls_failed = self.controls.is_none();
        }
        let Some(controls) = &mut self.controls else {
            return;
        };
        for report in self.reporter.next(&snapshot, now) {
            match report {
                Report::Metadata(info) => controls.set_metadata(&info),
                Report::Playback(state) => controls.set_playback(&state),
            }
        }
        // The position is told again every few seconds while it plays.
        if snapshot.state.transport == Transport::Playing {
            ctx.request_repaint_after(REFRESH);
        }
    }

    /// The app is quitting: the display may sleep and the OS forgets the controls.
    pub fn shut_down(&mut self) {
        self.awake.set(false);
        self.controls = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> MediaInfo {
        MediaInfo {
            title: "Pilot".into(),
            artist: Some("Show".into()),
            album: Some("Season 1".into()),
            duration: Some(Duration::from_secs(2600)),
            artwork: None,
            heading: "Show · S1:E1 · Pilot".into(),
        }
    }

    fn state(transport: Transport, secs: u64) -> PlaybackState {
        PlaybackState {
            transport,
            position: Duration::from_secs(secs),
            duration: Some(Duration::from_secs(2600)),
        }
    }

    fn snapshot(transport: Transport, secs: u64) -> Snapshot {
        Snapshot {
            info: info(),
            state: state(transport, secs),
        }
    }

    /// What a command does, as text: `UiCommand` has no `PartialEq`.
    fn does(command: Command) -> String {
        format!("{:?}", command.action(15.))
    }

    #[test]
    fn commands_map_to_the_players_controls() {
        assert_eq!(does(Command::Play), "Control(SetPause(false))");
        assert_eq!(does(Command::Pause), "Control(SetPause(true))");
        assert_eq!(does(Command::Toggle), "Control(TogglePause)");
        assert_eq!(does(Command::Stop), "Control(Stop)");
        assert_eq!(does(Command::Next), "Next");
        assert_eq!(does(Command::Previous), "Previous");
        assert_eq!(
            does(Command::SeekBy(false, Duration::from_secs(10))),
            "Control(SeekBy(-10.0))"
        );
        assert_eq!(
            does(Command::SeekBy(true, Duration::from_secs(10))),
            "Control(SeekBy(10.0))"
        );
        // Without an amount the OS gets the seek step of Settings (15 s here).
        assert_eq!(does(Command::SeekStep(false)), "Control(SeekBy(-15.0))");
        assert_eq!(does(Command::SeekStep(true)), "Control(SeekBy(15.0))");
        assert_eq!(does(Command::SeekTo(Duration::from_secs(90))), "Control(SeekTo(90s))");
    }

    #[test]
    fn the_os_hears_a_change_once() {
        let mut reporter = Reporter::default();
        let t0 = Instant::now();
        let first = reporter.next(&snapshot(Transport::Playing, 10), t0);
        assert_eq!(
            first,
            vec![
                Report::Metadata(info()),
                Report::Playback(state(Transport::Playing, 10))
            ]
        );
        // A second later the position is where it was expected: nothing to say.
        let quiet = reporter.next(&snapshot(Transport::Playing, 11), t0 + Duration::from_secs(1));
        assert!(quiet.is_empty());
        // Pausing, a seek, and the refresh each say the state again, without the metadata.
        let paused = reporter.next(&snapshot(Transport::Paused, 11), t0 + Duration::from_secs(1));
        assert_eq!(paused, vec![Report::Playback(state(Transport::Paused, 11))]);
        let seek = reporter.next(&snapshot(Transport::Paused, 900), t0 + Duration::from_secs(2));
        assert_eq!(seek, vec![Report::Playback(state(Transport::Paused, 900))]);
        let refresh = reporter.next(&snapshot(Transport::Paused, 900), t0 + Duration::from_secs(8));
        assert_eq!(refresh, vec![Report::Playback(state(Transport::Paused, 900))]);
    }

    #[test]
    fn a_new_item_sends_its_metadata_again() {
        let mut reporter = Reporter::default();
        let t0 = Instant::now();
        reporter.next(&snapshot(Transport::Playing, 0), t0);
        let mut next = snapshot(Transport::Playing, 0);
        next.info.title = "Second".into();
        let reports = reporter.next(&next, t0);
        assert_eq!(reports, vec![Report::Metadata(next.info)]);
    }

    #[derive(Default)]
    struct Counting {
        taken: u32,
        released: u32,
    }

    impl Inhibit for &mut Counting {
        fn acquire(&mut self) {
            self.taken += 1;
        }
        fn release(&mut self) {
            self.released += 1;
        }
    }

    #[test]
    fn the_display_is_held_only_while_video_plays() {
        let mut counts = Counting::default();
        {
            let mut awake = Awake::new(&mut counts);
            let playing = state(Transport::Playing, 0);
            let paused = state(Transport::Paused, 0);
            for shown in [
                None,
                Some(&playing),
                Some(&playing),
                Some(&paused),
                Some(&paused),
                Some(&playing),
            ] {
                awake.set(Awake::<&mut Counting>::wanted(shown));
            }
            // Dropping while held (the app exits mid-video) lets go.
        }
        assert_eq!((counts.taken, counts.released), (2, 2));
    }

    #[test]
    fn stopping_releases_the_display() {
        let mut counts = Counting::default();
        let mut awake = Awake::new(&mut counts);
        awake.set(true);
        awake.set(Awake::<&mut Counting>::wanted(None));
        drop(awake);
        assert_eq!((counts.taken, counts.released), (1, 1));
    }

    #[test]
    fn artwork_urls_carry_no_credentials() {
        let url = Url::parse("https://tv.example/Items/1/Images/Primary?tag=abc&ApiKey=secret&fillHeight=480").unwrap();
        assert_eq!(
            artwork_url(&url).unwrap(),
            "https://tv.example/Items/1/Images/Primary?tag=abc&fillHeight=480"
        );
        let url = Url::parse("http://user:pass@tv.example/i?api_key=x&Token=y").unwrap();
        assert_eq!(artwork_url(&url).unwrap(), "http://tv.example/i");
        assert_eq!(artwork_url(&Url::parse("file:///etc/passwd").unwrap()), None);
    }

    #[test]
    fn the_window_is_titled_for_what_plays() {
        assert_eq!(window_title(Some(&info())), "Show · S1:E1 · Pilot – Tabbyfin");
        assert_eq!(window_title(None), "Tabbyfin");
    }

    /// An episode playing from the fixtures, 12 minutes in, as the session leaves it.
    fn episode_snapshot(paused: bool, later: Duration) -> Snapshot {
        episode_snapshot_at(paused, later, 1.)
    }

    fn episode_snapshot_at(paused: bool, later: Duration, speed: f64) -> Snapshot {
        use crate::playback::PlaybackManager;
        use crate::ui_tests::{ConfigDir, TOKEN, fixture_json};

        let _config = ConfigDir::new("desktop-snapshot");
        let client = Client::new("http://tv.example", crate::session::client_info("tests"))
            .unwrap()
            .with_token(TOKEN);
        let mut playback = PlaybackManager::new(&egui::Context::default(), client.clone(), "account".into());
        let episodes: serde_json::Value = fixture_json("episodes");
        let item: jellyfin::models::BaseItemDto = serde_json::from_value(episodes["Items"][1].clone()).unwrap();
        let movie: jellyfin::models::BaseItemDto = fixture_json("item_movie");
        let _commands = playback.start_for_test(item, movie.media_sources[0].clone());
        let state = playback.now_playing_mut().unwrap();
        state.paused = paused;
        state.adjustments.speed = speed;
        let at = state.position_at;
        Snapshot::of(playback.now_playing().unwrap(), &client, at + later)
    }

    /// The OS's timeline runs at the playback speed, as the player's seek bar does.
    #[test]
    fn the_position_moves_on_at_the_playback_speed() {
        let doubled = episode_snapshot_at(false, Duration::from_secs(30), 2.);
        assert_eq!(doubled.state.position, Duration::from_secs(13 * 60));
        let slowed = episode_snapshot_at(false, Duration::from_secs(30), 0.5);
        assert_eq!(slowed.state.position, Duration::from_secs(12 * 60 + 15));
    }

    #[test]
    fn an_episode_maps_to_the_metadata_the_os_shows() {
        let playing = episode_snapshot(false, Duration::from_secs(30));
        let info = &playing.info;
        assert!(info.artist.as_deref().is_some_and(|series| !series.is_empty()));
        assert!(info.heading.starts_with(info.artist.as_deref().unwrap()));
        assert!(info.heading.contains(&info.title));
        assert_eq!(info.duration, Some(Duration::from_secs(48 * 60)));
        // The artwork is the item's image, with nothing to authenticate it.
        let art = info.artwork.as_deref().expect("an image");
        assert!(art.starts_with("http://tv.example/Items/"), "{art}");
        assert!(
            !art.contains(crate::ui_tests::TOKEN) && !art.to_lowercase().contains("api"),
            "{art}"
        );
        // Playing, the position moves on from the last report; paused, it stays.
        assert_eq!(playing.state.transport, Transport::Playing);
        assert_eq!(playing.state.position, Duration::from_secs(12 * 60 + 30));
        let paused = episode_snapshot(true, Duration::from_secs(30));
        assert_eq!(paused.state.transport, Transport::Paused);
        assert_eq!(paused.state.position, Duration::from_secs(12 * 60));
    }
}
