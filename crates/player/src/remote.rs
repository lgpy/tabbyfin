//! The app's side of the player host process (see [`crate::host`]).
//!
//! [`RemotePlayer::spawn`] starts this executable again with `--player-host` and talks to it
//! over its stdin and stdout. If the host dies (an mpv or GPU driver crash), its events end
//! with an [`EndReason::Error`] and [`PlayerEvent::Exited`], like a failed stream, and the app
//! keeps running.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, watch};

use crate::host::HOST_ARG;
#[cfg(any(windows, target_os = "linux"))]
use crate::libmpv::SharedFrame;
use crate::libmpv::{Frame, VideoFrame, VideoOutput};
use crate::wire::{self, Message, Request};
use crate::{EndReason, Error, PlayerEvent, Result};

/// Environment variable naming the host's end of the fd socket (Linux).
#[cfg(target_os = "linux")]
pub(crate) const FD_ENV: &str = "TABBYFIN_PLAYER_FD";

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<()>>>>>;

pub struct RemotePlayer {
    requests: mpsc::UnboundedSender<Request>,
    next_id: AtomicU64,
    pending: Pending,
    /// Killed when dropped, in case closing its stdin didn't end it.
    _child: Child,
}

impl RemotePlayer {
    pub async fn spawn() -> Result<(Self, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput)> {
        let exe = std::env::current_exe().map_err(Error::Spawn)?;
        let mut command = Command::new(exe);
        command
            .arg(HOST_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(target_os = "linux")]
        let fd_socket = FdSocket::new(&mut command).map_err(Error::Spawn)?;
        let mut child = command.spawn().map_err(Error::Spawn)?;
        #[cfg(target_os = "linux")]
        let fds = fd_socket.listen();

        let stdin = child.stdin.take().ok_or(Error::Closed)?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or(Error::Closed)?);
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward_log(stderr));
        }

        match wire::read::<_, Message>(&mut stdout).await {
            Ok(Some((Message::Started(Ok(())), _))) => {}
            Ok(Some((Message::Started(Err(err)), _))) => return Err(err.into()),
            Ok(other) => return Err(Error::Host(format!("unexpected first message: {other:?}"))),
            Err(err) => return Err(Error::Host(format!("the player process didn't start: {err}"))),
        }

        let (requests, requests_rx) = mpsc::unbounded_channel();
        tokio::spawn(write_requests(BufWriter::new(stdin), requests_rx));

        let (events_tx, events) = mpsc::unbounded_channel();
        let (frames_tx, frames) = watch::channel(None);
        let pending: Pending = Default::default();
        tokio::spawn(read_messages(
            stdout,
            events_tx,
            frames_tx,
            pending.clone(),
            #[cfg(target_os = "linux")]
            fds,
        ));

        let set_target = {
            let requests = requests.clone();
            let last = Mutex::new((0, 0));
            Arc::new(move |width, height| {
                let mut last = last.lock().unwrap();
                if *last != (width, height) {
                    *last = (width, height);
                    let _ = requests.send(Request::SetTarget { width, height });
                }
            })
        };
        let video = VideoOutput::remote(frames, set_target);
        let player = Self {
            requests,
            next_id: AtomicU64::new(1),
            pending,
            _child: child,
        };
        Ok((player, events, video))
    }

    pub async fn command(&self, args: Vec<Value>) -> Result<()> {
        self.call(|id| Request::Command { id, args }).await
    }

    pub async fn set_property(&self, name: &str, value: Value) -> Result<()> {
        let name = name.to_owned();
        self.call(|id| Request::SetProperty { id, name, value }).await
    }

    async fn call(&self, request: impl FnOnce(u64) -> Request) -> Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, reply_tx);
        if self.requests.send(request(id)).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(Error::Closed);
        }
        reply.await.map_err(|_| Error::Closed)?
    }
}

async fn write_requests(mut stdin: BufWriter<ChildStdin>, mut requests: mpsc::UnboundedReceiver<Request>) {
    while let Some(request) = requests.recv().await {
        if wire::write(&mut stdin, &request, &[]).await.is_err() {
            break;
        }
    }
    // Dropping stdin tells the host to stop.
}

