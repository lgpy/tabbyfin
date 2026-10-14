//! In-window video through libmpv's software render API. Experimental: see `docs/PLAN.md`
//! §5.4 option C and `docs/spike-inline-video.md`.
//!
//! libmpv is loaded with `dlopen` at runtime, so builds don't need it and the app still works
//! (with the separate mpv window) where it's missing. mpv decodes and scales each frame into a
//! CPU buffer on a render thread; frames are published on a `watch` channel for the UI to
//! upload as an image.

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use libloading::Library;
use tokio::sync::watch;

/// A frame in BGRA order with opaque alpha, tightly packed (`width * 4` bytes per row).
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderStats {
    pub frames: u64,
    /// Total time spent copying frames out of mpv's buffer. mpv's own render time isn't
    /// measurable here: the render call also waits for each frame's display time.
    pub copy_time: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum SoftwareError {
    #[error("libmpv isn't available: {0}")]
    Unavailable(String),
    #[error("libmpv call `{call}` failed: {message}")]
    Call { call: &'static str, message: String },
}

type Result<T> = std::result::Result<T, SoftwareError>;

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_SW_SIZE: c_int = 17;
const PARAM_SW_FORMAT: c_int = 18;
const PARAM_SW_STRIDE: c_int = 19;
const PARAM_SW_POINTER: c_int = 20;
const UPDATE_FRAME: u64 = 1;
const FORMAT_INT64: c_int = 4;
const EVENT_SHUTDOWN: c_int = 1;

/// The subset of the libmpv C API used here.
struct Api {
    _lib: Library,
    create: unsafe extern "C" fn() -> *mut c_void,
    set_option_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut c_void, *mut *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    get_property: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    wait_event: unsafe extern "C" fn(*mut c_void, c_double) -> *const c_int,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    terminate_destroy: unsafe extern "C" fn(*mut c_void),
    render_create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int,
    render_set_update_callback: unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    render_update: unsafe extern "C" fn(*mut c_void) -> u64,
    render_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut c_void),
}

const LIBRARY_NAMES: &[&str] = if cfg!(windows) {
    &["libmpv-2.dll", "mpv-2.dll"]
} else if cfg!(target_os = "macos") {
    &["libmpv.2.dylib", "libmpv.dylib"]
} else {
    &["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
};

impl Api {
    fn load() -> Result<Self> {
        let mut last_error = String::new();
        for name in LIBRARY_NAMES {
            // SAFETY: loading libmpv runs no unexpected initialization code.
            match unsafe { Library::new(name) } {
                Ok(lib) => return Self::from_library(lib),
                Err(err) => last_error = err.to_string(),
            }
        }
        Err(SoftwareError::Unavailable(last_error))
    }

    fn from_library(lib: Library) -> Result<Self> {
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the declared types match the libmpv C API for these symbols.
                *unsafe { lib.get($name) }.map_err(|e| SoftwareError::Unavailable(e.to_string()))?
            };
        }
        Ok(Self {
            create: sym!(b"mpv_create\0"),
            set_option_string: sym!(b"mpv_set_option_string\0"),
            initialize: sym!(b"mpv_initialize\0"),
            command: sym!(b"mpv_command\0"),
            set_property_string: sym!(b"mpv_set_property_string\0"),
            get_property: sym!(b"mpv_get_property\0"),
            wait_event: sym!(b"mpv_wait_event\0"),
            error_string: sym!(b"mpv_error_string\0"),
            terminate_destroy: sym!(b"mpv_terminate_destroy\0"),
            render_create: sym!(b"mpv_render_context_create\0"),
            render_set_update_callback: sym!(b"mpv_render_context_set_update_callback\0"),
            render_update: sym!(b"mpv_render_context_update\0"),
            render_render: sym!(b"mpv_render_context_render\0"),
            render_free: sym!(b"mpv_render_context_free\0"),
            _lib: lib,
        })
    }

    fn check(&self, call: &'static str, code: c_int) -> Result<()> {
        if code >= 0 {
            return Ok(());
        }
        // SAFETY: mpv_error_string returns a static string for any code.
        let message = unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned();
        Err(SoftwareError::Call { call, message })
    }
}

