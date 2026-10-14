//! Bridge between GPUI's executor and tokio, which `reqwest` needs.
//!
//! GPUI code never awaits `jellyfin` calls directly. It wraps them in [`spawn`] and awaits the
//! returned future, which works from any executor.

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

/// Runs `fut` on the tokio runtime and returns a future resolving to its output.
pub fn spawn<F>(fut: F) -> impl Future<Output = F::Output> + 'static
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handle = runtime().spawn(fut);
    async move {
        match handle.await {
            Ok(output) => output,
            Err(err) => std::panic::resume_unwind(err.into_panic()),
        }
    }
}

/// Runs `fut` on the tokio runtime without waiting for it.
pub fn detach<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    drop(runtime().spawn(fut));
}
