//! Playback negotiation, stream URLs, and session reporting. See `docs/PLAN.md` §5.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::client::Query;
use crate::models::{MediaSourceInfo, MediaStream};
use crate::{Client, Result, Ticks};

#[derive(Debug, Clone, Default)]
pub struct PlaybackInfoRequest {
    pub media_source_id: Option<String>,
    pub start_position: Option<Ticks>,
    pub audio_stream_index: Option<i32>,
    pub subtitle_stream_index: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfoResponse {
    #[serde(default)]
    pub media_sources: Vec<MediaSourceInfo>,
    pub play_session_id: Option<String>,
    /// Set when the server refuses playback, e.g. `NotAllowed` or `NoCompatibleStream`.
    pub error_code: Option<String>,
}

/// Progress for the `/Sessions/Playing*` endpoints.
#[derive(Debug, Clone, Default)]
pub struct PlaybackReport {
    pub item_id: String,
    pub media_source_id: String,
    pub play_session_id: Option<String>,
    pub position: Ticks,
    pub is_paused: bool,
    pub audio_stream_index: Option<i32>,
    pub subtitle_stream_index: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ProgressBody<'a> {
    item_id: &'a str,
    media_source_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    play_session_id: Option<&'a str>,
    position_ticks: Ticks,
    is_paused: bool,
    can_seek: bool,
    play_method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_stream_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle_stream_index: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StopBody<'a> {
    item_id: &'a str,
    media_source_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    play_session_id: Option<&'a str>,
    position_ticks: Ticks,
}

impl PlaybackReport {
    fn progress_body(&self) -> ProgressBody<'_> {
        ProgressBody {
            item_id: &self.item_id,
            media_source_id: &self.media_source_id,
            play_session_id: self.play_session_id.as_deref(),
            position_ticks: self.position,
            is_paused: self.is_paused,
            can_seek: true,
            play_method: "DirectPlay",
            audio_stream_index: self.audio_stream_index,
            subtitle_stream_index: self.subtitle_stream_index,
        }
    }
}

/// mpv plays nearly everything, so the profile allows direct play of any container and
/// codec and offers no transcoding. External text subtitles are delivered as files; anything
/// else stays embedded.
fn device_profile() -> serde_json::Value {
    let external = ["srt", "subrip", "ass", "ssa", "vtt", "webvtt", "sub", "smi", "ttml"];
    let mut subtitle_profiles: Vec<serde_json::Value> = external
        .iter()
        .map(|f| serde_json::json!({ "Format": f, "Method": "External" }))
        .collect();
    subtitle_profiles.extend(
        ["srt", "subrip", "ass", "ssa", "vtt", "webvtt", "pgssub", "dvdsub", "dvbsub", "vobsub"]
            .iter()
            .map(|f| serde_json::json!({ "Format": f, "Method": "Embed" })),
    );
    serde_json::json!({
        "Name": "jelly-gpui (mpv)",
        "MaxStreamingBitrate": 1_000_000_000u64,
        "MaxStaticBitrate": 1_000_000_000u64,
        "DirectPlayProfiles": [
            { "Type": "Video" },
            { "Type": "Audio" },
        ],
        "TranscodingProfiles": [],
        "SubtitleProfiles": subtitle_profiles,
    })
}

impl Client {
    /// Asks the server how to play an item. Returns the media sources and a play session id
    /// to use when reporting.
    pub async fn playback_info(&self, item_id: &str, req: &PlaybackInfoRequest) -> Result<PlaybackInfoResponse> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            media_source_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            start_time_ticks: Option<Ticks>,
            #[serde(skip_serializing_if = "Option::is_none")]
            audio_stream_index: Option<i32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            subtitle_stream_index: Option<i32>,
            enable_direct_play: bool,
            enable_direct_stream: bool,
            enable_transcoding: bool,
            device_profile: serde_json::Value,
        }
        let body = Body {
            media_source_id: req.media_source_id.as_deref(),
            start_time_ticks: req.start_position,
            audio_stream_index: req.audio_stream_index,
            subtitle_stream_index: req.subtitle_stream_index,
            enable_direct_play: true,
            enable_direct_stream: true,
            enable_transcoding: false,
            device_profile: device_profile(),
        };
        self.post(&format!("/Items/{item_id}/PlaybackInfo"), &Query::new(), Some(&body))
            .await
    }

    /// The original file, served as-is. Authenticate with [`Client::authorization_header`];
    /// the URL deliberately carries no token.
    pub fn stream_url(&self, item_id: &str, media_source_id: &str, play_session_id: Option<&str>) -> Url {
        let mut url = self.url(&format!("/Videos/{item_id}/stream"));
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("static", "true").append_pair("mediaSourceId", media_source_id);
            if let Some(id) = play_session_id {
                q.append_pair("playSessionId", id);
            }
        }
        url
    }

    /// URL of an external subtitle stream (one with `is_external`), in its native text format.
    pub fn subtitle_url(&self, item_id: &str, media_source_id: &str, stream: &MediaStream) -> Url {
        let format = match stream.codec.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("ass") => "ass",
            Some("ssa") => "ssa",
            Some("vtt" | "webvtt") => "vtt",
            _ => "srt",
        };
        self.url(&format!(
            "/Videos/{item_id}/{media_source_id}/Subtitles/{}/Stream.{format}",
            stream.index
        ))
    }

    pub async fn report_playback_start(&self, report: &PlaybackReport) -> Result<()> {
        self.post_no_content("/Sessions/Playing", &Query::new(), Some(&report.progress_body()))
            .await
    }

    pub async fn report_playback_progress(&self, report: &PlaybackReport) -> Result<()> {
        self.post_no_content("/Sessions/Playing/Progress", &Query::new(), Some(&report.progress_body()))
            .await
    }

    pub async fn report_playback_stopped(&self, report: &PlaybackReport) -> Result<()> {
        let body = StopBody {
            item_id: &report.item_id,
            media_source_id: &report.media_source_id,
            play_session_id: report.play_session_id.as_deref(),
            position_ticks: report.position,
        };
        self.post_no_content("/Sessions/Playing/Stopped", &Query::new(), Some(&body))
            .await
    }
}