fn run_command(api: &Api, mpv: *mut c_void, args: &[&str]) -> c_int {
    let args: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
    let mut ptrs: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(ptr::null());
    // SAFETY: null-terminated array of C strings that outlive the call.
    unsafe { (api.command)(mpv, ptrs.as_mut_ptr()) }
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("nul bytes removed")
}

/// Raw libmpv handles, shared with the render and event threads.
struct Handles {
    api: Api,
    mpv: *mut c_void,
    render: *mut c_void,
}

// SAFETY: the mpv client API is thread-safe. The render context is only used from the render
// thread (and freed after it has been joined).
unsafe impl Send for Handles {}
unsafe impl Sync for Handles {}

/// Wakes the render thread when mpv has a new frame.
#[derive(Default)]
struct Wakeup {
    pending: Mutex<bool>,
    signal: Condvar,
}

unsafe extern "C" fn on_update(ctx: *mut c_void) {
    // SAFETY: `ctx` is the `Wakeup` kept alive by the player for the render context's lifetime.
    let wakeup = unsafe { &*(ctx as *const Wakeup) };
    *wakeup.pending.lock().unwrap() = true;
    wakeup.signal.notify_one();
}

pub struct SoftwarePlayer {
    handles: Arc<Handles>,
    wakeup: Arc<Wakeup>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    copy_nanos: Arc<AtomicU64>,
    threads: Vec<JoinHandle<()>>,
}

impl SoftwarePlayer {
    /// Starts playing `url`, rendering frames no larger than `max_size` (aspect preserved).
    pub fn new(
        url: &str,
        http_headers: &[String],
        max_size: (u32, u32),
    ) -> Result<(Self, watch::Receiver<Option<Arc<VideoFrame>>>)> {
        let api = Api::load()?;
        // SAFETY: plain libmpv calls on a handle we own; every pointer passed is valid for the
        // duration of the call.
        let (mpv, render) = unsafe {
            let mpv = (api.create)();
            if mpv.is_null() {
                return Err(SoftwareError::Unavailable("mpv_create returned null".into()));
            }
            let set = |name: &str, value: &str| {
                let (name, value) = (cstring(name), cstring(value));
                (api.set_option_string)(mpv, name.as_ptr(), value.as_ptr())
            };
            api.check("set vo", set("vo", "libmpv"))?;
            set("hwdec", "auto-copy-safe");
            set("keep-open", "yes");
            api.check("mpv_initialize", (api.initialize)(mpv))?;
            for header in http_headers {
                // `append` adds one entry without splitting on the commas inside the header.
                api.check(
                    "change-list http-header-fields",
                    run_command(&api, mpv, &["change-list", "http-header-fields", "append", header]),
                )?;
            }

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
            let mut render = ptr::null_mut();
            api.check(
                "mpv_render_context_create",
                (api.render_create)(&mut render, mpv, params.as_mut_ptr()),
            )?;
            (mpv, render)
        };

        let handles = Arc::new(Handles { api, mpv, render });
        let wakeup = Arc::new(Wakeup::default());
        // SAFETY: `wakeup` outlives the render context: the callback is cleared in `Drop`
        // before the Arc is released.
        unsafe {
            (handles.api.render_set_update_callback)(render, Some(on_update), Arc::as_ptr(&wakeup) as *mut c_void);
        }

        let (frames_tx, frames_rx) = watch::channel(None);
        let mut player = Self {
            handles,
            wakeup,
            stop: Arc::new(AtomicBool::new(false)),
            frames: Arc::default(),
            copy_nanos: Arc::default(),
            threads: Vec::new(),
        };
        player.spawn_render_thread(frames_tx, max_size);
        player.spawn_event_thread();
        player.command(&["loadfile", url])?;
        Ok((player, frames_rx))
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        let api = &self.handles.api;
        api.check("mpv_command", run_command(api, self.handles.mpv, args))
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<()> {
        let (name, value) = (cstring(name), cstring(value));
        let api = &self.handles.api;
        // SAFETY: valid C strings for the duration of the call.
        api.check("mpv_set_property_string", unsafe {
            (api.set_property_string)(self.handles.mpv, name.as_ptr(), value.as_ptr())
        })
    }

    pub fn stats(&self) -> RenderStats {
        RenderStats {
            frames: self.frames.load(Ordering::Relaxed),
            copy_time: Duration::from_nanos(self.copy_nanos.load(Ordering::Relaxed)),
        }
    }

    fn spawn_render_thread(&mut self, frames_tx: watch::Sender<Option<Arc<VideoFrame>>>, max_size: (u32, u32)) {
        let (handles, wakeup, stop) = (self.handles.clone(), self.wakeup.clone(), self.stop.clone());
        let (frames, copy_nanos) = (self.frames.clone(), self.copy_nanos.clone());
        let thread = std::thread::Builder::new().name("mpv-render".into());
        self.threads.push(thread.spawn(move || {
            let mut surface = Surface::default();
            loop {
                {
                    let mut pending = wakeup.pending.lock().unwrap();
                    while !*pending && !stop.load(Ordering::Relaxed) {
                        pending = wakeup.signal.wait(pending).unwrap();
                    }
                    *pending = false;
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                // SAFETY: render context calls happen only on this thread.
                let flags = unsafe { (handles.api.render_update)(handles.render) };
                if flags & UPDATE_FRAME == 0 {
                    continue;
                }
                let Some((width, height)) = target_size(&handles, max_size) else { continue };
                if let Some((frame, copy_time)) = surface.render(&handles, width, height) {
                    copy_nanos.fetch_add(copy_time.as_nanos() as u64, Ordering::Relaxed);
                    frames.fetch_add(1, Ordering::Relaxed);
                    frames_tx.send_replace(Some(Arc::new(frame)));
                }
            }
        }).expect("failed to spawn render thread"));
    }

    /// libmpv queues events for every client; drain them so the queue never fills up.
    fn spawn_event_thread(&mut self) {
        let (handles, stop) = (self.handles.clone(), self.stop.clone());
        let thread = std::thread::Builder::new().name("mpv-events".into());
        self.threads.push(thread.spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // SAFETY: mpv_wait_event returns a pointer to an mpv_event, whose first field is
                // the event id.
                let event = unsafe { *(handles.api.wait_event)(handles.mpv, 0.25) };
                if event == EVENT_SHUTDOWN {
                    break;
                }
            }
        }).expect("failed to spawn event thread"));
    }
}

