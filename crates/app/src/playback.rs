//! Playback sessions: negotiate with the server, run mpv, and report progress back.
//!
//! One session at a time. Starting a new one stops the current one. The session itself runs on
//! tokio and sends its updates through a [`task::channel`] that [`Services::tick`] drains into
//! the [`NowPlaying`] state the player page draws and controls.
//!
//! [`PlaybackManager`] is the part that needs nothing else: the sessions, the queue, the volume
//! and adjustments. What also involves SyncPlay (playing, the controls, switching tracks) is on
//! [`Services`], below, and asks SyncPlay through the seam in `syncplay.rs`.

use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use jellyfin::models::{
    BaseItemDto, BaseItemKind, ChapterInfo, ItemField, MediaSegmentDto, MediaSourceInfo, MediaStream, MediaStreamType,
    TrickplayInfo,
};
use jellyfin::{Client, PlayMethod, PlaybackInfoRequest, PlaybackOrder, PlaybackReport, RepeatMode, Ticks};
use player::libmpv::{VideoImport, VideoOutput};
use player::{Cues, EndReason, LoadRequest, Player, PlayerEvent};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::config::{Config, SkipAction, SkipActions, SubtitleColor, SubtitleStyle, UnfinishedPlayback};
use crate::i18n::tr;
use crate::info::{self, Stats};
use crate::local_queue::{AfterEnd, LocalQueue};
use crate::markers::{self, Chapter, Segment, SegmentKind};
use crate::runtime;
use crate::services::{AppEvent, Services};
use crate::session::describe_error;
use crate::syncplay::GroupRequest;
use crate::task::{self, Deadline};
use crate::toasts::Toast;
use crate::tracks;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// How long after a volume or mute change its progress report goes out. A wheel spin or held
/// key changes the volume many times a second; one report covers them all.
const VOLUME_REPORT_AFTER: Duration = Duration::from_secs(1);
const DEFAULT_VOLUME: f64 = 100.;

/// The most the volume reaches, in percent: mpv's boost on the desktop (`volume-max`), 100 on
/// Android, whose players have no boost. What's reported to the server and what remote control
/// sets stay within 0 to 100 either way.
pub const fn volume_max() -> f64 {
    if cfg!(target_os = "android") {
        100.
    } else {
        player::MAX_VOLUME
    }
}

/// The seconds a seek key or skip button jumps, as Settings offers them.
pub const SEEK_STEPS: [u32; 4] = [5, 10, 15, 30];
pub const DEFAULT_SEEK_STEP: u32 = 10;
/// How long quitting the app waits for a running session to report its stop. The next launch
/// reports it instead if this runs out (`report_unfinished`).
const EXIT_WAIT: Duration = Duration::from_secs(3);
/// How long after a frame step the pause and seek it causes go unreported.
const FRAME_STEP_QUIET: Duration = Duration::from_millis(600);

/// What to play. Stream indexes are Jellyfin's `MediaStream.Index`; `None` leaves the choice
/// to mpv (and the user's mpv config).
#[derive(Debug, Clone, Default)]
pub struct PlayOptions {
    pub start: Option<Ticks>,
    pub media_source_id: Option<String>,
    pub audio_stream_index: Option<i32>,
    pub subtitle: SubtitleChoice,
    /// Load paused (SyncPlay starts playback with a scheduled command).
    pub start_paused: bool,
    /// The SyncPlay queue entry this plays, when the group controls playback.
    pub group_item: Option<String>,
    /// `start` is the item's resume point, taken without asking (next episode, autoplay), so
    /// the player offers to start over.
    pub resumed: bool,
    /// The picture choices of the playback this restarts (the same item again, in another
    /// version or with other tracks, or after a lost stream); `None` starts as a new item does.
    pub picture: Option<Picture>,
}

/// What a restart of the same item keeps of the picture choices, which belong to the item and
/// are applied to the new player again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picture {
    /// The media source (version) these choices were made on.
    pub source_id: String,
    pub aspect: Aspect,
    pub deinterlace: bool,
    /// The user turned deinterlacing on or off, rather than it being what the stream asks for.
    pub deinterlace_chosen: bool,
    /// A stream index of the second subtitle.
    pub secondary_subtitle: Option<i32>,
    pub ab_loop: AbLoop,
}

impl Picture {
    /// The choices a new session of `source` starts with. Those `carried` over from the same
    /// version stay (the second subtitle only if that track is still there and still a text
    /// track other than the first; the tracks of a transcode are the ones sent as files), and
    /// deinterlacing is worked out again unless the user set it. Another version of the item
    /// keeps only the aspect: its tracks, length and interlacing are its own. Without anything
    /// carried, the defaults of a new item.
    fn starting(
        carried: Option<&Picture>,
        source: &MediaSourceInfo,
        stream: &StreamInfo,
        subtitle: SubtitleChoice,
    ) -> Self {
        let by_default = deinterlaces_by_default(source, stream.method);
        let fresh = Self {
            source_id: source.id.clone(),
            deinterlace: by_default,
            ..Self::default()
        };
        let Some(carried) = carried else {
            return fresh;
        };
        if carried.source_id != source.id {
            return Self {
                aspect: carried.aspect,
                ..fresh
            };
        }
        let primary = match subtitle {
            SubtitleChoice::Stream(index) => Some(index),
            SubtitleChoice::Off => None,
            SubtitleChoice::Auto => source.default_subtitle_stream_index.filter(|&index| index >= 0),
        };
        let files = (stream.method == PlayMethod::Transcode).then_some(stream.subtitle_files.as_slice());
        let secondary_subtitle = carried.secondary_subtitle.filter(|&wanted| {
            tracks::secondary_choices(source, files, primary)
                .iter()
                .any(|choice| choice.value == wanted)
        });
        Self {
            deinterlace: if carried.deinterlace_chosen {
                carried.deinterlace
            } else {
                by_default
            },
            secondary_subtitle,
            ..carried.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubtitleChoice {
    #[default]
    Auto,
    Off,
    Stream(i32),
}

/// Tweaks from the player's settings menu. Speed and subtitle size carry over to the next
/// item; the delays belong to one item's files and reset when another item starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustments {
    /// 1.0 is normal.
    pub speed: f64,
    /// Seconds; positive shows subtitles later.
    pub subtitle_delay: f64,
    /// Seconds; positive plays the audio later.
    pub audio_delay: f64,
    /// 1.0 is the subtitles' own size.
    pub subtitle_scale: f64,
    /// mpv's `sub-pos`: 100 is the bottom, and less is higher up the picture.
    pub subtitle_position: u8,
    /// Night mode: compressed dynamic range. Carries to the next item; Settings has its default.
    pub night_mode: bool,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            speed: 1.,
            subtitle_delay: 0.,
            audio_delay: 0.,
            subtitle_scale: 1.,
            subtitle_position: Self::BOTTOM,
            night_mode: false,
        }
    }
}

impl Adjustments {
    pub const SPEEDS: [f64; 7] = [0.5, 0.75, 1., 1.25, 1.5, 1.75, 2.];
    pub const DELAY_STEP: f64 = 0.1;
    pub const SCALE_STEP: f64 = 0.1;
    const MAX_DELAY: f64 = 30.;
    const SCALE_RANGE: (f64, f64) = (0.5, 3.);
    /// `sub-pos` at the bottom, where subtitles start. Lower is higher up; mpv cuts text
    /// subtitles off below it (above 100), so the menu doesn't offer that.
    pub const BOTTOM: u8 = 100;
    pub const POSITION_STEP: u8 = 5;
    const POSITION_RANGE: (u8, u8) = (0, Self::BOTTOM);

    /// The next speed in [`Self::SPEEDS`] above (`up`) or below the current one.
    pub fn step_speed(self, up: bool) -> f64 {
        let (first, last) = (Self::SPEEDS[0], Self::SPEEDS[Self::SPEEDS.len() - 1]);
        if up {
            Self::SPEEDS
                .into_iter()
                .find(|&speed| speed > self.speed + 1e-6)
                .unwrap_or(last)
        } else {
            Self::SPEEDS
                .into_iter()
                .rev()
                .find(|&speed| speed < self.speed - 1e-6)
                .unwrap_or(first)
        }
    }

    /// Within the ranges the menu offers, rounded to its steps.
    fn clamped(self) -> Self {
        let tenths = |value: f64| (value * 10.).round() / 10.;
        Self {
            speed: self.speed.clamp(Self::SPEEDS[0], Self::SPEEDS[Self::SPEEDS.len() - 1]),
            subtitle_delay: tenths(self.subtitle_delay.clamp(-Self::MAX_DELAY, Self::MAX_DELAY)),
            audio_delay: tenths(self.audio_delay.clamp(-Self::MAX_DELAY, Self::MAX_DELAY)),
            subtitle_scale: tenths(self.subtitle_scale.clamp(Self::SCALE_RANGE.0, Self::SCALE_RANGE.1)),
            subtitle_position: self
                .subtitle_position
                .clamp(Self::POSITION_RANGE.0, Self::POSITION_RANGE.1),
            night_mode: self.night_mode,
        }
    }

    /// For the next item: the same speed, subtitle size and position and night mode, no delays.
    fn for_next_item(self) -> Self {
        Self {
            speed: self.speed,
            subtitle_scale: self.subtitle_scale,
            subtitle_position: self.subtitle_position,
            night_mode: self.night_mode,
            ..Self::default()
        }
    }
}

/// How the picture fills the window. Not kept between items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Aspect {
    /// The whole picture, in its own aspect, with bars where it doesn't fill the window.
    #[default]
    Fit,
    /// Fills the window, cropping what overflows.
    Fill,
    /// Fills the window, distorting the picture.
    Stretch,
    /// The picture taken as 16:9 whatever the file says.
    Widescreen,
    /// The picture taken as 4:3.
    Standard,
}

impl Aspect {
    pub const ALL: [Self; 5] = [Self::Fit, Self::Fill, Self::Stretch, Self::Widescreen, Self::Standard];

    /// The next mode, after the last the first: what the key cycles through.
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|&aspect| aspect == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The mpv properties that make this mode, all of them, so one mode's leftovers don't stay:
    /// `panscan` crops to fill, `keepaspect` off stretches, `video-aspect-override` retakes the
    /// aspect.
    pub fn mpv_properties(self) -> [(&'static str, serde_json::Value); 3] {
        use serde_json::json;
        let (panscan, keep, ratio) = match self {
            Self::Fit => (0., true, "no"),
            Self::Fill => (1., true, "no"),
            Self::Stretch => (0., false, "no"),
            Self::Widescreen => (0., true, "16:9"),
            Self::Standard => (0., true, "4:3"),
        };
        [
            ("panscan", json!(panscan)),
            ("keepaspect", json!(keep)),
            ("video-aspect-override", json!(ratio)),
        ]
    }
}

/// The A–B loop's state: the next press of its key sets the point after the one it has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AbLoop {
    #[default]
    Off,
    /// The start is set, the end isn't.
    A(Duration),
    /// Playing from the start to the end and back.
    Loop(Duration, Duration),
}

impl AbLoop {
    /// What a press at `position` makes of this: sets A, then B, then clears both, as mpv's own
    /// `ab-loop` command does.
    pub fn next(self, position: Duration) -> Self {
        match self {
            Self::Off => Self::A(position),
            Self::A(a) => Self::Loop(a, position),
            Self::Loop(..) => Self::Off,
        }
    }

    /// The two points, earliest first, for drawing: the end is `None` until it's set.
    pub fn points(self) -> Option<(Duration, Option<Duration>)> {
        match self {
            Self::Off => None,
            Self::A(a) => Some((a, None)),
            Self::Loop(a, b) => Some((a.min(b), Some(a.max(b)))),
        }
    }
}

/// Requests from the player UI to the running session.
#[derive(Debug, Clone)]
pub enum UiCommand {
    TogglePause,
    SetPause(bool),
    /// Playback speed, 1.0 is normal. SyncPlay's drift correction.
    SetSpeed(f64),
    SeekBy(f64),
    SeekTo(Duration),
    /// Percent, 0 to 100.
    SetVolume(f64),
    SetMute(bool),
    /// Seconds.
    SetSubtitleDelay(f64),
    /// Seconds.
    SetAudioDelay(f64),
    SetSubtitleScale(f64),
    /// mpv's `sub-pos`, 100 at the bottom ([`Adjustments::subtitle_position`]).
    SetSubtitlePosition(u8),
    /// Night mode on or off, applied as an audio filter (not with passthrough on).
    SetNightMode(bool),
    ToggleMute,
    /// Asks the player for the numbers behind the info overlay; the answer comes back as
    /// [`NowPlaying::player_stats`]. Only the local player has them, so it never goes to a group.
    PollInfo,
    Audio(i32),
    Subtitle(SubtitleChoice),
    /// The queue's repeat and shuffle, which the progress reports carry to the server.
    SetQueueModes(RepeatMode, PlaybackOrder),
    /// A subtitle the server downloaded while this plays: loaded into mpv and selected.
    AddSubtitle(Box<AddedSubtitle>),
    /// A second text track shown with the first (a stream index), or none.
    SetSecondarySubtitle(Option<i32>),
    SetAspect(Aspect),
    SetDeinterlace(bool),
    /// One frame forward (`true`) or back, while paused.
    FrameStep(bool),
    /// mpv's `ab-loop`: sets A, then B, then clears both.
    CycleAbLoop,
    Stop,
}

/// A subtitle added to the item while it plays (the search's download), with what the session
/// needs to number mpv's tracks again.
#[derive(Debug, Clone)]
pub struct AddedSubtitle {
    /// The new stream's index in `source`.
    pub index: i32,
    pub url: Url,
    pub title: String,
    pub language: Option<String>,
    /// The item's streams after the server's refresh.
    pub source: MediaSourceInfo,
    /// The external subtitles in the order mpv has them: the ones it had, then the new one
    /// (their indexes may have moved with the refresh).
    pub files: Vec<i32>,
}

/// Session to UI messages.
enum SessionUpdate {
    Started {
        source: Box<MediaSourceInfo>,
        video: VideoOutput,
        audio: Option<i32>,
        subtitle: SubtitleChoice,
        stream: StreamInfo,
        picture: Box<Picture>,
    },
    /// Fetched after playback starts.
    Details(Box<Details>),
    /// The player's answer to [`UiCommand::PollInfo`].
    Stats(Box<Stats>),
    Player(PlayerEvent),
    /// `run_session` returned: the last message of a session. A panic in it is raised again on
    /// the UI thread, as a [`crate::task::Task`]'s is.
    Finished(std::thread::Result<Result<Ended, String>>),
}

/// How the session's stream was set up, for switching tracks.
#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    pub method: PlayMethod,
    /// Subtitles added to mpv as files (Jellyfin stream indexes), in mpv's order after the
    /// tracks embedded in the stream.
    pub subtitle_files: Vec<i32>,
    /// A subtitle the transcoder burns into the video.
    pub burned_subtitle: Option<i32>,
    /// Why the server transcodes (`TranscodeReasons` of its URL, e.g. `VideoCodecNotSupported`).
    pub transcode_reasons: Vec<String>,
    /// The codecs the transcode produces (`VideoCodec` and `AudioCodec` of its URL).
    pub transcode_video_codec: Option<String>,
    pub transcode_audio_codec: Option<String>,
}

/// Another version (media source) of the playing item.
#[derive(Debug, Clone)]
pub struct Version {
    pub id: String,
    pub name: String,
}

#[derive(Default)]
struct Details {
    versions: Vec<Version>,
    trickplay: TrickplayTiles,
    previous: Option<BaseItemDto>,
    next: Option<BaseItemDto>,
    /// `next` is the following movie of a collection, which is offered but not autoplayed.
    next_in_collection: bool,
    chapters: Vec<Chapter>,
    segments: Vec<Segment>,
}

/// Trickplay tile sets by media source id, then thumbnail width (see [`crate::trickplay`]).
pub type TrickplayTiles = HashMap<String, HashMap<String, TrickplayInfo>>;

/// How long into playback from a resume point the player offers to start over.
const RESUME_PROMPT_FOR: Duration = Duration::from_secs(10);

/// When the up-next card shows without an Outro segment: this long before the end.
const UP_NEXT_BEFORE_END: Duration = Duration::from_secs(20);

/// Pauses playback after a while, or holds autoplay when the item ends. Not saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SleepTimer {
    /// Pauses at this time.
    In(Instant),
    /// Nothing plays after the end of the item with this id, however many sessions it takes to
    /// get there (a reconnect, another audio track on a transcode).
    EndOfItem(String),
}

