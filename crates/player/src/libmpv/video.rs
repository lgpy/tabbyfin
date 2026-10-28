//! Video output. A render thread owns an mpv render context and publishes frames for the UI.
//!
//! The UI says what it can draw when the player starts ([`VideoImport`]), and the render thread
//! only sets up an output whose frames it can show:
//! - **GPU (Windows):** mpv renders through OpenGL ES on ANGLE (`angle.rs`, with zero-copy
//!   hardware decoding) or desktop OpenGL (`wgl.rs`) into shared D3D11 textures, which the UI's
//!   renderer opens and draws with no CPU copy.
//! - **GPU (Linux):** mpv renders into exported Vulkan memory that the UI's Vulkan device
//!   imports (`egl.rs`).
//! - **Software:** mpv renders RGBA pixels into a CPU buffer that the UI uploads into a
//!   texture. Used when the UI takes pixels only, or the GPU path isn't available.
//!
//! See `docs/in-app-video.md`.

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::{Core, RenderParam, cstring};
use crate::{Error, Result};

pub(crate) const PARAM_INVALID: c_int = 0;
pub(crate) const PARAM_API_TYPE: c_int = 1;
const PARAM_SW_SIZE: c_int = 17;
const PARAM_SW_FORMAT: c_int = 18;
const PARAM_SW_STRIDE: c_int = 19;
const PARAM_SW_POINTER: c_int = 20;
pub(crate) const UPDATE_FRAME: u64 = 1;

/// Rendering size used until the UI reports its size.
const DEFAULT_TARGET: (u32, u32) = (1920, 1080);

/// What the UI can draw, given to the player when it starts (and passed on to the player host
/// process), so the render thread only sets up an output whose frames the UI can show. The app
/// reads it from its graphics adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoImport {
    /// Software-rendered pixels ([`Frame::Pixels`]) only: the UI's renderer can't import the
    /// GPU paths' memory (wgpu on OpenGL, macOS for now), or its import isn't built yet.
    #[default]
    Pixels,
    /// Vulkan memory exported as opaque fds, imported by the Vulkan device with this
    /// `VkPhysicalDeviceIDProperties::deviceUUID` (Linux). The exporter must be on the same GPU.
    VulkanMemory { device_uuid: [u8; 16] },
    /// D3D11 textures shared through NT handles, opened by a D3D12 device on the DXGI adapter
    /// with this LUID (Windows). The textures must be made on the same adapter.
    D3d11Textures { adapter_luid: Luid },
}

/// A Windows adapter LUID (`LUID { LowPart, HighPart }`): which GPU, until the system restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Luid {
    pub low: u32,
    pub high: i32,
}

/// A frame in RGBA order with opaque alpha, tightly packed (`width * 4` bytes per row): what a
/// `Rgba8Unorm` texture takes as is.
#[derive(Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A GPU texture holding a finished frame. The render thread keeps it alive and doesn't write
/// to it again for the next two frames.
///
/// Windows: a D3D11 texture on the UI's adapter, shared through a named NT handle, which the
/// UI's D3D12 device opens by `name`. A new texture gets a new `id` and name; the frames of one
/// texture share them. In the process that made it, a frame also holds the texture's handle,
/// which keeps the name (and the texture) alive for as long as the frame is held, even after the
/// render thread has moved on to a new ring (a resize). Frames from the player host process
/// arrive without it; the host keeps the handles of the frames it sent last (`host.rs`).
#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct SharedFrame {
    pub id: u64,
    pub name: Arc<str>,
    pub width: u32,
    pub height: u32,
    pub(crate) _handle: Option<Arc<std::os::windows::io::OwnedHandle>>,
}

/// A GPU texture holding a finished frame. The render thread keeps it alive and doesn't write
/// to it again for the next two frames.
///
/// Linux: an RGBA8, linearly tiled Vulkan image whose memory is exported as an opaque fd. A new
/// image gets a new `id`; the frames of one image share its fd. The fd keeps the memory alive,
/// so a frame can be imported for as long as it's held, even after the render thread has moved
/// on to a new ring (a resize).
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct SharedFrame {
    pub id: u64,
    pub fd: Arc<std::os::fd::OwnedFd>,
    pub allocation_size: u64,
    pub width: u32,
    pub height: u32,
}

/// Placeholder so `Frame` has the same shape everywhere; never constructed here.
#[cfg(not(any(windows, target_os = "linux")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedFrame {}

#[derive(Debug, Clone)]
pub enum Frame {
    Pixels(Arc<VideoFrame>),
    Shared(SharedFrame),
}

