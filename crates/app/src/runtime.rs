//! The tokio runtime, which `reqwest` needs.
//!
//! UI code never awaits `jellyfin` calls. It runs them here through a [`crate::task::Task`],
//! whose result it polls each frame.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tokio")
            .enable_all()
            .build()
            .expect("failed to start tokio runtime")
    })
}

/// Starts `fut` on the tokio runtime. Dropping the handle leaves it running; aborting it stops
/// the future at its next await.
pub fn start<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(fut)
}

/// Runs `fut` on the tokio runtime without waiting for it.
pub fn detach<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    drop(runtime().spawn(fut));
}