/// The sleep timer's minutes choices.
pub const SLEEP_MINUTES: [u64; 4] = [15, 30, 45, 60];

/// State of the embedded player, for the player page.
pub struct NowPlaying {
    /// The session this is the state of (the manager's generation when it started), so SyncPlay
    /// can tell a new session from the one it watches.
    pub session: u64,
    pub item: BaseItemDto,
    pub source: MediaSourceInfo,
    pub video: VideoOutput,
    pub position: Duration,
    /// When `position` was reported, to extrapolate between mpv's (once a second) updates.
    pub position_at: Instant,
    /// The player has reported a position: until then `position` is the zero a session starts
    /// with, not where it resumes.
    pub position_known: bool,
    pub duration: Option<Duration>,
    pub paused: bool,
    /// Until the file has loaded.
    pub loading: bool,
    pub audio: Option<i32>,
    pub subtitle: SubtitleChoice,
    /// A second text track shown at the top of the picture with the first, if one is picked.
    /// Belongs to this item: the next starts without.
    pub secondary_subtitle: Option<i32>,
    /// libmpv plays this session; Media3 (Android) has no second subtitle, aspect modes and the
    /// like, so the player hides those.
    pub mpv: bool,
    /// How the picture fills the window. Belongs to this item.
    pub aspect: Aspect,
    /// mpv deinterlaces the picture: on from the start when the video stream is interlaced.
    pub deinterlace: bool,
    /// The user set `deinterlace`, so a restart keeps it instead of working it out again.
    pub deinterlace_chosen: bool,
    /// The loop between two points this item plays.
    pub ab_loop: AbLoop,
    pub volume: f64,
    pub muted: bool,
    pub adjustments: Adjustments,
    /// Whether night mode can act on this session: not with a format passed through.
    pub night_available: bool,
    /// All versions of the item, once known; empty or one entry means there's nothing to
    /// switch to.
    pub versions: Vec<Version>,
    /// Seek-bar preview tiles, once known; empty if the server has none.
    pub trickplay: TrickplayTiles,
    /// The item before this one: the previous episode, or the previous movie of its
    /// collection.
    pub previous: Option<BaseItemDto>,
    /// The previous episode or movie of its collection, which `previous` falls back to when the
    /// queue has nothing played before this item.
    neighbour_previous: Option<BaseItemDto>,
    /// What plays next: the queue's next item, the following episode, or the following movie
    /// of the item's collection.
    pub next: Option<BaseItemDto>,
    /// `next` is the queue's front, not the following episode or movie.
    next_queued: bool,
    /// The following episode or movie, which `next` falls back to when the queue runs dry, and
    /// whether it's the next movie of a collection, which is offered but not autoplayed.
    following: Option<BaseItemDto>,
    following_in_collection: bool,
    /// The user dismissed the up-next card: don't play the next episode at the end.
    pub next_cancelled: bool,
    /// Play the next episode at the end (the setting, at session start).
    pub autoplay: bool,
    pub chapters: Vec<Chapter>,
    /// Intro, credits and similar, once known.
    pub segments: Vec<Segment>,
    /// What to do at each kind of segment (the setting, at session start).
    pub skip_actions: SkipActions,
    pub stream: StreamInfo,
    /// What the player reported at the last poll, while the info overlay asks.
    pub player_stats: Option<Stats>,
    /// The SyncPlay queue entry, while a group controls playback.
    pub group_item: Option<String>,
    /// Where playback resumed without asking, while the player may still offer to start over.
    pub resumed_from: Option<Duration>,
    /// Counts [`PlayerEvent::Restarted`]: playback resumed after loading or seeking.
    pub restarts: u64,
    /// The subtitle cues to draw, from a player that leaves them to the UI (Media3), and how
    /// many times they've changed.
    pub cues: Cues,
    pub cues_changed: u64,
    commands: mpsc::UnboundedSender<UiCommand>,
}

impl NowPlaying {
    /// Sends a command straight to this session's player, bypassing a SyncPlay group (SyncPlay's
    /// own scheduled commands). Controls from the UI go through [`Services::control`].
    pub fn send(&self, command: UiCommand) {
        let _ = self.commands.send(command);
    }

    /// Where [`NowPlaying::send`] sends, for code that sends later from tokio: SyncPlay's
    /// scheduled commands, which mustn't wait for the UI's next frame. Sends to a session that
    /// has ended go nowhere.
    pub(crate) fn commands(&self) -> mpsc::UnboundedSender<UiCommand> {
        self.commands.clone()
    }

    /// Where playback is now: the last position mpv reported, carried on for the time since while
    /// it plays.
    pub fn position_now(&self) -> Duration {
        if self.paused || self.loading {
            return self.position;
        }
        self.position + self.position_at.elapsed().mul_f64(self.adjustments.speed.max(0.))
    }

    /// The segment the skip button would skip, if one is playing. Credits are the up-next
    /// card's while it shows.
    pub fn skippable(&self) -> Option<Segment> {
        markers::at(&self.segments, self.position, &self.skip_actions, SkipAction::Ask)
            .filter(|s| s.kind != SegmentKind::Outro || !self.show_up_next())
    }

    /// The segment to skip by itself right now, if the kind's action is Skip. Not before the
    /// player has said where playback is: a session resuming at 40:00 starts at zero, inside an
    /// intro that begins there, and skipping that would seek away from the resume point.
    pub fn auto_skippable(&self) -> Option<Segment> {
        if self.loading || !self.position_known {
            return None;
        }
        markers::at(&self.segments, self.position, &self.skip_actions, SkipAction::Skip)
    }

    /// Whether to show the up-next card: from the credits (or near the end) of an episode
    /// with a next one, unless dismissed.
    pub fn show_up_next(&self) -> bool {
        if self.next.is_none() || self.next_cancelled || self.loading {
            return false;
        }
        self.up_next_from().is_some_and(|from| self.position >= from)
    }

    /// Where the up-next card comes up: the credits' start, or a little before the end.
    pub fn up_next_from(&self) -> Option<Duration> {
        let credits = self
            .segments
            .iter()
            .find(|s| s.kind == SegmentKind::Outro)
            .map(|s| s.start);
        credits.or_else(|| self.duration.map(|d| d.saturating_sub(UP_NEXT_BEFORE_END)))
    }

    /// The resume point to offer starting over from: shown for the first [`RESUME_PROMPT_FOR`]
    /// of playback from it, and gone after a seek elsewhere.
    pub fn resume_prompt(&self) -> Option<Duration> {
        let from = self.resumed_from.filter(|_| !self.loading)?;
        let window = from.saturating_sub(Duration::from_secs(2))..from + RESUME_PROMPT_FOR;
        window.contains(&self.position).then_some(from)
    }

    /// Whether the next item starts by itself at the end: the setting allows it, and it isn't a
    /// collection's next movie (a sequel isn't started without asking; the card offers it).
    pub fn autoplays(&self) -> bool {
        self.autoplay && (self.next_queued || !self.following_in_collection)
    }

    /// What plays after this item ends by itself.
    fn autoplay_next(&self) -> Option<BaseItemDto> {
        self.next.clone().filter(|_| self.autoplays() && !self.next_cancelled)
    }

    /// What plays next now that the queue's front is `front`: that, or the following episode
    /// when the queue is empty.
    ///
    /// The item before this one is what the queue played before it, or the previous episode.
    fn follow_queue(&mut self, front: Option<&BaseItemDto>, played: Option<&BaseItemDto>) {
        self.next_queued = front.is_some();
        self.next = front.cloned().or_else(|| self.following.clone());
        self.previous = played.cloned().or_else(|| self.neighbour_previous.clone());
    }

    /// The picture choices a restart of this item keeps.
    fn picture(&self) -> Picture {
        Picture {
            source_id: self.source.id.clone(),
            aspect: self.aspect,
            deinterlace: self.deinterlace,
            deinterlace_chosen: self.deinterlace_chosen,
            secondary_subtitle: self.secondary_subtitle,
            ab_loop: self.ab_loop,
        }
    }

    /// What plays this item again as it plays now: the same version, audio and subtitle.
    fn track_options(&self) -> PlayOptions {
        PlayOptions {
            media_source_id: Some(self.source.id.clone()),
            audio_stream_index: self.audio,
            subtitle: self.subtitle,
            ..Default::default()
        }
    }

    pub fn is_transcoding(&self) -> bool {
        self.stream.method == PlayMethod::Transcode
    }

    /// The SyncPlay request a control becomes while a group controls playback; `None` for the
    /// ones that stay local (volume, tracks, delays).
    fn group_request(&self, command: &UiCommand) -> Option<GroupRequest> {
        match *command {
            UiCommand::TogglePause => Some(GroupRequest::SetPause(!self.paused)),
            UiCommand::SetPause(paused) => Some(GroupRequest::SetPause(paused)),
            UiCommand::SeekBy(seconds) => {
                let target = (self.position.as_secs_f64() + seconds).max(0.);
                Some(GroupRequest::Seek(Duration::from_secs_f64(target)))
            }
            UiCommand::SeekTo(position) => Some(GroupRequest::Seek(position)),
            UiCommand::Stop => Some(GroupRequest::Stop),
            _ => None,
        }
    }

    fn apply(&mut self, event: &PlayerEvent) {
        match event {
            PlayerEvent::FileLoaded => self.loading = false,
            PlayerEvent::Position(position) => {
                self.position = *position;
                self.position_known = true;
                self.position_at = Instant::now();
            }
            PlayerEvent::Duration(duration) => self.duration = Some(*duration),
            PlayerEvent::Paused(paused) => self.paused = *paused,
            PlayerEvent::Volume(volume) => self.volume = *volume,
            PlayerEvent::Muted(muted) => self.muted = *muted,
            PlayerEvent::Restarted => self.restarts += 1,
            PlayerEvent::Cues(cues) => {
                self.cues = cues.clone();
                self.cues_changed += 1;
            }
            _ => {}
        }
    }

    /// Takes the details fetched after the start. `queued`: the play queue has items, which take
    /// the place of the next episode; `played`: what it played right before this item, which is
    /// the previous one.
    fn apply_details(&mut self, details: Details, queued: bool, played: Option<&BaseItemDto>) {
        self.versions = details.versions;
        self.trickplay = details.trickplay;
        self.neighbour_previous = details.previous;
        self.previous = played.cloned().or_else(|| self.neighbour_previous.clone());
        self.following = details.next;
        self.following_in_collection = details.next_in_collection;
        if !queued {
            self.next = self.following.clone();
        }
        self.chapters = details.chapters;
        self.segments = details.segments;
    }
}

/// The playback part of [`Services`]: the sessions, the local play queue, and what carries over
/// from one item to the next (volume, speed, subtitle size).
pub struct PlaybackManager {
    /// Wakes the UI when a session sends something, and for a reconnect that's due.
    ctx: egui::Context,
    client: Client,
    /// The signed-in account ([`crate::config::SavedServer::credential_account`]).
    account: String,
    /// The session can report to the server. Offline it doesn't try: against a server that
    /// drops packets every report would wait out the connect timeout inside the session's
    /// loop, holding up pause and seek. A downloaded file's stop goes to `pending.json`.
    online: bool,
    /// Stops the running session when sent or dropped.
    stop: Option<oneshot::Sender<()>>,
    /// Counts sessions, so the end of a replaced session doesn't close its successor's player.
    generation: u64,
    /// A session is running (from `play` until its end is handled).
    active: bool,
    /// Sessions whose end hasn't been handled yet: the current one, and replaced ones still
    /// reporting their stop.
    sessions: Vec<Running>,
    /// A reconnect waiting to start after a lost stream. Stopping cancels it.
    reconnect: Option<Reconnect>,
    /// What plays after the current item ("Play all" on a playlist or collection). Takes the
    /// place of the next episode for the up-next card and autoplay.
    queue: LocalQueue,
    /// The item the last session was started for, which is the one before what starts next.
    started: Option<BaseItemDto>,
    /// The running session's state, once it has started. A replaced session's stays until its
    /// successor starts, so the player has something to show meanwhile.
    now_playing: Option<NowPlaying>,
    volume: f64,
    muted: bool,
    adjustments: Adjustments,
    /// Seconds per seek key press, skip button and remote fast-forward (a setting).
    seek_step: f64,
    /// The item `adjustments` were made for; another one starts without the delays.
    adjusted_item: Option<String>,
    /// What the window's renderer can draw, which picks the player's video output. Pixels until
    /// the app sets it ([`PlaybackManager::set_video_import`]).
    video_import: VideoImport,
    sleep: Option<SleepTimer>,
}

/// A session from its start until its end is handled.
struct Running {
    generation: u64,
    item: BaseItemDto,
    /// Counts reconnects after a lost stream.
    attempt: usize,
    /// What it was started with, to reconnect with.
    options: PlayOptions,
    updates: task::Receiver<SessionUpdate>,
    commands: mpsc::UnboundedSender<UiCommand>,
    /// What its [`NowPlaying`] starts with.
    volume: f64,
    muted: bool,
    adjustments: Adjustments,
    night_available: bool,
    autoplay: bool,
    skip_actions: SkipActions,
    resumed_from: Option<Duration>,
    /// Plays through libmpv (see [`NowPlaying::mpv`]).
    mpv: bool,
}

/// A session to start again after its stream was lost, once `due` fires.
struct Reconnect {
    due: Deadline,
    item: BaseItemDto,
    options: PlayOptions,
    attempt: usize,
}

impl PlaybackManager {
    /// The manager for a signed-in session.
    pub fn new(ctx: &egui::Context, client: Client, account: String) -> Self {
        let config = Config::load();
        let volume = config.volume.unwrap_or(DEFAULT_VOLUME);
        let adjustments = Adjustments {
            subtitle_scale: config.subtitle_scale.unwrap_or(1.),
            subtitle_position: config.subtitle_position.unwrap_or(Adjustments::BOTTOM),
            night_mode: config.night_mode.unwrap_or(false),
            speed: config.default_speed.unwrap_or(1.),
            ..Adjustments::default()
        }
        .clamped();
        Self {
            ctx: ctx.clone(),
            client,
            account,
            online: true,
            stop: None,
            generation: 0,
            active: false,
            sessions: Vec::new(),
            reconnect: None,
            queue: LocalQueue::default(),
            started: None,
            now_playing: None,
            volume,
            muted: false,
            adjustments,
            seek_step: Self::seek_step_of(config.seek_step),
            adjusted_item: None,
            video_import: VideoImport::Pixels,
            sleep: None,
        }
    }

    /// What the window's renderer can draw (`widgets::video::import`), for the sessions started
    /// from now on.
    /// Stops the sessions from reporting to the server, for offline services.
    pub fn set_offline(&mut self) {
        self.online = false;
    }

