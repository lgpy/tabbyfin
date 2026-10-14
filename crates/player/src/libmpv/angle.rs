//! GPU video output on Windows through ANGLE: mpv renders with OpenGL ES (libmpv's `opengl`
//! render API) on ANGLE's D3D11 backend, into D3D11 textures shared with GPUI's DirectX
//! renderer. Because mpv's GL context sits on a D3D11 device, its `d3d11va` hardware decoder
//! hands decoded frames straight to the renderer (mpv's `d3d11egl` interop): zero-copy from
//! decoder to screen, including 10-bit (P010) video.
//!
//! Needs ANGLE (`libEGL.dll`, `libGLESv2.dll`) next to the executable; without it, playback
//! uses the WGL interop in `wgl.rs`, which decodes with a copy back to the CPU.
//! See `docs/in-app-video.md`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use khronos_egl as egl;
use tokio::sync::watch;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource;
use windows::core::Interface;

use super::video::{
    Frame, PARAM_API_TYPE, PARAM_INVALID, RenderControl, SharedFrame, UPDATE_FRAME, Wakeup, fit, on_update,
};
use super::{Core, RenderParam, cstring};
use crate::{Error, Result};

const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RENDERER: u32 = 0x1F01;

// EGL_ANGLE_platform_angle(_d3d), EGL_EXT_device_query, EGL_ANGLE_device_d3d,
// EGL_ANGLE_d3d_texture_client_buffer.
const EGL_PLATFORM_ANGLE_ANGLE: egl::Enum = 0x3202;
const EGL_PLATFORM_ANGLE_TYPE_ANGLE: egl::Attrib = 0x3203;
const EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE: egl::Attrib = 0x3208;
const EGL_DEVICE_EXT: egl::Int = 0x322C;
const EGL_D3D11_DEVICE_ANGLE: egl::Int = 0x33A1;
const EGL_D3D_TEXTURE_ANGLE: egl::Enum = 0x33A3;

/// Frames in flight: GPUI draws the latest while mpv renders into the oldest.
const RING: usize = 3;

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

type Egl = egl::DynamicInstance<egl::EGL1_5>;

#[allow(non_snake_case)]
struct Gl {
    glGetString: unsafe extern "system" fn(u32) -> *const u8,
    glGenTextures: unsafe extern "system" fn(i32, *mut u32),
    glDeleteTextures: unsafe extern "system" fn(i32, *const u32),
    glBindTexture: unsafe extern "system" fn(u32, u32),
    glGenFramebuffers: unsafe extern "system" fn(i32, *mut u32),
    glDeleteFramebuffers: unsafe extern "system" fn(i32, *const u32),
    glBindFramebuffer: unsafe extern "system" fn(u32, u32),
    glFramebufferTexture2D: unsafe extern "system" fn(u32, u32, u32, u32, i32),
    glCheckFramebufferStatus: unsafe extern "system" fn(u32) -> u32,
    glFinish: unsafe extern "system" fn(),
}

#[allow(non_snake_case)]
struct EglExt {
    eglQueryDisplayAttribEXT: unsafe extern "system" fn(*mut c_void, egl::Int, *mut egl::Attrib) -> egl::Boolean,
    eglQueryDeviceAttribEXT: unsafe extern "system" fn(*mut c_void, egl::Int, *mut egl::Attrib) -> egl::Boolean,
}

/// An EGL function pointer as its declared type.
///
/// # Safety
/// `T` must be the function's actual signature.
unsafe fn cast<T: Copy>(f: extern "system" fn()) -> T {
    // SAFETY: function pointers have the same size; the caller guarantees the signature.
    unsafe { std::mem::transmute_copy(&f) }
}

fn failed(what: impl std::fmt::Display) -> Error {
    Error::GpuUnavailable(what.to_string())
}

