//! Video output. For now this is libmpv's software renderer: mpv decodes and scales each frame
//! into a CPU buffer on a render thread, and frames are published on a `watch` channel for the
//! UI to upload. GPU texture sharing replaces this per platform (`docs/in-app-video.md`).

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use tokio::sync::watch;

use super::{Core, RenderParam, cstring};
use crate::{Error, Result};

const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_SW_SIZE: c_int = 17;
const PARAM_SW_FORMAT: c_int = 18;
const PARAM_SW_STRIDE: c_int = 19;
const PARAM_SW_POINTER: c_int = 20;
const UPDATE_FRAME: u64 = 1;

/// Rendering size used until the UI reports its size.
const DEFAULT_TARGET: (u32, u32) = (1920, 1080);

/// A frame in BGRA order with opaque alpha, tightly packed (`width * 4` bytes per row).
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Handle to the video of an [`super::EmbeddedPlayer`]. Cheap to clone; the renderer itself
/// stops when the player is dropped.
#[derive(Clone)]
pub struct VideoOutput {
    inner: Arc<Renderer>,
    frames: watch::Receiver<Option<Arc<VideoFrame>>>,
}

struct Renderer {
    core: Arc<Core>,
    context: *mut c_void,
    wakeup: Wakeup,
    /// Requested output size in device pixels, packed as `w << 32 | h`.
    target: AtomicU64,
    stop: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

// SAFETY: the render context is only used from the render thread, and freed in `Drop` after
// that thread is joined.
unsafe impl Send for Renderer {}
unsafe impl Sync for Renderer {}

#[derive(Default)]
struct Wakeup {
    pending: Mutex<bool>,
    signal: Condvar,
}

impl Wakeup {
    fn notify(&self) {
        *self.pending.lock().unwrap() = true;
        self.signal.notify_one();
    }
}

unsafe extern "C" fn on_update(ctx: *mut c_void) {
    // SAFETY: `ctx` is the `Wakeup` inside the `Renderer`, which clears the callback before
    // it's freed.
    let wakeup = unsafe { &*(ctx as *const Wakeup) };
    wakeup.notify();
}

/// Stops and joins the render thread when dropped. Owned by the player, so rendering ends
/// with playback even while the UI still holds [`VideoOutput`] clones.
pub(crate) struct RenderThread(Arc<Renderer>);

impl Drop for RenderThread {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
        self.0.wakeup.notify();
        if let Some(thread) = self.0.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

impl VideoOutput {
    pub(crate) fn new(core: Arc<Core>) -> Result<(Self, RenderThread)> {
        let api_type = cstring("sw");
        let mut params = [
            RenderParam {
                kind: PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            RenderParam {
                kind: PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let mut context = ptr::null_mut();
        // SAFETY: valid core handle and parameter list.
        let code = unsafe { (core.api.render_create)(&mut context, core.mpv, params.as_mut_ptr()) };
        core.api.check("mpv_render_context_create", code)?;

        let (frames_tx, frames) = watch::channel(None);
        let inner = Arc::new(Renderer {
            core,
            context,
            wakeup: Wakeup::default(),
            target: AtomicU64::new(pack(DEFAULT_TARGET)),
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
        });
        // SAFETY: the callback context (`inner.wakeup`) outlives the render context.
        unsafe {
            (inner.core.api.render_set_update_callback)(
                context,
                Some(on_update),
                &inner.wakeup as *const Wakeup as *mut c_void,
            );
        }
        let thread = std::thread::Builder::new()
            .name("mpv-render".into())
            .spawn({
                let inner = inner.clone();
                move || render_loop(&inner, &frames_tx)
            })
            .map_err(Error::Spawn)?;
        *inner.thread.lock().unwrap() = Some(thread);
        Ok((Self { inner: inner.clone(), frames }, RenderThread(inner)))
    }

    /// Latest frame, updated as mpv renders.
    pub fn frames(&self) -> watch::Receiver<Option<Arc<VideoFrame>>> {
        self.frames.clone()
    }

    /// Size of the area the video is shown in, in device pixels. Frames are rendered to fit
    /// it (never larger than the video), and the current frame is redrawn on change.
    pub fn set_target_size(&self, width: u32, height: u32) {
        let packed = pack((width.max(2), height.max(2)));
        if self.inner.target.swap(packed, Ordering::Relaxed) != packed {
            self.inner.wakeup.notify();
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: the last reference is gone, so the render thread has exited; freeing the
        // context here also happens before `core` (a field) can destroy the core.
        unsafe {
            (self.core.api.render_set_update_callback)(self.context, None, ptr::null_mut());
            (self.core.api.render_free)(self.context);
        }
    }
}

fn pack((w, h): (u32, u32)) -> u64 {
    (w as u64) << 32 | h as u64
}

fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

fn render_loop(renderer: &Renderer, frames: &watch::Sender<Option<Arc<VideoFrame>>>) {
    let mut surface = Surface::default();
    let mut last_size = None;
    loop {
        {
            let mut pending = renderer.wakeup.pending.lock().unwrap();
            while !*pending && !renderer.stop.load(Ordering::Relaxed) {
                pending = renderer.wakeup.signal.wait(pending).unwrap();
            }
            *pending = false;
        }
        if renderer.stop.load(Ordering::Relaxed) {
            break;
        }
        // SAFETY: render context calls happen only on this thread.
        let flags = unsafe { (renderer.core.api.render_update)(renderer.context) };
        let Some(size) = fit(renderer) else { continue };
        // Redraw on a new frame, or when the output size changed (e.g. while paused).
        if flags & UPDATE_FRAME == 0 && last_size == Some(size) {
            continue;
        }
        last_size = Some(size);
        if let Some(frame) = surface.render(renderer, size) {
            frames.send_replace(Some(Arc::new(frame)));
        }
    }
}

/// The video's display size scaled down to fit the target, or `None` without video.
fn fit(renderer: &Renderer) -> Option<(u32, u32)> {
    let w = renderer.core.get_i64("dwidth").filter(|v| *v > 0)? as f64;
    let h = renderer.core.get_i64("dheight").filter(|v| *v > 0)? as f64;
    let (max_w, max_h) = unpack(renderer.target.load(Ordering::Relaxed));
    let scale = (max_w as f64 / w).min(max_h as f64 / h).min(1.0);
    Some(((w * scale).round().max(2.) as u32, (h * scale).round().max(2.) as u32))
}

/// 64-byte aligned render target, as the software renderer prefers.
#[derive(Default)]
struct Surface {
    buffer: Vec<Aligned>,
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct Aligned([u8; 64]);

impl Surface {
    fn render(&mut self, renderer: &Renderer, (width, height): (u32, u32)) -> Option<VideoFrame> {
        let row = width as usize * 4;
        let stride = row.div_ceil(64) * 64;
        self.buffer.resize((stride * height as usize).div_ceil(64), Aligned([0; 64]));

        let mut size = [width as c_int, height as c_int];
        let format = cstring("bgr0");
        let mut stride_param = stride;
        let mut params = [
            RenderParam { kind: PARAM_SW_SIZE, data: size.as_mut_ptr() as *mut c_void },
            RenderParam { kind: PARAM_SW_FORMAT, data: format.as_ptr() as *mut c_void },
            RenderParam { kind: PARAM_SW_STRIDE, data: &mut stride_param as *mut usize as *mut c_void },
            RenderParam { kind: PARAM_SW_POINTER, data: self.buffer.as_mut_ptr() as *mut c_void },
            RenderParam { kind: PARAM_INVALID, data: ptr::null_mut() },
        ];
        let api = &renderer.core.api;
        // SAFETY: the buffer holds `stride * height` bytes and outlives the call.
        let code = unsafe { (api.render_render)(renderer.context, params.as_mut_ptr()) };
        if let Err(err) = api.check("mpv_render_context_render", code) {
            tracing::warn!("{err}");
            return None;
        }

        // SAFETY: the chunks are plain bytes; view them as one contiguous byte slice.
        let bytes = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr() as *const u8, stride * height as usize) };
        let mut pixels = Vec::with_capacity(row * height as usize);
        for line in bytes.chunks_exact(stride) {
            pixels.extend_from_slice(&line[..row]);
        }
        // "bgr0" leaves the fourth byte undefined; the UI needs it opaque.
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Some(VideoFrame { width, height, pixels })
    }
}
