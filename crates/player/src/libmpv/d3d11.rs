//! What the two Windows GPU renderers (`angle.rs`, `wgl.rs`) share: D3D11 textures the UI's
//! D3D12 device can open, and finding the adapter the UI draws on.
//!
//! D3D12 can't open D3D11's legacy share handles, only NT handles. Each texture gets a named NT
//! handle, and the UI opens it by name (`ID3D12Device::OpenSharedHandleByName`), so no handle has
//! to be duplicated into the app's process: the same frames work from the player host process and
//! in the app. Names are unique per texture and never reused, so a name can only ever open the
//! texture it was made for; once the texture's last handle is closed, opening it fails cleanly.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIAdapter1, IDXGIDevice,
    IDXGIFactory4, IDXGIResource1,
};
use windows::core::{HSTRING, Interface};

use super::opengl::failed;
use super::video::{Luid, SharedFrame};
use crate::Result;

/// Ids for shared textures, unique across players in this process.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A D3D11 texture with a named NT handle, for the UI to open.
pub(crate) struct SharedTexture {
    pub texture: ID3D11Texture2D,
    id: u64,
    name: Arc<str>,
    /// Shared with the frames of this texture, so its name (and the texture) stays alive until
    /// the last of them is gone, even after the ring moved on (a resize): the UI may still be
    /// about to open the frame it holds.
    handle: Arc<OwnedHandle>,
}

impl SharedTexture {
    /// A render target the UI can sample, on `device`, shared through a named NT handle.
    pub fn new(device: &ID3D11Device, width: u32, height: u32, format: DXGI_FORMAT) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            // NT handles (which D3D12 opens) need `SHARED_NTHANDLE`, together with `SHARED` (no
            // keyed mutex: frames are synced by `glFinish` and the ring, see `opengl::RING`).
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
            ..Default::default()
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let name = shared_name(id);
        // SAFETY: plain D3D11/DXGI object creation; the handle is owned from here on.
        unsafe {
            let mut texture = None;
            device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(|err| failed(format!("creating a shared texture: {err}")))?;
            let texture: ID3D11Texture2D = texture.ok_or_else(|| failed("no texture"))?;
            let handle = texture
                .cast::<IDXGIResource1>()
                .and_then(|resource| {
                    resource.CreateSharedHandle(
                        None,
                        (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                        &HSTRING::from(&*name),
                    )
                })
                .map_err(|err| failed(format!("sharing the texture: {err}")))?;
            Ok(Self {
                texture,
                id,
                name: name.into(),
                handle: Arc::new(OwnedHandle::from_raw_handle(handle.0)),
            })
        }
    }

    /// A finished frame in this texture, for the UI.
    pub fn frame(&self, width: u32, height: u32) -> SharedFrame {
        SharedFrame {
            id: self.id,
            name: self.name.clone(),
            width,
            height,
            _handle: Some(self.handle.clone()),
        }
    }
}

/// The NT handle's name for texture `id`: in this session's namespace (`Local\`), with this
/// process's id and start time, so it's unique among the session's processes and never reused,
/// even by a later process with the same id.
fn shared_name(id: u64) -> String {
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    static STARTED: OnceLock<u128> = OnceLock::new();
    let started = STARTED.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    });
    format!("Local\\tabbyfin-video-{}-{started:x}-{id}", std::process::id())
}

/// The DXGI adapter with this LUID: the GPU the UI draws on, where the shared textures have to
/// be made (D3D12 only opens textures from its own adapter).
pub(crate) fn adapter(luid: Luid) -> Result<IDXGIAdapter1> {
    let luid = LUID {
        LowPart: luid.low,
        HighPart: luid.high,
    };
    // SAFETY: plain DXGI calls.
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(failed)?;
        factory
            .EnumAdapterByLuid(luid)
            .map_err(|err| failed(format!("no adapter with the window's LUID: {err}")))
    }
}

/// The LUID of the adapter `device` runs on.
pub(crate) fn adapter_luid(device: &ID3D11Device) -> Result<Luid> {
    // SAFETY: reads the device's adapter description.
    unsafe {
        let desc = device
            .cast::<IDXGIDevice>()
            .and_then(|device| device.GetAdapter())
            .and_then(|adapter| adapter.GetDesc())
            .map_err(failed)?;
        Ok(Luid {
            low: desc.AdapterLuid.LowPart,
            high: desc.AdapterLuid.HighPart,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::shared_name;

    #[test]
    fn names_are_unique_per_texture() {
        let (a, b) = (shared_name(1), shared_name(2));
        assert_ne!(a, b);
        assert!(a.starts_with("Local\\tabbyfin-video-"));
    }
}
