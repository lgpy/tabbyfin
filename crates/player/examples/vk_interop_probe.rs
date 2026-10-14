//! Linux GPU interop probe for docs/in-app-video.md phase 3.
//!
//! Models the planned path: a Vulkan device (the player) allocates an exportable, linearly
//! tiled image and exports its memory twice as opaque fds. OpenGL (which libmpv renders
//! through, via EGL) imports one with GL_EXT_memory_object_fd and clears it to a known color.
//! A second, independent Vulkan device (standing in for GPUI's wgpu renderer) imports the other
//! and reads the pixel back.
//!
//! ```sh
//! cargo run -p player --example vk_interop_probe
//! ```

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("this probe only runs on Linux");
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(err) = probe::run() {
        eprintln!("probe failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod probe {
    use std::ffi::{CStr, c_void};

    use ash::{khr, vk};

    type Error = Box<dyn std::error::Error>;

    const SIZE: u32 = 64;
    const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
    /// GL clears to r=1.0, g=0.0, b=0.5; read back as RGBA bytes.
    const EXPECTED_RGBA: [u8; 4] = [255, 0, 128, 255];

    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_RGBA8: u32 = 0x8058;
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_VERSION: u32 = 0x1F02;
    const GL_EXTENSIONS: u32 = 0x1F03;
    const GL_NUM_EXTENSIONS: u32 = 0x821D;
    const GL_HANDLE_TYPE_OPAQUE_FD_EXT: u32 = 0x9586;
    const GL_DEDICATED_MEMORY_OBJECT_EXT: u32 = 0x9581;
    const GL_TEXTURE_TILING_EXT: u32 = 0x9580;
    const GL_LINEAR_TILING_EXT: i32 = 0x9585;
    const GL_DEVICE_UUID_EXT: u32 = 0x9597;
    const GL_NUM_DEVICE_UUIDS_EXT: u32 = 0x9596;

    /// One Vulkan device with an image of the shared layout.
    struct Vk {
        _entry: ash::Entry,
        instance: ash::Instance,
        physical: vk::PhysicalDevice,
        device: ash::Device,
        queue: vk::Queue,
        queue_family: u32,
        uuid: [u8; 16],
        name: String,
    }

    impl Vk {
        fn new() -> Result<Self, Error> {
            unsafe {
                let entry = ash::Entry::load()?;
                let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
                let instance = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None)?;
                let physical = *instance.enumerate_physical_devices()?.first().ok_or("no Vulkan device")?;
                let mut id = vk::PhysicalDeviceIDProperties::default();
                let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
                instance.get_physical_device_properties2(physical, &mut props);
                let name = props.properties.device_name_as_c_str()?.to_string_lossy().into_owned();
                let uuid = id.device_uuid;
                let queue_family = instance
                    .get_physical_device_queue_family_properties(physical)
                    .iter()
                    .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                    .ok_or("no graphics queue")? as u32;
                let priorities = [1.0];
                let queues = [vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(queue_family)
                    .queue_priorities(&priorities)];
                let extensions = [khr::external_memory_fd::NAME.as_ptr()];
                let device = instance.create_device(
                    physical,
                    &vk::DeviceCreateInfo::default()
                        .queue_create_infos(&queues)
                        .enabled_extension_names(&extensions),
                    None,
                )?;
                let queue = device.get_device_queue(queue_family, 0);
                Ok(Self { _entry: entry, instance, physical, device, queue, queue_family, uuid, name })
            }
        }

        /// The shared image layout: the creation parameters must match on every importer.
        fn create_image(&self, initial_layout: vk::ImageLayout) -> Result<vk::Image, Error> {
            let mut external = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
            let info = vk::ImageCreateInfo::default()
                .push_next(&mut external)
                .image_type(vk::ImageType::TYPE_2D)
                .format(FORMAT)
                .extent(vk::Extent3D { width: SIZE, height: SIZE, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::LINEAR)
                .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                .initial_layout(initial_layout);
            Ok(unsafe { self.device.create_image(&info, None)? })
        }

        fn memory_type(&self, bits: u32) -> Result<u32, Error> {
            let props = unsafe { self.instance.get_physical_device_memory_properties(self.physical) };
            (0..props.memory_type_count)
                .find(|i| bits & (1 << i) != 0 && props.memory_types[*i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
                .or_else(|| (0..props.memory_type_count).find(|i| bits & (1 << i) != 0))
                .ok_or_else(|| "no memory type".into())
        }

        /// Allocates exportable dedicated memory for `image`. Returns it and its size.
        fn export(&self, image: vk::Image) -> Result<(vk::DeviceMemory, u64), Error> {
            unsafe {
                let req = self.device.get_image_memory_requirements(image);
                let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
                let info = vk::MemoryAllocateInfo::default()
                    .push_next(&mut export)
                    .push_next(&mut dedicated)
                    .allocation_size(req.size)
                    .memory_type_index(self.memory_type(req.memory_type_bits)?);
                let memory = self.device.allocate_memory(&info, None)?;
                self.device.bind_image_memory(image, memory, 0)?;
                Ok((memory, req.size))
            }
        }

        fn fd(&self, memory: vk::DeviceMemory) -> Result<i32, Error> {
            let loader = khr::external_memory_fd::Device::new(&self.instance, &self.device);
            let info = vk::MemoryGetFdInfoKHR::default().memory(memory).handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
            Ok(unsafe { loader.get_memory_fd(&info)? })
        }

        /// Imports `fd` (taking ownership) as the memory of a new image with the shared layout.
        fn import(&self, fd: i32) -> Result<vk::Image, Error> {
            unsafe {
                let image = self.create_image(vk::ImageLayout::PREINITIALIZED)?;
                let req = self.device.get_image_memory_requirements(image);
                let mut import = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD).fd(fd);
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
                let info = vk::MemoryAllocateInfo::default()
                    .push_next(&mut import)
                    .push_next(&mut dedicated)
                    .allocation_size(req.size)
                    .memory_type_index(self.memory_type(req.memory_type_bits)?);
                let memory = self.device.allocate_memory(&info, None)?;
                self.device.bind_image_memory(image, memory, 0)?;
                Ok(image)
            }
        }

        /// Copies pixel (0, 0) of `image` to a host-visible buffer and returns it.
        fn read_pixel(&self, image: vk::Image) -> Result<[u8; 4], Error> {
            unsafe {
                let d = &self.device;
                let buffer = d.create_buffer(
                    &vk::BufferCreateInfo::default().size(4).usage(vk::BufferUsageFlags::TRANSFER_DST),
                    None,
                )?;
                let req = d.get_buffer_memory_requirements(buffer);
                let props = self.instance.get_physical_device_memory_properties(self.physical);
                let host = (0..props.memory_type_count)
                    .find(|i| {
                        req.memory_type_bits & (1 << i) != 0
                            && props.memory_types[*i as usize].property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)
                    })
                    .ok_or("no host memory")?;
                let memory = d.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(host), None)?;
                d.bind_buffer_memory(buffer, memory, 0)?;

                let pool = d.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(self.queue_family), None)?;
                let cmd = d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default().command_pool(pool).command_buffer_count(1),
                )?[0];
                d.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())?;
                let range = vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1);
                // PREINITIALIZED keeps the linear image's contents through the transition.
                let barrier = vk::ImageMemoryBarrier::default()
                    .image(image)
                    .old_layout(vk::ImageLayout::PREINITIALIZED)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(self.queue_family)
                    .subresource_range(range);
                d.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[barrier]);
                let region = vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1))
                    .image_extent(vk::Extent3D { width: 1, height: 1, depth: 1 });
                d.cmd_copy_image_to_buffer(cmd, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, buffer, &[region]);
                d.end_command_buffer(cmd)?;
                let cmds = [cmd];
                d.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], vk::Fence::null())?;
                d.queue_wait_idle(self.queue)?;
                let ptr = d.map_memory(memory, 0, 4, vk::MemoryMapFlags::empty())? as *const u8;
                let pixel = [*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)];
                d.unmap_memory(memory);
                Ok(pixel)
            }
        }
    }

    #[allow(non_snake_case)]
    struct Gl {
        glGetString: unsafe extern "C" fn(u32) -> *const u8,
        glGetStringi: unsafe extern "C" fn(u32, u32) -> *const u8,
        glGetIntegerv: unsafe extern "C" fn(u32, *mut i32),
        glGetUnsignedBytei_vEXT: unsafe extern "C" fn(u32, u32, *mut u8),
        glCreateMemoryObjectsEXT: unsafe extern "C" fn(i32, *mut u32),
        glMemoryObjectParameterivEXT: unsafe extern "C" fn(u32, u32, *const i32),
        glImportMemoryFdEXT: unsafe extern "C" fn(u32, u64, u32, i32),
        glGenTextures: unsafe extern "C" fn(i32, *mut u32),
        glBindTexture: unsafe extern "C" fn(u32, u32),
        glTexParameteri: unsafe extern "C" fn(u32, u32, i32),
        glTexStorageMem2DEXT: unsafe extern "C" fn(u32, i32, u32, i32, i32, u32, u64),
        glGenFramebuffers: unsafe extern "C" fn(i32, *mut u32),
        glBindFramebuffer: unsafe extern "C" fn(u32, u32),
        glFramebufferTexture2D: unsafe extern "C" fn(u32, u32, u32, u32, i32),
        glCheckFramebufferStatus: unsafe extern "C" fn(u32) -> u32,
        glViewport: unsafe extern "C" fn(i32, i32, i32, i32),
        glClearColor: unsafe extern "C" fn(f32, f32, f32, f32),
        glClear: unsafe extern "C" fn(u32),
        glFinish: unsafe extern "C" fn(),
    }

    pub fn run() -> Result<(), Error> {
        let exporter = Vk::new()?;
        let reader = Vk::new()?;
        println!("Vulkan device: {} (uuid {:02x?})", exporter.name, exporter.uuid);

        // OpenGL through EGL, without a window.
        let egl = unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()? };
        let display = unsafe { egl.get_display(khronos_egl::DEFAULT_DISPLAY) }.ok_or("no EGL display")?;
        let (major, minor) = egl.initialize(display)?;
        egl.bind_api(khronos_egl::OPENGL_API)?;
        let attributes = [khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_BIT, khronos_egl::NONE];
        let config = egl.choose_first_config(display, &attributes)?.ok_or("no EGL config")?;
        let context = egl.create_context(display, config, None, &[khronos_egl::NONE])?;
        egl.make_current(display, None, None, Some(context))?;
        println!("EGL {major}.{minor}");

        macro_rules! gl {
            ($($name:ident),*) => {
                Gl { $($name: {
                    let f = egl.get_proc_address(stringify!($name)).ok_or(concat!(stringify!($name), " missing"))?;
                    unsafe { std::mem::transmute::<extern "system" fn(), _>(f) }
                },)* }
            };
        }
        let gl = gl!(
            glGetString, glGetStringi, glGetIntegerv, glGetUnsignedBytei_vEXT, glCreateMemoryObjectsEXT,
            glMemoryObjectParameterivEXT, glImportMemoryFdEXT, glGenTextures, glBindTexture, glTexParameteri,
            glTexStorageMem2DEXT, glGenFramebuffers, glBindFramebuffer, glFramebufferTexture2D,
            glCheckFramebufferStatus, glViewport, glClearColor, glClear, glFinish
        );

        unsafe {
            let string = |name| CStr::from_ptr((gl.glGetString)(name) as *const i8).to_string_lossy().into_owned();
            println!("GL renderer: {}", string(GL_RENDERER));
            println!("GL version:  {}", string(GL_VERSION));
            let _ = GL_EXTENSIONS;
            let mut count = 0;
            (gl.glGetIntegerv)(GL_NUM_EXTENSIONS, &mut count);
            let extensions: Vec<String> = (0..count as u32)
                .map(|i| CStr::from_ptr((gl.glGetStringi)(GL_EXTENSIONS, i) as *const i8).to_string_lossy().into_owned())
                .collect();
            let has = |name: &str| extensions.iter().any(|e| e == name);
            println!("GL_EXT_memory_object_fd: {}", has("GL_EXT_memory_object_fd"));
            if !has("GL_EXT_memory_object_fd") {
                return Err("GL_EXT_memory_object_fd isn't supported".into());
            }
            let mut uuids = 0;
            (gl.glGetIntegerv)(GL_NUM_DEVICE_UUIDS_EXT, &mut uuids);
            let mut gl_uuid = [0u8; 16];
            (gl.glGetUnsignedBytei_vEXT)(GL_DEVICE_UUID_EXT, 0, gl_uuid.as_mut_ptr());
            println!("GL device uuid matches Vulkan: {} ({uuids} uuid(s))", gl_uuid == exporter.uuid);

            // Exporter: the image and its memory, exported twice.
            let image = exporter.create_image(vk::ImageLayout::UNDEFINED)?;
            let (memory, size) = exporter.export(image)?;
            let gl_fd = exporter.fd(memory)?;
            let reader_fd = exporter.fd(memory)?;

            // GL: import, render.
            let mut object = 0;
            (gl.glCreateMemoryObjectsEXT)(1, &mut object);
            (gl.glMemoryObjectParameterivEXT)(object, GL_DEDICATED_MEMORY_OBJECT_EXT, &1);
            (gl.glImportMemoryFdEXT)(object, size, GL_HANDLE_TYPE_OPAQUE_FD_EXT, gl_fd);
            let mut texture = 0;
            (gl.glGenTextures)(1, &mut texture);
            (gl.glBindTexture)(GL_TEXTURE_2D, texture);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_TILING_EXT, GL_LINEAR_TILING_EXT);
            (gl.glTexStorageMem2DEXT)(GL_TEXTURE_2D, 1, GL_RGBA8, SIZE as i32, SIZE as i32, object, 0);
            let mut fbo = 0;
            (gl.glGenFramebuffers)(1, &mut fbo);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
            let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("GL framebuffer incomplete: {status:#x}").into());
            }
            (gl.glViewport)(0, 0, SIZE as i32, SIZE as i32);
            (gl.glClearColor)(1.0, 0.0, 0.5, 1.0);
            (gl.glClear)(GL_COLOR_BUFFER_BIT);
            (gl.glFinish)();

            // Reader ("GPUI"): import the other fd and read back.
            let imported = reader.import(reader_fd)?;
            let pixel = reader.read_pixel(imported)?;
            let verdict = if pixel == EXPECTED_RGBA { "OK" } else { "WRONG COLOR" };
            println!("GL -> exported Vulkan memory -> second Vulkan device: {verdict} (read {pixel:?}, expected {EXPECTED_RGBA:?})");
            let _: *const c_void = std::ptr::null();
        }
        Ok(())
    }
}
