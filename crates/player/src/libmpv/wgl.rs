//! GPU video output on Windows: mpv renders through OpenGL (libmpv's only GPU render API) into
//! D3D11 textures registered with `WGL_NV_DX_interop2`. GPUI's DirectX renderer opens the
//! textures by their share handles and draws them, so frames never pass through the CPU.
//! See `docs/in-app-video.md` (phase 2) and `examples/dx_interop_probe.rs`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;

use tokio::sync::watch;
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource;
use windows::Win32::Graphics::Gdi::{GetDC, HDC, ReleaseDC};
use windows::Win32::Graphics::OpenGL::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW};
use windows::core::{BOOL, Interface, PCSTR, w};

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
const WGL_ACCESS_WRITE_DISCARD_NV: u32 = 0x0002;

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

#[allow(non_snake_case)]
struct Interop {
    wglDXOpenDeviceNV: unsafe extern "system" fn(*mut c_void) -> HANDLE,
    wglDXCloseDeviceNV: unsafe extern "system" fn(HANDLE) -> BOOL,
    wglDXRegisterObjectNV: unsafe extern "system" fn(HANDLE, *mut c_void, u32, u32, u32) -> HANDLE,
    wglDXUnregisterObjectNV: unsafe extern "system" fn(HANDLE, HANDLE) -> BOOL,
    wglDXLockObjectsNV: unsafe extern "system" fn(HANDLE, i32, *mut HANDLE) -> BOOL,
    wglDXUnlockObjectsNV: unsafe extern "system" fn(HANDLE, i32, *mut HANDLE) -> BOOL,
    glGenFramebuffers: unsafe extern "system" fn(i32, *mut u32),
    glDeleteFramebuffers: unsafe extern "system" fn(i32, *const u32),
    glBindFramebuffer: unsafe extern "system" fn(u32, u32),
    glFramebufferTexture2D: unsafe extern "system" fn(u32, u32, u32, u32, i32),
    glCheckFramebufferStatus: unsafe extern "system" fn(u32) -> u32,
}

/// Any OpenGL function: `wglGetProcAddress` for extensions and GL > 1.1, `opengl32.dll` for the
/// rest (for which `wglGetProcAddress` returns 0..=3 or -1).
unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    unsafe {
        let name = PCSTR(name as *const u8);
        if let Some(f) = wglGetProcAddress(name) {
            let address = f as usize;
            if !matches!(address, 1 | 2 | 3 | usize::MAX) {
                return address as *mut c_void;
            }
        }
        match GetModuleHandleW(w!("opengl32.dll")) {
            Ok(module) => GetProcAddress(module, name).map_or(ptr::null_mut(), |f| f as *mut c_void),
            Err(_) => ptr::null_mut(),
        }
    }
}

fn gl_proc<T: Copy>(name: &str) -> Result<T> {
    let cname = CString::new(name).expect("no nul");
    // SAFETY: a current context exists; the caller declares `T` as the matching fn type.
    let f = unsafe { get_proc_address(ptr::null_mut(), cname.as_ptr()) };
    if f.is_null() {
        return Err(Error::LibmpvUnavailable(format!("OpenGL function {name} missing")));
    }
    // SAFETY: `T` is a function pointer type of the same size.
    Ok(unsafe { std::mem::transmute_copy(&f) })
}

fn failed(what: impl std::fmt::Display) -> Error {
    Error::GpuUnavailable(what.to_string())
}

/// One shared texture, registered with GL and attached to its own framebuffer.
struct Slot {
    texture: ID3D11Texture2D,
    share: HANDLE,
    gl_texture: u32,
    fbo: u32,
    interop: HANDLE,
}

pub(crate) struct GpuRenderer {
    hwnd: HWND,
    hdc: HDC,
    glrc: HGLRC,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    interop_device: HANDLE,
    fns: Interop,
    mpv_context: *mut c_void,
    ring: Vec<Slot>,
    size: (u32, u32),
    next: usize,
}

