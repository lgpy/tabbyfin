//! Audio output settings as mpv options, and mpv's list of audio devices.
//!
//! The app keeps the choices in its config and hands them over as [`AudioSettings`]; the
//! properties it turns into are set before a file loads (all of them can also change while one
//! plays). Names and values are from mpv's manual: `audio-device`, `audio-channels`,
//! `audio-spdif`, `audio-exclusive`, `af` and `volume-max`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// An audio output mpv found: `name` is what `audio-device` takes (`wasapi/{guid}`,
/// `alsa/hdmi:CARD=…`), `description` what to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

/// The entries of `audio-device-list`'s JSON, without mpv's own `auto` entry (the app's
/// "Automatic" is that).
pub(crate) fn parse_devices(json: &str) -> Result<Vec<AudioDevice>> {
    let bad = |message: &str| Error::Command {
        command: "get audio-device-list".into(),
        message: message.into(),
    };
    let list: Value = serde_json::from_str(json).map_err(|err| bad(&err.to_string()))?;
    let entries = list.as_array().ok_or_else(|| bad("not a list"))?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?;
            let description = entry.get("description").and_then(Value::as_str).unwrap_or(name);
            (name != "auto").then(|| AudioDevice {
                name: name.to_owned(),
                description: description.to_owned(),
            })
        })
        .collect())
}

/// The channel layout mpv sends to the device (`audio-channels`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channels {
    /// mpv's own default, `auto-safe`: the system's preferred layout, stereo where the device
    /// can't say (a bare hardware device).
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "stereo")]
    Stereo,
    #[serde(rename = "5.1")]
    Surround5_1,
    #[serde(rename = "7.1")]
    Surround7_1,
}

impl Channels {
    pub const ALL: [Channels; 4] = [
        Channels::Auto,
        Channels::Stereo,
        Channels::Surround5_1,
        Channels::Surround7_1,
    ];

    /// The `audio-channels` value.
    pub fn mpv(self) -> &'static str {
        match self {
            Channels::Auto => "auto-safe",
            Channels::Stereo => "stereo",
            Channels::Surround5_1 => "5.1",
            Channels::Surround7_1 => "7.1",
        }
    }
}

/// The formats sent to the receiver as they are (`audio-spdif`), instead of decoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Passthrough {
    pub ac3: bool,
    pub eac3: bool,
    pub dts: bool,
    pub dts_hd: bool,
    pub truehd: bool,
}

impl Passthrough {
    pub fn any(self) -> bool {
        self.ac3 || self.eac3 || self.dts || self.dts_hd || self.truehd
    }

    /// The `audio-spdif` value: mpv's codec names, comma separated; empty for none.
    pub fn mpv(self) -> String {
        [
            (self.ac3, "ac3"),
            (self.eac3, "eac3"),
            (self.dts, "dts"),
            (self.dts_hd, "dts-hd"),
            (self.truehd, "truehd"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect::<Vec<_>>()
        .join(",")
    }
}

/// What the app asks of the audio output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioSettings {
    /// An mpv device name; `None` is automatic.
    pub device: Option<String>,
    pub channels: Channels,
    pub passthrough: Passthrough,
    /// `audio-exclusive`: only mpv gets the device (where the output has such a mode).
    pub exclusive: bool,
    /// Compress the dynamic range, for quiet evenings.
    pub night_mode: bool,
}

/// The percentage the volume can reach (`volume-max`); mpv's default is 130. The server and
/// remote control keep to 0–100.
pub const MAX_VOLUME: f64 = 150.;

/// FFmpeg's `acompressor` through mpv's `lavfi` filter (its parameters: `threshold` is a linear
/// level, `ratio` the compression, `attack` and `release` are in milliseconds, `makeup` a
/// linear gain). Loud scenes come down and quiet speech comes up. The brackets keep the
/// filter's own colons out of mpv's option parser.
const NIGHT_FILTER: &str = "lavfi=[acompressor=threshold=0.05:ratio=4:attack=20:release=250:makeup=2]";

impl AudioSettings {
    /// Night mode as the `af` value. A passed-through stream reaches the receiver undecoded, so
    /// no filter can act on it, and a filter would be in passthrough's way: with passthrough
    /// on, night mode stays off.
    pub fn filter(&self) -> &'static str {
        night_filter(self.night_mode, self.passthrough)
    }

