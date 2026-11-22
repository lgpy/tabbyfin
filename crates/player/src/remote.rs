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
use crate::libmpv::{Frame, VideoFrame, VideoImport, VideoOutput};
use crate::wire::{self, Message, Request};
use crate::{EndReason, Error, PlayerEvent, Result};

/// Environment variable naming the host's end of the fd socket (Linux).
#[cfg(target_os = "linux")]
pub(crate) const FD_ENV: &str = "TABBYFIN_PLAYER_FD";

/// Requests waiting for the host's answer, by id. Closed once the host's output has ended: nothing
/// answers after that, so a request made then must fail at once. Checking and adding happen under
/// the same lock as closing, so none slips in between.
struct Waiting<T> {
    closed: bool,
    replies: HashMap<u64, oneshot::Sender<T>>,
}

impl<T> Default for Waiting<T> {
    fn default() -> Self {
        Self {
            closed: false,
            replies: HashMap::new(),
        }
    }
}

impl<T> Waiting<T> {
    /// Waits for the answer to `id`; `false` (and nothing waits) if the host is gone.
    fn add(&mut self, id: u64, reply: oneshot::Sender<T>) -> bool {
        if self.closed {
            return false;
        }
        self.replies.insert(id, reply);
        true
    }

    fn take(&mut self, id: u64) -> Option<oneshot::Sender<T>> {
        self.replies.remove(&id)
    }

    /// The host is gone: returns everything that waited, and refuses what comes after.
    fn close(&mut self) -> Vec<oneshot::Sender<T>> {
        self.closed = true;
        self.replies.drain().map(|(_, reply)| reply).collect()
    }
}

type Pending = Arc<Mutex<Waiting<Result<Option<String>>>>>;
type PendingProperties = Arc<Mutex<Waiting<Vec<Option<String>>>>>;

pub struct RemotePlayer {
    requests: mpsc::UnboundedSender<Request>,
    next_id: AtomicU64,
    pending: Pending,
    properties: PendingProperties,
    /// Killed when dropped, in case closing its stdin didn't end it.
    _child: Child,
}