impl GpuRenderer {
    /// Sets up OpenGL, D3D11 and the mpv render context on the calling (render) thread.
    pub fn new(core: &Core, control: &RenderControl) -> Result<Self> {
        // SAFETY: Win32/WGL/D3D11 setup on this thread; every handle is checked.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(failed)?;
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("jelly-gpui mpv"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(failed)?;
            let hdc = GetDC(Some(hwnd));
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                ..Default::default()
            };
            let cleanup_window = || {
                ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
            };
            if SetPixelFormat(hdc, ChoosePixelFormat(hdc, &pfd), &pfd).is_err() {
                cleanup_window();
                return Err(failed("SetPixelFormat failed"));
            }
            let glrc = match wglCreateContext(hdc) {
                Ok(glrc) => glrc,
                Err(err) => {
                    cleanup_window();
                    return Err(failed(err));
                }
            };
            if let Err(err) = wglMakeCurrent(hdc, glrc) {
                let _ = wglDeleteContext(glrc);
                cleanup_window();
                return Err(failed(err));
            }

            let fail = |err: Error| {
                let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                let _ = wglDeleteContext(glrc);
                cleanup_window();
                err
            };
            let renderer = CStr::from_ptr(glGetString(GL_RENDERER) as *const c_char).to_string_lossy();
            tracing::info!(%renderer, "OpenGL context for mpv");
            if !super::video::hardware_renderer(&renderer) {
                return Err(fail(failed(format!("{renderer} is a software OpenGL renderer"))));
            }

            let extensions: unsafe extern "system" fn(HDC) -> *const c_char =
                gl_proc("wglGetExtensionsStringARB").map_err(fail)?;
            let extensions = CStr::from_ptr(extensions(hdc)).to_string_lossy().into_owned();
            if !extensions.split(' ').any(|e| e == "WGL_NV_DX_interop2") {
                return Err(fail(failed("WGL_NV_DX_interop2 isn't supported")));
            }
            let fns = (|| {
                Ok(Interop {
                    wglDXOpenDeviceNV: gl_proc("wglDXOpenDeviceNV")?,
                    wglDXCloseDeviceNV: gl_proc("wglDXCloseDeviceNV")?,
                    wglDXRegisterObjectNV: gl_proc("wglDXRegisterObjectNV")?,
                    wglDXUnregisterObjectNV: gl_proc("wglDXUnregisterObjectNV")?,
                    wglDXLockObjectsNV: gl_proc("wglDXLockObjectsNV")?,
                    wglDXUnlockObjectsNV: gl_proc("wglDXUnlockObjectsNV")?,
                    glGenFramebuffers: gl_proc("glGenFramebuffers")?,
                    glDeleteFramebuffers: gl_proc("glDeleteFramebuffers")?,
                    glBindFramebuffer: gl_proc("glBindFramebuffer")?,
                    glFramebufferTexture2D: gl_proc("glFramebufferTexture2D")?,
                    glCheckFramebufferStatus: gl_proc("glCheckFramebufferStatus")?,
                })
            })()
            .map_err(fail)?;

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| fail(failed(e)))?;
            let (device, context): (ID3D11Device, ID3D11DeviceContext) =
                (device.ok_or_else(|| fail(failed("no D3D11 device")))?, context.unwrap());
            let interop_device = (fns.wglDXOpenDeviceNV)(device.as_raw());
            if interop_device.0.is_null() {
                return Err(fail(failed("wglDXOpenDeviceNV failed")));
            }