    pub fn set_video_import(&mut self, import: VideoImport) {
        self.video_import = import;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sets night mode for the next items (the settings page; the player's menu goes through
    /// [`Services::adjust`]).
    pub fn set_night_mode(&mut self, on: bool) {
        self.adjustments.night_mode = on;
    }

    /// Sets the subtitle size for the next items (the settings page; the player's menu goes
    /// through [`Services::adjust`]).
    pub fn set_subtitle_scale(&mut self, scale: f64) {
        self.adjustments = Adjustments {
            subtitle_scale: scale,
            ..self.adjustments
        }
        .clamped();
    }

    /// Sets the subtitle position for the next items, as [`Self::set_subtitle_scale`] sets the
    /// size.
    pub fn set_subtitle_position(&mut self, position: u8) {
        self.adjustments = Adjustments {
            subtitle_position: position,
            ..self.adjustments
        }
        .clamped();
    }

    /// Sets the speed the next items start at (the settings page).
    pub fn set_default_speed(&mut self, speed: f64) {
        self.adjustments = Adjustments {
            speed,
            ..self.adjustments
        }
        .clamped();
    }

    /// Seconds per seek key press, skip button and remote fast-forward.
    pub fn seek_step(&self) -> f64 {
        self.seek_step
    }

    /// Sets the seek step, in seconds (the settings page).
    pub fn set_seek_step(&mut self, seconds: Option<u32>) {
        self.seek_step = Self::seek_step_of(seconds);
    }

    /// What plays after the current item, in order.
    pub(crate) fn queued(&self) -> impl ExactSizeIterator<Item = &BaseItemDto> {
        self.queue.upcoming()
    }

    /// Forgets the queue (it went to the TV with a cast).
    pub(crate) fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// The signed-in session's client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The running session's state; `None` while loading, between sessions or reconnecting.
    pub fn now_playing(&self) -> Option<&NowPlaying> {
        self.now_playing.as_ref()
    }

    /// The running session's state, for the player page to change what's its own (dismissing
    /// the up-next card).
    pub fn now_playing_mut(&mut self) -> Option<&mut NowPlaying> {
        self.now_playing.as_mut()
    }

    /// What plays by itself after the session numbered `generation` ends, taking its state.
    pub(crate) fn take_next(&mut self, generation: u64) -> Option<BaseItemDto> {
        self.now_playing
            .take()
            .filter(|state| state.session == generation)
            .and_then(|state| state.autoplay_next())
    }

    /// Whether the sleep timer holds what comes after the end of the item `item_id`: nothing
    /// plays after it, neither the queue, nor autoplay, nor a repeat. Takes the timer, which
    /// this spends.
    fn hold_after_end(&mut self, item_id: &str) -> bool {
        let held = matches!(&self.sleep, Some(SleepTimer::EndOfItem(id)) if id == item_id);
        if held {
            self.sleep = None;
        }
        held
    }

    pub fn sleep(&self) -> Option<&SleepTimer> {
        self.sleep.as_ref()
    }

    /// Sets the sleep timer, or clears it with `None`.
    pub fn set_sleep(&mut self, sleep: Option<SleepTimer>) {
        self.sleep = sleep;
        self.ctx.request_repaint();
    }

    /// Time left on a timed sleep timer.
    pub fn sleep_left(&self) -> Option<Duration> {
        match self.sleep {
            Some(SleepTimer::In(due)) => Some(due.saturating_duration_since(Instant::now())),
            _ => None,
        }
    }

    /// A session as if `item` had started playing `source`, with no player behind it, for the
    /// player page's tests: 12 minutes into 48, playing. Returns what the page sends it.
    #[cfg(test)]
    pub fn start_for_test(&mut self, item: BaseItemDto, source: MediaSourceInfo) -> mpsc::UnboundedReceiver<UiCommand> {
        let (commands, received) = mpsc::unbounded_channel();
        self.generation += 1;
        self.active = true;
        self.started = Some(item.clone());
        self.now_playing = Some(NowPlaying {
            session: self.generation,
            item,
            source,
            video: VideoOutput::external(),
            position: Duration::from_secs(12 * 60),
            position_at: Instant::now(),
            position_known: true,
            duration: Some(Duration::from_secs(48 * 60)),
            paused: false,
            loading: false,
            audio: None,
            subtitle: SubtitleChoice::Auto,
            secondary_subtitle: None,
            mpv: true,
            aspect: Aspect::Fit,
            deinterlace: false,
            deinterlace_chosen: false,
            ab_loop: AbLoop::Off,
            volume: self.volume,
            muted: false,
            adjustments: self.adjustments,
            night_available: true,
            versions: Vec::new(),
            trickplay: TrickplayTiles::new(),
            previous: None,
            neighbour_previous: None,
            next: None,
            next_queued: false,
            following: None,
            following_in_collection: false,
            next_cancelled: false,
            autoplay: false,
            chapters: Vec::new(),
            segments: Vec::new(),
            skip_actions: SkipActions::default(),
            stream: StreamInfo::default(),
            player_stats: None,
            group_item: None,
            resumed_from: None,
            restarts: 0,
            cues: Cues::default(),
            cues_changed: 0,
            commands,
        });
        received
    }

    /// The state of the current session, not of one being replaced: commands sent to it reach
    /// a session that's still going.
    fn current(&self) -> Option<&NowPlaying> {
        self.now_playing
            .as_ref()
            .filter(|state| state.session == self.generation)
    }

    /// The state of the session numbered `generation`, if it's the one held.
    fn state_of(&mut self, generation: u64) -> Option<&mut NowPlaying> {
        self.now_playing.as_mut().filter(|state| state.session == generation)
    }

    /// Stops the current session, if any. Returns whether that cancelled a pending reconnect,
    /// which closes the player (a running session closes it when it ends).
    fn stop(&mut self) -> bool {
        self.stop = None;
        self.reconnect.take().is_some()
    }

    /// Takes the queue's front when `item` is it, moving the queue on; playing the same item
    /// again (another version, a restart) leaves it be, and playing anything else drops it,
    /// unless a SyncPlay group plays it.
    fn advance_queue(&mut self, item: &BaseItemDto, options: &PlayOptions) {
        if self.queue.advance(item, self.started.as_ref()) {
            return;
        }
        let again = self.started.as_ref().is_some_and(|started| started.id == item.id);
        if !again && options.group_item.is_none() {
            self.queue.clear();
        }
    }

    /// Repeat, and whether the queue is shuffled.
    pub(crate) fn modes(&self) -> (RepeatMode, bool) {
        (self.queue.repeat, self.queue.is_shuffled())
    }

    /// Shows the playing item's `next` as the queue's front, or the following episode once the
    /// queue is empty.
    fn refresh_next(&mut self) {
        let (front, played) = (self.queue.front().cloned(), self.queue.last_played().cloned());
        if let Some(state) = &mut self.now_playing {
            state.follow_queue(front.as_ref(), played.as_ref());
        }
    }

    /// Starts a session; `attempt` counts reconnects after a lost stream.
    fn start_session(&mut self, item: &BaseItemDto, options: PlayOptions, attempt: usize) {
        self.reconnect = None;
        // An end-of-item timer belongs to the item it was set on: a restart of that item keeps
        // it, another item takes it away.
        if matches!(&self.sleep, Some(SleepTimer::EndOfItem(id)) if *id != item.id) {
            self.sleep = None;
        }
        self.started = Some(item.clone());
        // Dropping the previous sender stops that session; it still reports its final position.
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);
        self.generation += 1;
        self.active = true;
        let (volume, muted, import, online) = (self.volume, self.muted, self.video_import, self.online);
        let modes = self.queue.modes();
        if self.adjusted_item.as_ref() != Some(&item.id) {
            self.adjustments = self.adjustments.for_next_item();
            self.adjusted_item = Some(item.id.clone());
        }
        let mut adjustments = self.adjustments;
        if options.group_item.is_some() {
            adjustments.speed = 1.;
        }
        let config = Config::load();
        let autoplay = config.autoplay_next_episode.unwrap_or(true);
        let skip_actions = config.skip_actions;
        let mpv =
            !cfg!(target_os = "android") || config.android_player.unwrap_or_default().engine() == player::Engine::Mpv;
        let resumed_from = options.start.filter(|_| options.resumed).map(Duration::from);
        let (updates_tx, updates) = task::channel(&self.ctx);
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();

        {
            let (client, item, account) = (self.client.clone(), item.clone(), self.account.clone());
            let options = options.clone();
            // Not a `Task`: a session outlives the services that started it (signing out stops
            // it), so it can still report its stop.
            runtime::detach(async move {
                let channels = Channels {
                    stop: stop_rx,
                    updates: updates_tx.clone(),
                    commands: commands_rx,
                };
                let session = run_session(
                    client,
                    item,
                    options,
                    (volume, muted),
                    adjustments,
                    (account, online),
                    import,
                    modes,
                    channels,
                );
                let result = AssertUnwindSafe(session).catch_unwind().await;
                let _ = updates_tx.send(SessionUpdate::Finished(result));
            });
        }

        self.sessions.push(Running {
            generation: self.generation,
            item: item.clone(),
            attempt,
            options,
            updates,
            commands: commands_tx,
            volume,
            muted,
            adjustments,
            night_available: !Config::load().audio_settings().passthrough.any(),
            autoplay,
            skip_actions,
            resumed_from,
            mpv,
        });
    }

    /// Everything the sessions sent since the last call, each with its session's generation.
    fn drain(&mut self) -> Vec<(u64, SessionUpdate)> {
        let mut updates = Vec::new();
        for session in &mut self.sessions {
            updates.extend(session.updates.drain().map(|update| (session.generation, update)));
        }
        updates
    }

    /// The reconnect to start now, once its delay is over.
    fn reconnect_due(&mut self) -> Option<Reconnect> {
        let reconnect = self.reconnect.as_mut()?;
        if reconnect.due.fire(&self.ctx) {
            self.reconnect.take()
        } else {
            None
        }
    }

    /// The configured seek step in seconds: one of [`SEEK_STEPS`], else the default.
    fn seek_step_of(configured: Option<u32>) -> f64 {
        f64::from(
            configured
                .filter(|seconds| SEEK_STEPS.contains(seconds))
                .unwrap_or(DEFAULT_SEEK_STEP),
        )
    }

    /// Remembers the volume and subtitle size for the next launch.
    fn save_preferences(&self) {
        save_preferences(self.volume, self.adjustments);
    }

    /// Stops every session and lets them finish on tokio, reporting their stop, then saves the
    /// volume. Signals on the returned channel when they're all done.
    fn finish(&mut self) -> std::sync::mpsc::Receiver<()> {
        self.stop = None;
        self.reconnect = None;
        let (done_tx, done) = std::sync::mpsc::channel();
        let sessions = std::mem::take(&mut self.sessions);
        if sessions.is_empty() {
            let _ = done_tx.send(());
            return done;
        }
        let (current, mut volume, adjustments) = (self.generation, self.volume, self.adjustments);
        runtime::detach(async move {
            for mut session in sessions {
                while let Some(update) = session.updates.recv().await {
                    match update {
                        SessionUpdate::Player(PlayerEvent::Volume(level)) if session.generation == current => {
                            volume = level;
                        }
                        SessionUpdate::Finished(_) => break,
                        _ => {}
                    }
                }
            }
            save_preferences(volume, adjustments);
            let _ = done_tx.send(());
        });
        done
    }

    /// Quitting the app: stops playback, letting the sessions report it in the background.
    /// Returns what [`wait_for_exit`] waits on, if a session was running.
    pub fn stop_for_exit(&mut self) -> Option<std::sync::mpsc::Receiver<()>> {
        let running = !self.sessions.is_empty();
        let done = self.finish();
        running.then_some(done)
    }
}

/// Waits (a little, [`EXIT_WAIT`] in all) for the sessions [`PlaybackManager::stop_for_exit`]
/// stopped to report it, since the process ends right after.
pub fn wait_for_exit(stopping: Vec<std::sync::mpsc::Receiver<()>>) {
    if stopping.is_empty() {
        return;
    }
    tracing::info!("stopping playback before quitting");
    let deadline = Instant::now() + EXIT_WAIT;
    for done in stopping {
        if done
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_err()
        {
            tracing::warn!("playback didn't stop in time; the next launch reports it");
            return;
        }
    }
}

/// Dropped with the signed-in session (after `Services::end` on sign-out, an expired session or
/// switching accounts): the running sessions stop, if `end` didn't already, and report it in
/// the background.
impl Drop for PlaybackManager {
    fn drop(&mut self) {
        drop(self.finish());
    }
}

/// Remembers the volume and subtitle size for the next launch.
fn save_preferences(volume: f64, adjustments: Adjustments) {
    let mut config = Config::load();
    let scale = Some(adjustments.subtitle_scale).filter(|&scale| scale != 1.);
    let position = Some(adjustments.subtitle_position).filter(|&position| position != Adjustments::BOTTOM);
    if config.volume != Some(volume) || config.subtitle_scale != scale || config.subtitle_position != position {
        config.volume = Some(volume);
        config.subtitle_scale = scale;
        config.subtitle_position = position;
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
    }
}

/// Playing, and the controls: these may go to a SyncPlay group or a cast instead of the local
/// player.
impl Services {
    /// A control from the player UI (or remote control). In a SyncPlay group, pause, seek and
    /// stop are requests to the group, which then commands every member; while casting they go
    /// to the TV. Stop with no session to send it to (waiting to reconnect, or the next one
    /// still loading) stops that instead.
    pub fn control(&mut self, command: UiCommand) {
        if self.is_casting() {
            self.cast_control(command);
            return;
        }
        let Some(state) = self.playback.current() else {
            if matches!(command, UiCommand::Stop) {
                self.stop_playback();
            }
            return;
        };
        if state.group_item.is_some()
            && let Some(request) = state.group_request(&command)
        {
            self.group_request(request);
            return;
        }
        state.send(command);
    }

    /// Stops the current session, if any, or a pending reconnect (closing the player).
    pub fn stop_playback(&mut self) {
        if self.playback.stop() {
            self.events.push(AppEvent::PlaybackEnded);
        }
    }

    pub fn play(&mut self, item: &BaseItemDto, options: PlayOptions) {
        if self.is_casting() {
            self.cast_play(std::slice::from_ref(item), &options);
            return;
        }
        // Playing the queue's next item moves the queue on; playing anything else drops it.
        self.playback.advance_queue(item, &options);
        self.play_now(item, options);
    }

    /// Plays an item from its resume point, with default tracks (e.g. the next episode).
    pub fn play_from_resume(&mut self, item: &BaseItemDto) {
        self.play(item, resume_options(item));
    }

    /// Plays `items` in order: the first now, the rest as a queue.
    pub fn play_all(&mut self, items: Vec<BaseItemDto>) {
        self.play_all_with(items, PlayOptions::default());
    }

    /// Plays the first of `items` from its resume point and the rest after it, as *Play from
    /// here* on an episode of a season or an entry of a playlist.
    pub fn play_from_here(&mut self, items: Vec<BaseItemDto>) {
        let Some(first) = items.first() else { return };
        let options = resume_options(first);
        self.play_all_with(items, options);
    }

    /// Plays `items` in order, the first with `options`. A SyncPlay group gets them as its
    /// new queue instead, from the start; a cast plays them on the TV.
    pub fn play_all_with(&mut self, items: Vec<BaseItemDto>, options: PlayOptions) {
        if self.is_casting() {
            self.cast_play(&items, &options);
            return;
        }
        let mut items = VecDeque::from(items);
        let Some(first) = items.pop_front() else { return };
        if self.in_group() {
            let mut all = vec![first];
            all.extend(items);
            self.group_start_items(&all);
            return;
        }
        self.playback.queue.start(items);
        self.play_now(&first, options);
    }

    /// Adds `items` to the queue: right after the playing item with `next`, otherwise at the
    /// end. With nothing playing they play now.
    pub fn enqueue(&mut self, items: Vec<BaseItemDto>, next: bool) {
        if self.is_casting() {
            self.cast_enqueue(&items, next);
            return;
        }
        if self.in_group() {
            self.group_enqueue(&items, next);
            return;
        }
        let playback = &mut self.playback;
        // A reconnect that waits is the playback going on: its queue is the one to add to.
        if !playback.active && playback.reconnect.is_none() {
            self.play_all(items);
            return;
        }
        if next {
            playback.queue.push_next(items);
        } else {
            playback.queue.push_last(items);
        }
        // The queue takes the place of the next episode for the up-next card and autoplay, and
        // asking for something to play next undoes dismissing the card.
        playback.refresh_next();
        if let Some(state) = &mut playback.now_playing {
            state.next_cancelled = false;
        }
    }

    /// Sets what the queue does at the end of an item (the panel's repeat button, or remote
    /// control). In a SyncPlay group it's the group's, set on the server; while casting it's the
    /// TV's, which has no such control.
    pub fn set_repeat(&mut self, mode: RepeatMode) {
        if self.is_casting() {
            return;
        }
        if self.in_group() {
            self.syncplay.set_repeat(mode);
            return;
        }
        self.playback.queue.repeat = mode;
        self.queue_modes_changed();
    }

    /// Off, all, one.
    pub fn cycle_repeat(&mut self) {
        let next = if self.in_group() {
            self.syncplay.repeat_mode().next()
        } else {
            self.playback.queue.repeat.next()
        };
        self.set_repeat(next);
    }

    /// Shuffles what's queued, or puts it back in order (the panel's shuffle button, or remote
    /// control). A group's and a cast's, as [`Services::set_repeat`].
    pub fn set_shuffle(&mut self, on: bool) {
        if self.is_casting() {
            return;
        }
        if self.in_group() {
            self.syncplay.set_shuffle(on);
            return;
        }
        self.playback.queue.set_shuffle(on);
        self.queue_modes_changed();
    }

    pub fn toggle_shuffle(&mut self) {
        let on = !self.playback.queue.is_shuffled();
        self.set_shuffle(on);
    }

