//! Jellyfin's own receiver app (jellyfin-chromecast) and the messages it speaks.
//!
//! The app does the Jellyfin work itself: it calls `PlaybackInfo`, chooses direct play or a
//! transcode, reports to the server and plays the next episode. This crate is only the remote, so
//! what's here is the contract: the credentials every message carries, the commands it accepts, and
//! the events and media status it sends back.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// The namespace the receiver listens on, in both directions.
pub const NAMESPACE: &str = "urn:x-cast:com.connectsdk";

/// The published app's id, used when the server doesn't say otherwise.
pub const STABLE_APP_ID: &str = "F007D354";
/// The nightly build of the same app.
pub const UNSTABLE_APP_ID: &str = "6F511C87";

/// Ticks per second, the unit Jellyfin reports positions and durations in.
pub const TICKS_PER_SECOND: f64 = 10_000_000.0;

/// What the receiver needs to reach the server as us. Every message carries it, and the last
/// sender's credentials win: casting from a second account takes the TV's session over.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// The server as the TV must reach it: a loopback address is replaced with the server's LAN
    /// address, since `localhost` on the TV is the TV.
    pub server_address: String,
    pub access_token: String,
    pub user_id: String,
    /// Our device id, so the receiver's reports show up with this app's name.
    pub device_id: String,
    pub server_id: String,
    pub server_version: String,
    /// The device's name. It becomes the receiver session's name, so each TV keeps one session in
    /// the dashboard.
    pub receiver_name: String,
    /// Bits/s cap. Once sent, the receiver keeps it.
    pub max_bitrate: Option<u64>,
    /// The web client's subtitle appearance settings, passed through as they are.
    pub subtitle_appearance: Option<Value>,
}

/// The commands the receiver has a handler for. Nothing else is representable, deliberately: an
/// unknown command falls through to the receiver's play handler, which throws.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Sent right after `CONNECT`, with credentials. While something plays, the receiver answers
    /// with the current state as a `playbackstop`-typed message.
    Identify,
    PlayNow(PlayRequest),
    /// Appends to the end of the queue.
    PlayNext {
        items: Vec<ItemStub>,
    },
    /// Appends to the end of the queue.
    PlayLast {
        items: Vec<ItemStub>,
    },
    /// The receiver picks up to 50 random items from the item.
    Shuffle {
        item: ItemStub,
    },
    Pause,
    Unpause,
    PlayPause,
    Stop,
    NextTrack,
    PreviousTrack,
    /// Seconds, not ticks.
    Seek {
        position_seconds: f64,
    },
    /// Reloads the stream.
    SetAudioStreamIndex(i32),
    /// `-1` turns subtitles off.
    SetSubtitleStreamIndex(i32),
    SetRepeatMode(RepeatMode),
}

impl Command {
    /// The `command` string the receiver dispatches on.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Identify => "Identify",
            Self::PlayNow(_) => "PlayNow",
            Self::PlayNext { .. } => "PlayNext",
            Self::PlayLast { .. } => "PlayLast",
            Self::Shuffle { .. } => "Shuffle",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::PlayPause => "PlayPause",
            Self::Stop => "Stop",
            Self::NextTrack => "NextTrack",
            Self::PreviousTrack => "PreviousTrack",
            Self::Seek { .. } => "Seek",
            Self::SetAudioStreamIndex(_) => "SetAudioStreamIndex",
            Self::SetSubtitleStreamIndex(_) => "SetSubtitleStreamIndex",
            Self::SetRepeatMode(_) => "SetRepeatMode",
        }
    }

    /// The `options` object. Commands that take nothing send `{}`.
    pub fn options(&self) -> Value {
        match self {
            Self::Identify
            | Self::Pause
            | Self::Unpause
            | Self::PlayPause
            | Self::Stop
            | Self::NextTrack
            | Self::PreviousTrack => json!({}),
            Self::PlayNow(request) => serde_json::to_value(request).expect("a play request is serializable"),
            Self::PlayNext { items } | Self::PlayLast { items } => json!({ "items": items }),
            Self::Shuffle { item } => json!({ "items": [item] }),
            Self::Seek { position_seconds } => json!({ "position": position_seconds }),
            Self::SetAudioStreamIndex(index) => json!({ "index": index }),
            Self::SetSubtitleStreamIndex(index) => json!({ "index": index }),
            Self::SetRepeatMode(mode) => json!({ "RepeatMode": mode }),
        }
    }
}

