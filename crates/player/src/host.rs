//! The player host process: `tabbyfin --player-host`, started by [`crate::remote`].
//!
//! It runs libmpv and its video output exactly as the embedded backend does, with the kind of
//! video output the app asked for in its first message, then takes requests on stdin and sends
//! events, replies and frames on stdout (see [`crate::wire`]). If mpv or a GPU driver crashes,
//! only this process dies; the app notices the closed pipe and carries on.
//! It exits when stdin closes, i.e. when the app drops the player or exits itself.

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncWrite, BufReader, BufWriter};
use tokio::sync::Mutex;

use crate::libmpv::{Frame, VideoOutput};
use crate::wire::{self, Message, Request, WireError};
use crate::{Player, PlayerEvent};

/// The command-line argument that makes the app binary run as the host.
pub const HOST_ARG: &str = "--player-host";

type Output = Arc<Mutex<BufWriter<tokio::io::Stdout>>>;

/// Runs the host until the app closes stdin. Returns the process exit code.
pub fn run() -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!("couldn't start the player host runtime: {err}");
            return 1;
        }
    };
    runtime.block_on(serve())
}

async fn serve() -> i32 {
    let out: Output = Arc::new(Mutex::new(BufWriter::new(tokio::io::stdout())));
    let mut input = BufReader::new(tokio::io::stdin());
    // What the app can draw comes first, before libmpv starts.
    let import = match wire::read::<_, Request>(&mut input).await {
        Ok(Some((Request::Start { import }, _))) => import,
        Ok(Some((other, _))) => {
            tracing::warn!("player host: expected Start, got {other:?}");
            return 1;
        }
        // The app went away before starting us.
        Ok(None) => return 0,
        Err(err) => {
            tracing::warn!("player host: bad request: {err}");
            return 1;
        }
    };
    let (player, mut events, video) = match Player::spawn_embedded(import).await {
        Ok(started) => started,
        Err(err) => {
            let _ = send(&out, &Message::Started(Err(WireError::from(&err))), &[]).await;
            return 1;
        }
    };
    if send(&out, &Message::Started(Ok(())), &[]).await.is_err() {
        return 1;
    }
    let player = Arc::new(player);

    let forward_events = tokio::spawn({
        let out = out.clone();
        async move {
            while let Some(event) = events.recv().await {
                let last = event == PlayerEvent::Exited;
                if send(&out, &Message::Event(event), &[]).await.is_err() || last {
                    break;
                }
            }
        }
    });
    let forward_frames = tokio::spawn(forward_frames(out.clone(), video.clone()));

    loop {
        let request = match wire::read::<_, Request>(&mut input).await {
            Ok(Some((request, _))) => request,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!("player host: bad request: {err}");
                break;
            }
        };
        // In order: the app relies on properties being set before `loadfile`.
        let (id, result) = match request {
            Request::Start { .. } => {
                tracing::warn!("player host: started twice");
                continue;
            }
            Request::SetTarget { width, height } => {
                video.set_target_size(width, height);
                continue;
            }
            Request::Command { id, args } => (id, player.command(Value::Array(args)).await),
            Request::SetProperty { id, name, value } => (id, player.set_property(&name, value).await),
        };
        let reply = Message::Reply {
            id,
            result: result.map_err(|err| WireError::from(&err)),
        };
        if send(&out, &reply, &[]).await.is_err() {
            break;
        }
    }
    // The app is gone or done: stop mpv (dropping the player quits it) and exit.
    forward_frames.abort();
    forward_events.abort();
    drop(player);
    0
}

/// Sends the newest frame whenever there's one. Frames that come while a write is in progress
/// are skipped (the watch channel keeps only the latest), so a slow app never queues them up.
async fn forward_frames(out: Output, video: VideoOutput) {
    #[cfg(target_os = "linux")]
    let mut fds = FdSender::from_env();
    // The frames sent last, whose handles keep their textures' names open: after a resize the
    // render thread releases the old ring, while the app may not have opened the frame it just
    // got. A few frames later it has a newer one, so the old textures can go.
    #[cfg(windows)]
    let mut sent = std::collections::VecDeque::new();
    let mut frames = video.frames();
    while frames.changed().await.is_ok() {
        let Some(frame) = frames.borrow_and_update().clone() else {
            continue;
        };
        let sent = match frame {
            Frame::Pixels(frame) => {
                let message = Message::Pixels {
                    width: frame.width,
                    height: frame.height,
                };
                send(&out, &message, &frame.pixels).await
            }
            #[cfg(windows)]
            Frame::Shared(frame) => {
                let message = Message::SharedHandle {
                    id: frame.id,
                    name: frame.name.to_string(),
                    width: frame.width,
                    height: frame.height,
                };
                sent.push_back(frame);
                if sent.len() > KEEP_SENT {
                    sent.pop_front();
                }
                send(&out, &message, &[]).await
            }
            #[cfg(target_os = "linux")]
            Frame::Shared(frame) => {
                let Some(fds) = &mut fds else {
                    tracing::warn!("player host: no fd socket for GPU frames");
                    return;
                };
                if let Err(err) = fds.send_once(frame.id, &frame.fd) {
                    tracing::warn!("player host: couldn't pass a frame's memory: {err}");
                    return;
                }
                let message = Message::SharedFd {
                    id: frame.id,
                    allocation_size: frame.allocation_size,
                    width: frame.width,
                    height: frame.height,
                };
                send(&out, &message, &[]).await
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            Frame::Shared(_) => Ok(()),
        };
        if sent.is_err() {
            return;
        }
    }
}

/// How many of the frames it sent last the host keeps open for the app (Windows): two rings'
/// worth, a fraction of a second of video.
#[cfg(windows)]
const KEEP_SENT: usize = 6;

async fn send<W: AsyncWrite + Unpin>(out: &Mutex<W>, message: &Message, payload: &[u8]) -> std::io::Result<()> {
    let mut out = out.lock().await;
    wire::write(&mut *out, message, payload).await
}

/// The host's end of the fd socket (`TABBYFIN_PLAYER_FD`), for the Vulkan memory behind GPU
/// frames. Each image's fd is sent once, with its id.
#[cfg(target_os = "linux")]
struct FdSender {
    socket: std::os::fd::OwnedFd,
    sent: std::collections::HashSet<u64>,
}

#[cfg(target_os = "linux")]
impl FdSender {
    fn from_env() -> Option<Self> {
        use std::os::fd::FromRawFd;
        let fd: i32 = std::env::var(crate::remote::FD_ENV).ok()?.parse().ok()?;
        // SAFETY: the app leaves this fd open for us (the inherited end of a socket pair) and
        // nothing else in this process uses it.
        let socket = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        Some(Self {
            socket,
            sent: Default::default(),
        })
    }

    fn send_once(&mut self, id: u64, fd: &std::os::fd::OwnedFd) -> std::io::Result<()> {
        use std::io::IoSlice;
        use std::os::fd::AsFd;

        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};

        if !self.sent.insert(id) {
            return Ok(());
        }
        let fds = [fd.as_fd()];
        let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        ancillary.push(SendAncillaryMessage::ScmRights(&fds));
        let id = id.to_le_bytes();
        sendmsg(&self.socket, &[IoSlice::new(&id)], &mut ancillary, SendFlags::empty())?;
        Ok(())
    }
}