/// Handle to a player's video. Cheap to clone; rendering stops when the player is dropped.
#[derive(Clone)]
pub struct VideoOutput {
    target: Target,
    frames: watch::Receiver<Option<Frame>>,
}

/// Where the requested output size goes.
#[derive(Clone)]
enum Target {
    /// The render thread in this process.
    Local(Arc<RenderControl>),
    /// A player host process (see `crate::remote`).
    Remote(Arc<dyn Fn(u32, u32) + Send + Sync>),
}

/// State shared between the render thread, the player and the UI.
pub(crate) struct RenderControl {
    pub wakeup: Wakeup,
    /// Requested output size in device pixels, packed as `w << 32 | h`.
    target: AtomicU64,
    pub stop: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl RenderControl {
    pub fn target(&self) -> (u32, u32) {
        let packed = self.target.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }

    /// Blocks until mpv or the UI asks for a redraw. Returns `false` when stopping.
    pub fn wait(&self) -> bool {
        let mut pending = self.wakeup.pending.lock().unwrap();
        while !*pending && !self.stop.load(Ordering::Relaxed) {
            pending = self.wakeup.signal.wait(pending).unwrap();
        }
        *pending = false;
        !self.stop.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub(crate) struct Wakeup {
    pending: Mutex<bool>,
    signal: Condvar,
}

impl Wakeup {
    pub fn notify(&self) {
        *self.pending.lock().unwrap() = true;
        self.signal.notify_one();
    }
}

/// mpv's update callback: `ctx` is the `Wakeup` in `RenderControl`, which outlives the render
/// context (the callback is cleared before the context is freed).
pub(crate) unsafe extern "C" fn on_update(ctx: *mut c_void) {
    // SAFETY: see above.
    let wakeup = unsafe { &*(ctx as *const Wakeup) };
    wakeup.notify();
}

/// Stops and joins the render thread when dropped. Owned by the player, so rendering (and the
/// render context) ends before the core is destroyed, even while the UI still holds
/// [`VideoOutput`] clones.
pub(crate) struct RenderThread(Arc<RenderControl>);

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
    /// Starts the render thread, with an output whose frames the UI can take (`import`), and
    /// waits until its render context exists (mpv needs one before a file with video starts).
    pub(crate) fn new(core: Arc<Core>, import: VideoImport) -> Result<(Self, RenderThread)> {
        let control = Arc::new(RenderControl {
            wakeup: Wakeup::default(),
            target: AtomicU64::new(pack(DEFAULT_TARGET)),
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
        });
        let (frames_tx, frames) = watch::channel(None);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("mpv-render".into())
            .spawn({
                let control = control.clone();
                move || render_thread(&core, &control, &frames_tx, ready_tx, import)
            })
            .map_err(Error::Spawn)?;
        *control.thread.lock().unwrap() = Some(thread);
        let render_thread = RenderThread(control.clone());
        ready_rx.recv().map_err(|_| Error::Closed)??;
        Ok((
            Self {
                target: Target::Local(control),
                frames,
            },
            render_thread,
        ))
    }

    /// Video rendered by a player host process: frames arrive on `frames`, and size requests
    /// go to `set_target`.
    pub(crate) fn remote(
        frames: watch::Receiver<Option<Frame>>,
        set_target: Arc<dyn Fn(u32, u32) + Send + Sync>,
    ) -> Self {
        Self {
            target: Target::Remote(set_target),
            frames,
        }
    }

    /// Latest frame, updated as mpv renders.
    pub fn frames(&self) -> watch::Receiver<Option<Frame>> {
        self.frames.clone()
    }

    /// Size of the area the video is shown in, in device pixels. Frames are rendered to fit
    /// it (never larger than the video), and the current frame is redrawn on change.
    pub fn set_target_size(&self, width: u32, height: u32) {
        let (width, height) = (width.max(2), height.max(2));
        match &self.target {
            Target::Local(control) => {
                let packed = pack((width, height));
                if control.target.swap(packed, Ordering::Relaxed) != packed {
                    control.wakeup.notify();
                }
            }
            Target::Remote(set_target) => set_target(width, height),
        }
    }
}

fn pack((w, h): (u32, u32)) -> u64 {
    (w as u64) << 32 | h as u64
}

type Ready = std::sync::mpsc::Sender<Result<()>>;

fn render_thread(
    core: &Core,
    control: &RenderControl,
    frames: &watch::Sender<Option<Frame>>,
    ready: Ready,
    import: VideoImport,
) {
    #[cfg(any(windows, target_os = "linux"))]
    if let Some((mut gpu, kind)) = gpu_renderer(core, control, import) {
        tracing::info!("video output: GPU ({kind})");
        // Decoded frames can stay on the GPU where mpv has an interop for this context
        // (d3d11va with ANGLE, nvdec everywhere); otherwise mpv copies them back.
        if let Err(err) = core.set_property("hwdec", "auto-safe") {
            tracing::warn!("{err}");
        }
        let _ = ready.send(Ok(()));
        gpu.run(core, control, frames);
        return;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = import;
    match SoftwareRenderer::new(core, control) {
        Ok(mut software) => {
            tracing::info!("video output: software");
            let _ = ready.send(Ok(()));
            software.run(core, control, frames);
        }
        Err(err) => {
            let _ = ready.send(Err(err));
        }
    }
}

/// The GPU output for what the UI imports, or `None` for software rendering: when the UI takes
/// pixels only, the import is for another OS, setting up the GPU path failed, or
/// `TABBYFIN_VIDEO=software` forces software (e.g. to rule out GPU driver issues).
#[cfg(any(windows, target_os = "linux"))]
fn gpu_renderer(core: &Core, control: &RenderControl, import: VideoImport) -> Option<(GpuRenderer, &'static str)> {
    use super::opengl::GlRenderer;

    if std::env::var("TABBYFIN_VIDEO").as_deref() == Ok("software") {
        return None;
    }
    let gpu = match import {
        // On Windows, ANGLE first: it also gives zero-copy hardware decoding. Then plain WGL.
        // Both make their textures on the UI's adapter (`adapter_luid`), the only one whose
        // D3D12 device can open them.
        #[cfg(windows)]
        VideoImport::D3d11Textures { adapter_luid } => super::angle::Angle::new(core, adapter_luid)
            .and_then(|angle| GlRenderer::new(core, control, angle))
            .map(|gpu| (GpuRenderer::Angle(gpu), "ANGLE/D3D11"))
            .or_else(|err| {
                tracing::info!("ANGLE video output unavailable: {err}");
                super::wgl::Wgl::new(core, adapter_luid)
                    .and_then(|wgl| GlRenderer::new(core, control, wgl))
                    .map(|gpu| (GpuRenderer::Wgl(gpu), "OpenGL/D3D11 interop"))
            }),
        // OpenGL and the exporting Vulkan device on the UI's GPU, the only one whose memory
        // its device can import.
        #[cfg(target_os = "linux")]
        VideoImport::VulkanMemory { device_uuid } => super::egl::EglVulkan::new(core, device_uuid)
            .and_then(|egl| GlRenderer::new(core, control, egl))
            .map(|gpu| (gpu, "OpenGL/Vulkan interop")),
        other => {
            tracing::info!("no GPU video output for {other:?}");
            return None;
        }
    };
    gpu.inspect_err(|err| tracing::warn!("GPU video output unavailable, using software rendering: {err}"))
        .ok()
}

#[cfg(target_os = "linux")]
type GpuRenderer = super::opengl::GlRenderer<super::egl::EglVulkan>;

#[cfg(windows)]
enum GpuRenderer {
    Angle(super::opengl::GlRenderer<super::angle::Angle>),
    Wgl(super::opengl::GlRenderer<super::wgl::Wgl>),
}

#[cfg(windows)]
impl GpuRenderer {
    fn run(&mut self, core: &Core, control: &RenderControl, frames: &watch::Sender<Option<Frame>>) {
        match self {
            GpuRenderer::Angle(gpu) => gpu.run(core, control, frames),
            GpuRenderer::Wgl(gpu) => gpu.run(core, control, frames),
        }
    }
}

/// Whether to use the GPU path with this OpenGL renderer. CPU-emulated OpenGL (Mesa's
/// llvmpipe/softpipe, SwiftShader, Windows' "GDI Generic") makes mpv's GPU shaders far more
/// expensive than its software renderer, so those use the software path unless
/// `TABBYFIN_VIDEO=gpu` forces the GPU path.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn hardware_renderer(renderer: &str) -> bool {
    std::env::var("TABBYFIN_VIDEO").as_deref() == Ok("gpu") || !software_renderer(renderer)
}

#[cfg(any(windows, target_os = "linux"))]
fn software_renderer(renderer: &str) -> bool {
    let renderer = renderer.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swiftshader",
        "gdi generic",
        "software rasterizer",
    ]
    .iter()
    .any(|name| renderer.contains(name))
}

/// On a software rasterizer (only with `TABBYFIN_VIDEO=gpu`), mpv's default scalers draw
/// black frames: with mpv 0.37 on llvmpipe, scaling up or down rendered black every time, or
/// only sometimes, depending on start-up timing. Bilinear scaling works there, and it's
/// also the cheapest.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn adapt_to_renderer(core: &Core, renderer: &str) {
    if !software_renderer(renderer) {
        return;
    }
    for name in ["scale", "dscale", "cscale"] {
        if let Err(err) = core.set_property(name, "bilinear") {
            tracing::warn!("couldn't set mpv's {name}: {err}");
        }
    }
}

