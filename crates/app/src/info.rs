//! The player's info overlay: how the item plays (direct play, direct stream or a transcode and
//! why) from what the server negotiated, and what mpv reports of the running stream.
//!
//! The server's side is in the media source and [`StreamInfo`]; mpv's is [`Stats`], asked for about
//! once a second while the overlay shows (`UiCommand::PollInfo`). Where mpv has nothing (Media3),
//! the rows that need it are left out rather than shown empty.

use jellyfin::PlayMethod;
use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

use crate::i18n::{self, tr};
use crate::playback::{NowPlaying, StreamInfo};

/// The mpv properties [`Stats::parse`] reads, in order.
pub const PROPERTIES: [&str; 11] = [
    "hwdec-current",
    "video-params/w",
    "video-params/h",
    "container-fps",
    "video-bitrate",
    "audio-bitrate",
    "video-params/gamma",
    "audio-params/hr-channels",
    "frame-drop-count",
    "decoder-frame-drop-count",
    "demuxer-cache-duration",
];

/// What mpv reported at the last poll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// The hardware decoder in use; `no` or empty for software decoding.
    pub hwdec: Option<String>,
    pub size: Option<(u32, u32)>,
    pub fps: Option<f64>,
    /// Bits per second.
    pub video_bitrate: Option<f64>,
    pub audio_bitrate: Option<f64>,
    /// The video's transfer function: `pq` and `hlg` are HDR.
    pub gamma: Option<String>,
    /// The audio's layout in words, e.g. `5.1(side)`.
    pub channels: Option<String>,
    /// Frames dropped by the decoder and the video output together.
    pub dropped: Option<u64>,
    /// Seconds of video read ahead.
    pub buffered: Option<f64>,
}

impl Stats {
    /// From the values of [`PROPERTIES`], in order.
    pub fn parse(values: &[Option<String>]) -> Self {
        let text = |i: usize| values.get(i).cloned().flatten().filter(|v| !v.is_empty());
        let number = |i: usize| text(i).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
        let whole = |i: usize| number(i).map(|v| v.max(0.) as u64);
        let size = whole(1).zip(whole(2)).filter(|&(w, h)| w > 0 && h > 0);
        let drops = [whole(8), whole(9)];
        Stats {
            hwdec: text(0),
            size: size.map(|(w, h)| (w as u32, h as u32)),
            fps: number(3).filter(|&fps| fps > 0.),
            video_bitrate: number(4).filter(|&bits| bits > 0.),
            audio_bitrate: number(5).filter(|&bits| bits > 0.),
            gamma: text(6),
            channels: text(7),
            dropped: drops.iter().any(Option::is_some).then(|| drops.iter().flatten().sum()),
            buffered: number(10).map(|seconds| seconds.max(0.)),
        }
    }

    /// Whether mpv answered with anything at all.
    fn any(&self) -> bool {
        *self != Stats::default()
    }
}

/// How the picture gets to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPath {
    /// GPU frames the window's renderer imports.
    Gpu,
    /// Pixels read back from the GPU or rendered by the CPU.
    Software,
    /// The player draws into a surface of its own under the window (Android).
    Surface,
}

/// How the item plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    DirectPlay,
    DirectStream,
    Transcode,
}

impl Method {
    /// Direct play when the server can serve the file as is; direct stream when it only has to
    /// remux it.
    pub fn of(source: &MediaSourceInfo, stream: &StreamInfo) -> Self {
        if stream.method == PlayMethod::Transcode {
            Method::Transcode
        } else if !source.supports_direct_play && source.supports_direct_stream {
            Method::DirectStream
        } else {
            Method::DirectPlay
        }
    }
}

/// One line of the overlay. A row with no label continues the one above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
}

fn row(label: String, value: String) -> Row {
    Row { label, value }
}

