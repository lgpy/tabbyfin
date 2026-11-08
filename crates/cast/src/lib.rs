//! The CASTV2 sender and the Jellyfin Chromecast receiver's message contract.
//!
//! There is no Google Cast SDK for the desktop, and on Android the SDK would mean Play Services,
//! Java glue and a second implementation, so the app talks the protocol itself (see
//! docs/chromecast.md, "Decision"). We don't use `rust_cast`: its TLS pulls in aws-lc-rs (a C/CMake
//! build that's painful to cross-compile for Android), it blocks, it can only send to `"*"`
//! rather than to an app's `transportId`, and it drops `MEDIA_STATUS`'s `customData`. The protocol
//! is small — a length-prefixed protobuf with seven fields, JSON payloads and four namespaces — so
//! this crate carries its own sender on `tokio` and `rustls` with the ring provider.
//!
//! The pieces:
//!
//! - [`codec`]: the `CastMessage` encoding and the 4-byte length-prefixed frames.
//! - [`channel`]: the TLS connection to a device. One task owns the socket, runs the heartbeat and
//!   matches the device's replies to requests; everything else arrives on a stream.
//! - [`receiver`]: the device's own namespace — status, launching and stopping apps, volume.
//! - [`discovery`]: `mdns-sd` browsing into a list of devices.
//! - [`jellyfin`]: the message contract of Jellyfin's receiver app (jellyfin-chromecast).
//!
//! Nothing here touches the UI, and every call is async: the app drives it from `task::Task`s.

pub mod channel;
pub mod codec;
pub mod discovery;
#[cfg(feature = "fake-device")]
pub mod fake;
pub mod jellyfin;
pub mod receiver;

pub use channel::{Channel, ChannelConfig, Message, NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_RECEIVER};
pub use codec::{CastMessage, MAX_FRAME, PayloadType};
pub use discovery::{Device, Discovery, DiscoveryEvent};
pub use receiver::{Application, ReceiverStatus, Volume};

/// Anything the cast crate can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("malformed CASTV2 message: {0}")]
    Malformed(&'static str),
    #[error("a frame of {len} bytes is over the {max} byte limit")]
    FrameTooLarge { len: usize, max: usize },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// The device connection is gone: the socket closed, the heartbeat timed out, or the
    /// [`Channel`] was dropped.
    #[error("the device connection is closed")]
    Closed,
    #[error("the device did not answer in time")]
    Timeout,
    /// A well-formed reply we didn't expect, or one without the fields it must have.
    #[error("unexpected device reply: {0}")]
    Device(String),
    /// The device answered a `LAUNCH` with `LAUNCH_ERROR`.
    #[error("the app failed to launch: {reason}")]
    Launch { reason: String },
    /// The device answered with `INVALID_REQUEST`.
    #[error("the device rejected the request: {reason}")]
    InvalidRequest { reason: String },
    #[error("discovery: {0}")]
    Discovery(String),
    /// Only the fake device mints certificates.
    #[error("certificate: {0}")]
    Certificate(String),
}

pub type Result<T> = std::result::Result<T, Error>;
