//! Remote control: the commands other clients (the web dashboard, phone apps) send this
//! session over the websocket, and the capabilities that tell the server we take them.
//!
//! The server forwards them as `Play`, `Playstate` and `GeneralCommand` messages. It sends
//! general commands even when they aren't in our `SupportedCommands`, so unknown ones parse as
//! [`GeneralCommand::Other`] for the app to ignore.

use std::collections::HashMap;

use serde::Deserialize;

use crate::client::Query;
use crate::{Client, Result, Ticks};

/// "Play these items": `PlayNow` replaces what's playing, the others add to the queue. The
/// server has already expanded folders (a series, a season) into their playable items.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayRequest {
    #[serde(default)]
    pub item_ids: Vec<String>,
    pub play_command: PlayCommand,
    /// Where the first item starts. Absent means from the beginning.
    pub start_position_ticks: Option<Ticks>,
    /// Which of `item_ids` to start with; the ones before it are skipped.
    pub start_index: Option<usize>,
    pub media_source_id: Option<String>,
    pub audio_stream_index: Option<i32>,
    /// `-1` turns subtitles off.
    pub subtitle_stream_index: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PlayCommand {
    PlayNow,
    PlayNext,
    PlayLast,
    PlayShuffle,
    PlayInstantMix,
    #[serde(other)]
    Other,
}

/// A transport control for what's playing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaystateRequest {
    pub command: PlaystateCommand,
    /// For `Seek`.
    pub seek_position_ticks: Option<Ticks>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PlaystateCommand {
    Stop,
    Pause,
    Unpause,
    PlayPause,
    Seek,
    Rewind,
    FastForward,
    NextTrack,
    PreviousTrack,
    #[serde(other)]
    Other,
}

/// The general commands the app handles, with their arguments. The server sends every
/// argument as a string.
#[derive(Debug, Clone, PartialEq)]
pub enum GeneralCommand {
    VolumeUp,
    VolumeDown,
    /// Percent, 0 to 100.
    SetVolume(i32),
    Mute,
    Unmute,
    ToggleMute,
    /// A Jellyfin stream index.
    SetAudioStreamIndex(i32),
    /// A Jellyfin stream index, or `-1` for off.
    SetSubtitleStreamIndex(i32),
    ToggleFullscreen,
    /// A message to show, e.g. sent from the dashboard's session view.
    DisplayMessage {
        header: Option<String>,
        text: String,
    },
    /// Anything else, or a known command with arguments missing.
    Other(String),
}

impl GeneralCommand {
    /// The names to advertise in `SupportedCommands`.
    pub const SUPPORTED: &[&str] = &[
        "VolumeUp",
        "VolumeDown",
        "SetVolume",
        "Mute",
        "Unmute",
        "ToggleMute",
        "SetAudioStreamIndex",
        "SetSubtitleStreamIndex",
        "ToggleFullscreen",
        "DisplayMessage",
    ];

    pub(crate) fn parse(data: serde_json::Value) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Raw {
            name: String,
            #[serde(default)]
            arguments: HashMap<String, String>,
        }
        let raw: Raw = serde_json::from_value(data)?;
        let int = |key: &str| {
            raw.arguments
                .get(key)
                .and_then(|value| value.trim().parse::<i32>().ok())
        };
        let command = match raw.name.as_str() {
            "VolumeUp" => Some(Self::VolumeUp),
            "VolumeDown" => Some(Self::VolumeDown),
            "SetVolume" => int("Volume").map(Self::SetVolume),
            "Mute" => Some(Self::Mute),
            "Unmute" => Some(Self::Unmute),
            "ToggleMute" => Some(Self::ToggleMute),
            "SetAudioStreamIndex" => int("Index").map(Self::SetAudioStreamIndex),
            "SetSubtitleStreamIndex" => int("Index").map(Self::SetSubtitleStreamIndex),
            "ToggleFullscreen" => Some(Self::ToggleFullscreen),
            "DisplayMessage" => raw.arguments.get("Text").map(|text| Self::DisplayMessage {
                header: raw.arguments.get("Header").filter(|h| !h.is_empty()).cloned(),
                text: text.clone(),
            }),
            _ => None,
        };
        Ok(command.unwrap_or(Self::Other(raw.name)))
    }
}