/// Jellyfin's repeat modes, as the receiver stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RepeatMode {
    #[serde(rename = "RepeatNone")]
    None,
    #[serde(rename = "RepeatAll")]
    All,
    #[serde(rename = "RepeatOne")]
    One,
}

/// One message to the receiver: the command, its options and the credentials.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub command: &'static str,
    pub options: Value,
    pub server_address: String,
    pub access_token: String,
    pub user_id: String,
    pub device_id: String,
    pub server_id: String,
    pub server_version: String,
    pub receiver_name: String,
    /// Bits/s. The receiver runs a bitrate test when it's absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bitrate: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_appearance: Option<Value>,
}

impl Envelope {
    pub fn new(credentials: &Credentials, command: Command) -> Self {
        Self {
            command: command.name(),
            options: command.options(),
            server_address: credentials.server_address.clone(),
            access_token: credentials.access_token.clone(),
            user_id: credentials.user_id.clone(),
            device_id: credentials.device_id.clone(),
            server_id: credentials.server_id.clone(),
            server_version: credentials.server_version.clone(),
            receiver_name: credentials.receiver_name.clone(),
            max_bitrate: credentials.max_bitrate,
            subtitle_appearance: credentials.subtitle_appearance.clone(),
        }
    }

    /// The JSON to send on [`NAMESPACE`].
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("an envelope is serializable")
    }
}

/// `PlayNow`'s options. The receiver fetches each item itself when it plays it; the position and
/// tracks apply only to the first item played.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    pub items: Vec<ItemStub>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_position_ticks: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_stream_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_stream_index: Option<i32>,
}

/// An item as the receiver gets it: just enough to fetch it from the server. The keys are the
/// server's own (`Id`, `Type`, …), which is what the receiver expects.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ItemStub {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "ServerId")]
    pub server_id: String,
    #[serde(rename = "Name")]
    pub name: String,
    /// The item kind, `Movie`, `Episode`, …
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(rename = "MediaType")]
    pub media_type: String,
    #[serde(rename = "IsFolder")]
    pub is_folder: bool,
}

/// What the receiver sends: `{type, data?, message?}`, and only to the sender of the last custom
/// message. `data` is normally an object; an older build could send it as a JSON string, which is
/// parsed too.
#[derive(Debug, Clone)]
pub enum ReceiverEvent {
    PlaybackStart(Event),
    /// About once a second while playing.
    PlaybackProgress(Event),
    PlayStateChange(Event),
    VolumeChange(Event),
    PlaybackStop(Event),
    /// `message` is the error code, e.g. `NoCompatibleStream`.
    PlaybackError(Event),
    /// The receiver couldn't reach the server (`PlaybackInfo` failed).
    ConnectionError(Event),
    Error(Event),
    RepeatModeChange(Event),
    /// A type we don't know. Kept rather than dropped: the receiver has changed before.
    Other(String, Event),
}