    /// The modes changed: the session reports them, and the up-next card follows the new order.
    fn queue_modes_changed(&mut self) {
        let (repeat, order) = self.playback.queue.modes();
        if let Some(state) = self.playback.current() {
            state.send(UiCommand::SetQueueModes(repeat, order));
        }
        self.playback.refresh_next();
    }

    /// Takes the queue's entry at `index` out, from the player's queue panel.
    pub fn queue_remove(&mut self, index: usize) {
        if self.playback.queue.remove(index).is_some() {
            self.playback.refresh_next();
        }
    }

    /// Moves the queue's entry at `index` up (`-1`) or down (`1`). Returns where it went.
    pub fn queue_move(&mut self, index: usize, by: isize) -> Option<usize> {
        let to = self.playback.queue.move_entry(index, by)?;
        self.playback.refresh_next();
        Some(to)
    }

    /// The running session playing to its end, as the session reports it, for tests of what
    /// follows. Returns the item a new session was started for, if one was.
    #[cfg(test)]
    pub fn end_for_test(&mut self) -> Option<String> {
        self.end_with_for_test(PlayOptions::default())
    }

    /// As [`Services::end_for_test`], for a session that was started with `options`.
    #[cfg(test)]
    pub fn end_with_for_test(&mut self, options: PlayOptions) -> Option<String> {
        self.end_session_for_test(Ok(Ended::Finished), options)
    }

    #[cfg(test)]
    fn end_session_for_test(&mut self, result: Result<Ended, String>, options: PlayOptions) -> Option<String> {
        let state = self.playback.now_playing()?;
        let generation = self.playback.generation;
        let (commands, _) = mpsc::unbounded_channel();
        let (_, updates) = task::channel(&self.playback.ctx);
        let running = Running {
            generation,
            item: state.item.clone(),
            attempt: 0,
            options,
            updates,
            commands,
            volume: self.playback.volume,
            muted: false,
            adjustments: Adjustments::default(),
            night_available: true,
            mpv: true,
            autoplay: true,
            skip_actions: SkipActions::default(),
            resumed_from: None,
        };
        self.session_ended(running, result);
        (self.playback.generation != generation)
            .then(|| self.playback.started.as_ref().map(|item| item.id.clone()))
            .flatten()
    }

    /// The running session's stream dropping, which waits to reconnect.
    #[cfg(test)]
    pub fn lose_for_test(&mut self) {
        let lost = Ended::Lost {
            position: Ticks::from(Duration::from_secs(60)),
            message: "the test dropped it".into(),
            crashed: false,
        };
        self.end_session_for_test(Ok(lost), PlayOptions::default());
    }

    /// An event from the running session's player, as the session forwards it.
    #[cfg(test)]
    pub fn player_event_for_test(&mut self, event: PlayerEvent) {
        let generation = self.playback.generation;
        self.on_session_update(generation, SessionUpdate::Player(event));
    }

    /// A reconnect is waiting to start.
    #[cfg(test)]
    pub fn reconnect_pending_for_test(&self) -> bool {
        self.playback.reconnect.is_some()
    }

    /// What the waiting reconnect starts with.
    #[cfg(test)]
    pub fn reconnect_options_for_test(&self) -> Option<PlayOptions> {
        self.playback
            .reconnect
            .as_ref()
            .map(|reconnect| reconnect.options.clone())
    }

    /// The item the last session was started for, and what it was started with.
    #[cfg(test)]
    pub fn last_started_for_test(&self) -> (Option<String>, Option<PlayOptions>) {
        (
            self.playback.started.as_ref().map(|item| item.id.clone()),
            self.playback.sessions.last().map(|session| session.options.clone()),
        )
    }

    /// Plays the queue's entry at `index` now. What's before it counts as played.
    pub fn queue_jump(&mut self, index: usize) {
        if self.is_casting() || self.in_group() {
            return;
        }
        let previous = self.playback.started.clone();
        let Some(item) = self.playback.queue.jump(index, previous.as_ref()) else {
            return;
        };
        self.play_now(&item, resume_options(&item));
    }

    fn play_now(&mut self, item: &BaseItemDto, options: PlayOptions) {
        if self.is_casting() {
            self.cast_play(std::slice::from_ref(item), &options);
            return;
        }
        // In a SyncPlay group, starting something means giving the group a new queue; the
        // server then has every member (this one included) load it.
        if options.group_item.is_none() && self.in_group() {
            let start = options.start.unwrap_or_default();
            self.group_start_queue(item, start);
            return;
        }
        self.playback.start_session(item, options, 0);
    }

    /// Plays the next episode (or queued item, or movie of the collection) now.
    pub fn play_next(&mut self) {
        if self.is_casting() {
            self.cast_skip(true);
            return;
        }
        if let Some(next) = self.playback.now_playing().and_then(|state| state.next.clone()) {
            self.play_from_resume(&next);
        }
    }

    /// Plays the previous item from its resume point: what the queue played before this one,
    /// keeping the queue (this item is next again), or else the previous episode (or movie of
    /// the collection).
    pub fn play_previous(&mut self) {
        if self.is_casting() {
            self.cast_skip(false);
            return;
        }
        if !self.in_group()
            && let Some(current) = self.playback.started.clone()
            && let Some(previous) = self.playback.queue.step_back(&current)
        {
            self.playback.refresh_next();
            self.play_now(&previous, resume_options(&previous));
            return;
        }
        if let Some(previous) = self.playback.now_playing().and_then(|state| state.previous.clone()) {
            self.play_from_resume(&previous);
        }
    }

    /// Plays from the beginning instead of the resume point.
    pub fn start_over(&mut self) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        state.resumed_from = None;
        self.control(UiCommand::SeekTo(Duration::ZERO));
    }

    /// Switches the audio track. A transcode carries only the chosen track, so it restarts
    /// at the current position.
    pub fn change_audio(&mut self, index: i32) {
        if self.is_casting() {
            self.cast_control(UiCommand::Audio(index));
            return;
        }
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if state.audio == Some(index) {
            return;
        }
        state.audio = Some(index);
        if state.is_transcoding() {
            self.restart();
        } else {
            state.send(UiCommand::Audio(index));
        }
    }

    /// Switches subtitles. A transcode restarts if a burned-in subtitle has to go, or the new
    /// one has to be burned in.
    pub fn change_subtitle(&mut self, choice: SubtitleChoice) {
        if self.is_casting() {
            self.cast_control(UiCommand::Subtitle(choice));
            return;
        }
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if state.subtitle == choice {
            return;
        }
        state.subtitle = choice;
        // The same track can't be both.
        if let SubtitleChoice::Stream(index) = choice
            && state.secondary_subtitle == Some(index)
        {
            state.secondary_subtitle = None;
            state.send(UiCommand::SetSecondarySubtitle(None));
        }
        let as_file = match choice {
            SubtitleChoice::Stream(index) => state.stream.subtitle_files.contains(&index),
            SubtitleChoice::Off | SubtitleChoice::Auto => true,
        };
        if state.is_transcoding() && (state.stream.burned_subtitle.is_some() || !as_file) {
            self.restart();
        } else {
            state.send(UiCommand::Subtitle(choice));
        }
    }

    /// Takes in a subtitle the server downloaded for `item_id`, if that still plays: `sources`
    /// are the item's versions as the server lists them now. Direct play loads it into mpv
    /// and selects it with no restart; a transcode restarts with it chosen, as any switch to a
    /// subtitle the stream doesn't carry does. Returns whether it was taken in.
    pub fn add_downloaded_subtitle(&mut self, item_id: &str, sources: &[MediaSourceInfo]) -> bool {
        if self.is_casting() {
            return false;
        }
        let client = self.session.client.clone();
        let Some(state) = self
            .playback
            .now_playing
            .as_mut()
            .filter(|state| state.item.id == item_id)
        else {
            return false;
        };
        let Some(new) = sources.iter().find(|source| source.id == state.source.id) else {
            return false;
        };
        let Some((added, files)) = added_subtitle(&state.source, &state.stream.subtitle_files, new) else {
            return false;
        };
        let index = added.index;
        if state.is_transcoding() {
            state.source = new.clone();
            state.subtitle = SubtitleChoice::Stream(index);
            self.restart();
            return true;
        }
        let command = UiCommand::AddSubtitle(Box::new(AddedSubtitle {
            index,
            url: client.subtitle_url(item_id, &new.id, added),
            title: added.display_title.clone().unwrap_or_else(|| "External".into()),
            language: added.language.clone(),
            source: new.clone(),
            files: files.clone(),
        }));
        state.source = new.clone();
        state.stream.subtitle_files = files;
        state.subtitle = SubtitleChoice::Stream(index);
        state.send(command);
        true
    }

    /// Shows a second text track (a stream index) with the first, or none. mpv draws it at the
    /// top; a transcode only has the tracks the server sends as files.
    pub fn change_secondary_subtitle(&mut self, index: Option<i32>) {
        if self.is_casting() {
            return;
        }
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if !state.mpv || state.secondary_subtitle == index {
            return;
        }
        state.secondary_subtitle = index;
        state.send(UiCommand::SetSecondarySubtitle(index));
    }

    /// Sets how the picture fills the window (the player's menu and key). mpv only, and not on
    /// a TV.
    pub fn set_aspect(&mut self, aspect: Aspect) {
        if self.is_casting() {
            return;
        }
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if !state.mpv || state.aspect == aspect {
            return;
        }
        state.aspect = aspect;
        state.send(UiCommand::SetAspect(aspect));
    }

    /// Steps one frame forward or back, while paused (the keys `.` and `,`). Not in a SyncPlay
    /// group, where the position is the group's, nor on a TV or without mpv.
    pub fn frame_step(&mut self, forward: bool) {
        if self.is_casting() {
            return;
        }
        let Some(state) = self.playback.current() else {
            return;
        };
        if state.mpv && state.paused && state.group_item.is_none() {
            state.send(UiCommand::FrameStep(forward));
        }
    }

    /// Sets the loop's start, then its end, then clears it (the key `L`). The same limits as
    /// [`Services::frame_step`]: a loop jumps back on its own, which the group wouldn't follow.
    pub fn cycle_ab_loop(&mut self) {
        if self.is_casting() {
            return;
        }
        let generation = self.playback.generation;
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if !state.mpv || state.group_item.is_some() || state.session != generation {
            return;
        }
        state.ab_loop = state.ab_loop.next(state.position_now());
        state.send(UiCommand::CycleAbLoop);
    }

    /// Turns mpv's deinterlacing on or off (the player's menu). mpv only, and not on a TV.
    pub fn set_deinterlace(&mut self, on: bool) {
        if self.is_casting() {
            return;
        }
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        if !state.mpv || state.deinterlace == on {
            return;
        }
        state.deinterlace = on;
        state.deinterlace_chosen = true;
        state.send(UiCommand::SetDeinterlace(on));
    }

    /// Applies changes from the settings menu. Speed stays at normal in a SyncPlay group,
    /// whose drift correction uses it.
    pub fn adjust(&mut self, adjustments: Adjustments) {
        let Some(state) = self.playback.now_playing.as_mut() else {
            return;
        };
        let mut new = adjustments.clamped();
        if state.group_item.is_some() {
            new.speed = 1.;
        }
        let old = state.adjustments;
        if new == old {
            return;
        }
        state.adjustments = new;
        let changes = [
            (new.speed != old.speed).then_some(UiCommand::SetSpeed(new.speed)),
            (new.subtitle_delay != old.subtitle_delay).then_some(UiCommand::SetSubtitleDelay(new.subtitle_delay)),
            (new.audio_delay != old.audio_delay).then_some(UiCommand::SetAudioDelay(new.audio_delay)),
            (new.subtitle_scale != old.subtitle_scale).then_some(UiCommand::SetSubtitleScale(new.subtitle_scale)),
            (new.subtitle_position != old.subtitle_position)
                .then_some(UiCommand::SetSubtitlePosition(new.subtitle_position)),
            (new.night_mode != old.night_mode).then_some(UiCommand::SetNightMode(new.night_mode)),
        ];
        for command in changes.into_iter().flatten() {
            state.send(command);
        }
        self.playback.adjustments = new;
    }

    /// The window couldn't import the player's GPU frames (`Video::import_failed`): plays on
    /// from here with software frames, which always show, and keeps to them for the rest of the
    /// sign-in, since the import would fail again.
    pub fn fall_back_to_pixels(&mut self) {
        if self.playback.video_import == VideoImport::Pixels {
            return;
        }
        tracing::warn!("video: switching to software frames");
        self.playback.set_video_import(VideoImport::Pixels);
        self.restart();
    }

    /// Plays the same version again from the current position with the current tracks.
    fn restart(&mut self) {
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        let options = PlayOptions {
            start: Some(Ticks::from(state.position)),
            picture: Some(state.picture()),
            ..state.track_options()
        };
        self.reload(options);
    }

    /// Loads the playing item again here only, e.g. in another version: at the current
    /// position, or `options.start`. In a SyncPlay group the others wait meanwhile
    /// (`SyncPlay::reload`), rather than the group getting a new queue as when something new is
    /// played.
    pub fn reload(&mut self, mut options: PlayOptions) {
        if self.is_casting() {
            self.cast_reload(options);
            return;
        }
        let Some(state) = self.playback.now_playing() else {
            return;
        };
        options.start = options.start.or(Some(Ticks::from(state.position)));
        if options.picture.is_none() {
            options.picture = Some(state.picture());
        }
        let item = state.item.clone();
        if state.group_item.is_some() {
            self.group_reload(&item, options);
        } else {
            self.play(&item, options);
        }
    }

    /// Takes in what the sessions sent, and starts a reconnect that's due. From
    /// [`Services::tick`], so it goes on while the window is hidden.
    pub(crate) fn tick_playback(&mut self) {
        for (generation, update) in self.playback.drain() {
            self.on_session_update(generation, update);
        }
        self.tick_sleep();
        if let Some(reconnect) = self.playback.reconnect_due() {
            let Reconnect {
                item, options, attempt, ..
            } = reconnect;
            self.playback.start_session(&item, options, attempt);
        }
    }

    /// Whether the player offers a sleep timer: not in a SyncPlay group, where the pause it
    /// makes would be every member's, and the end of an item is the server's to follow.
    pub fn sleep_timer_offered(&self) -> bool {
        !self.in_group()
            && !self
                .playback
                .now_playing()
                .is_some_and(|state| state.group_item.is_some())
    }

    /// Fires the sleep timer when it's due: pauses (so the position is reported and kept) and holds
    /// autoplay of what comes after this item. A timer with nothing playing or on its way is
    /// dropped, and so is one in a SyncPlay group, where a pause is every member's. One that
    /// comes due between sessions (a reconnect waits, the next item loads) waits for the next.
    fn tick_sleep(&mut self) {
        if self.playback.sleep.is_none() {
            return;
        }
        if !self.sleep_timer_offered()
            || (!self.playback.active && self.playback.reconnect.is_none() && self.playback.now_playing.is_none())
        {
            self.playback.sleep = None;
            return;
        }
        let Some(SleepTimer::In(due)) = self.playback.sleep else {
            return;
        };
        let now = Instant::now();
        if now < due {
            self.playback.ctx.request_repaint_after(due - now);
            return;
        }
        // The pause goes to a live session, or to the TV; with neither it would be dropped.
        if !self.is_casting() && self.playback.current().is_none() {
            return;
        }
        self.playback.sleep = None;
        if let Some(state) = self.playback.now_playing_mut() {
            state.autoplay = false;
        }
        self.control(UiCommand::SetPause(true));
        self.toast(Toast::info(tr!("player-sleep-paused")));
    }

