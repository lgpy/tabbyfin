//! Async work and timers for UI code, which runs once a frame and never blocks or awaits.
//!
//! - [`Task`]: a future on the tokio runtime whose result the UI polls.
//! - [`channel`]: a stream of updates from tokio to the UI, drained each frame.
//! - [`Deadline`]: a timer checked each frame.
//!
//! Each wakes the UI when it has something new, so nothing needs to repaint every frame to
//! notice: the app stays near 0% CPU while idle.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use tokio::sync::{mpsc, oneshot};

use crate::runtime;

type Panic = Box<dyn std::any::Any + Send>;

/// A future running on tokio, owned by UI code that polls for its result each frame.
///
/// Dropping the task aborts the future, so work never outlives the page or service that asked
/// for it. Owners keep an `Option<Task<T>>`: replacing it cancels the work it held, and they
/// clear it once [`Task::poll`] has handed the result over.
#[must_use = "dropping a Task cancels it"]
pub struct Task<T> {
    result: oneshot::Receiver<Result<T, Panic>>,
    abort: tokio::task::AbortHandle,
}

impl<T: Send + 'static> Task<T> {
    /// Runs `future` on the tokio runtime, and repaints when it's done so the next frame's
    /// [`Task::poll`] finds the result.
    pub fn spawn<F>(ctx: &egui::Context, future: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let (sender, result) = oneshot::channel();
        let ctx = ctx.clone();
        let handle = runtime::start(async move {
            let output = AssertUnwindSafe(future).catch_unwind().await;
            // Sent before the repaint, so the frame it causes finds the result.
            if sender.send(output).is_ok() {
                ctx.request_repaint();
            }
        });
        Self {
            result,
            abort: handle.abort_handle(),
        }
    }
}

impl<T> Task<T> {
    /// The result, on the first call after the future finished; `None` before that and after.
    /// A panic in the future is raised again here, on the UI thread, rather than leaving its
    /// owner waiting forever.
    pub fn poll(&mut self) -> Option<T> {
        match self.result.try_recv() {
            Ok(Ok(output)) => Some(output),
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => None,
        }
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// A channel from async code to the UI whose sends wake the UI: for long-lived streams like a
/// playback session's updates or the session websocket's messages. `Services::tick` drains the
/// receivers; it runs from `App::logic`, which eframe also calls while the window is hidden, so
/// the stream keeps flowing when the window is minimised.
pub fn channel<T>(ctx: &egui::Context) -> (Sender<T>, Receiver<T>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        Sender {
            inner: sender,
            ctx: ctx.clone(),
        },
        Receiver(receiver),
    )
}

/// The sending half of a [`channel`].
pub struct Sender<T> {
    inner: mpsc::UnboundedSender<T>,
    ctx: egui::Context,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            ctx: self.ctx.clone(),
        }
    }
}

impl<T> Sender<T> {
    /// Sends `value` and wakes the UI. Gives it back if the receiver is gone, which means its
    /// owner was dropped and the sender should stop.
    pub fn send(&self, value: T) -> Result<(), T> {
        self.inner.send(value).map_err(|err| err.0)?;
        self.ctx.request_repaint();
        Ok(())
    }

    /// Resolves once the receiver is gone, for async code to stop work nobody listens to.
    pub async fn closed(&self) {
        self.inner.closed().await;
    }
}

/// The UI's half of a [`channel`].
pub struct Receiver<T>(mpsc::UnboundedReceiver<T>);

impl<T> Receiver<T> {
    /// Everything sent since the last call, oldest first. Never waits.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| self.0.try_recv().ok())
    }

    /// The next value, waiting for it; `None` once every sender is gone. For async code that
    /// takes a stream over from the UI (playback finishing its sessions after sign-out).
    pub async fn recv(&mut self) -> Option<T> {
        self.0.recv().await
    }
}