/// The overlay's rows for the playing item. `path` is how the video widget draws the picture.
pub fn rows(state: &NowPlaying, path: Option<OutputPath>) -> Vec<Row> {
    let stats = state.player_stats.as_ref();
    let method = Method::of(&state.source, &state.stream);
    let mut rows = vec![row(
        tr!("player-info-playing"),
        match method {
            Method::DirectPlay => tr!("player-info-direct-play"),
            Method::DirectStream => tr!("player-info-direct-stream"),
            Method::Transcode => tr!("player-info-transcoding"),
        },
    )];
    for (i, reason) in state.stream.transcode_reasons.iter().enumerate() {
        let label = if i == 0 {
            tr!("player-info-because")
        } else {
            String::new()
        };
        rows.push(row(label, transcode_reason(reason)));
    }
    let transcoding = method == Method::Transcode;
    let stream_of = |kind| {
        let streams = state.source.media_streams.iter().filter(move |s| s.kind == kind);
        // The audio track playing, else the first.
        let chosen = (kind == MediaStreamType::Audio)
            .then(|| streams.clone().find(|s| Some(s.index) == state.audio))
            .flatten();
        chosen.or_else(|| streams.clone().next())
    };
    let video = stream_of(MediaStreamType::Video);
    let audio = stream_of(MediaStreamType::Audio);

    let container = if transcoding {
        Some("HLS".to_owned())
    } else {
        state.source.container.as_deref().map(codec_name)
    };
    if let Some(container) = container {
        rows.push(row(tr!("player-info-container"), container));
    }

    // A transcode's picture isn't the file's: mpv's numbers are the stream actually playing.
    let video_codec = transcoding
        .then_some(state.stream.transcode_video_codec.as_deref())
        .flatten()
        .or_else(|| video.and_then(|v| v.codec.as_deref()));
    if let Some(codec) = video_codec {
        rows.push(row(tr!("player-info-video"), codec_name(codec)));
    }
    let size = stats
        .and_then(|s| s.size)
        .or_else(|| video.and_then(|v| v.width.zip(v.height)));
    if let Some((width, height)) = size {
        rows.push(row(tr!("player-info-resolution"), format!("{width}×{height}")));
    }
    let fps = stats.and_then(|s| s.fps).or_else(|| {
        video
            .and_then(|v| v.real_frame_rate.or(v.average_frame_rate))
            .map(f64::from)
    });
    if let Some(fps) = fps.filter(|&fps| fps > 0.) {
        rows.push(row(
            tr!("player-info-frame-rate"),
            tr!("player-info-fps", fps = fps_text(fps)),
        ));
    }
    let video_bitrate = stats
        .and_then(|s| s.video_bitrate)
        .filter(|_| transcoding)
        .or_else(|| video.and_then(|v| v.bit_rate).map(|bits| bits as f64))
        .or_else(|| stats.and_then(|s| s.video_bitrate));
    if let Some(bits) = video_bitrate {
        rows.push(row(tr!("player-info-video-bitrate"), bitrate_text(bits)));
    }
    if let Some(range) = range_text(video, stats.and_then(|s| s.gamma.as_deref()), transcoding) {
        rows.push(row(tr!("player-info-range"), range));
    }

    let audio_codec = transcoding
        .then_some(state.stream.transcode_audio_codec.as_deref())
        .flatten()
        .or_else(|| audio.and_then(|a| a.codec.as_deref()));
    if let Some(codec) = audio_codec {
        rows.push(row(tr!("player-info-audio"), codec_name(codec)));
    }
    let channels = stats
        .and_then(|s| s.channels.clone())
        .or_else(|| audio.and_then(channels_text));
    if let Some(channels) = channels {
        rows.push(row(tr!("player-info-channels"), channels));
    }
    let audio_bitrate = audio
        .and_then(|a| a.bit_rate)
        .map(|bits| bits as f64)
        .filter(|_| !transcoding)
        .or_else(|| stats.and_then(|s| s.audio_bitrate));
    if let Some(bits) = audio_bitrate {
        rows.push(row(tr!("player-info-audio-bitrate"), bitrate_text(bits)));
    }

    if let Some(stats) = stats.filter(|s| s.any()) {
        if let Some(hwdec) = &stats.hwdec {
            rows.push(row(tr!("player-info-decoder"), decoder_text(hwdec)));
        }
        if let Some(dropped) = stats.dropped {
            rows.push(row(tr!("player-info-dropped"), dropped.to_string()));
        }
        if let Some(buffered) = stats.buffered {
            rows.push(row(
                tr!("player-info-buffered"),
                tr!("player-info-seconds", seconds = i18n::decimal(format!("{buffered:.1}"))),
            ));
        }
    }
    if let Some(path) = path {
        let zero_copy = stats.and_then(|s| s.hwdec.as_deref()).is_some_and(zero_copy);
        let value = match path {
            OutputPath::Gpu if zero_copy => tr!("player-info-output-zero-copy"),
            OutputPath::Gpu => tr!("player-info-output-gpu"),
            OutputPath::Software => tr!("player-info-output-software"),
            OutputPath::Surface => tr!("player-info-output-surface"),
        };
        rows.push(row(tr!("player-info-output"), value));
    }
    rows
}

