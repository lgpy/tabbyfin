//! Locating, launching and connecting to the mpv process.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::oneshot;

use crate::{Error, Result, SpawnOptions};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const MPV_BINARY: &str = if cfg!(windows) { "mpv.exe" } else { "mpv" };

/// Finds mpv: an explicit path, then a bundled copy (next to the executable, or inside the
/// macOS app bundle as `Contents/Resources/mpv.app`), then `mpv` on the `PATH`.
pub fn locate_mpv(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        let dir = exe.parent()?;
        [
            dir.join(MPV_BINARY),
            dir.join("../Resources/mpv.app/Contents/MacOS/mpv"),
        ]
        .into_iter()
        .find(|path| path.is_file())
    });
    bundled.unwrap_or_else(|| PathBuf::from(MPV_BINARY))
}

#[cfg(unix)]
pub(crate) type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
pub(crate) type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Starts mpv and connects to its IPC socket. The returned sender kills mpv when dropped.
pub(crate) async fn spawn(options: &SpawnOptions) -> Result<(IpcStream, oneshot::Sender<()>)> {
    let mpv = locate_mpv(options.mpv_path.clone());
    let endpoint = ipc_endpoint();
    #[cfg(unix)]
    let _ = std::fs::remove_file(&endpoint);

    let mut child = Command::new(&mpv)
        .arg(format!("--input-ipc-server={endpoint}"))
        // Quit once the file ends; show the window while it loads.
        .args(["--idle=once", "--force-window=immediate", "--keep-open=no", "--terminal=no"])
        .args(&options.extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(mpv.clone()),
            _ => Error::Spawn(err),
        })?;
    tracing::info!(mpv = %mpv.display(), %endpoint, "started mpv");

    let stream = connect(&endpoint, &mut child).await?;

    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => tracing::info!("mpv exited: {status:?}"),
            _ = kill_rx => {
                let _ = child.kill().await;
            }
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(&endpoint);
    });
    Ok((stream, kill_tx))
}

fn ipc_endpoint() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "jelly-gpui-mpv-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    if cfg!(windows) {
        format!(r"\\.\pipe\{name}")
    } else {
        std::env::temp_dir().join(format!("{name}.sock")).display().to_string()
    }
}

/// mpv creates the socket shortly after starting, so retry until it appears.
async fn connect(endpoint: &str, child: &mut tokio::process::Child) -> Result<IpcStream> {
    let started = Instant::now();
    loop {
        match try_connect(endpoint).await {
            Ok(stream) => return Ok(stream),
            Err(_) if started.elapsed() < CONNECT_TIMEOUT => {
                if let Ok(Some(status)) = child.try_wait() {
                    tracing::warn!("mpv exited before accepting IPC connections: {status}");
                    return Err(Error::Connect);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => {
                tracing::warn!("couldn't connect to mpv at {endpoint}: {err}");
                return Err(Error::Connect);
            }
        }
    }
}

#[cfg(unix)]
async fn try_connect(endpoint: &str) -> std::io::Result<IpcStream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
async fn try_connect(endpoint: &str) -> std::io::Result<IpcStream> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
}