    /// The mpv properties to set, in order, before a file loads. `audio-device` is left out
    /// for automatic, which is mpv's own default.
    pub fn properties(&self) -> Vec<(&'static str, String)> {
        let mut properties = Vec::new();
        if let Some(device) = &self.device {
            properties.push(("audio-device", device.clone()));
        }
        properties.extend([
            ("audio-channels", self.channels.mpv().to_owned()),
            ("audio-spdif", self.passthrough.mpv()),
            ("audio-exclusive", if self.exclusive { "yes" } else { "no" }.to_owned()),
            ("af", self.filter().to_owned()),
            ("volume-max", MAX_VOLUME.to_string()),
        ]);
        properties
    }
}

/// The `af` value for night mode on or off with `passthrough`; empty is no filter.
pub fn night_filter(on: bool, passthrough: Passthrough) -> &'static str {
    if on && !passthrough.any() { NIGHT_FILTER } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_changes_nothing_in_mpv() {
        // mpv's own defaults: auto-safe, no passthrough, shared mode, no filter, no device
        // (`auto`), and a wider volume range.
        let properties = AudioSettings::default().properties();
        assert_eq!(
            properties,
            [
                ("audio-channels", "auto-safe".to_owned()),
                ("audio-spdif", String::new()),
                ("audio-exclusive", "no".to_owned()),
                ("af", String::new()),
                ("volume-max", "150".to_owned()),
            ]
        );
    }

    #[test]
    fn each_setting_becomes_its_option() {
        let settings = AudioSettings {
            device: Some("wasapi/{abc}".into()),
            channels: Channels::Surround7_1,
            passthrough: Passthrough {
                ac3: true,
                dts_hd: true,
                truehd: true,
                ..Passthrough::default()
            },
            exclusive: true,
            night_mode: false,
        };
        let properties = settings.properties();
        assert_eq!(properties[0], ("audio-device", "wasapi/{abc}".to_owned()));
        assert_eq!(properties[1], ("audio-channels", "7.1".to_owned()));
        assert_eq!(properties[2], ("audio-spdif", "ac3,dts-hd,truehd".to_owned()));
        assert_eq!(properties[3], ("audio-exclusive", "yes".to_owned()));
        for (channels, value) in [
            (Channels::Auto, "auto-safe"),
            (Channels::Stereo, "stereo"),
            (Channels::Surround5_1, "5.1"),
        ] {
            assert_eq!(channels.mpv(), value);
        }
        let all = Passthrough {
            ac3: true,
            eac3: true,
            dts: true,
            dts_hd: true,
            truehd: true,
        };
        assert_eq!(all.mpv(), "ac3,eac3,dts,dts-hd,truehd");
    }

    #[test]
    fn night_mode_steps_aside_for_passthrough() {
        let mut settings = AudioSettings {
            night_mode: true,
            ..AudioSettings::default()
        };
        assert!(settings.filter().starts_with("lavfi=[acompressor="));
        assert!(settings.properties().contains(&("af", settings.filter().to_owned())));
        settings.passthrough.eac3 = true;
        assert_eq!(settings.filter(), "");
        assert_eq!(night_filter(false, Passthrough::default()), "");
    }

    #[test]
    fn devices_come_from_the_json_without_the_auto_entry() {
        let json = r#"[
            {"name": "auto", "description": "Autoselect device"},
            {"name": "wasapi/{1}", "description": "Speakers (Realtek)"},
            {"name": "wasapi/{2}"}
        ]"#;
        let devices = parse_devices(json).unwrap();
        assert_eq!(
            devices,
            [
                AudioDevice {
                    name: "wasapi/{1}".into(),
                    description: "Speakers (Realtek)".into()
                },
                AudioDevice {
                    name: "wasapi/{2}".into(),
                    description: "wasapi/{2}".into()
                },
            ]
        );
        assert!(parse_devices("{}").is_err());
        assert!(parse_devices("nope").is_err());
    }

    #[test]
    fn channels_are_saved_by_their_layout() {
        for (channels, text) in [
            (Channels::Auto, "\"auto\""),
            (Channels::Stereo, "\"stereo\""),
            (Channels::Surround5_1, "\"5.1\""),
            (Channels::Surround7_1, "\"7.1\""),
        ] {
            assert_eq!(serde_json::to_string(&channels).unwrap(), text);
            assert_eq!(serde_json::from_str::<Channels>(text).unwrap(), channels);
        }
    }
}
