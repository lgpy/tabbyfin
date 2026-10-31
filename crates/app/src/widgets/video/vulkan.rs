//! GPU frames on Linux: the player's exported Vulkan memory (an opaque fd), imported into wgpu's
//! Vulkan device and wrapped as a `wgpu::Texture`. The image must be described exactly as the
//! player made it (`player::libmpv::egl`), or the import is undefined.

use std::os::fd::{AsFd as _, FromRawFd as _, IntoRawFd as _, OwnedFd};

use ash::vk;
use egui_wgpu::wgpu;
use player::libmpv::SharedFrame;

/// The player's image format, as Vulkan and as wgpu name it.
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const WGPU_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Imports the memory behind `frame` into `device`, which must run on Vulkan with
/// `VULKAN_EXTERNAL_MEMORY_FD` (`widgets::video::import` only asks for these frames then). The
/// texture owns its image and memory; dropping it frees both once the GPU is done with it.
pub fn import(device: &wgpu::Device, frame: &SharedFrame) -> Result<wgpu::Texture, String> {
    // SAFETY: only used to create an image on this device and hand it to wgpu.
    let hal_device =
        unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("the window doesn't draw with Vulkan")?;
    let raw = hal_device.raw_device().clone();
    let instance = hal_device.shared_instance().raw_instance();
    let physical = hal_device.raw_physical_device();

    // Vulkan takes ownership of the fd it imports: give it a duplicate, since the frame's stays
    // with the frame (other frames of the same image share it).
    let fd: OwnedFd = frame
        .fd
        .as_fd()
        .try_clone_to_owned()
        .map_err(|err| format!("duplicating the frame's fd: {err}"))?;

    let mut external =
        vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let info = vk::ImageCreateInfo::default()
        .push_next(&mut external)
        .image_type(vk::ImageType::TYPE_2D)
        .format(FORMAT)
        .extent(vk::Extent3D {
            width: frame.width,
            height: frame.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::LINEAR)
        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
        .initial_layout(vk::ImageLayout::PREINITIALIZED);

    // SAFETY: plain Vulkan object creation on wgpu's device; everything is destroyed on failure
    // here, or by the texture's drop callback.
    unsafe {
        let image = raw
            .create_image(&info, None)
            .map_err(|err| format!("creating the image: {err}"))?;
        // The same choice as the player's (DEVICE_LOCAL first): an opaque fd is imported with
        // the memory type it was allocated with.
        let requirements = raw.get_image_memory_requirements(image);
        let properties = instance.get_physical_device_memory_properties(physical);
        let candidates = (0..properties.memory_type_count).filter(|i| requirements.memory_type_bits & (1 << i) != 0);
        let memory_type = candidates
            .clone()
            .find(|i| {
                properties.memory_types[*i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| candidates.clone().next());
        let Some(memory_type) = memory_type else {
            raw.destroy_image(image, None);
            return Err("no memory type for the image".into());
        };
        let raw_fd = fd.into_raw_fd();
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
            .fd(raw_fd);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocate = vk::MemoryAllocateInfo::default()
            .push_next(&mut import)
            .push_next(&mut dedicated)
            .allocation_size(frame.allocation_size)
            .memory_type_index(memory_type);
        // A successful import owns the fd; a failed one leaves it with us.
        let memory = match raw.allocate_memory(&allocate, None) {
            Ok(memory) => memory,
            Err(err) => {
                raw.destroy_image(image, None);
                drop(OwnedFd::from_raw_fd(raw_fd));
                return Err(format!("importing the memory: {err}"));
            }
        };
        if let Err(err) = raw.bind_image_memory(image, memory, 0) {
            raw.destroy_image(image, None);
            raw.free_memory(memory, None);
            return Err(format!("binding the memory: {err}"));
        }

        // wgpu runs this once the texture is dropped and the GPU no longer uses it. The memory
        // is ours (`TextureMemory::External`), so wgpu leaves both to the callback.
        let drop_device = raw.clone();
        let drop_callback: wgpu::hal::DropCallback = Box::new(move || {
            drop_device.destroy_image(image, None);
            drop_device.free_memory(memory, None);
        });
        let size = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        };
        let hal_texture = hal_device.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some("video frame (imported)"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WGPU_FORMAT,
                usage: wgpu::TextureUses::RESOURCE,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            },
            Some(drop_callback),
            wgpu::hal::vulkan::TextureMemory::External,
        );
        drop(hal_device);
        // Uninitialized: wgpu's first barrier moves the image out of `UNDEFINED`, once per image,
        // as renderers usually do. That may discard contents in principle, which would lose the
        // first showing of each image only (there's no barrier after it); linear images have no
        // layout-dependent state to discard on Mesa. Claiming another layout would be wrong for
        // an image Vulkan never transitioned.
        Ok(device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("video frame (imported)"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WGPU_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::TextureUses::UNINITIALIZED,
        ))
    }
}