/// A timer for UI code (hiding the player's controls, debouncing a search, saving the window's
/// placement): set it, then check it with [`Deadline::fire`] each frame. While it's waiting
/// it asks egui to wake up when it's due, instead of anything sleeping or repainting meanwhile.
#[derive(Debug, Clone, Copy, Default)]
pub struct Deadline(Option<Instant>);

impl Deadline {
    /// Starts the timer, due `after` from now. Setting it again replaces the earlier time, so
    /// only the last of a burst of changes fires.
    pub fn set(&mut self, after: Duration) {
        self.0 = Some(Instant::now() + after);
    }

    /// Stops the timer, when the reason for it went away.
    pub fn cancel(&mut self) {
        self.0 = None;
    }

    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }

    /// True on the first check once the time has come, which also stops the timer. Before
    /// that, asks egui to run another frame when it's due.
    pub fn fire(&mut self, ctx: &egui::Context) -> bool {
        self.fire_at(ctx, Instant::now())
    }

    fn fire_at(&mut self, ctx: &egui::Context, now: Instant) -> bool {
        let Some(due) = self.0 else {
            return false;
        };
        match due.checked_duration_since(now) {
            Some(remaining) if !remaining.is_zero() => {
                ctx.request_repaint_after(remaining);
                false
            }
            _ => {
                self.0 = None;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::{Deadline, Task, channel};

    /// Polls `task` until it's done, as frames would.
    fn wait_for<T: Send + 'static>(task: &mut Task<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(output) = task.poll() {
                return output;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "the task never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_task_hands_its_result_over_once() {
        let ctx = egui::Context::default();
        let mut task = Task::spawn(&ctx, async { 42 });
        assert_eq!(wait_for(&mut task), 42);
        assert_eq!(task.poll(), None);
    }

    #[test]
    fn dropping_a_task_cancels_it() {
        let ctx = egui::Context::default();
        let finished = Arc::new(AtomicBool::new(false));
        let task = Task::spawn(&ctx, {
            let finished = finished.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                finished.store(true, Ordering::SeqCst);
            }
        });
        drop(task);
        std::thread::sleep(Duration::from_millis(150));
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn a_panic_reaches_the_poller() {
        let ctx = egui::Context::default();
        let mut task = Task::spawn(&ctx, async { panic!("boom") });
        wait_for::<()>(&mut task);
    }

    #[test]
    fn a_channel_drains_in_order() {
        let ctx = egui::Context::default();
        let (sender, mut receiver) = channel(&ctx);
        sender.send(1).unwrap();
        sender.send(2).unwrap();
        assert_eq!(receiver.drain().collect::<Vec<_>>(), [1, 2]);
        assert_eq!(receiver.drain().count(), 0);
        drop(receiver);
        assert_eq!(sender.send(3), Err(3));
    }

    #[test]
    fn a_channel_can_be_awaited_until_its_senders_are_gone() {
        let ctx = egui::Context::default();
        let (sender, mut receiver) = channel(&ctx);
        sender.send(1).unwrap();
        let mut received = Task::spawn(&ctx, async move {
            let mut values = Vec::new();
            while let Some(value) = receiver.recv().await {
                values.push(value);
            }
            values
        });
        sender.send(2).unwrap();
        drop(sender);
        assert_eq!(wait_for(&mut received), [1, 2]);
    }

    #[test]
    fn a_deadline_fires_once_when_due() {
        let ctx = egui::Context::default();
        let now = Instant::now();
        let mut deadline = Deadline::default();
        assert!(!deadline.fire_at(&ctx, now));
        deadline.set(Duration::from_secs(1));
        assert!(!deadline.fire_at(&ctx, now));
        assert!(deadline.fire_at(&ctx, now + Duration::from_secs(2)));
        assert!(!deadline.fire_at(&ctx, now + Duration::from_secs(3)));
        deadline.set(Duration::from_secs(1));
        deadline.cancel();
        assert!(!deadline.fire_at(&ctx, now + Duration::from_secs(3)));
    }
}