/// libmpv's GL loader callback; `ctx` is the `Egl` instance. ANGLE resolves core GLES
/// functions through `eglGetProcAddress` too.
unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` points to the renderer's `Egl`, alive while the mpv context exists.
    let egl = unsafe { &*(ctx as *const Egl) };
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else { return ptr::null_mut() };
    egl.get_proc_address(name).map_or(ptr::null_mut(), |f| f as *mut c_void)
}

/// `libEGL.dll` next to the executable. mpv's `d3d11egl` interop loads `libEGL.dll` by name,
/// which then resolves to this same, already loaded module.
fn load_egl() -> Result<Egl> {
    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("libEGL.dll")))
        .filter(|path| path.exists())
        .ok_or_else(|| failed("ANGLE (libEGL.dll) isn't installed next to the app"))?;
    // SAFETY: loading ANGLE runs no unexpected initialization code.
    unsafe { Egl::load_required_from_filename(&path) }.map_err(failed)
}

/// One shared texture on ANGLE's device, bound as a GL texture through a pbuffer.
struct Slot {
    _texture: ID3D11Texture2D,
    share: HANDLE,
    surface: egl::Surface,
    gl_texture: u32,
    fbo: u32,
}

pub(crate) struct GpuRenderer {
    /// Boxed so its address is stable for libmpv's `get_proc_address` context.
    egl: Box<Egl>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    /// 1×1 surface the context is current on; mpv renders into the slots' framebuffers.
    surface: egl::Surface,
    device: ID3D11Device,
    gl: Gl,
    mpv_context: *mut c_void,
    ring: Vec<Slot>,
    size: (u32, u32),
    next: usize,
}

impl GpuRenderer {
    /// Sets up ANGLE (D3D11), the shared device and the mpv render context on the calling
    /// (render) thread.
    pub fn new(core: &Core, control: &RenderControl) -> Result<Self> {
        let egl = Box::new(load_egl()?);
        // SAFETY: EGL/GL setup on this thread; every object is checked and cleaned up.
        unsafe {
            let attributes = [
                EGL_PLATFORM_ANGLE_TYPE_ANGLE,
                EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE,
                egl::ATTRIB_NONE,
            ];
            let display = egl
                .get_platform_display(EGL_PLATFORM_ANGLE_ANGLE, egl::DEFAULT_DISPLAY, &attributes)
                .map_err(failed)?;
            egl.initialize(display).map_err(failed)?;
            let terminate = |egl: &Egl, err: Error| {
                let _ = egl.terminate(display);
                err
            };
            // Display extensions, plus client ones (newer ANGLE lists EGL_EXT_device_query there).
            let mut extensions = egl
                .query_string(Some(display), egl::EXTENSIONS)
                .map_err(|e| terminate(&egl, failed(e)))?
                .to_string_lossy()
                .into_owned();
            if let Ok(client) = egl.query_string(None, egl::EXTENSIONS) {
                extensions = format!("{extensions} {}", client.to_string_lossy());
            }
            tracing::debug!(%extensions, "ANGLE EGL extensions");
            for required in [
                "EGL_EXT_device_query",
                "EGL_ANGLE_d3d_texture_client_buffer",
                "EGL_ANGLE_stream_producer_d3d_texture",
            ] {
                if !extensions.split(' ').any(|e| e == required) {
                    return Err(terminate(&egl, failed(format!("ANGLE lacks {required}"))));
                }
            }

            egl.bind_api(egl::OPENGL_ES_API).map_err(|e| terminate(&egl, failed(e)))?;
            let config_attributes = [
                egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8, egl::ALPHA_SIZE, 8,
                egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT,
                egl::SURFACE_TYPE, egl::PBUFFER_BIT,
                egl::NONE,
            ];
            let config = egl
                .choose_first_config(display, &config_attributes)
                .map_err(|e| terminate(&egl, failed(e)))?
                .ok_or_else(|| terminate(&egl, failed("no EGL config")))?;
            let context = egl
                .create_context(display, config, None, &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE])
                .map_err(|e| terminate(&egl, failed(e)))?;
            let surface = match egl.create_pbuffer_surface(display, config, &[egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE]) {
                Ok(surface) => surface,
                Err(err) => {
                    let _ = egl.destroy_context(display, context);
                    return Err(terminate(&egl, failed(err)));
                }
            };
            let fail = |egl: &Egl, err: Error| {
                let _ = egl.make_current(display, None, None, None);
                let _ = egl.destroy_surface(display, surface);
                let _ = egl.destroy_context(display, context);
                terminate(egl, err)
            };
            if let Err(err) = egl.make_current(display, Some(surface), Some(surface), Some(context)) {
                return Err(fail(&egl, failed(err)));
            }

            macro_rules! procs {
                ($ty:ident { $($name:ident),* }) => {
                    $ty { $($name: match egl.get_proc_address(stringify!($name)) {
                        Some(f) => cast(f),
                        None => return Err(fail(&egl, failed(concat!(stringify!($name), " missing")))),
                    },)* }
                };
            }
            let gl = procs!(Gl {
                glGetString, glGenTextures, glDeleteTextures, glBindTexture, glGenFramebuffers,
                glDeleteFramebuffers, glBindFramebuffer, glFramebufferTexture2D, glCheckFramebufferStatus, glFinish
            });
            let ext = procs!(EglExt { eglQueryDisplayAttribEXT, eglQueryDeviceAttribEXT });
            let renderer = CStr::from_ptr((gl.glGetString)(GL_RENDERER) as *const c_char).to_string_lossy();
            tracing::info!(%renderer, "OpenGL ES (ANGLE) context for mpv");
            // ANGLE falls back to WARP (a CPU rasterizer) without a usable GPU.
            if !super::video::hardware_renderer(&renderer) || renderer.contains("WARP") {
                return Err(fail(&egl, failed(format!("{renderer} is a software renderer"))));
            }

            // ANGLE's own D3D11 device: the shared textures (and mpv's decoder) live on it.
            let mut device: egl::Attrib = 0;
            let mut d3d: egl::Attrib = 0;
            if (ext.eglQueryDisplayAttribEXT)(display.as_ptr(), EGL_DEVICE_EXT, &mut device) != egl::TRUE
                || (ext.eglQueryDeviceAttribEXT)(device as *mut c_void, EGL_D3D11_DEVICE_ANGLE, &mut d3d) != egl::TRUE
            {
                return Err(fail(&egl, failed("ANGLE isn't running on D3D11")));
            }
            let raw = d3d as *mut c_void;
            let Some(device) = ID3D11Device::from_raw_borrowed(&raw).cloned() else {
                return Err(fail(&egl, failed("ANGLE returned no D3D11 device")));
            };

            let api_type = cstring("opengl");
            let mut init = OpenGlInitParams {
                get_proc_address,
                get_proc_address_ctx: &*egl as *const Egl as *mut c_void,
            };
            let mut params = [
                RenderParam { kind: PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
                RenderParam { kind: PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
                RenderParam { kind: PARAM_INVALID, data: ptr::null_mut() },
            ];
            let mut mpv_context = ptr::null_mut();
            let code = (core.api.render_create)(&mut mpv_context, core.mpv, params.as_mut_ptr());
            if let Err(err) = core.api.check("mpv_render_context_create(opengl)", code) {
                return Err(fail(&egl, err));
            }
            (core.api.render_set_update_callback)(
                mpv_context,
                Some(on_update),
                &control.wakeup as *const Wakeup as *mut c_void,
            );

            Ok(Self {
                egl,
                display,
                config,
                context,
                surface,
                device,
                gl,
                mpv_context,
                ring: Vec::new(),
                size: (0, 0),
                next: 0,
            })
        }
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
        // SAFETY: the context is current on this thread; free mpv's context (which releases
        // its hold on the D3D11 device) before ours.
        unsafe {
            (core.api.render_set_update_callback)(self.mpv_context, None, ptr::null_mut());
            (core.api.render_free)(self.mpv_context);
        }
    }

    fn render(&mut self, core: &Core, (width, height): (u32, u32)) -> Result<SharedFrame> {
        if self.size != (width, height) || self.ring.is_empty() {
            self.release_ring();
            for _ in 0..RING {
                let slot = self.create_slot(width, height)?;
                self.ring.push(slot);
            }
            self.size = (width, height);
            self.next = 0;
        }
        let slot = &self.ring[self.next];
        self.next = (self.next + 1) % RING;

        let mut fbo = OpenGlFbo {
            fbo: slot.fbo as c_int,
            w: width as c_int,
            h: height as c_int,
            internal_format: 0,
        };
        let mut flip: c_int = 0;
        let mut params = [
            RenderParam { kind: PARAM_OPENGL_FBO, data: &mut fbo as *mut _ as *mut c_void },
            RenderParam { kind: PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: PARAM_INVALID, data: ptr::null_mut() },
        ];
        // SAFETY: GL calls on the thread owning the context; `slot` is complete.
        unsafe {
            let code = (core.api.render_render)(self.mpv_context, params.as_mut_ptr());
            // Make sure the frame is complete before GPUI's device reads it.
            (self.gl.glFinish)();
            core.api.check("mpv_render_context_render", code)?;
        }
        Ok(SharedFrame {
            handle: slot.share.0 as usize,
            width,
            height,
        })
    }

    fn create_slot(&self, width: u32, height: u32) -> Result<Slot> {
        // SAFETY: D3D11/EGL/GL calls with valid objects on the context's thread.
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
                ..Default::default()
            };
            let mut texture = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut texture)).map_err(failed)?;
            let texture: ID3D11Texture2D = texture.ok_or_else(|| failed("no texture"))?;
            let share = texture.cast::<IDXGIResource>().map_err(failed)?.GetSharedHandle().map_err(failed)?;

            let attributes = [
                egl::WIDTH, width as egl::Int,
                egl::HEIGHT, height as egl::Int,
                egl::TEXTURE_FORMAT, egl::TEXTURE_RGBA,
                egl::TEXTURE_TARGET, egl::TEXTURE_2D,
                egl::NONE,
            ];
            let surface = self
                .egl
                .create_pbuffer_from_client_buffer(
                    self.display,
                    EGL_D3D_TEXTURE_ANGLE,
                    egl::ClientBuffer::from_ptr(texture.as_raw()),
                    self.config,
                    &attributes,
                )
                .map_err(|e| failed(format!("wrapping the D3D11 texture: {e}")))?;

            let mut gl_texture = 0;
            (self.gl.glGenTextures)(1, &mut gl_texture);
            (self.gl.glBindTexture)(GL_TEXTURE_2D, gl_texture);
            let bound = self.egl.bind_tex_image(self.display, surface, egl::BACK_BUFFER);
            (self.gl.glBindTexture)(GL_TEXTURE_2D, 0);
            let mut fbo = 0;
            (self.gl.glGenFramebuffers)(1, &mut fbo);
            let slot = Slot { _texture: texture, share, surface, gl_texture, fbo };
            if let Err(err) = bound {
                self.release_slot(&slot);
                return Err(failed(format!("binding the D3D11 texture: {err}")));
            }
            (self.gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
            (self.gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, gl_texture, 0);
            let status = (self.gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            (self.gl.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
            if status != GL_FRAMEBUFFER_COMPLETE {
                self.release_slot(&slot);
                return Err(failed(format!("framebuffer incomplete: {status:#x}")));
            }
            Ok(slot)
        }
    }

    fn release_slot(&self, slot: &Slot) {
        // SAFETY: objects created by this renderer, on the context's thread.
        unsafe {
            (self.gl.glDeleteFramebuffers)(1, &slot.fbo);
            let _ = self.egl.release_tex_image(self.display, slot.surface, egl::BACK_BUFFER);
            (self.gl.glDeleteTextures)(1, &slot.gl_texture);
        }
        let _ = self.egl.destroy_surface(self.display, slot.surface);
    }

    fn release_ring(&mut self) {
        for slot in std::mem::take(&mut self.ring) {
            self.release_slot(&slot);
        }
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        self.release_ring();
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