    fn on_session_update(&mut self, generation: u64, update: SessionUpdate) {
        let playback = &mut self.playback;
        match update {
            SessionUpdate::Started {
                source,
                video,
                audio,
                subtitle,
                stream,
                picture,
            } => {
                // A replaced session that got this far is stopping; its player never shows.
                if generation != playback.generation {
                    return;
                }
                let Some(running) = playback.sessions.iter().find(|s| s.generation == generation) else {
                    return;
                };
                playback.now_playing = Some(NowPlaying {
                    session: generation,
                    item: running.item.clone(),
                    source: *source,
                    video,
                    position: Duration::ZERO,
                    position_at: Instant::now(),
                    position_known: false,
                    duration: None,
                    paused: false,
                    loading: true,
                    audio,
                    subtitle,
                    secondary_subtitle: picture.secondary_subtitle,
                    mpv: running.mpv,
                    aspect: picture.aspect,
                    deinterlace: picture.deinterlace,
                    deinterlace_chosen: picture.deinterlace_chosen,
                    ab_loop: picture.ab_loop,
                    volume: running.volume,
                    muted: running.muted,
                    adjustments: running.adjustments,
                    night_available: running.night_available,
                    versions: Vec::new(),
                    trickplay: TrickplayTiles::new(),
                    previous: playback.queue.last_played().cloned(),
                    neighbour_previous: None,
                    // Read when the session starts, not when it was asked for: a remote play
                    // next or last queued while it was loading counts too.
                    next: playback.queue.front().cloned(),
                    next_queued: !playback.queue.is_empty(),
                    following: None,
                    following_in_collection: false,
                    next_cancelled: false,
                    autoplay: running.autoplay,
                    chapters: Vec::new(),
                    segments: Vec::new(),
                    skip_actions: running.skip_actions,
                    stream,
                    player_stats: None,
                    group_item: running.options.group_item.clone(),
                    resumed_from: running.resumed_from,
                    restarts: 0,
                    cues: Cues::default(),
                    cues_changed: 0,
                    commands: running.commands.clone(),
                });
                self.events.push(AppEvent::PlaybackStarted);
                self.group_playback_started();
            }
            SessionUpdate::Details(details) => {
                // Items queued since the start (remote control) count too.
                let queued = !playback.queue.is_empty();
                let played = playback.queue.last_played().cloned();
                if let Some(state) = playback.state_of(generation) {
                    state.apply_details(*details, queued, played.as_ref());
                }
            }
            SessionUpdate::Stats(stats) => {
                if let Some(state) = playback.state_of(generation) {
                    state.player_stats = Some(*stats);
                }
            }
            SessionUpdate::Player(event) => {
                if generation == playback.generation {
                    match event {
                        PlayerEvent::Volume(volume) => playback.volume = volume,
                        PlayerEvent::Muted(muted) => playback.muted = muted,
                        _ => {}
                    }
                }
                if let Some(state) = playback.state_of(generation) {
                    state.apply(&event);
                }
            }
            SessionUpdate::Finished(result) => {
                let result = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                let Some(index) = playback.sessions.iter().position(|s| s.generation == generation) else {
                    return;
                };
                let running = playback.sessions.remove(index);
                self.session_ended(running, result);
            }
        }
    }

    fn session_ended(&mut self, running: Running, result: Result<Ended, String>) {
        self.playback.save_preferences();
        // A downloaded file's playback writes its new watch state to the item's `item.json` on
        // its way out; the Downloads page's own copy follows.
        self.downloads.refresh_item(&running.item.id);
        // Watched state and resume points may have changed.
        self.user_data += 1;
        // A newer session took over (another item, version or episode); its player is
        // already showing.
        if running.generation != self.playback.generation {
            return;
        }
        let playback = &mut self.playback;
        playback.active = false;
        // The sleep timer's end of the item: nothing plays after it, whatever the queue and the
        // repeat mode would do. The queue stays as it was.
        let held = matches!(result, Ok(Ended::Finished)) && playback.hold_after_end(&running.item.id);
        // What a reconnect keeps of the picture choices.
        let picture = playback.current().map(NowPlaying::picture);
        // Repeat one plays it again as it played, from its start.
        let replay = playback
            .current()
            .map(NowPlaying::track_options)
            .unwrap_or_else(|| PlayOptions {
                media_source_id: running.options.media_source_id.clone(),
                audio_stream_index: running.options.audio_stream_index,
                subtitle: running.options.subtitle,
                ..Default::default()
            });
        let next = playback.take_next(running.generation);
        let group_item = running.options.group_item.clone();
        // What the queue's modes make of an item that played to its end; a group's queue is the
        // server's, so its modes are too.
        let after = match result {
            Ok(Ended::Finished) if group_item.is_none() && !held => self.playback.queue.after_end(&running.item, next),
            _ => AfterEnd::Stop,
        };
        match result {
            // In a group, every member reports the end and the server moves the queue on
            // (once); the next item then comes as a queue update.
            Ok(Ended::Finished) if let Some(group_item) = &group_item => {
                self.group_item_ended(group_item);
                self.events.push(AppEvent::PlaybackEnded);
            }
            Ok(Ended::Finished) if !matches!(after, AfterEnd::Stop) => match after {
                AfterEnd::Replay => {
                    tracing::info!(item = %running.item.name, "repeating the item");
                    self.play(&running.item, replay);
                }
                AfterEnd::Next(next) => {
                    tracing::info!(next = %next.name, "playing the next episode");
                    self.play_from_resume(&next);
                }
                // The queue is made again from what played, so the first of it plays without
                // asking the queue to move on.
                AfterEnd::Restart(first) => {
                    tracing::info!(first = %first.name, "repeating the queue");
                    self.play_now(&first, PlayOptions::default());
                }
                AfterEnd::Stop => self.events.push(AppEvent::PlaybackEnded),
            },
            // Keep the player up and try again from where it stopped.
            Ok(Ended::Lost {
                position,
                message,
                crashed,
            }) if group_item.is_none() && running.attempt < RECONNECT_DELAYS.len() => {
                let delay = RECONNECT_DELAYS[running.attempt];
                tracing::warn!(attempt = running.attempt, ?delay, "stream lost: {message}");
                let notice = if crashed {
                    tr!("player-restarting")
                } else {
                    tr!("player-reconnecting")
                };
                self.toast(Toast::info(notice));
                let mut due = Deadline::default();
                due.set(delay);
                self.playback.reconnect = Some(Reconnect {
                    due,
                    item: running.item,
                    options: PlayOptions {
                        start: Some(position),
                        resumed: false,
                        picture: picture.or(running.options.picture.clone()),
                        ..running.options
                    },
                    attempt: running.attempt + 1,
                });
            }
            Ok(Ended::Lost { message, crashed, .. }) => {
                tracing::warn!("stream lost for good: {message}");
                let error = if crashed {
                    tr!("player-crashing")
                } else {
                    tr!("player-connection-lost")
                };
                self.toast(Toast::error(error));
                self.events.push(AppEvent::PlaybackEnded);
            }
            // Handed over to a TV: the player stays up, showing the cast.
            Ok(_) if self.cast.is_handing_over() => {}
            Ok(_) => self.events.push(AppEvent::PlaybackEnded),
            Err(message) => {
                tracing::warn!("playback failed: {message}");
                self.toast(Toast::error(message));
                self.events.push(AppEvent::PlaybackEnded);
            }
        }
    }
}

/// Plays `item` from its resume point, with the player offering to start over.
fn resume_options(item: &BaseItemDto) -> PlayOptions {
    let start = item
        .user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0);
    PlayOptions {
        start,
        resumed: start.is_some(),
        ..Default::default()
    }
}

/// How a session ended without an error.
enum Ended {
    /// Played to the end.
    Finished,
    Stopped,
    /// The stream failed (or ended early) after playback had started, at `position`: likely
    /// the network, so worth retrying.
    Lost {
        position: Ticks,
        message: String,
        /// The player process crashed, rather than the stream failing.
        crashed: bool,
    },
}

/// Waits before each reconnect attempt after a lost stream.
const RECONNECT_DELAYS: [Duration; 3] = [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)];
/// An end of file this long before the duration is a dropped stream, not the end.
const EARLY_END: Duration = Duration::from_secs(10);

struct Channels {
    stop: oneshot::Receiver<()>,
    updates: task::Sender<SessionUpdate>,
    commands: mpsc::UnboundedReceiver<UiCommand>,
}

/// The item as mpv's window title and the shell's now-playing line show it.
pub fn title(item: &BaseItemDto) -> String {
    match (item.kind, &item.series_name) {
        (BaseItemKind::Episode, Some(series)) => {
            let label = crate::widgets::text::episode_label(item);
            format!("{series} {label} - {}", item.name)
        }
        _ => item.name.clone(),
    }
}

/// mpv numbers the tracks of each type in a file from 1, in stream order. External subtitles
/// are added separately, so only embedded streams count.
fn mpv_track_id(source: &MediaSourceInfo, kind: MediaStreamType, index: i32) -> Option<usize> {
    let mut embedded: Vec<&MediaStream> = source
        .media_streams
        .iter()
        .filter(|s| s.kind == kind && !s.is_external)
        .collect();
    embedded.sort_by_key(|s| s.index);
    embedded.iter().position(|s| s.index == index).map(|i| i + 1)
}

/// mpv's id for a subtitle once the file is loaded and the external ones were added (after
/// the embedded tracks, in `media_streams` order).
fn mpv_subtitle_id(source: &MediaSourceInfo, files: &[i32], index: i32) -> Option<usize> {
    if let Some(id) = mpv_track_id(source, MediaStreamType::Subtitle, index) {
        return Some(id);
    }
    let embedded = source
        .media_streams
        .iter()
        .filter(|s| s.kind == MediaStreamType::Subtitle && !s.is_external)
        .count();
    let position = files.iter().position(|&file| file == index)?;
    Some(embedded + position + 1)
}

/// How an external subtitle is told from the others when the server's list of streams is read
/// again: by its file, or by its labels when it has none (a downloaded copy's).
fn subtitle_key(stream: &MediaStream) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    match stream.path.as_deref() {
        Some(path) => (Some(path), None, None, None),
        None => (
            None,
            stream.display_title.as_deref(),
            stream.language.as_deref(),
            stream.codec.as_deref(),
        ),
    }
}

/// The external subtitle `new` has that `old` didn't, and `files` (the externals in mpv's
/// order) renumbered for `new`, with the new one last. Adding a subtitle file can move the
/// others' indexes, so they're matched by [`subtitle_key`] rather than by index. `None` when
/// nothing was added.
fn added_subtitle<'a>(
    old: &MediaSourceInfo,
    old_files: &[i32],
    new: &'a MediaSourceInfo,
) -> Option<(&'a MediaStream, Vec<i32>)> {
    let mut unmatched: Vec<&MediaStream> = new
        .media_streams
        .iter()
        .filter(|s| s.kind == MediaStreamType::Subtitle && s.is_external)
        .collect();
    let mut files = Vec::with_capacity(old_files.len() + 1);
    for stream in old_files
        .iter()
        .filter_map(|&index| old.media_streams.iter().find(|s| s.index == index))
    {
        let at = unmatched.iter().position(|s| subtitle_key(s) == subtitle_key(stream))?;
        files.push(unmatched.remove(at).index);
    }
    let added = *unmatched.first()?;
    files.push(added.index);
    Some((added, files))
}

/// Whether a session starts with deinterlacing on: when the server says the video stream is
/// interlaced, and mpv gets the stream as it is. A transcode comes out of the server's encoder
/// already progressive.
fn deinterlaces_by_default(source: &MediaSourceInfo, method: PlayMethod) -> bool {
    method == PlayMethod::DirectPlay
        && source
            .media_streams
            .iter()
            .any(|stream| stream.kind == MediaStreamType::Video && stream.is_interlaced)
}

/// mpv's id for the subtitle with Jellyfin stream `index`. A transcode has only the tracks sent as
/// files, in the order they were added.
fn subtitle_track(source: &MediaSourceInfo, stream: &StreamInfo, index: i32) -> Option<usize> {
    if stream.method == PlayMethod::Transcode {
        stream.subtitle_files.iter().position(|&i| i == index).map(|i| i + 1)
    } else {
        mpv_subtitle_id(source, &stream.subtitle_files, index)
    }
}

/// The downloaded copy of `item`, when the account has one and the version asked for is the
/// one that was downloaded. The lookup reads the disk, so it goes to a blocking thread.
async fn local_media(
    account: &str,
    item: &BaseItemDto,
    media_source_id: Option<&str>,
) -> Option<crate::downloads::LocalMedia> {
    let (account, item_id) = (account.to_owned(), item.id.clone());
    let local = tokio::task::spawn_blocking(move || crate::downloads::find_local(&account, &item_id))
        .await
        .unwrap_or_else(|err| {
            tracing::warn!("couldn't look for a downloaded copy: {err}");
            None
        })?;
    match media_source_id {
        Some(id) if id != local.source.id => None,
        _ => Some(local),
    }
}

/// The original file when the server allows direct play, else its HLS transcode.
fn stream_url(
    client: &Client,
    item_id: &str,
    source: &MediaSourceInfo,
    play_session_id: Option<&str>,
) -> Result<(Url, PlayMethod), String> {
    if source.supports_direct_play || source.supports_direct_stream {
        return Ok((
            client.stream_url(item_id, &source.id, play_session_id),
            PlayMethod::DirectPlay,
        ));
    }
    let relative = source
        .transcoding_url
        .as_deref()
        .ok_or_else(|| tr!("player-not-streamable"))?;
    let url = client.transcode_url(relative);
    let reasons = query_param(&url, "TranscodeReasons").unwrap_or_default();
    tracing::info!(%reasons, "the server is transcoding");
    Ok((url, PlayMethod::Transcode))
}

/// The first of a transcoding URL's comma-separated codecs (`VideoCodec=h264`).
fn first_codec(url: &Url, name: &str) -> Option<String> {
    query_param(url, name)?
        .split(',')
        .next()
        .filter(|codec| !codec.is_empty())
        .map(str::to_owned)
}

fn query_param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Subtitles mpv loads as separate files: external ones when direct playing; when
/// transcoding, those the server sends as files (embedded text subtitles included).
fn subtitle_files(
    client: &Client,
    item_id: &str,
    source: &MediaSourceInfo,
    method: PlayMethod,
) -> Vec<(MediaStream, Url)> {
    source
        .media_streams
        .iter()
        .filter(|s| s.kind == MediaStreamType::Subtitle)
        .filter_map(|stream| {
            let url = match method {
                PlayMethod::DirectPlay if stream.is_external => client.subtitle_url(item_id, &source.id, stream),
                PlayMethod::Transcode if stream.delivery_method.as_deref() == Some("External") => {
                    client.transcode_url(stream.delivery_url.as_deref()?)
                }
                _ => return None,
            };
            Some((stream.clone(), url))
        })
        .collect()
}

fn subtitle_index(choice: SubtitleChoice) -> Option<i32> {
    match choice {
        SubtitleChoice::Stream(index) => Some(index),
        SubtitleChoice::Off => Some(-1),
        SubtitleChoice::Auto => None,
    }
}

/// Sets the audio output's mpv options from the settings, which only the desktop's mpv has
/// (Android's mpv is `ao=aaudio` and its own default player is Media3). A device that's no
/// longer there is left out, so playback goes to the automatic one; a rejected option is logged
/// and skipped rather than failing the playback. Returns what was applied.
async fn apply_audio(player: &Player, mut settings: player::AudioSettings) -> player::AudioSettings {
    if cfg!(target_os = "android") {
        return settings;
    }
    if let Some(device) = settings.device.clone() {
        let present = match player.audio_devices().await {
            Ok(devices) => devices.iter().any(|d| d.name == device),
            Err(err) => {
                tracing::debug!("couldn't list the audio devices: {err}");
                false
            }
        };
        if !present {
            tracing::warn!(%device, "the chosen audio device isn't there; using the automatic one");
            settings.device = None;
        }
    }
    for (name, value) in settings.properties() {
        if let Err(err) = player.set_property(name, value.as_str()).await {
            tracing::warn!("mpv didn't take {name}={value}: {err}");
        }
    }
    settings
}

async fn start_player(
    import: VideoImport,
) -> Result<(Player, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput), String> {
    let engine = Config::load().android_player.unwrap_or_default().engine();
    Player::spawn(import, engine).await.map_err(|err| match err {
        player::Error::LibmpvUnavailable(reason) if cfg!(target_os = "android") => {
            tracing::warn!("libmpv unavailable: {reason}");
            tr!("player-mpv-android")
        }
        player::Error::LibmpvUnavailable(reason) => {
            tracing::warn!("libmpv unavailable: {reason}");
            if cfg!(target_os = "macos") {
                tr!("player-mpv-missing-macos")
            } else if cfg!(windows) {
                tr!("player-mpv-missing-windows")
            } else {
                tr!("player-mpv-missing-linux")
            }
        }
        err => tr!("player-start-failed", error = err.to_string()),
    })
}

/// A downloaded copy's chapters and segments, for the player's controls when the server can't
/// answer (`details`).
type LocalMarkers = (Vec<ChapterInfo>, Vec<MediaSegmentDto>);