impl RemotePlayer {
    /// Starts the host and tells it what the UI can draw (`import`), which picks its video
    /// output.
    pub async fn spawn(import: VideoImport) -> Result<(Self, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput)> {
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

        let mut stdin = BufWriter::new(child.stdin.take().ok_or(Error::Closed)?);
        let mut stdout = BufReader::new(child.stdout.take().ok_or(Error::Closed)?);
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward_log(stderr));
        }

        if let Err(err) = wire::write(&mut stdin, &Request::Start { import }, &[]).await {
            return Err(Error::Host(format!("the player process didn't start: {err}")));
        }
        match wire::read::<_, Message>(&mut stdout).await {
            Ok(Some((Message::Started(Ok(())), _))) => {}
            Ok(Some((Message::Started(Err(err)), _))) => return Err(err.into()),
            Ok(other) => return Err(Error::Host(format!("unexpected first message: {other:?}"))),
            Err(err) => return Err(Error::Host(format!("the player process didn't start: {err}"))),
        }

        let (requests, requests_rx) = mpsc::unbounded_channel();
        tokio::spawn(write_requests(stdin, requests_rx));

        let (events_tx, events) = mpsc::unbounded_channel();
        let (frames_tx, frames) = watch::channel(None);
        let pending: Pending = Default::default();
        let properties: PendingProperties = Default::default();
        tokio::spawn(read_messages(
            stdout,
            events_tx,
            frames_tx,
            (pending.clone(), properties.clone()),
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
            properties,
            _child: child,
        };
        Ok((player, events, video))
    }

    pub async fn command(&self, args: Vec<Value>) -> Result<()> {
        self.call(|id| Request::Command { id, args }).await.map(|_| ())
    }

    pub async fn set_property(&self, name: &str, value: Value) -> Result<()> {
        let name = name.to_owned();
        self.call(|id| Request::SetProperty { id, name, value })
            .await
            .map(|_| ())
    }

    /// mpv properties as text, in the order asked; `None` for one mpv has no value for, and for
    /// all of them if the host is gone.
    pub async fn properties(&self, names: &[&str]) -> Vec<Option<String>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply) = oneshot::channel();
        if !self.properties.lock().unwrap().add(id, reply_tx) {
            return vec![None; names.len()];
        }
        let names_owned = names.iter().map(|name| (*name).to_owned()).collect();
        let request = Request::GetProperties { id, names: names_owned };
        if self.requests.send(request).is_err() {
            self.properties.lock().unwrap().take(id);
            return vec![None; names.len()];
        }
        reply.await.unwrap_or_else(|_| vec![None; names.len()])
    }

    pub async fn get_property(&self, name: &str) -> Result<String> {
        let name = name.to_owned();
        let reply = self.call(|id| Request::GetProperty { id, name }).await?;
        reply.ok_or_else(|| Error::Host("the player process sent no value".into()))
    }

    async fn call(&self, request: impl FnOnce(u64) -> Request) -> Result<Option<String>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply) = oneshot::channel();
        if !self.pending.lock().unwrap().add(id, reply_tx) {
            return Err(Error::Closed);
        }
        if self.requests.send(request(id)).is_err() {
            self.pending.lock().unwrap().take(id);
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
    (pending, properties): (Pending, PendingProperties),
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
                if let Some(reply) = pending.lock().unwrap().take(id) {
                    let _ = reply.send(result.map_err(Error::from));
                }
            }
            Message::Properties { id, values } => {
                if let Some(reply) = properties.lock().unwrap().take(id) {
                    let _ = reply.send(values);
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
            Message::SharedHandle {
                id,
                name,
                width,
                height,
            } => {
                let frame = SharedFrame {
                    id,
                    name: name.into(),
                    width,
                    height,
                    // The host's to keep (`host.rs`); the app opens the texture by its name.
                    _handle: None,
                };
                frames.send_replace(Some(Frame::Shared(frame)));
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
    let waiting = pending.lock().unwrap().close();
    for reply in waiting {
        let _ = reply.send(Err(Error::Closed));
    }
    // Dropping the senders answers the property queries with nothing.
    drop(properties.lock().unwrap().close());
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

/// Fds received from the host, by image id. The newest few are kept for frames still to come;
/// each frame holds its own reference, so the UI can import (duplicate) the fd of any frame it
/// has, however many images came since.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct FdTable {
    fds: Mutex<Vec<(u64, Arc<std::os::fd::OwnedFd>)>>,
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
                        table.push((id, Arc::new(fd)));
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
    async fn get(&self, id: u64) -> Option<Arc<std::os::fd::OwnedFd>> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let arrived = self.arrived.notified();
            if let Some((_, fd)) = self.fds.lock().unwrap().iter().find(|(i, _)| *i == id) {
                return Some(fd.clone());
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A host that does nothing and exits, so its output ends at once.
    fn exiting_host() -> Command {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/C", "exit"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = Command::new("true");
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true);
        command
    }

    /// A request made after the host's output has ended (the reader has already answered what was
    /// waiting) fails at once. Its write can still succeed, since the writer only learns of the
    /// dead pipe on a later write, and nothing would ever answer it: the playback info overlay
    /// asks every second from inside the session's loop, so a hang here would stall that loop.
    #[test]
    fn requests_after_the_host_is_gone_fail_instead_of_waiting() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let (done, result) = std::sync::mpsc::channel();
        // On its own thread, so that a hang fails the test instead of hanging it.
        std::thread::spawn(move || {
            let outcome = runtime.block_on(async {
                let mut child = exiting_host().spawn().expect("a child process");
                let stdout = BufReader::new(child.stdout.take().expect("its output"));
                let (events, _events_rx) = mpsc::unbounded_channel();
                let (frames, _frames_rx) = watch::channel(None);
                let (pending, properties): (Pending, PendingProperties) = Default::default();
                read_messages(
                    stdout,
                    events,
                    frames,
                    (pending.clone(), properties.clone()),
                    #[cfg(target_os = "linux")]
                    Arc::new(FdTable::default()),
                )
                .await;

                // The writer's channel is still open, as it is until a write to the pipe fails.
                let (requests, _requests_rx) = mpsc::unbounded_channel();
                let player = RemotePlayer {
                    requests,
                    next_id: AtomicU64::new(1),
                    pending,
                    properties,
                    _child: child,
                };
                let values = player.properties(&["a", "b"]).await;
                let set = player.set_property("pause", Value::Bool(true)).await;
                let command = player.command(vec![Value::from("stop")]).await;
                (values, set, command)
            });
            let _ = done.send(outcome);
        });
        let (values, set, command) = result
            .recv_timeout(Duration::from_secs(5))
            .expect("a request after the host's output ended never completed");
        assert_eq!(values, [None, None]);
        assert!(matches!(set, Err(Error::Closed)), "{set:?}");
        assert!(matches!(command, Err(Error::Closed)), "{command:?}");
    }
}