impl ReceiverEvent {
    /// Parses `{type, data?, message?}`. Only a payload with no `type` at all is an error.
    pub fn from_value(value: &Value) -> Result<Self> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Device("a receiver event without a type".into()))?
            .to_string();
        let event = Event {
            data: event_data(value.get("data")),
            // Empty strings are how the receiver says "no message" (`connectionerror` does).
            message: value
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty())
                .map(str::to_string),
        };
        Ok(match kind.as_str() {
            "playbackstart" => Self::PlaybackStart(event),
            "playbackprogress" => Self::PlaybackProgress(event),
            "playstatechange" => Self::PlayStateChange(event),
            "volumechange" => Self::VolumeChange(event),
            "playbackstop" => Self::PlaybackStop(event),
            "playbackerror" => Self::PlaybackError(event),
            "connectionerror" => Self::ConnectionError(event),
            "error" => Self::Error(event),
            "repeatmodechange" => Self::RepeatModeChange(event),
            _ => Self::Other(kind, event),
        })
    }

    /// The parsed `data` and `message`, whichever variant this is.
    pub fn event(&self) -> &Event {
        match self {
            Self::PlaybackStart(event)
            | Self::PlaybackProgress(event)
            | Self::PlayStateChange(event)
            | Self::VolumeChange(event)
            | Self::PlaybackStop(event)
            | Self::PlaybackError(event)
            | Self::ConnectionError(event)
            | Self::Error(event)
            | Self::RepeatModeChange(event)
            | Self::Other(_, event) => event,
        }
    }
}

/// The payload of an event.
#[derive(Debug, Clone, Default)]
pub struct Event {
    pub data: EventData,
    pub message: Option<String>,
}

/// `data`: the item, its play state and what's playing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventData {
    #[serde(rename = "ItemId")]
    pub item_id: Option<String>,
    #[serde(rename = "PlayState")]
    pub play_state: Option<PlayState>,
    #[serde(rename = "NowPlayingItem")]
    pub now_playing_item: Option<NowPlayingItem>,
}

/// The receiver's playback state. `VolumeLevel` is 0–100; positions are ticks.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PlayState {
    pub position_ticks: Option<i64>,
    pub is_paused: Option<bool>,
    pub is_muted: Option<bool>,
    pub volume_level: Option<f64>,
    pub audio_stream_index: Option<i32>,
    pub subtitle_stream_index: Option<i32>,
    pub media_source_id: Option<String>,
    pub play_method: Option<String>,
    pub can_seek: Option<bool>,
}

/// The item the receiver is playing, as the server returned it, plus the image ids and tags the UI
/// draws with. Streams and chapters stay as JSON: the app picks out what it needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct NowPlayingItem {
    pub id: Option<String>,
    pub name: Option<String>,
    /// The item kind (`Episode`, `Movie`, …): serde's `Type` rename below.
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub run_time_ticks: Option<i64>,
    pub series_name: Option<String>,
    pub index_number: Option<i32>,
    pub parent_index_number: Option<i32>,
    pub media_streams: Vec<Value>,
    pub chapters: Vec<Value>,
    pub image_tags: Map<String, Value>,
    pub backdrop_image_tags: Vec<String>,
    pub parent_backdrop_item_id: Option<String>,
    pub parent_backdrop_image_tags: Vec<String>,
    pub series_id: Option<String>,
    pub series_primary_image_tag: Option<String>,
    pub parent_thumb_item_id: Option<String>,
    pub parent_thumb_image_tag: Option<String>,
}

fn event_data(data: Option<&Value>) -> EventData {
    match data {
        Some(Value::Object(_)) => data
            .and_then(|data| serde_json::from_value(data.clone()).ok())
            .unwrap_or_default(),
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_default(),
        _ => EventData::default(),
    }
}

/// `urn:x-cast:com.google.cast.media` `MEDIA_STATUS`: the standard status every sender gets, not
/// just the last one. We follow it when another sender has taken over.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaStatus {
    pub status: Vec<MediaStatusEntry>,
}