impl Drop for SoftwarePlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wakeup.signal.notify_all();
        let api = &self.handles.api;
        // SAFETY: wake the event thread with a quit, join both threads, then free the render
        // context before destroying the core, as the API requires.
        unsafe {
            let quit = cstring("quit");
            let mut args = [quit.as_ptr(), ptr::null()];
            (api.command)(self.handles.mpv, args.as_mut_ptr());
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
            (api.render_set_update_callback)(self.handles.render, None, ptr::null_mut());
            (api.render_free)(self.handles.render);
            (api.terminate_destroy)(self.handles.mpv);
        }
    }
}

/// The video's display size scaled down to fit `max_size`, or `None` before a video is loaded.
fn target_size(handles: &Handles, (max_w, max_h): (u32, u32)) -> Option<(u32, u32)> {
    let get = |name: &str| {
        let name = cstring(name);
        let mut value: i64 = 0;
        // SAFETY: MPV_FORMAT_INT64 writes an i64 into `value`.
        let code = unsafe {
            (handles.api.get_property)(handles.mpv, name.as_ptr(), FORMAT_INT64, &mut value as *mut i64 as *mut c_void)
        };
        (code >= 0 && value > 0).then_some(value as f64)
    };
    let (w, h) = (get("dwidth")?, get("dheight")?);
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
    /// Renders a frame and returns it with the time spent copying it out.
    fn render(&mut self, handles: &Handles, width: u32, height: u32) -> Option<(VideoFrame, Duration)> {
        let row = width as usize * 4;
        let stride = row.div_ceil(64) * 64;
        let chunks = (stride * height as usize).div_ceil(64);
        self.buffer.resize(chunks, Aligned([0; 64]));

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
        // SAFETY: the buffer holds `stride * height` bytes and outlives the call.
        let code = unsafe { (handles.api.render_render)(handles.render, params.as_mut_ptr()) };
        if handles.api.check("mpv_render_context_render", code).is_err() {
            return None;
        }

        let started = Instant::now();
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
        Some((VideoFrame { width, height, pixels }, started.elapsed()))
    }
}