/// Versions, chapters, segments and the next episode, for the player's controls. Failures
/// only lose those controls; a downloaded item's own chapters and segments stand in when the
/// server can't be asked (offline, or the connection dropped), so the chapter marks and skip
/// buttons work either way. `local` is what `run_session` already found on disk.
async fn details(client: &Client, item: &BaseItemDto, local: Option<LocalMarkers>) -> Details {
    let (full, segments, neighbours) = tokio::join!(
        client.item(
            &item.id,
            &[ItemField::MediaSources, ItemField::Chapters, ItemField::Trickplay]
        ),
        client.media_segments(&item.id),
        neighbours(client, item),
    );
    let full = full
        .inspect_err(|err| tracing::warn!("couldn't load versions and chapters: {err}"))
        .ok();
    let neighbours = neighbours
        .inspect_err(|err| tracing::warn!("couldn't find the previous and next items: {err}"))
        .unwrap_or_default();
    let sources = full.as_ref().map(|f| f.media_sources.clone()).unwrap_or_default();
    let (local_chapters, local_segments) = local.unwrap_or_default();
    let chapters = match full.as_ref().map(|f| f.chapters.as_slice()) {
        Some(chapters) if !chapters.is_empty() => markers::chapters(chapters),
        _ => markers::chapters(&local_chapters),
    };
    let segments = match segments.inspect_err(|err| tracing::warn!("couldn't load media segments: {err}")) {
        Ok(segments) if !segments.is_empty() => markers::from_server(&segments),
        _ if !local_segments.is_empty() => markers::from_server(&local_segments),
        _ => markers::from_chapters(&chapters, item.run_time_ticks.map(Duration::from)),
    };
    let versions = sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let number = i + 1;
            Version {
                id: source.id.clone(),
                name: source
                    .name
                    .clone()
                    .unwrap_or_else(|| tr!("player-version-number", number = number)),
            }
        })
        .collect();
    Details {
        versions,
        trickplay: full.map(|full| full.trickplay).unwrap_or_default(),
        previous: neighbours.previous,
        next: neighbours.next,
        next_in_collection: neighbours.in_collection,
        chapters,
        segments,
    }
}

#[derive(Default)]
struct Neighbours {
    previous: Option<BaseItemDto>,
    next: Option<BaseItemDto>,
    /// They're movies of the item's collection, not episodes.
    in_collection: bool,
}

/// The episodes around an episode, across seasons; or for a movie, the movies around it in
/// its first collection, in the collection page's order.
async fn neighbours(client: &Client, item: &BaseItemDto) -> jellyfin::Result<Neighbours> {
    match (item.kind, &item.series_id) {
        (BaseItemKind::Episode, Some(series_id)) => {
            let episodes = client.episodes(series_id, None, &[]).await?.items;
            let (previous, next) = adjacent(episodes, &item.id);
            Ok(Neighbours {
                previous,
                next,
                in_collection: false,
            })
        }
        (BaseItemKind::Movie | BaseItemKind::Video, _) => {
            let Some(collection) = client.item_collections(&item.id).await?.into_iter().next() else {
                return Ok(Neighbours::default());
            };
            let mut movies = crate::views::item::collection_items(client, &collection.id, &[]).await?;
            movies.retain(|movie| matches!(movie.kind, BaseItemKind::Movie | BaseItemKind::Video));
            let (previous, next) = adjacent(movies, &item.id);
            Ok(Neighbours {
                previous,
                next,
                in_collection: true,
            })
        }
        _ => Ok(Neighbours::default()),
    }
}

/// The items before and after the one with `id`, if it's there.
fn adjacent(mut items: Vec<BaseItemDto>, id: &str) -> (Option<BaseItemDto>, Option<BaseItemDto>) {
    let Some(index) = items.iter().position(|item| item.id == id) else {
        return (None, None);
    };
    let next = (index + 1 < items.len()).then(|| items.remove(index + 1));
    let previous = index.checked_sub(1).map(|i| items.swap_remove(i));
    (previous, next)
}

/// How long a connection measurement is reused.
const BITRATE_TEST_TTL: Duration = Duration::from_secs(10 * 60);
/// The share of the measured speed to stream at, leaving room for other traffic and dips.
const BITRATE_HEADROOM: f64 = 0.8;
/// Measured above this, the connection isn't the limit: no cap.
const BITRATE_NO_LIMIT: u64 = 200_000_000;

/// The last connection measurement: when, and the cap it gave (`None`: no limit).
static MEASURED_BITRATE: std::sync::Mutex<Option<(Instant, Option<u64>)>> = std::sync::Mutex::new(None);

/// The bitrate cap for the next playback: the setting, or with *Automatic* a measurement of
/// the connection (reused for a while).
async fn max_bitrate(client: &Client) -> Option<u64> {
    let config = Config::load();
    if !config.automatic_bitrate.unwrap_or(false) {
        return config.max_streaming_bitrate;
    }
    if let Some((at, cap)) = *MEASURED_BITRATE.lock().unwrap()
        && at.elapsed() < BITRATE_TEST_TTL
    {
        return cap;
    }
    let cap = match measure_bitrate(client).await {
        Ok(bits) => {
            let cap = (bits < BITRATE_NO_LIMIT).then_some((bits as f64 * BITRATE_HEADROOM) as u64);
            tracing::info!(measured_mbps = bits / 1_000_000, cap_mbps = ?cap.map(|c| c / 1_000_000), "connection measured");
            cap
        }
        Err(err) => {
            // Without a measurement, fall back to the fixed setting.
            tracing::warn!("couldn't measure the connection: {err}");
            return config.max_streaming_bitrate;
        }
    };
    *MEASURED_BITRATE.lock().unwrap() = Some((Instant::now(), cap));
    cap
}

/// Like the web client: a small download first, and a bigger one when that was quick, so a
/// fast connection isn't judged by its latency.
async fn measure_bitrate(client: &Client) -> jellyfin::Result<u64> {
    let small = client.bitrate_test(500_000).await?;
    if small < 10_000_000 {
        return Ok(small);
    }
    client.bitrate_test(3_000_000).await
}

/// Runs one playback from negotiation to the final stop report. Returns a user-facing error.
#[allow(clippy::too_many_arguments)]
async fn run_session(
    client: Client,
    item: BaseItemDto,
    options: PlayOptions,
    (volume, muted): (f64, bool),
    adjustments: Adjustments,
    (account, online): (String, bool),
    import: VideoImport,
    (repeat_mode, playback_order): (RepeatMode, PlaybackOrder),
    mut channels: Channels,
) -> Result<Ended, String> {
    // Offline, nothing goes to the server: each call would only wait to fail.
    let send = |result| {
        if online {
            report_or_log(result, "progress")
        }
    };
    // A downloaded copy plays from disk, with no negotiation and no auth header; the server
    // still hears about the playback, since the reports below are the same.
    let local = local_media(&account, &item, options.media_source_id.as_deref()).await;
    let from_disk = local.is_some();
    // What the player's chapter marks and skip buttons fall back to when the server can't be
    // asked; the lookup is done already, so `details` doesn't have to wait for it.
    let local_markers = local
        .as_ref()
        .map(|local| (local.chapters.clone(), local.segments.clone()));
    let (mut source, url, method, subtitle_files, play_session_id) = match local {
        Some(local) => {
            tracing::info!(item = %item.name, path = %local.path.display(), "playing a downloaded file");
            let url = Url::from_file_path(&local.path)
                .map_err(|()| tr!("player-download-path", path = local.path.display().to_string()))?;
            (
                local.source.clone(),
                url,
                PlayMethod::DirectPlay,
                local.subtitle_files(),
                None,
            )
        }
        None => {
            let info = client
                .playback_info(
                    &item.id,
                    &PlaybackInfoRequest {
                        media_source_id: options.media_source_id.clone(),
                        start_position: options.start,
                        audio_stream_index: options.audio_stream_index,
                        subtitle_stream_index: subtitle_index(options.subtitle),
                        max_streaming_bitrate: max_bitrate(&client).await,
                        prefer_hevc: Config::load().prefer_hevc.unwrap_or(false),
                        passthrough_eac3: Config::load().passthrough_eac3.unwrap_or(false),
                    },
                )
                .await
                .map_err(|err| describe_error(&err))?;
            if let Some(code) = info.error_code {
                return Err(tr!("player-server-refused", code = code));
            }
            let source = match &options.media_source_id {
                Some(id) => info.media_sources.into_iter().find(|s| &s.id == id),
                None => info.media_sources.into_iter().next(),
            }
            .ok_or_else(|| tr!("player-no-media"))?;
            let (url, method) = stream_url(&client, &item.id, &source, info.play_session_id.as_deref())?;
            let subtitle_files = subtitle_files(&client, &item.id, &source, method);
            (source, url, method, subtitle_files, info.play_session_id)
        }
    };
    // Tracks not chosen explicitly (autoplay, the next episode, SyncPlay) follow the server's
    // defaults, which apply the user's language preferences and subtitle mode; mpv on its own
    // would go by the file's default flags. A transcode names its audio track in the URL.
    let audio = options.audio_stream_index.or_else(|| match method {
        PlayMethod::Transcode => query_param(&url, "AudioStreamIndex").and_then(|i| i.parse().ok()),
        PlayMethod::DirectPlay => source.default_audio_stream_index,
    });
    let subtitle = match (options.subtitle, source.default_subtitle_stream_index) {
        (SubtitleChoice::Auto, Some(-1)) => SubtitleChoice::Off,
        (SubtitleChoice::Auto, Some(index)) => SubtitleChoice::Stream(index),
        (choice, _) => choice,
    };
    let mut stream = StreamInfo {
        method,
        subtitle_files: subtitle_files.iter().map(|(stream, _)| stream.index).collect(),
        burned_subtitle: (method == PlayMethod::Transcode)
            .then(|| {
                source
                    .media_streams
                    .iter()
                    .find(|s| s.delivery_method.as_deref() == Some("Encode"))
            })
            .flatten()
            .map(|s| s.index),
        transcode_reasons: (method == PlayMethod::Transcode)
            .then(|| query_param(&url, "TranscodeReasons"))
            .flatten()
            .map(|reasons| {
                reasons
                    .split(',')
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        transcode_video_codec: (method == PlayMethod::Transcode)
            .then(|| first_codec(&url, "VideoCodec"))
            .flatten(),
        transcode_audio_codec: (method == PlayMethod::Transcode)
            .then(|| first_codec(&url, "AudioCodec"))
            .flatten(),
    };

    let picture = Picture::starting(options.picture.as_ref(), &source, &stream, subtitle);
    let (player, mut events, video) = start_player(import).await?;

    // Track choices are options, so they're set before loading the file. A transcode has
    // just the chosen audio track and no embedded subtitles.
    let direct = method == PlayMethod::DirectPlay;
    if let Some(aid) = audio
        .filter(|_| direct)
        .and_then(|index| mpv_track_id(&source, MediaStreamType::Audio, index))
    {
        player.set_property("aid", aid).await.map_err(|err| err.to_string())?;
    }
    let file_selected = match subtitle {
        // The server's default, when it sends that one as a file.
        SubtitleChoice::Auto if !direct => source
            .default_subtitle_stream_index
            .filter(|index| stream.subtitle_files.contains(index)),
        SubtitleChoice::Auto => None,
        SubtitleChoice::Off => {
            player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
            None
        }
        SubtitleChoice::Stream(index) => match mpv_track_id(&source, MediaStreamType::Subtitle, index) {
            Some(sid) if direct => {
                player.set_property("sid", sid).await.map_err(|err| err.to_string())?;
                None
            }
            // A file: no embedded track until it's added and selected after loading.
            _ => {
                player.set_property("sid", "no").await.map_err(|err| err.to_string())?;
                Some(index)
            }
        },
    };

    if options.start_paused {
        player
            .set_property("pause", true)
            .await
            .map_err(|err| err.to_string())?;
    }
    // The audio output before the volume: `volume-max` sets how far the volume goes.
    let config = Config::load();
    let audio_output = apply_audio(
        &player,
        player::AudioSettings {
            night_mode: adjustments.night_mode,
            ..config.audio_settings()
        },
    )
    .await;
    player
        .set_property("volume", volume.clamp(0., volume_max()))
        .await
        .map_err(|err| err.to_string())?;
    player
        .set_property("mute", muted)
        .await
        .map_err(|err| err.to_string())?;
    // Cosmetic, so a property this mpv doesn't have is skipped: before 0.38 there's no
    // `sub-border-style`, and a visible `sub-back-color` draws the box on its own.
    for (name, value) in subtitle_look(
        config.subtitle_color.unwrap_or_default(),
        config.subtitle_style.unwrap_or_default(),
    ) {
        if let Err(err) = player.set_property(name, value).await {
            tracing::debug!("mpv didn't take {name}={value}: {err}");
        }
    }
    // Cosmetic too: a setting that fails leaves subtitles where mpv puts them.
    if let Err(err) = player
        .set_property("sub-pos", i64::from(adjustments.subtitle_position))
        .await
    {
        tracing::debug!("mpv didn't take sub-pos: {err}");
    }
    // Before the file loads, like the rest, so the first frame is already deinterlaced.
    player
        .set_property("deinterlace", picture.deinterlace)
        .await
        .map_err(|err| err.to_string())?;
    // A restart of the item keeps how its picture fills the window; a new item has mpv's own.
    if picture.aspect != Aspect::Fit {
        for (name, value) in picture.aspect.mpv_properties() {
            if let Err(err) = player.set_property(name, value).await {
                tracing::debug!("mpv didn't take {name}: {err}");
            }
        }
    }
    for (name, value) in [
        ("speed", adjustments.speed),
        ("sub-delay", adjustments.subtitle_delay),
        ("audio-delay", adjustments.audio_delay),
        ("sub-scale", adjustments.subtitle_scale),
    ] {
        player.set_property(name, value).await.map_err(|err| err.to_string())?;
    }

    player
        .load(LoadRequest {
            url: url.to_string(),
            title: Some(title(&item)),
            start: options.start.map(Duration::from),
            // The local file needs no auth; the server's stream does.
            http_headers: if from_disk {
                Vec::new()
            } else {
                vec![format!("Authorization: {}", client.authorization_header())]
            },
        })
        .await
        .map_err(|err| tr!("player-stream-open-failed", error = err.to_string()))?;

    let _ = channels.updates.send(SessionUpdate::Started {
        source: Box::new(source.clone()),
        video,
        audio,
        subtitle,
        stream: stream.clone(),
        picture: Box::new(picture.clone()),
    });
    let details_task = {
        let (client, item, updates) = (client.clone(), item.clone(), channels.updates.clone());
        tokio::spawn(async move {
            let details = details(&client, &item, local_markers).await;
            let _ = updates.send(SessionUpdate::Details(Box::new(details)));
        })
    };

    let mut report = PlaybackReport {
        item_id: item.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: play_session_id.clone(),
        position: options.start.unwrap_or_default(),
        audio_stream_index: audio,
        subtitle_stream_index: subtitle_index(subtitle),
        play_method: method,
        volume_level: volume_level(volume),
        is_muted: muted,
        repeat_mode,
        playback_order,
        ..Default::default()
    };
    let mut reporting = Reporting::default();
    let mut stopping = false;
    // Stepping frames pauses and resumes mpv each time; the server hears of it from the next
    // progress report instead of a pair for every frame.
    let mut step_quiet = StepQuiet::default();
    let mut finished = false;
    let mut error = None;
    let mut duration = None;
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                tracing::trace!(?event, "player event");
                let _ = channels.updates.send(SessionUpdate::Player(event.clone()));
                match &event {
                    // External subtitles go in before the start report.
                    PlayerEvent::FileLoaded => {
                        for (stream, url) in &subtitle_files {
                            let title = stream.display_title.as_deref().unwrap_or("External");
                            let select = file_selected == Some(stream.index);
                            if let Err(err) = player.add_subtitle(url.as_str(), title, stream.language.as_deref(), select).await {
                                tracing::warn!("couldn't add subtitle: {err}");
                            }
                        }
                        // What a restart of the item keeps, once there are tracks to pick from.
                        if let Some(sid) = picture
                            .secondary_subtitle
                            .and_then(|index| subtitle_track(&source, &stream, index))
                            && let Err(err) = player.set_property("secondary-sid", sid).await
                        {
                            tracing::warn!("couldn't show the second subtitle again: {err}");
                        }
                        let (a, b) = match picture.ab_loop {
                            AbLoop::Off => (None, None),
                            AbLoop::A(a) => (Some(a), None),
                            AbLoop::Loop(a, b) => (Some(a), Some(b)),
                        };
                        for (name, point) in [("ab-loop-a", a), ("ab-loop-b", b)] {
                            if let Some(point) = point
                                && let Err(err) = player.set_property(name, point.as_secs_f64()).await
                            {
                                tracing::warn!("couldn't set the loop again: {err}");
                            }
                        }
                    }
                    PlayerEvent::Duration(length) => duration = Some(*length),
                    PlayerEvent::EndFile(EndReason::Eof) if !stopping => finished = true,
                    PlayerEvent::EndFile(EndReason::Error(message)) => {
                        error = Some(tr!("player-play-failed", error = message));
                    }
                    PlayerEvent::Exited => break,
                    _ => {}
                }
                let quiet = step_quiet.hushes(&event, tokio::time::Instant::now());
                match reporting.on_event(&event, &mut report, tokio::time::Instant::now()) {
                    Outgoing::Start => {
                        if online {
                            report_or_log(client.report_playback_start(&report).await, "start");
                        }
                        save_unfinished(Some(&report), &account);
                    }
                    Outgoing::Progress if online && !quiet => send(client.report_playback_progress(&report).await),
                    Outgoing::Progress => {}
                    Outgoing::Nothing => {}
                }
            }
            Some(command) = channels.commands.recv() => {
                if matches!(command, UiCommand::PollInfo) {
                    let values = player.properties(&info::PROPERTIES).await;
                    let _ = channels.updates.send(SessionUpdate::Stats(Box::new(Stats::parse(&values))));
                    continue;
                }
                tracing::debug!(?command, "player command");
                step_quiet.on_command(&command, tokio::time::Instant::now());
                let send_now = reporting.on_command(&command);
                if let UiCommand::AddSubtitle(added) = &command {
                    match player.add_subtitle(added.url.as_str(), &added.title, added.language.as_deref(), true).await {
                        Ok(()) => {
                            source = added.source.clone();
                            stream.subtitle_files = added.files.clone();
                            report.subtitle_stream_index = Some(added.index);
                        }
                        Err(err) => tracing::warn!("couldn't add the downloaded subtitle: {err}"),
                    }
                }
                if let Err(err) = apply_command(&player, &source, &stream, &mut report, audio_output.passthrough, command).await {
                    tracing::warn!("player command failed: {err}");
                }
                if online && send_now == Outgoing::Progress {
                    send(client.report_playback_progress(&report).await);
                }
            }
            _ = tokio::time::sleep_until(reporting.volume_due.unwrap_or_else(tokio::time::Instant::now)), if reporting.volume_due.is_some() => {
                reporting.volume_reported();
                if online {
                    send(client.report_playback_progress(&report).await);
                }
            }
            _ = ticker.tick(), if reporting.started => {
                reporting.volume_reported();
                if online {
                    send(client.report_playback_progress(&report).await);
                }
                save_unfinished(Some(&report), &account);
            }
            _ = &mut channels.stop, if !stopping => {
                // Replaced by a new session or signed out: ask mpv to quit and keep going until
                // it exits, so the final position gets reported.
                stopping = true;
                let _ = player.quit().await;
            }
        }
    }

    details_task.abort();
    let started = reporting.started;
    let ended_early = duration.is_some_and(|d| Duration::from(report.position) + EARLY_END < d);
    // Played to the end (not stopped, lost or ended early), for the local watch state below.
    let completed = error.is_none() && finished && !ended_early;
    if started {
        let reported = online && {
            let stopped = client.report_playback_stopped(&report).await;
            let reported = stopped.is_ok();
            report_or_log(stopped, "stop");
            reported
        };
        save_unfinished(None, &account);
        if from_disk {
            // A downloaded file's watch state is the app's to keep: the server may not be there
            // to hold it. `item.json` gets the new position right away (so the next offline
            // start resumes there), and the stop report that didn't go out waits in
            // `pending.json` for the server's next answer.
            let (account, item_id, media_source_id) =
                (account.clone(), report.item_id.clone(), report.media_source_id.clone());
            let position = report.position;
            let played_at = (!reported).then(|| jellyfin::time::format_utc(std::time::SystemTime::now()));
            let written = tokio::task::spawn_blocking(move || {
                crate::downloads::record_progress(&account, &item_id, position, completed);
                if let Some(played_at) = played_at {
                    crate::downloads::add_pending(
                        &account,
                        crate::downloads::PendingReport {
                            item_id,
                            media_source_id,
                            position_ticks: position,
                            played_at,
                        },
                    );
                }
            })
            .await;
            if let Err(err) = written {
                tracing::warn!("couldn't save the downloaded item's watch state: {err}");
            }
        }
    }
    if let (PlayMethod::Transcode, Some(id)) = (method, &play_session_id) {
        report_or_log(client.stop_transcoding(id).await, "transcode stop");
    }
    tracing::info!(item = %item.name, position = report.position.as_secs_f64(), "playback ended");
    match error {
        // A failure right at the end isn't worth replaying from there.
        Some(message) if started && !stopping && ended_early => Ok(Ended::Lost {
            position: report.position,
            crashed: message.ends_with(player::HOST_CRASHED),
            message,
        }),
        Some(error) => Err(error),
        None if finished && ended_early => Ok(Ended::Lost {
            position: report.position,
            message: "the stream ended early".into(),
            crashed: false,
        }),
        None if finished => Ok(Ended::Finished),
        None => Ok(Ended::Stopped),
    }
}