            let api_type = cstring("opengl");
            let mut init = OpenGlInitParams {
                get_proc_address,
                get_proc_address_ctx: ptr::null_mut(),
            };
            let mut params = [
                RenderParam { kind: PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
                RenderParam { kind: PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
                RenderParam { kind: PARAM_INVALID, data: ptr::null_mut() },
            ];
            let mut mpv_context = ptr::null_mut();
            let code = (core.api.render_create)(&mut mpv_context, core.mpv, params.as_mut_ptr());
            if let Err(err) = core.api.check("mpv_render_context_create(opengl)", code) {
                let _ = (fns.wglDXCloseDeviceNV)(interop_device);
                return Err(fail(err));
            }
            (core.api.render_set_update_callback)(
                mpv_context,
                Some(on_update),
                &control.wakeup as *const Wakeup as *mut c_void,
            );

            Ok(Self {
                hwnd,
                hdc,
                glrc,
                device,
                context,
                interop_device,
                fns,
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
        // SAFETY: the context is current on this thread; free mpv's context before the GL one.
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

        // SAFETY: GL and interop calls on the thread owning the context; `slot` is registered.
        unsafe {
            let mut objects = [slot.interop];
            if !(self.fns.wglDXLockObjectsNV)(self.interop_device, 1, objects.as_mut_ptr()).as_bool() {
                return Err(failed("wglDXLockObjectsNV failed"));
            }
            let mut fbo = OpenGlFbo {
                fbo: slot.fbo as c_int,
                w: width as c_int,
                h: height as c_int,
                internal_format: 0,
            };
            // The interop texture already has D3D's row order (verified on AMD), so no flip.
            let mut flip: c_int = 0;
            let mut params = [
                RenderParam { kind: PARAM_OPENGL_FBO, data: &mut fbo as *mut _ as *mut c_void },
                RenderParam { kind: PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
                RenderParam { kind: PARAM_INVALID, data: ptr::null_mut() },
            ];
            let code = (core.api.render_render)(self.mpv_context, params.as_mut_ptr());
            // Make sure the frame is complete before another device reads it.
            glFinish();
            let _ = (self.fns.wglDXUnlockObjectsNV)(self.interop_device, 1, objects.as_mut_ptr());
            self.context.Flush();
            core.api.check("mpv_render_context_render", code)?;
        }
        Ok(SharedFrame {
            handle: slot.share.0 as usize,
            width,
            height,
        })
    }

    fn create_slot(&self, width: u32, height: u32) -> Result<Slot> {
        // SAFETY: D3D11/GL calls with valid devices on the context's thread.
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
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

            let mut gl_texture = 0;
            glGenTextures(1, &mut gl_texture);
            let interop = (self.fns.wglDXRegisterObjectNV)(
                self.interop_device,
                texture.as_raw(),
                gl_texture,
                GL_TEXTURE_2D,
                WGL_ACCESS_WRITE_DISCARD_NV,
            );
            if interop.0.is_null() {
                glDeleteTextures(1, &gl_texture);
                return Err(failed("wglDXRegisterObjectNV failed"));
            }

            // Attach while locked; the attachment stays valid across lock/unlock.
            let mut objects = [interop];
            let _ = (self.fns.wglDXLockObjectsNV)(self.interop_device, 1, objects.as_mut_ptr());
            let mut fbo = 0;
            (self.fns.glGenFramebuffers)(1, &mut fbo);
            (self.fns.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
            (self.fns.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, gl_texture, 0);
            let status = (self.fns.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            (self.fns.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
            let _ = (self.fns.wglDXUnlockObjectsNV)(self.interop_device, 1, objects.as_mut_ptr());
            let slot = Slot { texture, share, gl_texture, fbo, interop };
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
            (self.fns.glDeleteFramebuffers)(1, &slot.fbo);
            let _ = (self.fns.wglDXUnregisterObjectNV)(self.interop_device, slot.interop);
            glDeleteTextures(1, &slot.gl_texture);
        }
        let _ = &slot.texture;
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
        // SAFETY: tear down in reverse order of creation, on the thread that created them.
        unsafe {
            let _ = (self.fns.wglDXCloseDeviceNV)(self.interop_device);
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
            let _ = wglDeleteContext(self.glrc);
            ReleaseDC(Some(self.hwnd), self.hdc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
