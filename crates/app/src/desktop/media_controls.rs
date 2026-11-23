//! The system's media controls on Windows (the media overlay, SMTC) and macOS (Now Playing),
//! through `souvlaki`.

use std::ffi::c_void;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};

use super::{APP_NAME, Command, Controls, Link, MediaInfo, PlaybackState, Transport};

pub struct SystemControls {
    inner: MediaControls,
}

/// Registers with the OS and starts listening. `hwnd` is the main window's handle, which
/// Windows needs and macOS ignores. `None` when the OS refuses.
pub fn start(hwnd: Option<*mut c_void>, link: Link) -> Option<SystemControls> {
    let config = PlatformConfig {
        display_name: APP_NAME,
        dbus_name: "tabbyfin",
        hwnd,
    };
    let mut inner = MediaControls::new(config)
        .map_err(|err| tracing::info!("no system media controls: {err:?}"))
        .ok()?;
    inner
        .attach(move |event| {
            if let Some(command) = command_of(event) {
                link.send(command);
            }
        })
        .map_err(|err| tracing::info!("couldn't listen to the media keys: {err:?}"))
        .ok()?;
    Some(SystemControls { inner })
}

/// What the OS asked, as a player command; the events the player has no use for are dropped.
pub fn command_of(event: MediaControlEvent) -> Option<Command> {
    Some(match event {
        MediaControlEvent::Play => Command::Play,
        MediaControlEvent::Pause => Command::Pause,
        MediaControlEvent::Toggle => Command::Toggle,
        MediaControlEvent::Stop => Command::Stop,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Previous,
        MediaControlEvent::Seek(direction) => Command::SeekStep(direction == SeekDirection::Forward),
        MediaControlEvent::SeekBy(direction, by) => Command::SeekBy(direction == SeekDirection::Forward, by),
        MediaControlEvent::SetPosition(MediaPosition(to)) => Command::SeekTo(to),
        _ => return None,
    })
}

impl Controls for SystemControls {
    fn set_metadata(&mut self, info: &MediaInfo) {
        let result = self.inner.set_metadata(MediaMetadata {
            title: Some(&info.title),
            artist: info.artist.as_deref(),
            album: info.album.as_deref(),
            cover_url: info.artwork.as_deref(),
            duration: info.duration,
        });
        if let Err(err) = result {
            tracing::debug!("media controls metadata: {err:?}");
        }
    }

    fn set_playback(&mut self, state: &PlaybackState) {
        let progress = Some(MediaPosition(state.position));
        let playback = match state.transport {
            Transport::Playing => MediaPlayback::Playing { progress },
            Transport::Paused => MediaPlayback::Paused { progress },
        };
        if let Err(err) = self.inner.set_playback(playback) {
            tracing::debug!("media controls playback: {err:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_oss_events_become_commands() {
        assert_eq!(command_of(MediaControlEvent::Toggle), Some(Command::Toggle));
        assert_eq!(command_of(MediaControlEvent::Next), Some(Command::Next));
        assert_eq!(
            command_of(MediaControlEvent::Seek(SeekDirection::Backward)),
            Some(Command::SeekStep(false))
        );
        assert_eq!(
            command_of(MediaControlEvent::SetPosition(MediaPosition(Duration::from_secs(5)))),
            Some(Command::SeekTo(Duration::from_secs(5)))
        );
        assert_eq!(command_of(MediaControlEvent::SetVolume(0.5)), None);
        assert_eq!(command_of(MediaControlEvent::Quit), None);
    }
}
