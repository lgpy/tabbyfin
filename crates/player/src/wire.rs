//! Messages between the app and the player host process, over the host's stdin and stdout.
//!
//! Each message is a length-prefixed JSON header, then a length-prefixed payload (empty except
//! for software frames, whose pixels it carries). Lengths are little-endian `u32`s.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, PlayerEvent};

/// Largest header or payload accepted (an 8K frame is about 130 MB).
const MAX_LEN: usize = 256 << 20;

/// App to host.
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// An mpv command; answered with [`Message::Reply`].
    Command { id: u64, args: Vec<Value> },
    /// An mpv property; answered with [`Message::Reply`].
    SetProperty { id: u64, name: String, value: Value },
    /// The video's on-screen size in device pixels.
    SetTarget { width: u32, height: u32 },
}

/// Host to app.
#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    /// Sent once: libmpv and the video output are up, or why not.
    Started(Result<(), WireError>),
    Reply {
        id: u64,
        result: Result<(), WireError>,
    },
    Event(PlayerEvent),
    /// A software-rendered frame; the BGRA pixels are the payload.
    Pixels {
        width: u32,
        height: u32,
    },
    /// A frame in a D3D11 texture with a legacy share handle, which any process can open.
    SharedHandle {
        handle: usize,
        width: u32,
        height: u32,
    },
    /// A frame in exported Vulkan memory. The memory's fd for `id` is sent once, before the
    /// first frame that uses it, over the fd socket (`remote.rs`).
    SharedFd {
        id: u64,
        allocation_size: u64,
        width: u32,
        height: u32,
    },
}

/// [`Error`] across the process boundary.
#[derive(Debug, Serialize, Deserialize)]
pub enum WireError {
    LibmpvUnavailable(String),
    Command { command: String, message: String },
    Closed,
    Other(String),
}

impl From<&Error> for WireError {
    fn from(err: &Error) -> Self {
        match err {
            Error::LibmpvUnavailable(reason) => WireError::LibmpvUnavailable(reason.clone()),
            Error::Command { command, message } => WireError::Command {
                command: command.clone(),
                message: message.clone(),
            },
            Error::Closed => WireError::Closed,
            other => WireError::Other(other.to_string()),
        }
    }
}

impl From<WireError> for Error {
    fn from(err: WireError) -> Self {
        match err {
            WireError::LibmpvUnavailable(reason) => Error::LibmpvUnavailable(reason),
            WireError::Command { command, message } => Error::Command { command, message },
            WireError::Closed => Error::Closed,
            WireError::Other(message) => Error::Host(message),
        }
    }
}

pub async fn write<W, T>(writer: &mut W, header: &T, payload: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let header = serde_json::to_vec(header).map_err(std::io::Error::other)?;
    writer.write_all(&(header.len() as u32).to_le_bytes()).await?;
    writer.write_all(&header).await?;
    writer.write_all(&(payload.len() as u32).to_le_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

/// Reads one message. `Ok(None)` at a clean end of the stream.
pub async fn read<R, T>(reader: &mut R) -> std::io::Result<Option<(T, Vec<u8>)>>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let Some(header) = read_block(reader, true).await? else {
        return Ok(None);
    };
    let payload = read_block(reader, false).await?.unwrap_or_default();
    let header = serde_json::from_slice(&header).map_err(std::io::Error::other)?;
    Ok(Some((header, payload)))
}

async fn read_block<R: AsyncRead + Unpin>(reader: &mut R, eof_ok: bool) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(err) if eof_ok && err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_LEN {
        return Err(std::io::Error::other(format!("message of {len} bytes")));
    }
    let mut block = vec![0; len];
    reader.read_exact(&mut block).await?;
    Ok(Some(block))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Message, Request, read, write};
    use crate::PlayerEvent;

    #[tokio::test]
    async fn round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        write(&mut a, &Message::Pixels { width: 2, height: 1 }, &[1; 8])
            .await
            .unwrap();
        write(
            &mut a,
            &Message::Event(PlayerEvent::Position(Duration::from_millis(1500))),
            &[],
        )
        .await
        .unwrap();
        let request = Request::Command {
            id: 7,
            args: vec!["seek".into(), 10.into()],
        };
        write(&mut a, &request, &[]).await.unwrap();
        drop(a);

        let (message, payload): (Message, _) = read(&mut b).await.unwrap().unwrap();
        assert!(matches!(message, Message::Pixels { width: 2, height: 1 }));
        assert_eq!(payload, [1; 8]);
        let (message, _): (Message, _) = read(&mut b).await.unwrap().unwrap();
        assert!(matches!(message, Message::Event(PlayerEvent::Position(p)) if p == Duration::from_millis(1500)));
        let (request, _): (Request, _) = read(&mut b).await.unwrap().unwrap();
        assert!(matches!(request, Request::Command { id: 7, .. }));
        assert!(read::<_, Message>(&mut b).await.unwrap().is_none());
    }
}
