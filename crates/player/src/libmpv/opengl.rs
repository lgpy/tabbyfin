//! What the GPU renderers share: mpv renders through libmpv's `opengl` render API into a ring
//! of textures that GPUI's device can also open, and GPUI draws the latest while mpv renders
//! into the oldest. Each renderer is a `Backend` that brings the GL context, makes the shared
//! textures and syncs a frame for the other device: ANGLE (`angle.rs`) and WGL (`wgl.rs`) on
//! Windows, EGL with Vulkan memory (`egl.rs`) on Linux.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use tokio::sync::watch;

use super::video::{
    Frame, PARAM_API_TYPE, PARAM_INVALID, RenderControl, SharedFrame, UPDATE_FRAME, Wakeup, fit, on_update,
};
use super::{Core, RenderParam, cstring};
use crate::{Error, Result};

const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;

pub(crate) const GL_TEXTURE_2D: u32 = 0x0DE1;
pub(crate) const GL_FRAMEBUFFER: u32 = 0x8D40;
pub(crate) const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
pub(crate) const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
pub(crate) const GL_RENDERER: u32 = 0x1F01;

/// Frames in flight: GPUI draws the latest while mpv renders into the oldest.
const RING: usize = 3;

/// libmpv's GL loader callback: `(ctx, name) -> function`.
pub(crate) type GetProcAddress = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: GetProcAddress,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

/// A function pointer from `eglGetProcAddress` as its declared type.
///
/// # Safety
/// `T` must be the function's actual signature.
pub(crate) unsafe fn cast<T: Copy>(f: extern "system" fn()) -> T {
    // SAFETY: function pointers have the same size; the caller guarantees the signature.
    unsafe { std::mem::transmute_copy(&f) }
}

pub(crate) fn failed(what: impl std::fmt::Display) -> Error {
    Error::GpuUnavailable(what.to_string())
}

/// A GL context, current on the render thread, with textures GPUI's device can open.
pub(crate) trait Backend {
    /// One shared texture attached to a framebuffer.
    type Slot;

    /// libmpv's GL loader and its context pointer, valid as long as the backend.
    fn proc_address(&self) -> (GetProcAddress, *mut c_void);

    fn create_slot(&self, width: u32, height: u32) -> Result<Self::Slot>;

    fn release_slot(&self, slot: Self::Slot);

    /// Has mpv draw into `slot` (`draw` takes its framebuffer), makes sure the frame is
    /// complete before GPUI's device reads it, and describes it for GPUI.
    fn render(&self, slot: &Self::Slot, size: (u32, u32), draw: impl FnOnce(u32) -> Result<()>) -> Result<SharedFrame>;
}

pub(crate) struct GlRenderer<B: Backend> {
    mpv_context: *mut c_void,
    ring: Vec<B::Slot>,
    size: (u32, u32),
    next: usize,
    /// Last, so the GL context outlives the slots released in `drop`.
    backend: B,
}

impl<B: Backend> GlRenderer<B> {
    /// Creates mpv's render context on the backend's GL context, on the render thread.
    pub fn new(core: &Core, control: &RenderControl, backend: B) -> Result<Self> {
        let (get_proc_address, get_proc_address_ctx) = backend.proc_address();
        let api_type = cstring("opengl");
        let mut init = OpenGlInitParams {
            get_proc_address,
            get_proc_address_ctx,
        };
        let mut params = [
            RenderParam {
                kind: PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            RenderParam {
                kind: PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            RenderParam {
                kind: PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let mut mpv_context = ptr::null_mut();
        // SAFETY: the backend's context is current on this thread; `params` outlives the call.
        unsafe {
            let code = (core.api.render_create)(&mut mpv_context, core.mpv, params.as_mut_ptr());
            core.api.check("mpv_render_context_create(opengl)", code)?;
            (core.api.render_set_update_callback)(
                mpv_context,
                Some(on_update),
                &control.wakeup as *const Wakeup as *mut c_void,
            );
        }
        Ok(Self {
            mpv_context,
            ring: Vec::new(),
            size: (0, 0),
            next: 0,
            backend,
        })
    }

    pub fn run(&mut self, core: &Core, control: &RenderControl, frames: &watch::Sender<Option<Frame>>) {
        let mut last_size = None;
        while control.wait() {
            // SAFETY: render context calls happen only on this thread.
            let flags = unsafe { (core.api.render_update)(self.mpv_context) };
            let Some(size) = fit(core, control) else { continue };
            if flags & UPDATE_FRAME == 0 && last_size == Some(size) {
                continue;
            }
            last_size = Some(size);
            match self.render(core, size) {
                Ok(frame) => {
                    frames.send_replace(Some(Frame::Shared(frame)));
                }
                Err(err) => tracing::warn!("GPU frame failed: {err}"),
            }
        }
        // SAFETY: the GL context is current on this thread; free mpv's context (which, with
        // ANGLE, releases its hold on the D3D11 device) before the backend's.
        unsafe {
            (core.api.render_set_update_callback)(self.mpv_context, None, ptr::null_mut());
            (core.api.render_free)(self.mpv_context);
        }
    }

    fn render(&mut self, core: &Core, (width, height): (u32, u32)) -> Result<SharedFrame> {
        if self.size != (width, height) || self.ring.is_empty() {
            self.release_ring();
            for _ in 0..RING {
                let slot = self.backend.create_slot(width, height)?;
                self.ring.push(slot);
            }
            self.size = (width, height);
            self.next = 0;
        }
        let slot = &self.ring[self.next];
        self.next = (self.next + 1) % RING;

        let mpv_context = self.mpv_context;
        self.backend.render(slot, (width, height), |fbo| {
            let mut fbo = OpenGlFbo {
                fbo: fbo as c_int,
                w: width as c_int,
                h: height as c_int,
                internal_format: 0,
            };
            // Every backend's texture already has the row order GPUI samples in (checked with
            // D3D11 on AMD and with Vulkan on Mesa), so no flip.
            let mut flip: c_int = 0;
            let mut params = [
                RenderParam {
                    kind: PARAM_OPENGL_FBO,
                    data: &mut fbo as *mut _ as *mut c_void,
                },
                RenderParam {
                    kind: PARAM_FLIP_Y,
                    data: &mut flip as *mut _ as *mut c_void,
                },
                RenderParam {
                    kind: PARAM_INVALID,
                    data: ptr::null_mut(),
                },
            ];
            // SAFETY: on the render thread with the framebuffer complete; `params` outlives the call.
            let code = unsafe { (core.api.render_render)(mpv_context, params.as_mut_ptr()) };
            core.api.check("mpv_render_context_render", code)
        })
    }

    fn release_ring(&mut self) {
        for slot in std::mem::take(&mut self.ring) {
            self.backend.release_slot(slot);
        }
    }
}

impl<B: Backend> Drop for GlRenderer<B> {
    fn drop(&mut self) {
        self.release_ring();
    }
}