async fn read_messages(
    mut stdout: BufReader<tokio::process::ChildStdout>,
    events: mpsc::UnboundedSender<PlayerEvent>,
    frames: watch::Sender<Option<Frame>>,
    pending: Pending,
    #[cfg(target_os = "linux")] fds: Arc<FdTable>,
) {
    let mut exited = false;
    loop {
        let (message, payload) = match wire::read::<_, Message>(&mut stdout).await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!("player process: {err}");
                break;
            }
        };
        match message {
            Message::Event(event) => {
                exited |= event == PlayerEvent::Exited;
                let _ = events.send(event);
            }
            Message::Reply { id, result } => {
                if let Some(reply) = pending.lock().unwrap().remove(&id) {
                    let _ = reply.send(result.map_err(Error::from));
                }
            }
            Message::Pixels { width, height } => {
                let frame = VideoFrame {
                    width,
                    height,
                    pixels: payload,
                };
                frames.send_replace(Some(Frame::Pixels(Arc::new(frame))));
            }
            #[cfg(windows)]
            Message::SharedHandle { handle, width, height } => {
                frames.send_replace(Some(Frame::Shared(SharedFrame { handle, width, height })));
            }
            #[cfg(target_os = "linux")]
            Message::SharedFd {
                id,
                allocation_size,
                width,
                height,
            } => match fds.get(id).await {
                Some(fd) => {
                    let frame = SharedFrame {
                        id,
                        fd,
                        allocation_size,
                        width,
                        height,
                    };
                    frames.send_replace(Some(Frame::Shared(frame)));
                }
                None => tracing::warn!(id, "player process: a frame's memory never arrived"),
            },
            Message::Started(_) => {}
            #[allow(unreachable_patterns)]
            other => tracing::warn!("player process: unexpected message {other:?}"),
        }
    }
    // The host is gone. If mpv hadn't finished, it crashed: end like a failed stream.
    if !exited {
        tracing::error!("{}", crate::HOST_CRASHED);
        let reason = EndReason::Error(crate::HOST_CRASHED.into());
        let _ = events.send(PlayerEvent::EndFile(reason));
        let _ = events.send(PlayerEvent::Exited);
    }
    for (_, reply) in pending.lock().unwrap().drain() {
        let _ = reply.send(Err(Error::Closed));
    }
}

/// Logs what the host writes to stderr (its own log, already formatted) under `player`.
async fn forward_log(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // `LEVEL target: message`, as the host's logger writes it.
        let line = line.trim();
        let (level, rest) = line.split_once(' ').unwrap_or(("", line));
        let rest = rest.trim_start();
        match level {
            "ERROR" => tracing::error!(target: "player", "{rest}"),
            "WARN" => tracing::warn!(target: "player", "{rest}"),
            "DEBUG" | "TRACE" => tracing::debug!(target: "player", "{rest}"),
            "INFO" => tracing::info!(target: "player", "{rest}"),
            _ => tracing::info!(target: "player", "{line}"),
        }
    }
}

/// The fd socket: a socket pair whose host end the child inherits (`TABBYFIN_PLAYER_FD`).
/// The host sends each GPU image's memory fd over it once (SCM_RIGHTS), tagged with its id.
#[cfg(target_os = "linux")]
struct FdSocket {
    ours: std::os::fd::OwnedFd,
    theirs: std::os::fd::OwnedFd,
}

#[cfg(target_os = "linux")]
impl FdSocket {
    fn new(command: &mut Command) -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;

        use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};

        let (ours, theirs) = socketpair(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::CLOEXEC, None)?;
        // The child's end must survive exec.
        rustix::io::fcntl_setfd(&theirs, rustix::io::FdFlags::empty())?;
        command.env(FD_ENV, theirs.as_raw_fd().to_string());
        Ok(Self { ours, theirs })
    }

    /// After the child started: closes our copy of its end and receives fds on a thread.
    fn listen(self) -> Arc<FdTable> {
        drop(self.theirs);
        let table = Arc::new(FdTable::default());
        let receiver = table.clone();
        let socket = self.ours;
        let _ = std::thread::Builder::new()
            .name("player-fds".into())
            .spawn(move || receiver.receive(socket));
        table
    }
}

/// Fds received from the host, by image id. The newest few are kept open, since GPUI imports
/// (duplicates) an image's fd when it first draws it.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct FdTable {
    fds: Mutex<Vec<(u64, std::os::fd::OwnedFd)>>,
    arrived: tokio::sync::Notify,
}

#[cfg(target_os = "linux")]
impl FdTable {
    /// Images kept: the render thread's ring of three, plus ones from recent resizes.
    const KEEP: usize = 8;

    fn receive(&self, socket: std::os::fd::OwnedFd) {
        use std::io::IoSliceMut;
        use std::mem::MaybeUninit;

        use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};

        loop {
            let mut id = [0u8; 8];
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut space);
            let received = match recvmsg(
                &socket,
                &mut [IoSliceMut::new(&mut id)],
                &mut ancillary,
                RecvFlags::CMSG_CLOEXEC,
            ) {
                Ok(received) if received.bytes == id.len() => received,
                // Closed (the host exited) or broken.
                _ => return,
            };
            let _ = received;
            let id = u64::from_le_bytes(id);
            for message in ancillary.drain() {
                if let RecvAncillaryMessage::ScmRights(fds) = message {
                    for fd in fds {
                        let mut table = self.fds.lock().unwrap();
                        table.push((id, fd));
                        if table.len() > Self::KEEP {
                            table.remove(0);
                        }
                    }
                }
            }
            self.arrived.notify_waiters();
        }
    }

    /// The fd for image `id`, waiting briefly for it (it travels on a separate socket).
    async fn get(&self, id: u64) -> Option<i32> {
        use std::os::fd::AsRawFd;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let arrived = self.arrived.notified();
            if let Some((_, fd)) = self.fds.lock().unwrap().iter().find(|(i, _)| *i == id) {
                return Some(fd.as_raw_fd());
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                return None;
            }
        }
    }
}
