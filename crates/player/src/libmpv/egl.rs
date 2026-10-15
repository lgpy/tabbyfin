//! GPU video output on Linux: mpv renders through OpenGL (libmpv's only GPU render API) into
//! Vulkan images whose memory is exported as opaque fds (`VK_KHR_external_memory_fd`) and
//! imported into GL (`GL_EXT_memory_object_fd`). GPUI's wgpu (Vulkan) renderer imports the same
//! memory and draws it, so frames never pass through the CPU.
//! See `docs/in-app-video.md` (phase 3) and `examples/vk_interop_probe.rs`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use ash::{khr, vk};
use khronos_egl as egl;
use tokio::sync::watch;

use super::video::{
    Frame, PARAM_API_TYPE, PARAM_INVALID, RenderControl, SharedFrame, UPDATE_FRAME, Wakeup, fit, on_update,
};
use super::{Core, RenderParam, cstring};
use crate::{Error, Result};

const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA8: u32 = 0x8058;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RENDERER: u32 = 0x1F01;
const GL_EXTENSIONS: u32 = 0x1F03;
const GL_NUM_EXTENSIONS: u32 = 0x821D;
const GL_HANDLE_TYPE_OPAQUE_FD_EXT: u32 = 0x9586;
const GL_DEDICATED_MEMORY_OBJECT_EXT: u32 = 0x9581;
const GL_TEXTURE_TILING_EXT: u32 = 0x9580;
const GL_LINEAR_TILING_EXT: i32 = 0x9585;
const GL_DEVICE_UUID_EXT: u32 = 0x9597;

/// The shared image layout; GPUI's importer (`gpui_wgpu::shared_texture`) must match it.
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// Frames in flight: GPUI draws the latest while mpv renders into the oldest.
const RING: usize = 3;

/// Ids for shared images, unique across players in this process.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

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

type Egl = egl::DynamicInstance<egl::EGL1_4>;

#[allow(non_snake_case)]
struct Gl {
    glGetString: unsafe extern "C" fn(u32) -> *const u8,
    glGetStringi: unsafe extern "C" fn(u32, u32) -> *const u8,
    glGetIntegerv: unsafe extern "C" fn(u32, *mut i32),
    glGetUnsignedBytei_vEXT: unsafe extern "C" fn(u32, u32, *mut u8),
    glCreateMemoryObjectsEXT: unsafe extern "C" fn(i32, *mut u32),
    glDeleteMemoryObjectsEXT: unsafe extern "C" fn(i32, *const u32),
    glMemoryObjectParameterivEXT: unsafe extern "C" fn(u32, u32, *const i32),
    glImportMemoryFdEXT: unsafe extern "C" fn(u32, u64, u32, i32),
    glGenTextures: unsafe extern "C" fn(i32, *mut u32),
    glDeleteTextures: unsafe extern "C" fn(i32, *const u32),
    glBindTexture: unsafe extern "C" fn(u32, u32),
    glTexParameteri: unsafe extern "C" fn(u32, u32, i32),
    glTexStorageMem2DEXT: unsafe extern "C" fn(u32, i32, u32, i32, i32, u32, u64),
    glGenFramebuffers: unsafe extern "C" fn(i32, *mut u32),
    glDeleteFramebuffers: unsafe extern "C" fn(i32, *const u32),
    glBindFramebuffer: unsafe extern "C" fn(u32, u32),
    glFramebufferTexture2D: unsafe extern "C" fn(u32, u32, u32, u32, i32),
    glCheckFramebufferStatus: unsafe extern "C" fn(u32) -> u32,
    glFinish: unsafe extern "C" fn(),
}

/// A GL function pointer from EGL as its declared type.
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