impl MediaStatus {
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }

    /// The only entry a single-item load produces.
    pub fn entry(&self) -> Option<&MediaStatusEntry> {
        self.status.first()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaStatusEntry {
    /// The media element's position, in seconds.
    pub current_time: Option<f64>,
    /// `PLAYING`, `PAUSED`, `BUFFERING`, `IDLE`.
    pub player_state: Option<String>,
    pub media: Option<MediaInfo>,
}

impl MediaStatusEntry {
    /// The position the TV is really at, in seconds. A non-HLS transcode can't be seeked by the
    /// player, so its `currentTime` is where the transcode started and the offset has to be added
    /// (`canClientSeek` false). Anything else is already the real position.
    pub fn position_seconds(&self) -> Option<f64> {
        let current = self.current_time?;
        let custom = self.media.as_ref().and_then(|media| media.custom_data.as_ref());
        let start = custom.and_then(|custom| custom.start_position_ticks).unwrap_or(0) as f64 / TICKS_PER_SECOND;
        let can_client_seek = custom.and_then(|custom| custom.can_client_seek).unwrap_or(true);
        Some(if can_client_seek { current } else { current + start })
    }

    /// The item the TV is playing, which may not be ours if another sender took over.
    pub fn item_id(&self) -> Option<&str> {
        self.media
            .as_ref()
            .and_then(|media| media.custom_data.as_ref())
            .and_then(|custom| custom.item_id.as_deref())
    }
}

/// The media's own information, `customData` being the Jellyfin part.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub custom_data: Option<MediaCustomData>,
}