/// The video's display size scaled down to fit the target, or `None` without video.
pub(crate) fn fit(core: &Core, control: &RenderControl) -> Option<(u32, u32)> {
    let w = core.get_i64("dwidth").filter(|v| *v > 0)? as f64;
    let h = core.get_i64("dheight").filter(|v| *v > 0)? as f64;
    let (max_w, max_h) = control.target();
    let scale = (max_w as f64 / w).min(max_h as f64 / h).min(1.0);
    Some(((w * scale).round().max(2.) as u32, (h * scale).round().max(2.) as u32))
}

/// libmpv's software renderer into a 64-byte aligned buffer (as it prefers).
struct SoftwareRenderer {
    context: *mut c_void,
    buffer: Vec<Aligned>,
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct Aligned([u8; 64]);

impl SoftwareRenderer {
    fn new(core: &Core, control: &RenderControl) -> Result<Self> {
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
        // SAFETY: the wakeup outlives the context; the callback is cleared in `run`.
        unsafe {
            (core.api.render_set_update_callback)(
                context,
                Some(on_update),
                &control.wakeup as *const Wakeup as *mut c_void,
            );
        }
        Ok(Self {
            context,
            buffer: Vec::new(),
        })
    }

    fn run(&mut self, core: &Core, control: &RenderControl, frames: &watch::Sender<Option<Frame>>) {
        let mut last_size = None;
        while control.wait() {
            // SAFETY: render context calls happen only on this thread.
            let flags = unsafe { (core.api.render_update)(self.context) };
            let Some(size) = fit(core, control) else { continue };
            // Redraw on a new frame, or when the output size changed (e.g. while paused).
            if flags & UPDATE_FRAME == 0 && last_size == Some(size) {
                continue;
            }
            last_size = Some(size);
            if let Some(frame) = self.render(core, size) {
                frames.send_replace(Some(Frame::Pixels(Arc::new(frame))));
            }
        }
        // SAFETY: stop the callback, then free the context on its own thread, before the core.
        unsafe {
            (core.api.render_set_update_callback)(self.context, None, ptr::null_mut());
            (core.api.render_free)(self.context);
        }
    }