/// libmpv's GL loader callback; `ctx` is the `Egl` instance.
unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` points to the renderer's `Egl`, alive while the mpv context exists.
    let egl = unsafe { &*(ctx as *const Egl) };
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return ptr::null_mut();
    };
    egl.get_proc_address(name).map_or(ptr::null_mut(), |f| f as *mut c_void)
}

/// One shared image: Vulkan memory exported to GPUI (`fd`) and imported into GL.
struct Slot {
    id: u64,
    image: vk::Image,
    memory: vk::DeviceMemory,
    allocation_size: u64,
    /// Kept open for GPUI, which duplicates it on import.
    fd: OwnedFd,
    gl_memory: u32,
    gl_texture: u32,
    fbo: u32,
}

/// The exporting Vulkan device. It only allocates memory; GL renders and GPUI samples.
struct Exporter {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    fd_loader: khr::external_memory_fd::Device,
}

impl Exporter {
    /// Picks the Vulkan device with OpenGL's device UUID, so both share the same GPU.
    fn new(gl_uuid: [u8; 16]) -> Result<Self> {
        // SAFETY: plain Vulkan setup; objects are destroyed in `Drop`.
        unsafe {
            let entry = ash::Entry::load().map_err(failed)?;
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
            let instance = entry
                .create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None)
                .map_err(failed)?;
            let physical = instance
                .enumerate_physical_devices()
                .map_err(failed)?
                .into_iter()
                .find(|&device| {
                    let mut id = vk::PhysicalDeviceIDProperties::default();
                    let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
                    instance.get_physical_device_properties2(device, &mut props);
                    id.device_uuid == gl_uuid
                });
            let Some(physical) = physical else {
                instance.destroy_instance(None);
                return Err(failed("no Vulkan device matches the OpenGL device"));
            };
            let queue_family = instance
                .get_physical_device_queue_family_properties(physical)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .unwrap_or(0) as u32;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family)
                .queue_priorities(&priorities)];
            let extensions = [khr::external_memory_fd::NAME.as_ptr()];
            let device = match instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            ) {
                Ok(device) => device,
                Err(err) => {
                    instance.destroy_instance(None);
                    return Err(failed(err));
                }
            };
            let fd_loader = khr::external_memory_fd::Device::new(&instance, &device);
            Ok(Self {
                _entry: entry,
                instance,
                physical,
                device,
                fd_loader,
            })
        }
    }

    /// An image with exportable dedicated memory. Returns it, its memory and the size.
    fn allocate(&self, width: u32, height: u32) -> Result<(vk::Image, vk::DeviceMemory, u64)> {
        // SAFETY: valid device; objects are freed by `free` (or here on failure).
        unsafe {
            let mut external =
                vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
            let info = vk::ImageCreateInfo::default()
                .push_next(&mut external)
                .image_type(vk::ImageType::TYPE_2D)
                .format(FORMAT)
                .extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::LINEAR)
                .usage(
                    vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSFER_SRC,
                )
                .initial_layout(vk::ImageLayout::PREINITIALIZED);
            let image = self.device.create_image(&info, None).map_err(failed)?;
            let requirements = self.device.get_image_memory_requirements(image);
            let properties = self.instance.get_physical_device_memory_properties(self.physical);
            let candidates =
                (0..properties.memory_type_count).filter(|i| requirements.memory_type_bits & (1 << i) != 0);
            let memory_type = candidates
                .clone()
                .find(|i| {
                    properties.memory_types[*i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                })
                .or_else(|| candidates.clone().next());
            let Some(memory_type) = memory_type else {
                self.device.destroy_image(image, None);
                return Err(failed("no memory type for the shared image"));
            };
            let mut export =
                vk::ExportMemoryAllocateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let allocate = vk::MemoryAllocateInfo::default()
                .push_next(&mut export)
                .push_next(&mut dedicated)
                .allocation_size(requirements.size)
                .memory_type_index(memory_type);
            let memory = match self.device.allocate_memory(&allocate, None) {
                Ok(memory) => memory,
                Err(err) => {
                    self.device.destroy_image(image, None);
                    return Err(failed(err));
                }
            };
            if let Err(err) = self.device.bind_image_memory(image, memory, 0) {
                self.free(image, memory);
                return Err(failed(err));
            }
            Ok((image, memory, requirements.size))
        }
    }

    /// A new fd for `memory`, owned by the caller.
    fn export(&self, memory: vk::DeviceMemory) -> Result<OwnedFd> {
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        // SAFETY: valid exportable memory; the fd is new and owned by us.
        let fd = unsafe { self.fd_loader.get_memory_fd(&info) }.map_err(failed)?;
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn free(&self, image: vk::Image, memory: vk::DeviceMemory) {
        // SAFETY: objects created by this device and no longer used by it.
        unsafe {
            self.device.destroy_image(image, None);
            self.device.free_memory(memory, None);
        }
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        // SAFETY: all images and memory were freed first.
        unsafe {
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

pub(crate) struct GpuRenderer {
    /// Boxed so its address is stable for libmpv's `get_proc_address` context.
    egl: Box<Egl>,
    display: egl::Display,
    context: egl::Context,
    gl: Gl,
    exporter: Exporter,
    mpv_context: *mut c_void,
    ring: Vec<Slot>,
    size: (u32, u32),
    next: usize,
}

impl GpuRenderer {
    /// Sets up EGL/OpenGL, the exporting Vulkan device and the mpv render context on the
    /// calling (render) thread.
    pub fn new(core: &Core, control: &RenderControl) -> Result<Self> {
        // SAFETY: EGL/GL setup on this thread; every object is checked and cleaned up.
        unsafe {
            let egl = Box::new(Egl::load_required().map_err(failed)?);
            let display = egl
                .get_display(egl::DEFAULT_DISPLAY)
                .ok_or_else(|| failed("no EGL display"))?;
            egl.initialize(display).map_err(failed)?;
            egl.bind_api(egl::OPENGL_API).map_err(failed)?;
            let attributes = [egl::RENDERABLE_TYPE, egl::OPENGL_BIT, egl::NONE];
            let config = egl
                .choose_first_config(display, &attributes)
                .map_err(failed)?
                .ok_or_else(|| failed("no EGL config"))?;
            let context = egl
                .create_context(display, config, None, &[egl::NONE])
                .map_err(failed)?;
            let fail = |egl: &Egl, err: Error| {
                let _ = egl.make_current(display, None, None, None);
                let _ = egl.destroy_context(display, context);
                err
            };
            if let Err(err) = egl.make_current(display, None, None, Some(context)) {
                return Err(fail(&egl, failed(err)));
            }

            macro_rules! gl {
                ($($name:ident),*) => {
                    Gl { $($name: match egl.get_proc_address(stringify!($name)) {
                        Some(f) => cast(f),
                        None => return Err(fail(&egl, failed(concat!(stringify!($name), " missing")))),
                    },)* }
                };
            }
            let gl = gl!(
                glGetString,
                glGetStringi,
                glGetIntegerv,
                glGetUnsignedBytei_vEXT,
                glCreateMemoryObjectsEXT,
                glDeleteMemoryObjectsEXT,
                glMemoryObjectParameterivEXT,
                glImportMemoryFdEXT,
                glGenTextures,
                glDeleteTextures,
                glBindTexture,
                glTexParameteri,
                glTexStorageMem2DEXT,
                glGenFramebuffers,
                glDeleteFramebuffers,
                glBindFramebuffer,
                glFramebufferTexture2D,
                glCheckFramebufferStatus,
                glFinish
            );
            let renderer = CStr::from_ptr((gl.glGetString)(GL_RENDERER) as *const c_char).to_string_lossy();
            tracing::info!(%renderer, "OpenGL context for mpv");
            if !super::video::hardware_renderer(&renderer) {
                return Err(fail(&egl, failed(format!("{renderer} is a software OpenGL renderer"))));
            }
            super::video::adapt_to_renderer(core, &renderer);
            let mut count = 0;
            (gl.glGetIntegerv)(GL_NUM_EXTENSIONS, &mut count);
            let has_memory_object = (0..count as u32).any(|i| {
                CStr::from_ptr((gl.glGetStringi)(GL_EXTENSIONS, i) as *const c_char).to_bytes()
                    == b"GL_EXT_memory_object_fd"
            });
            if !has_memory_object {
                return Err(fail(&egl, failed("GL_EXT_memory_object_fd isn't supported")));
            }
            let mut uuid = [0u8; 16];
            (gl.glGetUnsignedBytei_vEXT)(GL_DEVICE_UUID_EXT, 0, uuid.as_mut_ptr());
            let exporter = Exporter::new(uuid).map_err(|err| fail(&egl, err))?;

            let api_type = cstring("opengl");
            let mut init = OpenGlInitParams {
                get_proc_address,
                get_proc_address_ctx: &*egl as *const Egl as *mut c_void,
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
            let code = (core.api.render_create)(&mut mpv_context, core.mpv, params.as_mut_ptr());
            if let Err(err) = core.api.check("mpv_render_context_create(opengl)", code) {
                drop(exporter);
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
                context,
                gl,
                exporter,
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
        // SAFETY: the GL context is current on this thread; free mpv's context before ours.
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
        // mpv's FBO output already has the row order Vulkan samples in (verified on Mesa).
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
        // SAFETY: render context calls on its thread; the FBO is complete.
        let code = unsafe { (core.api.render_render)(self.mpv_context, params.as_mut_ptr()) };
        // Make sure the frame is complete before GPUI's device reads it.
        unsafe { (self.gl.glFinish)() };
        core.api.check("mpv_render_context_render", code)?;
        Ok(SharedFrame {
            id: slot.id,
            fd: slot.fd.as_raw_fd(),
            allocation_size: slot.allocation_size,
            width,
            height,
        })
    }

    fn create_slot(&self, width: u32, height: u32) -> Result<Slot> {
        let (image, memory, allocation_size) = self.exporter.allocate(width, height)?;
        let exported = (|| Ok::<_, Error>((self.exporter.export(memory)?, self.exporter.export(memory)?)))();
        let (gpui_fd, gl_fd) = match exported {
            Ok(fds) => fds,
            Err(err) => {
                self.exporter.free(image, memory);
                return Err(err);
            }
        };
        let gl = &self.gl;
        // SAFETY: GL calls with the context current; the import takes ownership of `gl_fd`.
        unsafe {
            let mut gl_memory = 0;
            (gl.glCreateMemoryObjectsEXT)(1, &mut gl_memory);
            (gl.glMemoryObjectParameterivEXT)(gl_memory, GL_DEDICATED_MEMORY_OBJECT_EXT, &1);
            (gl.glImportMemoryFdEXT)(
                gl_memory,
                allocation_size,
                GL_HANDLE_TYPE_OPAQUE_FD_EXT,
                std::os::fd::IntoRawFd::into_raw_fd(gl_fd),
            );
            let mut gl_texture = 0;
            (gl.glGenTextures)(1, &mut gl_texture);
            (gl.glBindTexture)(GL_TEXTURE_2D, gl_texture);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_TILING_EXT, GL_LINEAR_TILING_EXT);
            (gl.glTexStorageMem2DEXT)(GL_TEXTURE_2D, 1, GL_RGBA8, width as i32, height as i32, gl_memory, 0);
            (gl.glBindTexture)(GL_TEXTURE_2D, 0);
            let mut fbo = 0;
            (gl.glGenFramebuffers)(1, &mut fbo);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, gl_texture, 0);
            let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
            let slot = Slot {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                image,
                memory,
                allocation_size,
                fd: gpui_fd,
                gl_memory,
                gl_texture,
                fbo,
            };
            if status != GL_FRAMEBUFFER_COMPLETE {
                self.release_slot(slot);
                return Err(failed(format!("framebuffer incomplete: {status:#x}")));
            }
            Ok(slot)
        }
    }

    fn release_slot(&self, slot: Slot) {
        let gl = &self.gl;
        // SAFETY: objects created by this renderer, with the context current. GPUI holds its
        // own import of the memory, so freeing ours doesn't pull it out from under a frame.
        unsafe {
            (gl.glDeleteFramebuffers)(1, &slot.fbo);
            (gl.glDeleteTextures)(1, &slot.gl_texture);
            (gl.glDeleteMemoryObjectsEXT)(1, &slot.gl_memory);
        }
        self.exporter.free(slot.image, slot.memory);
        drop(slot.fd);
    }

    fn release_ring(&mut self) {
        for slot in std::mem::take(&mut self.ring) {
            self.release_slot(slot);
        }
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        self.release_ring();
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
    }
}
