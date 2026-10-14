//! Textures shared by another Vulkan device (e.g. an embedded video player): exported memory
//! (opaque fd) imported into wgpu's Vulkan device and wrapped as a `wgpu::Texture`. The image
//! parameters must match the producer's exactly; see `gpui::SharedTexture`.

use std::os::fd::{BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};

use anyhow::{Context as _, Result};
use ash::vk;
use gpui::SharedTexture;

const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

pub(crate) fn import(device: &wgpu::Device, texture: &SharedTexture) -> Result<wgpu::Texture> {
    // SAFETY: only used to create and wrap an image on this device.
    let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
        .context("the renderer isn't using Vulkan")?;
    let raw = hal_device.raw_device().clone();
    let instance = hal_device.shared_instance().raw_instance();
    let physical = hal_device.raw_physical_device();

    // SAFETY: the producer keeps `fd` open while the texture may be drawn.
    let fd = unsafe { BorrowedFd::borrow_raw(texture.fd) }
        .try_clone_to_owned()
        .context("duplicating the shared texture fd")?
        .into_raw_fd();

    let mut external =
        vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let info = vk::ImageCreateInfo::default()
        .push_next(&mut external)
        .image_type(vk::ImageType::TYPE_2D)
        .format(FORMAT)
        .extent(vk::Extent3D {
            width: texture.width,
            height: texture.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::LINEAR)
        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
        .initial_layout(vk::ImageLayout::PREINITIALIZED);

    // SAFETY: plain Vulkan object creation; everything is destroyed on failure or by the
    // texture's drop callback.
    unsafe {
        let image = match raw.create_image(&info, None) {
            Ok(image) => image,
            Err(err) => {
                close(fd);
                return Err(err).context("creating the shared image");
            }
        };
        let requirements = raw.get_image_memory_requirements(image);
        let memory_properties = instance.get_physical_device_memory_properties(physical);
        let memory_type = (0..memory_properties.memory_type_count)
            .filter(|i| requirements.memory_type_bits & (1 << i) != 0)
            .find(|i| {
                memory_properties.memory_types[*i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| (0..memory_properties.memory_type_count).find(|i| requirements.memory_type_bits & (1 << i) != 0));
        let Some(memory_type) = memory_type else {
            raw.destroy_image(image, None);
            close(fd);
            anyhow::bail!("no memory type for the shared image");
        };
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
            .fd(fd);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocate = vk::MemoryAllocateInfo::default()
            .push_next(&mut import)
            .push_next(&mut dedicated)
            .allocation_size(texture.allocation_size)
            .memory_type_index(memory_type);
        // A successful import takes ownership of the fd.
        let memory = match raw.allocate_memory(&allocate, None) {
            Ok(memory) => memory,
            Err(err) => {
                raw.destroy_image(image, None);
                close(fd);
                return Err(err).context("importing the shared memory");
            }
        };
        if let Err(err) = raw.bind_image_memory(image, memory, 0) {
            raw.destroy_image(image, None);
            raw.free_memory(memory, None);
            return Err(err).context("binding the shared memory");
        }

        let drop_device = raw.clone();
        let drop_callback: wgpu::hal::DropCallback = Box::new(move || {
            drop_device.destroy_image(image, None);
            drop_device.free_memory(memory, None);
        });
        let size = wgpu::Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        };
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some("shared_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let hal_texture = hal_device.texture_from_raw(
            image,
            &hal_desc,
            Some(drop_callback),
            wgpu::hal::vulkan::TextureMemory::External,
        );
        drop(hal_device);
        Ok(device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("shared_texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        ))
    }
}

/// Closes an fd whose ownership wasn't passed to Vulkan.
fn close(fd: i32) {
    // SAFETY: `fd` is a duplicate this module owns.
    drop(unsafe { OwnedFd::from_raw_fd(fd) });
}