    fn render(&mut self, core: &Core, (width, height): (u32, u32)) -> Option<VideoFrame> {
        let row = width as usize * 4;
        let stride = row.div_ceil(64) * 64;
        self.buffer
            .resize((stride * height as usize).div_ceil(64), Aligned([0; 64]));

        let mut size = [width as c_int, height as c_int];
        let format = cstring("rgb0");
        let mut stride_param = stride;
        let mut params = [
            RenderParam {
                kind: PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            RenderParam {
                kind: PARAM_SW_FORMAT,
                data: format.as_ptr() as *mut c_void,
            },
            RenderParam {
                kind: PARAM_SW_STRIDE,
                data: &mut stride_param as *mut usize as *mut c_void,
            },
            RenderParam {
                kind: PARAM_SW_POINTER,
                data: self.buffer.as_mut_ptr() as *mut c_void,
            },
            RenderParam {
                kind: PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        // SAFETY: the buffer holds `stride * height` bytes and outlives the call.
        let code = unsafe { (core.api.render_render)(self.context, params.as_mut_ptr()) };
        if let Err(err) = core.api.check("mpv_render_context_render", code) {
            tracing::warn!("{err}");
            return None;
        }

        // SAFETY: the chunks are plain bytes; view them as one contiguous byte slice.
        let bytes = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr() as *const u8, stride * height as usize) };
        let mut pixels = Vec::with_capacity(row * height as usize);
        for line in bytes.chunks_exact(stride) {
            pixels.extend_from_slice(&line[..row]);
        }
        // "rgb0" leaves the fourth byte undefined; the UI draws the texture with its alpha, so
        // it has to be opaque.
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Some(VideoFrame { width, height, pixels })
    }
}