/// Upper-cased, with the ones people write with a dot or a plus spelled out.
fn codec_name(codec: &str) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "h264" => "H.264".into(),
        "hevc" | "h265" => "HEVC".into(),
        "eac3" => "E-AC-3".into(),
        "ac3" => "AC-3".into(),
        "truehd" => "TrueHD".into(),
        "dts" => "DTS".into(),
        other => other.to_uppercase(),
    }
}

fn fps_text(fps: f64) -> String {
    fps_in(i18n::current(), fps)
}

fn fps_in(language: i18n::Language, fps: f64) -> String {
    let text = format!("{fps:.3}");
    i18n::decimal_in(language, text.trim_end_matches('0').trim_end_matches('.'))
}

/// Bits per second as kbit/s or Mbit/s, with the language's decimal mark.
pub fn bitrate_text(bits: f64) -> String {
    bitrate_in(i18n::current(), bits)
}

fn bitrate_in(language: i18n::Language, bits: f64) -> String {
    if bits >= 1_000_000. {
        format!(
            "{} Mbit/s",
            i18n::decimal_in(language, format!("{:.1}", bits / 1_000_000.))
        )
    } else {
        format!("{:.0} kbit/s", bits / 1_000.)
    }
}

/// `SDR`, `HDR10`… from the server's range type, else from mpv's transfer function. A
/// transcode's range is what mpv sees, since the server may have tone-mapped it.
fn range_text(video: Option<&MediaStream>, gamma: Option<&str>, transcoding: bool) -> Option<String> {
    let from_mpv = gamma.map(|gamma| match gamma {
        "pq" => "HDR10".to_owned(),
        "hlg" => "HLG".to_owned(),
        _ => "SDR".to_owned(),
    });
    let from_server = video
        .and_then(|v| v.video_range_type.as_deref())
        .filter(|range| !range.is_empty() && *range != "Unknown")
        .map(|range| match range {
            "HDR10Plus" => "HDR10+".to_owned(),
            "DOVI" => "Dolby Vision".to_owned(),
            "DOVIWithHDR10" => "Dolby Vision, HDR10".to_owned(),
            "DOVIWithHDR10Plus" => "Dolby Vision, HDR10+".to_owned(),
            "DOVIWithHLG" => "Dolby Vision, HLG".to_owned(),
            "DOVIWithSDR" => "Dolby Vision, SDR".to_owned(),
            "DOVIWithEL" | "DOVIWithELHDR10Plus" => "Dolby Vision".to_owned(),
            other => other.to_owned(),
        });
    if transcoding {
        from_mpv.or(from_server)
    } else {
        from_server.or(from_mpv)
    }
}

fn channels_text(audio: &MediaStream) -> Option<String> {
    audio
        .channel_layout
        .clone()
        .filter(|layout| !layout.is_empty())
        .or_else(|| {
            audio
                .channels
                .map(|channels| tr!("player-info-channel-count", count = i64::from(channels)))
        })
}

/// mpv's `hwdec-current` in words: `no` is software decoding.
fn decoder_text(hwdec: &str) -> String {
    match hwdec {
        "" | "no" => tr!("player-info-decoder-software"),
        hwdec => hwdec.to_owned(),
    }
}

/// A hardware decoder whose frames stay on the GPU; the `-copy` ones read them back.
fn zero_copy(hwdec: &str) -> bool {
    !matches!(hwdec, "" | "no") && !hwdec.ends_with("-copy")
}