/// The short while after a frame step in which the pause and seek it causes go unreported.
#[derive(Debug, Default)]
struct StepQuiet {
    until: Option<tokio::time::Instant>,
}

impl StepQuiet {
    /// A frame step starts the quiet. A pause, unpause or seek the user asks for ends it, since
    /// what follows is theirs and the server should hear of it (a Space right after a `.` would
    /// otherwise leave it showing "paused" until the next periodic report).
    fn on_command(&mut self, command: &UiCommand, now: tokio::time::Instant) {
        match command {
            UiCommand::FrameStep(_) => self.until = Some(now + FRAME_STEP_QUIET),
            UiCommand::TogglePause | UiCommand::SetPause(_) | UiCommand::SeekBy(_) | UiCommand::SeekTo(_) => {
                self.until = None;
            }
            _ => {}
        }
    }

    /// Whether `event` is the echo of a frame step, which isn't reported.
    fn hushes(&self, event: &PlayerEvent, now: tokio::time::Instant) -> bool {
        self.until.is_some_and(|until| now < until) && matches!(event, PlayerEvent::Paused(_) | PlayerEvent::Seeked)
    }
}

/// A report to send to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outgoing {
    Nothing,
    Start,
    Progress,
}

/// When a session reports to the server, from the player's events and the user's commands.
/// `run_session` sends what this decides; the stop report goes out at the end if it started.
#[derive(Debug, Default)]
struct Reporting {
    /// The start report went out (mpv loaded the file). Nothing is reported before it, since
    /// the start report carries the state itself.
    started: bool,
    /// When the batched volume and mute report is due: a wheel spin or a held key sends one
    /// report, a second after the first change.
    volume_due: Option<tokio::time::Instant>,
}

impl Reporting {
    /// Updates `report` from a player event and says what to send.
    fn on_event(&mut self, event: &PlayerEvent, report: &mut PlaybackReport, now: tokio::time::Instant) -> Outgoing {
        match event {
            PlayerEvent::FileLoaded => {
                self.started = true;
                return Outgoing::Start;
            }
            PlayerEvent::Position(position) => report.position = Ticks::from(*position),
            PlayerEvent::Volume(volume) => {
                report.volume_level = volume_level(*volume);
                self.schedule_volume(now);
            }
            PlayerEvent::Muted(muted) => {
                report.is_muted = *muted;
                self.schedule_volume(now);
            }
            PlayerEvent::Paused(paused) => {
                report.is_paused = *paused;
                return self.progress();
            }
            PlayerEvent::Seeked => return self.progress(),
            _ => {}
        }
        Outgoing::Nothing
    }

    /// Commands that change what the reports carry are reported once applied. Volume and mute
    /// are reported from mpv's events instead, batched.
    fn on_command(&self, command: &UiCommand) -> Outgoing {
        match command {
            UiCommand::TogglePause
            | UiCommand::SetPause(_)
            | UiCommand::SeekBy(_)
            | UiCommand::SeekTo(_)
            | UiCommand::Audio(_)
            | UiCommand::Subtitle(_)
            | UiCommand::SetQueueModes(..)
            | UiCommand::AddSubtitle(_) => self.progress(),
            _ => Outgoing::Nothing,
        }
    }

    /// A progress report went out, which carries the volume too.
    fn volume_reported(&mut self) {
        self.volume_due = None;
    }

    fn schedule_volume(&mut self, now: tokio::time::Instant) {
        if self.started {
            self.volume_due.get_or_insert(now + VOLUME_REPORT_AFTER);
        }
    }

    fn progress(&self) -> Outgoing {
        if self.started {
            Outgoing::Progress
        } else {
            Outgoing::Nothing
        }
    }
}

/// mpv options for the text subtitle appearance from the settings. ASS subtitles keep their own
/// styling (mpv only scales them).
fn subtitle_look(color: SubtitleColor, style: SubtitleStyle) -> [(&'static str, &'static str); 3] {
    let color = match color {
        SubtitleColor::White => "#FFFFFFFF",
        SubtitleColor::Yellow => "#FFFFE14D",
    };
    let (border_style, back_color) = match style {
        SubtitleStyle::Outline => ("outline-and-shadow", "#00000000"),
        SubtitleStyle::Box => ("background-box", "#B0000000"),
    };
    [
        ("sub-color", color),
        ("sub-border-style", border_style),
        ("sub-back-color", back_color),
    ]
}

/// mpv's volume in percent as the whole number the server's reports expect.
fn volume_level(volume: f64) -> i32 {
    volume.clamp(0., 100.).round() as i32
}

async fn apply_command(
    player: &Player,
    source: &MediaSourceInfo,
    stream: &StreamInfo,
    report: &mut PlaybackReport,
    passthrough: player::Passthrough,
    command: UiCommand,
) -> player::Result<()> {
    match command {
        UiCommand::TogglePause => player.command(json!(["cycle", "pause"])).await,
        UiCommand::SetPause(paused) => player.set_property("pause", paused).await,
        UiCommand::SetSpeed(speed) => player.set_property("speed", speed).await,
        UiCommand::SeekBy(seconds) => player.command(json!(["seek", seconds, "relative"])).await,
        UiCommand::SeekTo(position) => {
            player
                .command(json!(["seek", position.as_secs_f64(), "absolute"]))
                .await
        }
        UiCommand::SetVolume(volume) => player.set_property("volume", volume.clamp(0., volume_max())).await,
        UiCommand::SetMute(muted) => player.set_property("mute", muted).await,
        UiCommand::SetSubtitleDelay(seconds) => player.set_property("sub-delay", seconds).await,
        UiCommand::SetAudioDelay(seconds) => player.set_property("audio-delay", seconds).await,
        UiCommand::SetSubtitleScale(scale) => player.set_property("sub-scale", scale).await,
        UiCommand::SetSubtitlePosition(position) => player.set_property("sub-pos", i64::from(position)).await,
        UiCommand::SetNightMode(on) => player.set_property("af", player::night_filter(on, passthrough)).await,
        UiCommand::ToggleMute => player.command(json!(["cycle", "mute"])).await,
        // A transcode restarts for a new audio track instead (`NowPlaying::change_audio`).
        UiCommand::Audio(_) if stream.method == PlayMethod::Transcode => Ok(()),
        UiCommand::Audio(index) => {
            if let Some(aid) = mpv_track_id(source, MediaStreamType::Audio, index) {
                player.set_property("aid", aid).await?;
                report.audio_stream_index = Some(index);
            }
            Ok(())
        }
        UiCommand::Subtitle(choice) => {
            let sid = match choice {
                SubtitleChoice::Stream(index) => subtitle_track(source, stream, index).map(|id| json!(id)),
                SubtitleChoice::Off => Some(json!("no")),
                SubtitleChoice::Auto => Some(json!("auto")),
            };
            if let Some(sid) = sid {
                player.set_property("sid", sid).await?;
                report.subtitle_stream_index = subtitle_index(choice);
            }
            Ok(())
        }
        UiCommand::SetQueueModes(repeat_mode, playback_order) => {
            report.repeat_mode = repeat_mode;
            report.playback_order = playback_order;
            Ok(())
        }
        // The session takes it in itself, since it renumbers its tracks.
        UiCommand::AddSubtitle(_) => Ok(()),
        UiCommand::SetSecondarySubtitle(index) => {
            let sid = match index {
                Some(index) => subtitle_track(source, stream, index).map(|id| json!(id)),
                None => Some(json!("no")),
            };
            match sid {
                Some(sid) => player.set_property("secondary-sid", sid).await,
                None => Ok(()),
            }
        }
        UiCommand::SetAspect(aspect) => {
            for (name, value) in aspect.mpv_properties() {
                player.set_property(name, value).await?;
            }
            Ok(())
        }
        UiCommand::SetDeinterlace(on) => player.set_property("deinterlace", on).await,
        UiCommand::FrameStep(true) => player.command(json!(["frame-step"])).await,
        UiCommand::FrameStep(false) => player.command(json!(["frame-back-step"])).await,
        UiCommand::CycleAbLoop => player.command(json!(["ab-loop"])).await,
        UiCommand::Stop => player.quit().await,
        UiCommand::PollInfo => Ok(()),
    }
}

/// Remembers the running playback in the config (or forgets it when it ends), so a crash
/// can be reported on the next launch.
fn save_unfinished(report: Option<&PlaybackReport>, account: &str) {
    let mut config = Config::load();
    config.unfinished_playback = report.map(|report| UnfinishedPlayback {
        account: Some(account.to_owned()),
        item_id: report.item_id.clone(),
        media_source_id: report.media_source_id.clone(),
        play_session_id: report.play_session_id.clone(),
        position_ticks: report.position.0,
    });
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
}

/// On launch: if the last run ended mid-playback (a crash), tell the server where it stopped,
/// so the resume point is right and the session doesn't look like it's still playing. Only
/// the account that was playing reports it; it waits for that one to sign in otherwise.
pub fn report_unfinished(client: &Client, account: &str) {
    let mut config = Config::load();
    let Some(unfinished) = config
        .unfinished_playback
        .take_if(|unfinished| unfinished.account.as_deref().is_none_or(|a| a == account))
    else {
        return;
    };
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
    let report = PlaybackReport {
        item_id: unfinished.item_id,
        media_source_id: unfinished.media_source_id,
        play_session_id: unfinished.play_session_id,
        position: Ticks(unfinished.position_ticks),
        ..Default::default()
    };
    tracing::warn!(item = %report.item_id, position = report.position.as_secs_f64(), "the last playback didn't end cleanly; reporting its stop");
    let client = client.clone();
    runtime::detach(async move { report_or_log(client.report_playback_stopped(&report).await, "stop") });
}

