//! GPU frames on Windows: the player's D3D11 texture, opened on wgpu's D3D12 device by the name
//! of its NT handle and wrapped as a `wgpu::Texture`. The player makes it on the window's adapter
//! (`player::libmpv` gets the adapter's LUID), since D3D12 only opens textures from its own.

use egui_wgpu::wgpu;
use player::libmpv::SharedFrame;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL};
use windows::Win32::Graphics::Direct3D12::{D3D12_RESOURCE_DIMENSION_TEXTURE2D, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM};
use windows::core::HSTRING;

use super::ImportError;

/// Opens the texture behind `frame` on `device`, which must run on DX12
/// (`widgets::video::import` only asks for these frames then). The texture holds the D3D12
/// resource; dropping it releases the resource once the GPU is done with it, and the player's
/// texture goes when the last process holding it lets go.
pub fn import(device: &wgpu::Device, frame: &SharedFrame) -> Result<wgpu::Texture, ImportError> {
    // SAFETY: only used to open a resource on this device and hand it to wgpu.
    let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
        .ok_or_else(|| ImportError::Failed("the window doesn't draw with DX12".into()))?;
    let raw = hal_device.raw_device().clone();
    drop(hal_device);

    // A name that no longer opens is a texture the player has let go (an old ring, after a
    // resize); newer frames follow.
    // SAFETY: plain D3D12 calls on a handle we own and close; the resource keeps its own
    // reference to the texture.
    let resource = unsafe {
        let handle = raw
            .OpenSharedHandleByName(&HSTRING::from(&*frame.name), GENERIC_ALL.0)
            .map_err(|err| ImportError::Gone(format!("opening {}: {err}", frame.name)))?;
        let mut resource: Option<ID3D12Resource> = None;
        let opened = raw.OpenSharedHandle(handle, &mut resource);
        let _ = CloseHandle(handle);
        opened.map_err(|err| ImportError::Failed(format!("opening the shared texture: {err}")))?;
        resource.ok_or_else(|| ImportError::Failed("no shared texture".into()))?
    };

    // SAFETY: reads the resource's description.
    let desc = unsafe { resource.GetDesc() };
    if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
        || desc.Width != u64::from(frame.width)
        || desc.Height != frame.height
    {
        return Err(ImportError::Failed(format!(
            "the shared texture is {}×{} ({:?}), the frame {}×{}",
            desc.Width, desc.Height, desc.Dimension, frame.width, frame.height
        )));
    }
    let format = format(desc.Format)
        .ok_or_else(|| ImportError::Failed(format!("the shared texture's format {:?}", desc.Format)))?;
    let size = wgpu::Extent3d {
        width: frame.width,
        height: frame.height,
        depth_or_array_layers: 1,
    };
    // SAFETY: a 2D texture of this format and size, one mip level and sample (checked above).
    let hal_texture =
        unsafe { wgpu::hal::dx12::Device::texture_from_raw(resource, format, wgpu::TextureDimension::D2, size, 1, 1) };
    // As already in the state sampling needs: wgpu then never transitions it. A texture opened
    // from another device starts in `COMMON`, and D3D12 promotes it to a shader resource on each
    // submit that samples it and lets it decay back to `COMMON` when that submit finishes
    // (implicit promotion and decay), which is the state the player's device needs to write it
    // again. Declaring it uninitialized instead would have wgpu move it out of `COMMON` once,
    // explicitly, and leave it there while the player draws into it.
    // SAFETY: the texture was made on this device's adapter with these properties; sampled only.
    Ok(unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("video frame (shared)"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::TextureUses::RESOURCE,
        )
    })
}

/// The player's texture formats (`player::libmpv`: RGBA from ANGLE, BGRA from WGL), as wgpu
/// names them.
fn format(format: DXGI_FORMAT) -> Option<wgpu::TextureFormat> {
    match format {
        DXGI_FORMAT_R8G8B8A8_UNORM => Some(wgpu::TextureFormat::Rgba8Unorm),
        DXGI_FORMAT_B8G8R8A8_UNORM => Some(wgpu::TextureFormat::Bgra8Unorm),
        _ => None,
    }
}