/// A `TranscodeReasons` value from the server in words. One it doesn't know is its name split
/// into words.
pub fn transcode_reason(reason: &str) -> String {
    const KNOWN: [&str; 27] = [
        "ContainerNotSupported",
        "VideoCodecNotSupported",
        "AudioCodecNotSupported",
        "SubtitleCodecNotSupported",
        "AudioIsExternal",
        "SecondaryAudioNotSupported",
        "VideoProfileNotSupported",
        "VideoLevelNotSupported",
        "VideoResolutionNotSupported",
        "VideoBitDepthNotSupported",
        "VideoFramerateNotSupported",
        "VideoRangeTypeNotSupported",
        "VideoCodecTagNotSupported",
        "RefFramesNotSupported",
        "AnamorphicVideoNotSupported",
        "InterlacedVideoNotSupported",
        "AudioChannelsNotSupported",
        "AudioProfileNotSupported",
        "AudioSampleRateNotSupported",
        "AudioBitDepthNotSupported",
        "ContainerBitrateExceedsLimit",
        "VideoBitrateNotSupported",
        "AudioBitrateNotSupported",
        "UnknownVideoStreamInfo",
        "UnknownAudioStreamInfo",
        "DirectPlayError",
        "StreamCountExceedsLimit",
    ];
    if KNOWN.contains(&reason) {
        return tr!("player-info-reason", reason = reason);
    }
    let mut words = String::new();
    for (i, c) in reason.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            words.push(' ');
            words.extend(c.to_lowercase());
        } else if i == 0 {
            words.extend(c.to_uppercase());
        } else {
            words.push(c);
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> Vec<Option<String>> {
        PROPERTIES
            .iter()
            .map(|name| {
                pairs
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
            .collect()
    }

    #[test]
    fn mpvs_text_becomes_numbers() {
        let stats = Stats::parse(&values(&[
            ("hwdec-current", "d3d11va"),
            ("video-params/w", "1920"),
            ("video-params/h", "1080"),
            ("container-fps", "23.976024"),
            ("video-bitrate", "7636775"),
            ("video-params/gamma", "pq"),
            ("frame-drop-count", "2"),
            ("decoder-frame-drop-count", "3"),
            ("demuxer-cache-duration", "12.25"),
        ]));
        assert_eq!(stats.hwdec.as_deref(), Some("d3d11va"));
        assert_eq!(stats.size, Some((1920, 1080)));
        assert_eq!(stats.dropped, Some(5));
        assert_eq!(stats.buffered, Some(12.25));
        assert_eq!(stats.audio_bitrate, None);
        // Nothing playing yet: nothing known.
        assert!(!Stats::parse(&values(&[])).any());
        assert_eq!(Stats::parse(&values(&[("video-params/w", "1920")])).size, None);
    }

    #[test]
    fn rates_and_codecs_read_as_people_write_them() {
        assert_eq!(bitrate_text(7_636_775.), "7.6 Mbit/s");
        assert_eq!(bitrate_text(192_000.), "192 kbit/s");
        assert_eq!(fps_text(24.), "24");
        assert_eq!(fps_text(23.976), "23.976");
        // The decimal mark is the language's.
        assert_eq!(bitrate_in(i18n::Language::PortuguesePortugal, 7_636_775.), "7,6 Mbit/s");
        assert_eq!(bitrate_in(i18n::Language::Spanish, 7_636_775.), "7,6 Mbit/s");
        assert_eq!(fps_in(i18n::Language::Spanish, 23.976), "23,976");
        assert_eq!(fps_in(i18n::Language::English, 23.976), "23.976");
        assert_eq!(codec_name("h264"), "H.264");
        assert_eq!(codec_name("aac"), "AAC");
        assert!(zero_copy("d3d11va") && !zero_copy("d3d11va-copy") && !zero_copy("no"));
    }

    #[test]
    fn the_servers_reasons_are_worded_for_people() {
        assert_eq!(
            transcode_reason("VideoCodecNotSupported"),
            "The video codec isn't supported by this player"
        );
        assert_eq!(
            transcode_reason("ContainerBitrateExceedsLimit"),
            "The bitrate is over the streaming limit"
        );
        // A reason from a newer server is its name in words.
        assert_eq!(transcode_reason("SomethingNewHappened"), "Something new happened");
    }

    #[test]
    fn the_range_prefers_the_servers_word_unless_transcoding() {
        let stream: MediaStream = serde_json::from_value(serde_json::json!({
            "Index": 0, "Type": "Video", "VideoRangeType": "DOVIWithHDR10"
        }))
        .unwrap();
        assert_eq!(
            range_text(Some(&stream), Some("pq"), false).as_deref(),
            Some("Dolby Vision, HDR10")
        );
        // Tone-mapped by the server: what plays is what mpv sees.
        assert_eq!(range_text(Some(&stream), Some("bt.1886"), true).as_deref(), Some("SDR"));
        assert_eq!(range_text(None, None, false), None);
    }
}
