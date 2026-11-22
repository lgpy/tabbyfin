//! The MPRIS service (`org.mpris.MediaPlayer2`) desktops read for their media widgets and media
//! keys. It lives on the session bus only while something plays: [`Mpris::start`] takes the
//! name and dropping it gives the name back. `souvlaki` has an MPRIS backend too, but it runs
//! its own thread outside tokio, which the `zbus` the app builds (its tokio feature, for the
//! portal code) can't be used from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use zbus::connection;
use zbus::zvariant::{ObjectPath, Value};

use super::super::{APP_NAME, Command, Controls, JUMP, Link, MediaInfo, PlaybackState, Transport};
use crate::playback::Adjustments;

const NAME: &str = "org.mpris.MediaPlayer2.tabbyfin";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const TRACK: &str = "/org/tabbyfin/track/0";

/// What the player interface answers from.
#[derive(Default)]
struct Shared {
    info: Option<MediaInfo>,
    state: Option<(PlaybackState, Instant)>,
}

impl Shared {
    fn status(&self) -> &'static str {
        match self.state {
            Some((state, _)) if state.transport == Transport::Playing => "Playing",
            Some(_) => "Paused",
            None => "Stopped",
        }
    }

    /// The playback speed, 1 while nothing is told. Never zero or less: MPRIS has no such rate.
    fn rate(&self) -> f64 {
        match self.state {
            Some((state, _)) if state.rate > 0. => state.rate,
            _ => 1.,
        }
    }

    /// Where the position is now: told, and moved on at the playback speed while it plays.
    fn position(&self) -> Duration {
        match self.state {
            Some((state, at)) if state.transport == Transport::Playing => {
                state.position + at.elapsed().mul_f64(self.rate())
            }
            Some((state, _)) => state.position,
            None => Duration::ZERO,
        }
    }

    fn metadata(&self) -> HashMap<String, Value<'static>> {
        let mut map: HashMap<String, Value<'static>> = HashMap::new();
        map.insert(
            "mpris:trackid".into(),
            Value::from(ObjectPath::from_static_str_unchecked(TRACK)),
        );
        let Some(info) = &self.info else {
            return map;
        };
        map.insert("xesam:title".into(), Value::from(info.title.clone()));
        if let Some(artist) = &info.artist {
            map.insert("xesam:artist".into(), Value::from(vec![artist.clone()]));
        }
        if let Some(album) = &info.album {
            map.insert("xesam:album".into(), Value::from(album.clone()));
        }
        if let Some(duration) = info.duration {
            map.insert("mpris:length".into(), Value::from(micros(duration)));
        }
        if let Some(art) = &info.artwork {
            map.insert("mpris:artUrl".into(), Value::from(art.clone()));
        }
        map
    }
}

fn micros(time: Duration) -> i64 {
    time.as_micros().min(i64::MAX as u128) as i64
}

/// A seek offset in microseconds, as a direction and an amount.
fn seek_command(offset: i64) -> Command {
    Command::SeekBy(offset >= 0, Duration::from_micros(offset.unsigned_abs()))
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        APP_NAME.to_owned()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    link: Link,
    shared: Arc<Mutex<Shared>>,
}

impl Player {
    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.link.send(Command::Next);
    }

    fn previous(&self) {
        self.link.send(Command::Previous);
    }

    fn pause(&self) {
        self.link.send(Command::Pause);
    }

    fn play_pause(&self) {
        self.link.send(Command::Toggle);
    }

    fn stop(&self) {
        self.link.send(Command::Stop);
    }

    fn play(&self) {
        self.link.send(Command::Play);
    }

    fn seek(&self, offset: i64) {
        self.link.send(seek_command(offset));
    }

    fn set_position(&self, _track: ObjectPath<'_>, position: i64) {
        self.link
            .send(Command::SeekTo(Duration::from_micros(position.max(0) as u64)));
    }

    fn open_uri(&self, _uri: &str) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.shared().status().to_owned()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.shared().rate()
    }

    // The speeds the player's menu offers.
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        Adjustments::SPEEDS[0]
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        Adjustments::SPEEDS[Adjustments::SPEEDS.len() - 1]
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        micros(self.shared().position())
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'static>> {
        self.shared().metadata()
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

enum Update {
    Metadata(MediaInfo),
    Playback(PlaybackState),
}

