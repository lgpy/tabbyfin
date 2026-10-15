//! Jellyfin API client. Kept free of GPUI and mpv so it can be built and tested on its own.
//!
//! See `docs/PLAN.md` §4. Only the `Authorization: MediaBrowser ...` header (or the `ApiKey`
//! query parameter) is used; legacy auth methods are disabled by default since Jellyfin 12.0.
//!
//! All requests are async and use `reqwest`, so they must run inside a tokio runtime.

mod auth;
mod client;
mod error;
mod images;
mod items;
pub mod models;
mod playback;
mod remote;
mod socket;
mod syncplay;
mod system;
mod ticks;
mod time;

pub use client::{Client, ClientInfo, parse_server_url};
pub use error::{Error, Result};
pub use images::ImageRequest;
pub use items::{ItemsQuery, SortOrder};
pub use playback::{PlayMethod, PlaybackInfoRequest, PlaybackInfoResponse, PlaybackReport};
pub use remote::{Capabilities, GeneralCommand, PlayCommand, PlayRequest, PlaystateCommand, PlaystateRequest};
pub use socket::{ServerMessage, SocketEvent};
pub use syncplay::{BufferReport, Command, CommandKind, GroupInfo, GroupState, GroupUpdate, PlayQueue, QueueItem};
pub use system::{MIN_SUPPORTED_VERSION, ServerVersion};
pub use ticks::Ticks;
pub use time::{ClockSample, ServerClock};