/// What the receiver puts in `media.customData`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaCustomData {
    pub item_id: Option<String>,
    pub media_source_id: Option<String>,
    pub audio_stream_index: Option<i32>,
    pub subtitle_stream_index: Option<i32>,
    /// False for a non-HLS transcode.
    pub can_client_seek: Option<bool>,
    pub start_position_ticks: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials {
            server_address: "http://192.168.1.5:8096".into(),
            access_token: "token-123".into(),
            user_id: "user-1".into(),
            device_id: "device-1".into(),
            server_id: "server-1".into(),
            server_version: "12.2.0".into(),
            receiver_name: "Living Room TV".into(),
            max_bitrate: None,
            subtitle_appearance: None,
        }
    }

    fn item() -> ItemStub {
        ItemStub {
            id: "item-1".into(),
            server_id: "server-1".into(),
            name: "Pilot".into(),
            kind: "Episode".into(),
            media_type: "Video".into(),
            is_folder: false,
        }
    }

    fn options(command: Command) -> Value {
        Envelope::new(&credentials(), command).to_value()["options"].clone()
    }

    #[test]
    fn envelopes_have_the_documented_shape() {
        let value = Envelope::new(&credentials(), Command::Identify).to_value();
        assert_eq!(
            value,
            json!({
                "command": "Identify",
                "options": {},
                "serverAddress": "http://192.168.1.5:8096",
                "accessToken": "token-123",
                "userId": "user-1",
                "deviceId": "device-1",
                "serverId": "server-1",
                "serverVersion": "12.2.0",
                "receiverName": "Living Room TV",
            })
        );

        // The optional ones appear only when set.
        let mut with_optionals = credentials();
        with_optionals.max_bitrate = Some(20_000_000);
        with_optionals.subtitle_appearance = Some(json!({"textSize": "large"}));
        let value = Envelope::new(&with_optionals, Command::Pause).to_value();
        assert_eq!(value["maxBitrate"], json!(20_000_000));
        assert_eq!(value["subtitleAppearance"], json!({"textSize": "large"}));
        let value = Envelope::new(&credentials(), Command::Pause).to_value();
        assert_eq!(value.get("maxBitrate"), None);
        assert_eq!(value.get("subtitleAppearance"), None);
    }

    #[test]
    fn command_options_match_the_receiver() {
        // No-argument commands all send `{}`.
        for command in [
            Command::Identify,
            Command::Pause,
            Command::Unpause,
            Command::PlayPause,
            Command::Stop,
            Command::NextTrack,
            Command::PreviousTrack,
        ] {
            assert_eq!(options(command.clone()), json!({}), "{command:?}");
        }

        let play = options(Command::PlayNow(PlayRequest {
            items: vec![item()],
            start_index: Some(0),
            start_position_ticks: Some(12_345_000),
            media_source_id: Some("source-1".into()),
            audio_stream_index: Some(1),
            subtitle_stream_index: Some(-1),
        }));
        assert_eq!(
            play,
            json!({
                "items": [{
                    "Id": "item-1",
                    "ServerId": "server-1",
                    "Name": "Pilot",
                    "Type": "Episode",
                    "MediaType": "Video",
                    "IsFolder": false
                }],
                "startIndex": 0,
                "startPositionTicks": 12_345_000,
                "mediaSourceId": "source-1",
                "audioStreamIndex": 1,
                "subtitleStreamIndex": -1
            })
        );

        // A bare play request has only the items.
        assert_eq!(
            options(Command::PlayNow(PlayRequest {
                items: vec![item()],
                ..PlayRequest::default()
            })),
            json!({"items": [{
                "Id": "item-1",
                "ServerId": "server-1",
                "Name": "Pilot",
                "Type": "Episode",
                "MediaType": "Video",
                "IsFolder": false
            }]})
        );

        for command in [
            Command::PlayNext { items: vec![item()] },
            Command::PlayLast { items: vec![item()] },
        ] {
            assert_eq!(
                options(command).get("items").and_then(Value::as_array).map(Vec::len),
                Some(1)
            );
        }
        // Shuffle takes a single item but sends it as a one-item list.
        assert_eq!(
            options(Command::Shuffle { item: item() }),
            json!({"items": [{
                "Id": "item-1",
                "ServerId": "server-1",
                "Name": "Pilot",
                "Type": "Episode",
                "MediaType": "Video",
                "IsFolder": false
            }]})
        );

        // Seconds, not ticks.
        assert_eq!(
            options(Command::Seek { position_seconds: 12.5 }),
            json!({"position": 12.5})
        );
        assert_eq!(options(Command::SetAudioStreamIndex(2)), json!({"index": 2}));
        assert_eq!(options(Command::SetSubtitleStreamIndex(-1)), json!({"index": -1}));
        assert_eq!(
            options(Command::SetRepeatMode(RepeatMode::One)),
            json!({"RepeatMode": "RepeatOne"})
        );
        assert_eq!(
            options(Command::SetRepeatMode(RepeatMode::None))["RepeatMode"],
            "RepeatNone"
        );
        assert_eq!(
            options(Command::SetRepeatMode(RepeatMode::All))["RepeatMode"],
            "RepeatAll"
        );

        // The command name is what the receiver dispatches on, and the envelope carries it.
        assert_eq!(
            Envelope::new(&credentials(), Command::Seek { position_seconds: 1.0 }).to_value()["command"],
            "Seek"
        );
    }

    #[test]
    fn envelope_json_is_a_string_of_the_value() {
        let envelope = Envelope::new(&credentials(), Command::NextTrack);
        let parsed: Value = serde_json::from_str(&envelope.to_json()).unwrap();
        assert_eq!(parsed, envelope.to_value());
    }

    #[test]
    fn receiver_events_parse() {
        for (kind, check) in [
            (
                "playbackstart",
                ReceiverEvent::PlaybackStart as fn(Event) -> ReceiverEvent,
            ),
            ("playbackprogress", ReceiverEvent::PlaybackProgress),
            ("playstatechange", ReceiverEvent::PlayStateChange),
            ("volumechange", ReceiverEvent::VolumeChange),
            ("playbackstop", ReceiverEvent::PlaybackStop),
            ("playbackerror", ReceiverEvent::PlaybackError),
            ("connectionerror", ReceiverEvent::ConnectionError),
            ("error", ReceiverEvent::Error),
            ("repeatmodechange", ReceiverEvent::RepeatModeChange),
        ] {
            let value = json!({"type": kind, "data": {"ItemId": "item-1"}});
            let event = ReceiverEvent::from_value(&value).unwrap();
            assert_eq!(event.event().data.item_id.as_deref(), Some("item-1"), "{kind}");
            assert_eq!(
                std::mem::discriminant(&event),
                std::mem::discriminant(&check(Event::default()))
            );
        }

        // An unknown type is carried, not dropped.
        let event = ReceiverEvent::from_value(&json!({"type": "somethingnew", "data": {"ItemId": "x"}})).unwrap();
        match event {
            ReceiverEvent::Other(kind, event) => {
                assert_eq!(kind, "somethingnew");
                assert_eq!(event.data.item_id.as_deref(), Some("x"));
            }
            other => panic!("expected Other, got {other:?}"),
        }

        // Errors carry the code in `message`.
        let event =
            ReceiverEvent::from_value(&json!({"type": "playbackerror", "message": "NoCompatibleStream"})).unwrap();
        assert_eq!(event.event().message.as_deref(), Some("NoCompatibleStream"));
        // An empty message means none.
        let event = ReceiverEvent::from_value(&json!({"type": "connectionerror", "message": ""})).unwrap();
        assert_eq!(event.event().message, None);

        // Missing or unusable data leaves the event with defaults instead of failing.
        let event = ReceiverEvent::from_value(&json!({"type": "playbackstop"})).unwrap();
        assert!(event.event().data.item_id.is_none());
        assert!(event.event().data.play_state.is_none());
        let event = ReceiverEvent::from_value(&json!({"type": "playbackstart", "data": "not json"})).unwrap();
        assert!(event.event().data.item_id.is_none());
        // ... and so does data sent as a JSON string.
        let event =
            ReceiverEvent::from_value(&json!({"type": "playbackstart", "data": "{\"ItemId\":\"item-2\"}"})).unwrap();
        assert_eq!(event.event().data.item_id.as_deref(), Some("item-2"));

        // Only a payload with no type is an error.
        assert!(ReceiverEvent::from_value(&json!({"data": {}})).is_err());
    }

    #[test]
    fn playback_state_parses_from_a_realistic_event() {
        // Trimmed from a jellyfin-chromecast playbackprogress message.
        let value = json!({
            "type": "playbackprogress",
            "data": {
                "ItemId": "5f2f4c8e0f5a4b3f8a1d9c7b",
                "PlayState": {
                    "AudioStreamIndex": 1,
                    "CanSeek": true,
                    "IsMuted": false,
                    "IsPaused": true,
                    "ItemId": "5f2f4c8e0f5a4b3f8a1d9c7b",
                    "MediaSourceId": "5f2f4c8e0f5a4b3f8a1d9c7c",
                    "PlayMethod": "Transcode",
                    "PlaySessionId": "abc",
                    "PositionTicks": 600000000,
                    "RepeatMode": "RepeatNone",
                    "SubtitleStreamIndex": -1,
                    "VolumeLevel": 40,
                    "SomethingNew": [1, 2, 3]
                },
                "NowPlayingItem": {
                    "Id": "5f2f4c8e0f5a4b3f8a1d9c7b",
                    "Name": "The Pilot",
                    "Type": "Episode",
                    "RunTimeTicks": 14400000000i64,
                    "SeriesName": "A Series",
                    "IndexNumber": 3,
                    "ParentIndexNumber": 1,
                    "MediaStreams": [{"Index": 1, "Type": "Audio", "Codec": "aac"}],
                    "Chapters": [{"StartPositionTicks": 0, "Name": "Opening"}],
                    "ImageTags": {"Primary": "tag1"},
                    "BackdropImageTags": ["backdrop1"],
                    "ParentBackdropItemId": "series-1",
                    "ParentBackdropImageTags": ["backdrop2"],
                    "SeriesId": "series-1",
                    "SeriesPrimaryImageTag": "series-tag",
                    "ParentThumbItemId": "series-1",
                    "ParentThumbImageTag": "thumb-tag",
                    "MediaType": "Video",
                    "ServerId": "server-1"
                }
            }
        });
        let event = ReceiverEvent::from_value(&value).unwrap();
        let data = &event.event().data;
        assert_eq!(data.item_id.as_deref(), Some("5f2f4c8e0f5a4b3f8a1d9c7b"));
        let state = data.play_state.as_ref().unwrap();
        assert_eq!(state.position_ticks, Some(600_000_000));
        assert_eq!(state.is_paused, Some(true));
        assert_eq!(state.is_muted, Some(false));
        assert_eq!(state.volume_level, Some(40.0));
        assert_eq!(state.audio_stream_index, Some(1));
        assert_eq!(state.subtitle_stream_index, Some(-1));
        assert_eq!(state.media_source_id.as_deref(), Some("5f2f4c8e0f5a4b3f8a1d9c7c"));
        assert_eq!(state.play_method.as_deref(), Some("Transcode"));
        assert_eq!(state.can_seek, Some(true));

        let now_playing = data.now_playing_item.as_ref().unwrap();
        assert_eq!(now_playing.id.as_deref(), Some("5f2f4c8e0f5a4b3f8a1d9c7b"));
        assert_eq!(now_playing.kind.as_deref(), Some("Episode"));
        assert_eq!(now_playing.run_time_ticks, Some(14_400_000_000));
        assert_eq!(now_playing.index_number, Some(3));
        assert_eq!(now_playing.parent_index_number, Some(1));
        assert_eq!(now_playing.series_name.as_deref(), Some("A Series"));
        assert_eq!(now_playing.media_streams.len(), 1);
        assert_eq!(now_playing.chapters.len(), 1);
        assert_eq!(
            now_playing.image_tags.get("Primary").and_then(Value::as_str),
            Some("tag1")
        );
        assert_eq!(now_playing.backdrop_image_tags, ["backdrop1"]);
        assert_eq!(now_playing.parent_backdrop_item_id.as_deref(), Some("series-1"));
        assert_eq!(now_playing.parent_backdrop_image_tags, ["backdrop2"]);
        assert_eq!(now_playing.series_id.as_deref(), Some("series-1"));
        assert_eq!(now_playing.series_primary_image_tag.as_deref(), Some("series-tag"));
        assert_eq!(now_playing.parent_thumb_item_id.as_deref(), Some("series-1"));
        assert_eq!(now_playing.parent_thumb_image_tag.as_deref(), Some("thumb-tag"));
    }

    #[test]
    fn media_status_position() {
        // A seekable stream: `currentTime` is the real position.
        let value = json!({
            "type": "MEDIA_STATUS",
            "requestId": 0,
            "status": [{
                "currentTime": 30.0,
                "playerState": "PLAYING",
                "media": {
                    "duration": 1440.0,
                    "customData": {
                        "itemId": "item-1",
                        "mediaSourceId": "source-1",
                        "audioStreamIndex": 1,
                        "subtitleStreamIndex": -1,
                        "canClientSeek": true,
                        "canSeek": true,
                        "startPositionTicks": 600000000,
                        "playMethod": "DirectStream"
                    }
                }
            }]
        });
        let status = MediaStatus::from_value(&value).unwrap();
        let entry = status.entry().unwrap();
        assert_eq!(entry.player_state.as_deref(), Some("PLAYING"));
        assert_eq!(entry.item_id(), Some("item-1"));
        assert_eq!(entry.position_seconds(), Some(30.0));
        assert_eq!(entry.media.as_ref().and_then(|media| media.duration), Some(1440.0));
        let custom = entry.media.as_ref().and_then(|m| m.custom_data.as_ref()).unwrap();
        assert_eq!(custom.media_source_id.as_deref(), Some("source-1"));
        assert_eq!(custom.audio_stream_index, Some(1));
        assert_eq!(custom.subtitle_stream_index, Some(-1));
        assert_eq!(custom.start_position_ticks, Some(600_000_000));

        // A non-HLS transcode: `currentTime` counts from the transcode's start, so 30 s in at the
        // 60 s mark means 90 s of the item.
        let value = json!({"status": [{
            "currentTime": 30.0,
            "media": {"customData": {"itemId": "item-1", "canClientSeek": false, "startPositionTicks": 600000000}}
        }]});
        let status = MediaStatus::from_value(&value).unwrap();
        assert_eq!(status.entry().unwrap().position_seconds(), Some(90.0));

        // No status entry, or no time to report.
        assert!(MediaStatus::from_value(&json!({})).unwrap().entry().is_none());
        let value = json!({"status": [{"playerState": "IDLE"}]});
        assert_eq!(
            MediaStatus::from_value(&value)
                .unwrap()
                .entry()
                .unwrap()
                .position_seconds(),
            None
        );
    }
}