/// The service, while it's registered. Dropping it closes the connection and with it the name.
pub struct Mpris {
    updates: mpsc::UnboundedSender<Update>,
}

impl Mpris {
    pub fn start(link: Link) -> Self {
        let (updates, receiver) = mpsc::unbounded_channel();
        crate::runtime::detach(serve(link, receiver));
        Self { updates }
    }
}

impl Controls for Mpris {
    fn set_metadata(&mut self, info: &MediaInfo) {
        let _ = self.updates.send(Update::Metadata(info.clone()));
    }

    fn set_playback(&mut self, state: &PlaybackState) {
        let _ = self.updates.send(Update::Playback(*state));
    }
}

async fn serve(link: Link, mut updates: mpsc::UnboundedReceiver<Update>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let player = Player {
        link,
        shared: shared.clone(),
    };
    let built = async {
        connection::Builder::session()?
            .name(NAME)?
            .serve_at(PATH, Root)?
            .serve_at(PATH, player)?
            .build()
            .await
    }
    .await;
    let connection = match built {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!("no MPRIS service, so no media keys: {err}");
            return;
        }
    };
    while let Some(update) = updates.recv().await {
        let (changed, seeked) = {
            let mut shared = shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut changed: HashMap<&str, Value<'static>> = HashMap::new();
            let mut seeked = None;
            match update {
                Update::Metadata(info) => {
                    shared.info = Some(info);
                    changed.insert("Metadata", Value::new(shared.metadata()));
                }
                Update::Playback(state) => {
                    let (before, rate_before) = (shared.status(), shared.rate());
                    // A position away from where playing would have put it is a seek.
                    if shared.state.is_some() && shared.position().abs_diff(state.position) > JUMP {
                        seeked = Some(micros(state.position));
                    }
                    shared.state = Some((state, Instant::now()));
                    if shared.status() != before {
                        changed.insert("PlaybackStatus", Value::from(shared.status()));
                    }
                    if shared.rate() != rate_before {
                        changed.insert("Rate", Value::from(shared.rate()));
                    }
                }
            }
            (changed, seeked)
        };
        if !changed.is_empty() {
            let result = connection
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(PLAYER, changed, Vec::<&str>::new()),
                )
                .await;
            if let Err(err) = result {
                tracing::debug!("MPRIS properties: {err}");
            }
        }
        if let Some(position) = seeked
            && let Err(err) = connection
                .emit_signal(None::<&str>, PATH, PLAYER, "Seeked", &(position,))
                .await
        {
            tracing::debug!("MPRIS seeked: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_offsets_are_signed_microseconds() {
        assert_eq!(seek_command(5_000_000), Command::SeekBy(true, Duration::from_secs(5)));
        assert_eq!(
            seek_command(-2_500_000),
            Command::SeekBy(false, Duration::from_micros(2_500_000))
        );
    }

    #[test]
    fn the_rate_and_the_position_follow_the_playback_speed() {
        let mut shared = Shared::default();
        assert_eq!(shared.rate(), 1.);
        let state = PlaybackState {
            transport: Transport::Playing,
            position: Duration::from_secs(10),
            duration: None,
            rate: 2.,
        };
        shared.state = Some((state, Instant::now() - Duration::from_secs(3)));
        assert_eq!(shared.rate(), 2.);
        // Three seconds at twice the speed.
        let position = shared.position();
        assert!(
            position >= Duration::from_secs(16) && position < Duration::from_secs(17),
            "{position:?}"
        );
        // No rate of zero or less goes out: MPRIS has none.
        shared.state = Some((PlaybackState { rate: 0., ..state }, Instant::now()));
        assert_eq!(shared.rate(), 1.);
    }

    #[test]
    fn metadata_names_the_track_and_the_art() {
        let shared = Shared {
            info: Some(MediaInfo {
                title: "Pilot".into(),
                artist: Some("Show".into()),
                album: None,
                duration: Some(Duration::from_secs(60)),
                artwork: Some("https://tv.example/Items/1/Images/Primary?tag=a".into()),
                heading: String::new(),
            }),
            state: None,
        };
        let metadata = shared.metadata();
        assert_eq!(metadata["mpris:length"], Value::from(60_000_000i64));
        assert!(metadata.contains_key("xesam:artist"));
        assert!(!metadata.contains_key("xesam:album"));
    }
}