/// What this session tells the server it can do.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    /// Takes `Play` and `Playstate` commands from other clients.
    pub supports_media_control: bool,
    /// General command names, e.g. [`GeneralCommand::SUPPORTED`].
    pub supported_commands: Vec<&'static str>,
}

impl Client {
    /// Tells the server what this session can do. Resend after reconnecting.
    pub async fn report_capabilities(&self, capabilities: &Capabilities) -> Result<()> {
        let body = serde_json::json!({
            "PlayableMediaTypes": ["Video"],
            "SupportedCommands": capabilities.supported_commands,
            "SupportsMediaControl": capabilities.supports_media_control,
            "SupportsPersistentIdentifier": true,
        });
        self.post_no_content("/Sessions/Capabilities/Full", &Query::new(), Some(&body))
            .await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{GeneralCommand, PlayCommand, PlayRequest, PlaystateCommand, PlaystateRequest};
    use crate::Ticks;

    // The payloads below are what a 12.2 server sent for the matching `/Sessions/{id}/...`
    // requests.

    #[test]
    fn parses_play_requests() {
        let request: PlayRequest = serde_json::from_value(json!({
            "ItemIds": ["a", "b"],
            "StartPositionTicks": 600000000,
            "PlayCommand": "PlayNow",
            "ControllingUserId": "u",
            "StartIndex": 1
        }))
        .expect("parses");
        assert_eq!(request.item_ids, ["a", "b"]);
        assert_eq!(request.play_command, PlayCommand::PlayNow);
        assert_eq!(request.start_position_ticks, Some(Ticks(600000000)));
        assert_eq!(request.start_index, Some(1));
        let request: PlayRequest =
            serde_json::from_value(json!({ "ItemIds": ["a"], "PlayCommand": "PlayNext" })).expect("parses");
        assert_eq!(request.play_command, PlayCommand::PlayNext);
        assert_eq!(request.start_position_ticks, None);
    }

    #[test]
    fn parses_playstate_requests() {
        let request: PlaystateRequest = serde_json::from_value(json!({
            "Command": "Seek",
            "SeekPositionTicks": 1200000000,
            "ControllingUserId": "u"
        }))
        .expect("parses");
        assert_eq!(request.command, PlaystateCommand::Seek);
        assert_eq!(request.seek_position_ticks, Some(Ticks(1200000000)));
        let request: PlaystateRequest = serde_json::from_value(json!({ "Command": "Spin" })).expect("parses");
        assert_eq!(request.command, PlaystateCommand::Other);
    }

    #[test]
    fn parses_general_commands() {
        let parse = |data| GeneralCommand::parse(data).expect("parses");
        assert_eq!(
            parse(json!({ "Name": "SetVolume", "ControllingUserId": "u", "Arguments": { "Volume": "42" } })),
            GeneralCommand::SetVolume(42)
        );
        assert_eq!(
            parse(json!({ "Name": "VolumeUp", "Arguments": {} })),
            GeneralCommand::VolumeUp
        );
        assert_eq!(
            parse(json!({ "Name": "SetSubtitleStreamIndex", "Arguments": { "Index": "-1" } })),
            GeneralCommand::SetSubtitleStreamIndex(-1)
        );
        assert_eq!(
            parse(json!({
                "Name": "DisplayMessage",
                "Arguments": { "Header": "Hi", "Text": "Hello there", "TimeoutMs": "5000" }
            })),
            GeneralCommand::DisplayMessage {
                header: Some("Hi".into()),
                text: "Hello there".into()
            }
        );
        assert_eq!(
            parse(json!({ "Name": "GoHome", "Arguments": {} })),
            GeneralCommand::Other("GoHome".into())
        );
        // A known command without its argument can't be carried out.
        assert_eq!(
            parse(json!({ "Name": "SetVolume" })),
            GeneralCommand::Other("SetVolume".into())
        );
    }
}