fn report_or_log(result: jellyfin::Result<()>, what: &str) {
    if let Err(err) = result {
        tracing::warn!("playback {what} report failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use jellyfin::models::{MediaSourceInfo, MediaStream, MediaStreamType};

    use std::time::Duration;

    use jellyfin::PlaybackReport;
    use player::PlayerEvent;

    use super::{
        Adjustments, Outgoing, Reporting, StepQuiet, UiCommand, VOLUME_REPORT_AFTER, added_subtitle, adjacent,
        mpv_subtitle_id, mpv_track_id, volume_level, volume_max,
    };

    #[test]
    fn nothing_is_reported_before_the_start() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        for event in [PlayerEvent::Paused(true), PlayerEvent::Seeked, PlayerEvent::Volume(40.)] {
            assert_eq!(reporting.on_event(&event, &mut report, now), Outgoing::Nothing);
        }
        assert_eq!(reporting.on_command(&UiCommand::SeekBy(10.)), Outgoing::Nothing);
        // The start report carries what changed meanwhile, so no volume report is pending.
        assert!(report.is_paused && report.volume_level == 40);
        assert_eq!(reporting.volume_due, None);
        assert_eq!(
            reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now),
            Outgoing::Start
        );
    }

    #[test]
    fn pauses_seeks_and_track_changes_report_progress() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now);
        assert_eq!(
            reporting.on_event(&PlayerEvent::Paused(true), &mut report, now),
            Outgoing::Progress
        );
        assert!(report.is_paused);
        assert_eq!(
            reporting.on_event(&PlayerEvent::Seeked, &mut report, now),
            Outgoing::Progress
        );
        let position = PlayerEvent::Position(Duration::from_secs(5));
        assert_eq!(reporting.on_event(&position, &mut report, now), Outgoing::Nothing);
        assert_eq!(Duration::from(report.position), Duration::from_secs(5));
        assert_eq!(reporting.on_command(&UiCommand::Audio(2)), Outgoing::Progress);
        assert_eq!(reporting.on_command(&UiCommand::SetSpeed(1.5)), Outgoing::Nothing);
        assert_eq!(reporting.on_command(&UiCommand::SetVolume(50.)), Outgoing::Nothing);
    }

    #[test]
    fn the_queues_modes_are_reported_once_applied() {
        use jellyfin::{PlaybackOrder, RepeatMode};

        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let command = UiCommand::SetQueueModes(RepeatMode::RepeatAll, PlaybackOrder::Shuffle);
        // Before the start report, which carries the modes itself.
        assert_eq!(reporting.on_command(&command), Outgoing::Nothing);
        reporting.on_event(&PlayerEvent::FileLoaded, &mut report, tokio::time::Instant::now());
        assert_eq!(reporting.on_command(&command), Outgoing::Progress);
    }

    #[test]
    fn volume_changes_are_batched_into_one_report() {
        let (mut reporting, mut report) = (Reporting::default(), PlaybackReport::default());
        let now = tokio::time::Instant::now();
        reporting.on_event(&PlayerEvent::FileLoaded, &mut report, now);
        reporting.on_event(&PlayerEvent::Volume(60.), &mut report, now);
        let later = now + Duration::from_millis(300);
        reporting.on_event(&PlayerEvent::Volume(65.), &mut report, later);
        reporting.on_event(&PlayerEvent::Muted(true), &mut report, later);
        // Due a second after the first change, carrying the last state.
        assert_eq!(reporting.volume_due, Some(now + VOLUME_REPORT_AFTER));
        assert!(report.volume_level == 65 && report.is_muted);
        reporting.volume_reported();
        assert_eq!(reporting.volume_due, None);
    }

    fn stream(index: i32, kind: MediaStreamType, is_external: bool) -> MediaStream {
        serde_json::from_value(serde_json::json!({
            "Index": index,
            "Type": kind.as_str(),
            "IsExternal": is_external,
        }))
        .unwrap()
    }

    fn source() -> MediaSourceInfo {
        let source: MediaSourceInfo = serde_json::from_value(serde_json::json!({ "Id": "s" })).unwrap();
        MediaSourceInfo {
            media_streams: vec![
                stream(0, MediaStreamType::Subtitle, true),
                stream(1, MediaStreamType::Video, false),
                stream(2, MediaStreamType::Audio, false),
                stream(3, MediaStreamType::Audio, false),
                stream(4, MediaStreamType::Subtitle, false),
                stream(5, MediaStreamType::Subtitle, true),
            ],
            ..source
        }
    }

    #[test]
    fn a_restart_keeps_the_picture_choices_and_a_new_item_starts_fresh() {
        use super::{AbLoop, Aspect, Picture, StreamInfo, SubtitleChoice};
        use jellyfin::PlayMethod;

        let text = |index: i32| -> MediaStream {
            serde_json::from_value(serde_json::json!({
                "Index": index,
                "Type": "Subtitle",
                "IsTextSubtitleStream": true,
            }))
            .unwrap()
        };
        let source = MediaSourceInfo {
            media_streams: vec![text(7), text(8)],
            ..source()
        };
        let direct = StreamInfo::default();
        let at = Duration::from_secs;
        let carried = Picture {
            source_id: "s".into(),
            aspect: Aspect::Fill,
            deinterlace: false,
            deinterlace_chosen: true,
            secondary_subtitle: Some(8),
            ab_loop: AbLoop::Loop(at(30), at(60)),
        };

        // Everything comes back, the second subtitle too while its track is there.
        let again = Picture::starting(Some(&carried), &source, &direct, SubtitleChoice::Stream(7));
        assert_eq!(again, carried);
        // Not when it's the first subtitle now, or the track is gone.
        let first = Picture::starting(Some(&carried), &source, &direct, SubtitleChoice::Stream(8));
        assert_eq!(first.secondary_subtitle, None);
        assert_eq!(first.aspect, Aspect::Fill);
        let gone = MediaSourceInfo {
            media_streams: vec![text(7)],
            ..source.clone()
        };
        assert_eq!(
            Picture::starting(Some(&carried), &gone, &direct, SubtitleChoice::Off).secondary_subtitle,
            None
        );
        // A transcode only has the tracks it sends as files.
        let transcode = StreamInfo {
            method: PlayMethod::Transcode,
            subtitle_files: vec![7],
            ..StreamInfo::default()
        };
        let sent = Picture::starting(Some(&carried), &source, &transcode, SubtitleChoice::Off);
        assert_eq!(sent.secondary_subtitle, None);
        // The deinterlace choice is the user's, even when the video is interlaced.
        let interlaced = interlaced_source("s");
        assert!(!Picture::starting(Some(&carried), &interlaced, &direct, SubtitleChoice::Off).deinterlace);
        // One the user never made is worked out again, and follows the stream.
        let unchosen = Picture {
            deinterlace_chosen: false,
            ..carried.clone()
        };
        let worked_out = Picture::starting(Some(&unchosen), &interlaced, &direct, SubtitleChoice::Off);
        assert!(worked_out.deinterlace && !worked_out.deinterlace_chosen);

        // Without anything carried (another item), the defaults, and the stream's own deinterlacing.
        let fresh = Picture::starting(None, &interlaced, &direct, SubtitleChoice::Off);
        assert_eq!(
            fresh,
            Picture {
                source_id: "s".into(),
                deinterlace: true,
                ..Picture::default()
            }
        );
    }

    fn interlaced_source(id: &str) -> MediaSourceInfo {
        serde_json::from_value(serde_json::json!({
            "Id": id,
            "MediaStreams": [{ "Index": 0, "Type": "Video", "IsInterlaced": true }],
        }))
        .unwrap()
    }

    #[test]
    fn a_version_switch_carries_only_the_aspect() {
        use super::{AbLoop, Aspect, Picture, StreamInfo, SubtitleChoice};

        let direct = StreamInfo::default();
        let at = Duration::from_secs;
        // Version "s" was progressive, and the user never touched deinterlacing.
        let carried = Picture {
            source_id: "s".into(),
            aspect: Aspect::Fill,
            deinterlace: false,
            deinterlace_chosen: false,
            secondary_subtitle: Some(8),
            ab_loop: AbLoop::Loop(at(30), at(60)),
        };
        // Version "t" is interlaced and has a text track under the same index.
        let other: MediaSourceInfo = serde_json::from_value(serde_json::json!({
            "Id": "t",
            "MediaStreams": [
                { "Index": 0, "Type": "Video", "IsInterlaced": true },
                { "Index": 8, "Type": "Subtitle", "IsTextSubtitleStream": true },
            ],
        }))
        .unwrap();
        let started = Picture::starting(Some(&carried), &other, &direct, SubtitleChoice::Off);
        assert_eq!(started.aspect, Aspect::Fill);
        assert!(started.deinterlace, "worked out afresh for the new version");
        assert_eq!(started.secondary_subtitle, None);
        assert_eq!(started.ab_loop, AbLoop::Off);
        assert_eq!(started.source_id, "t");

        // Even a deinterlace choice the user made belongs to the old file.
        let chosen = Picture {
            deinterlace_chosen: true,
            ..carried
        };
        let started = Picture::starting(Some(&chosen), &other, &direct, SubtitleChoice::Off);
        assert!(started.deinterlace && !started.deinterlace_chosen);
    }

    #[test]
    fn a_frame_steps_pause_and_seek_go_unreported_but_the_users_own_are_not() {
        use tokio::time::Instant;

        let now = Instant::now();
        let later = |millis| now + Duration::from_millis(millis);
        let (pause, seek) = (PlayerEvent::Paused(false), PlayerEvent::Seeked);
        let mut quiet = StepQuiet::default();
        assert!(!quiet.hushes(&pause, now), "no step, no quiet");

        quiet.on_command(&UiCommand::FrameStep(true), now);
        assert!(quiet.hushes(&pause, later(100)) && quiet.hushes(&seek, later(100)));
        assert!(
            !quiet.hushes(&PlayerEvent::Volume(50.), later(100)),
            "only the pause and the seek"
        );
        assert!(!quiet.hushes(&pause, later(700)), "and not for long");

        // A pause, unpause or seek the user asks for inside the window ends it: its echo is
        // reported, as is the one after it.
        for command in [
            UiCommand::TogglePause,
            UiCommand::SetPause(false),
            UiCommand::SeekBy(10.),
            UiCommand::SeekTo(Duration::from_secs(5)),
        ] {
            quiet.on_command(&UiCommand::FrameStep(true), now);
            quiet.on_command(&command, later(50));
            assert!(!quiet.hushes(&pause, later(100)), "{command:?}");
            assert!(!quiet.hushes(&seek, later(100)), "{command:?}");
        }
        // Other commands leave it be.
        quiet.on_command(&UiCommand::FrameStep(false), now);
        quiet.on_command(&UiCommand::SetVolume(40.), later(50));
        assert!(quiet.hushes(&seek, later(100)));
    }

    #[test]
    fn the_ab_loop_sets_a_then_b_then_clears() {
        use super::AbLoop;

        let at = Duration::from_secs;
        let a = AbLoop::Off.next(at(30));
        assert_eq!(a, AbLoop::A(at(30)));
        assert_eq!(a.points(), Some((at(30), None)));
        let both = a.next(at(10));
        assert_eq!(both, AbLoop::Loop(at(30), at(10)));
        // Drawn earliest first, whichever was set first.
        assert_eq!(both.points(), Some((at(10), Some(at(30)))));
        assert_eq!(both.next(at(50)), AbLoop::Off);
        assert_eq!(AbLoop::Off.points(), None);
    }

    #[test]
    fn deinterlacing_starts_on_for_an_interlaced_stream_played_as_it_is() {
        use super::deinterlaces_by_default;
        use jellyfin::PlayMethod;

        let source = |interlaced: bool| -> MediaSourceInfo {
            serde_json::from_value(serde_json::json!({
                "Id": "s",
                "MediaStreams": [{ "Index": 0, "Type": "Video", "IsInterlaced": interlaced }],
            }))
            .unwrap()
        };
        assert!(deinterlaces_by_default(&source(true), PlayMethod::DirectPlay));
        assert!(!deinterlaces_by_default(&source(false), PlayMethod::DirectPlay));
        // A transcode is already progressive.
        assert!(!deinterlaces_by_default(&source(true), PlayMethod::Transcode));
        // The server leaves the field out of a stream it didn't probe.
        let unknown: MediaSourceInfo = serde_json::from_value(serde_json::json!({
            "Id": "s",
            "MediaStreams": [{ "Index": 0, "Type": "Video", "IsInterlaced": null }],
        }))
        .unwrap();
        assert!(!deinterlaces_by_default(&unknown, PlayMethod::DirectPlay));
    }

    #[test]
    fn maps_jellyfin_indexes_to_mpv_track_ids() {
        let source = source();
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 2), Some(1));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Audio, 3), Some(2));
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 4), Some(1));
        // External streams aren't in the file.
        assert_eq!(mpv_track_id(&source, MediaStreamType::Subtitle, 0), None);
    }

    #[test]
    fn external_subtitles_follow_embedded_ones() {
        let source = source();
        let files = [0, 5];
        assert_eq!(mpv_subtitle_id(&source, &files, 4), Some(1));
        assert_eq!(mpv_subtitle_id(&source, &files, 0), Some(2));
        assert_eq!(mpv_subtitle_id(&source, &files, 5), Some(3));
        assert_eq!(mpv_subtitle_id(&source, &files, 9), None);
        // mpv's order is the order they were added in, not the streams' order.
        assert_eq!(mpv_subtitle_id(&source, &[5, 0], 5), Some(2));
    }

    fn external(index: i32, path: &str) -> MediaStream {
        let mut stream = stream(index, MediaStreamType::Subtitle, true);
        stream.path = Some(path.into());
        stream
    }

    #[test]
    fn a_downloaded_subtitle_is_found_among_moved_indexes() {
        let with = |streams: Vec<MediaStream>| MediaSourceInfo {
            media_streams: streams,
            ..source()
        };
        let old = with(vec![
            stream(0, MediaStreamType::Video, false),
            external(1, "m.en.srt"),
            external(2, "m.pt.srt"),
        ]);
        // `m.es.srt` sorts between the two, so the Portuguese one moves up.
        let new = with(vec![
            stream(0, MediaStreamType::Video, false),
            external(1, "m.en.srt"),
            external(2, "m.es.srt"),
            external(3, "m.pt.srt"),
        ]);
        let (added, files) = added_subtitle(&old, &[1, 2], &new).expect("a new subtitle");
        assert_eq!((added.index, files), (2, vec![1, 3, 2]));
        // Nothing new: the same list again.
        assert!(added_subtitle(&old, &[1, 2], &old).is_none());
    }

    #[test]
    fn a_boosted_volume_reports_as_full() {
        assert_eq!(volume_level(volume_max()), 100);
        assert_eq!(volume_level(125.), 100);
    }

    #[test]
    fn the_reported_volume_is_a_whole_percent() {
        assert_eq!(volume_level(54.6), 55);
        assert_eq!(volume_level(100.4), 100);
        assert_eq!(volume_level(-3.), 0);
        assert_eq!(volume_level(150.), 100);
    }

    #[test]
    fn speed_steps_through_the_menu() {
        let at = |speed| Adjustments {
            speed,
            ..Adjustments::default()
        };
        assert_eq!(at(1.).step_speed(true), 1.25);
        assert_eq!(at(1.).step_speed(false), 0.75);
        assert_eq!(at(2.).step_speed(true), 2.);
        assert_eq!(at(0.5).step_speed(false), 0.5);
        // From a speed between two steps (set some other way), to the neighbouring ones.
        assert_eq!(at(1.1).step_speed(true), 1.25);
        assert_eq!(at(1.1).step_speed(false), 1.);
    }

    #[test]
    fn adjustments_stay_in_range_and_on_their_steps() {
        let wild = Adjustments {
            speed: 9.,
            subtitle_delay: 0.1 + 0.2,
            audio_delay: -100.,
            subtitle_scale: 0.1,
            subtitle_position: 250,
            night_mode: true,
        }
        .clamped();
        assert_eq!(wild.speed, 2.);
        assert_eq!(wild.subtitle_delay, 0.3);
        assert_eq!(wild.audio_delay, -30.);
        assert_eq!(wild.subtitle_scale, 0.5);
        // Nothing lower than the bottom, where mpv would cut text subtitles off.
        assert_eq!(wild.subtitle_position, 100);
        assert!(wild.night_mode);
    }

    #[test]
    fn the_next_item_keeps_speed_and_size_but_not_delays() {
        let next = Adjustments {
            speed: 1.5,
            subtitle_delay: 1.2,
            audio_delay: -0.3,
            subtitle_scale: 1.4,
            subtitle_position: 85,
            night_mode: true,
        }
        .for_next_item();
        assert_eq!((next.speed, next.subtitle_scale, next.night_mode), (1.5, 1.4, true));
        assert_eq!(next.subtitle_position, 85);
        assert_eq!((next.subtitle_delay, next.audio_delay), (0., 0.));
    }

    #[test]
    fn adjacent_items() {
        let item = |id: &str| -> jellyfin::models::BaseItemDto {
            serde_json::from_value(serde_json::json!({ "Id": id, "Name": id })).expect("item")
        };
        let ids = |pair: (
            Option<jellyfin::models::BaseItemDto>,
            Option<jellyfin::models::BaseItemDto>,
        )| { (pair.0.map(|i| i.id), pair.1.map(|i| i.id)) };
        let items = || vec![item("a"), item("b"), item("c")];
        assert_eq!(ids(adjacent(items(), "b")), (Some("a".into()), Some("c".into())));
        assert_eq!(ids(adjacent(items(), "a")), (None, Some("b".into())));
        assert_eq!(ids(adjacent(items(), "c")), (Some("b".into()), None));
        assert_eq!(ids(adjacent(items(), "x")), (None, None));
    }
}
