//! The video: the player's frames, drawn letterboxed in the area the widget is given.
//!
//! The widget owns its textures in egui-wgpu's renderer rather than going through egui's
//! texture manager. Software frames are written with `queue.write_texture` into one texture per
//! frame size, registered once (egui's texture deltas would copy each frame twice more on the
//! way). GPU frames are the player's own textures, imported once per image of its ring
//! (`vulkan.rs` on Linux, `dx12.rs` on Windows) and drawn as they are.
//!
//! [`import`] is what the app tells the player it can draw when it starts it, so the player
//! only sets up a video output whose frames this widget can show.

#[cfg(windows)]
mod dx12;
#[cfg(target_os = "linux")]
mod vulkan;

use std::sync::Arc;
use std::time::Duration;

use egui::emath::GuiRounding as _;
use egui_wgpu::{RenderState, wgpu};
use player::libmpv::{Frame, VideoFrame, VideoImport, VideoOutput};
use tokio::sync::watch;

use crate::task::Task;
use crate::theme::over_video;

/// How soon after a new frame arrives the UI draws it.
const REPAINT_AFTER: Duration = Duration::from_millis(1);

/// The video of one playback session. Make a new one for a new session's [`VideoOutput`].
pub struct Video {
    ctx: egui::Context,
    output: VideoOutput,
    frames: watch::Receiver<Option<Frame>>,
    render_state: RenderState,
    /// The texture software frames are written into, while frames keep their size.
    texture: Option<FrameTexture>,
    /// GPU frames' textures, one per image of the player's ring, oldest first.
    imported: Vec<Imported>,
    /// What's drawn: the egui texture and the frame's size in pixels.
    shown: Option<(egui::TextureId, [u32; 2])>,
    /// A GPU frame couldn't be imported; see [`Video::import_failed`].
    import_failed: bool,
    /// GPU frames in a row whose image was already gone ([`ImportError::Gone`]).
    gone: u32,
    /// Repaints for each new frame. Dropping the widget stops it.
    _wake: Task<()>,
}

/// An imported image of the player's ring, registered with egui-wgpu.
struct Imported {
    /// The player's id for the image (`SharedFrame::id`).
    image: u64,
    /// Kept so the texture lives while egui may draw it; wgpu frees the image and its memory
    /// after it's dropped and the GPU is done with it.
    _texture: wgpu::Texture,
    id: egui::TextureId,
    size: [u32; 2],
}

/// The player renders into a ring of three images (`player::libmpv::opengl`), so that many
/// imports are kept: each image is imported once and drawn whenever it comes round again.
const RING: usize = 3;

/// Why a GPU frame wasn't imported.
#[derive(Debug)]
enum ImportError {
    /// The frame's image is gone: the player let go of it (an old ring, after a resize) before
    /// the UI got to it. Only Windows, where frames name their textures rather than hold them.
    Gone(String),
    /// Importing doesn't work here (a driver quirk, another GPU): software frames from now on.
    Failed(String),
}

/// Frames in a row whose image was gone, before the import counts as failed: far more than a
/// resize can cause, since the player's current ring stays alive.
const MAX_GONE: u32 = 10;

/// A texture for software frames of one size, registered with egui-wgpu.
struct FrameTexture {
    texture: wgpu::Texture,
    id: egui::TextureId,
    size: [u32; 2],
}

impl Video {
    pub fn new(ctx: &egui::Context, output: VideoOutput, render_state: &RenderState) -> Self {
        // Frames come at the video's rate, from tokio; each asks for one UI frame. Nothing
        // repaints while paused, since mpv renders nothing new then.
        let mut changes = output.frames();
        let repaint = ctx.clone();
        let wake = Task::spawn(ctx, async move {
            while changes.changed().await.is_ok() {
                // Not `request_repaint`: egui answers that with two passes ("to let things
                // settle"), which would draw the window twice per video frame. A delay, however
                // short, gets one.
                repaint.request_repaint_after(REPAINT_AFTER);
            }
        });
        Self {
            ctx: ctx.clone(),
            frames: output.frames(),
            output,
            render_state: render_state.clone(),
            texture: None,
            imported: Vec::new(),
            shown: None,
            import_failed: false,
            gone: 0,
            _wake: wake,
        }
    }

    /// Whether a frame has arrived to draw.
    pub fn has_frame(&self) -> bool {
        self.shown.is_some()
    }

    /// Whether a GPU frame failed to import, so nothing new will show: the session has to be
    /// played again with software frames (`Services::fall_back_to_pixels`). Driver quirks with
    /// shared memory can do that where the setup itself worked.
    pub fn import_failed(&self) -> bool {
        self.import_failed
    }

    /// Fills the space left in `ui` with the latest frame, letterboxed on black (contain). The
    /// player renders frames at this size in physical pixels (never larger than the video), so
    /// they're drawn one to one unless the video is smaller. The response senses clicks for the
    /// player's controls, and isn't focusable: a Tab that gave egui the focus would take the
    /// player's keys from it (U8's rule for the dialogs).
    pub fn show(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::CLICK);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let target = rect.size() * pixels_per_point;
        self.output
            .set_target_size(target.x.round() as u32, target.y.round() as u32);

        if self.frames.has_changed().unwrap_or(false) {
            let frame = self.frames.borrow_and_update().clone();
            match frame {
                Some(Frame::Pixels(frame)) => self.upload(&frame),
                Some(Frame::Shared(frame)) => self.show_shared(&frame),
                None => {}
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0., over_video::BACKDROP);
        if let Some((id, size)) = self.shown {
            // On whole pixels, so a frame rendered at the window's size isn't blurred by
            // sampling between them.
            let drawn = contain(rect, size).round_to_pixels(pixels_per_point);
            let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
            painter.image(id, drawn, uv, egui::Color32::WHITE);
        }
        response
    }

    /// Shows a GPU frame: its image's texture, imported the first time the image comes. A new
    /// size means a new ring, and the old ring's textures are let go.
    #[cfg(any(target_os = "linux", windows))]
    fn show_shared(&mut self, frame: &player::libmpv::SharedFrame) {
        let size = [frame.width, frame.height];
        if let Some(imported) = self.imported.iter().find(|i| i.image == frame.id) {
            self.shown = Some((imported.id, imported.size));
            self.gone = 0;
            return;
        }
        if self.import_failed {
            return;
        }
        let RenderState { device, renderer, .. } = &self.render_state;
        #[cfg(target_os = "linux")]
        let texture = vulkan::import(device, frame).map_err(ImportError::Failed);
        #[cfg(windows)]
        let texture = dx12::import(device, frame);
        let texture = match texture {
            Ok(texture) => texture,
            // The last frame stays up; the next one is from the player's current ring. Only if
            // that keeps happening does something else go wrong.
            Err(ImportError::Gone(err)) if self.gone < MAX_GONE => {
                tracing::debug!(?size, "video: skipped a GPU frame: {err}");
                self.gone += 1;
                return;
            }
            Err(ImportError::Gone(err) | ImportError::Failed(err)) => {
                tracing::warn!(?size, "video: couldn't import a GPU frame: {err}");
                self.import_failed = true;
                return;
            }
        };
        self.gone = 0;
        if self.imported.first().is_none_or(|i| i.size != size) {
            tracing::info!(width = frame.width, height = frame.height, "video: GPU frames");
        }
        // The images that won't come again: another ring's (a resize), and beyond a ring's
        // worth, the oldest.
        let mut stale: Vec<Imported> = self.imported.extract_if(.., |i| i.size != size).collect();
        let over = (self.imported.len() + 1).saturating_sub(RING);
        stale.extend(self.imported.drain(..over));
        for imported in stale {
            retire(&self.ctx, &self.render_state, imported.id);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Linear: frames are scaled up when the video is smaller than the window.
        let id = renderer
            .write()
            .register_native_texture(device, &view, wgpu::FilterMode::Linear);
        self.imported.push(Imported {
            image: frame.id,
            _texture: texture,
            id,
            size,
        });
        self.shown = Some((id, size));
        // Software frames don't come back within a session; their texture can go too.
        if let Some(texture) = self.texture.take() {
            retire(&self.ctx, &self.render_state, texture.id);
        }
    }

    /// GPU frames are only sent where the app asks for them ([`import`]), which it doesn't on
    /// this OS.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn show_shared(&mut self, _frame: &player::libmpv::SharedFrame) {
        tracing::warn!("a GPU video frame, which this build can't draw yet");
    }

    /// Writes a software frame into the texture, making a new texture when the size changed
    /// (under the same egui texture id).
    fn upload(&mut self, frame: &VideoFrame) {
        let size = [frame.width, frame.height];
        if frame.pixels.len() != frame.width as usize * frame.height as usize * 4 {
            tracing::warn!(?size, bytes = frame.pixels.len(), "a video frame of the wrong length");
            return;
        }
        let RenderState {
            device,
            queue,
            renderer,
            ..
        } = &self.render_state;
        let extent = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        };
        if self.texture.as_ref().map(|t| t.size) != Some(size) {
            tracing::info!(width = frame.width, height = frame.height, "video: software frames");
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video frame"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // The player's software output (RGBA, opaque), and what egui-wgpu's shader
                // takes from a native texture.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            // Linear: frames are scaled up when the video is smaller than the window.
            let filter = wgpu::FilterMode::Linear;
            let mut renderer = renderer.write();
            let id = match self.texture.take() {
                // The old texture is freed once the GPU is done with it.
                Some(old) => {
                    renderer.update_egui_texture_from_wgpu_texture(device, &view, filter, old.id);
                    old.id
                }
                None => renderer.register_native_texture(device, &view, filter),
            };
            self.texture = Some(FrameTexture { texture, id, size });
        }
        let Some(texture) = &self.texture else { return };
        self.shown = Some((texture.id, size));
        // Staged now and copied before the next submit, which draws this frame.
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            extent,
        );
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        // The textures themselves go once the GPU is done with them; their registrations are
        // ours to end, but not yet: the pass dropping the widget may have drawn one, and is
        // painted after.
        let ids = self.texture.take().map(|t| t.id).into_iter();
        for id in ids.chain(self.imported.drain(..).map(|i| i.id)) {
            retire(&self.ctx, &self.render_state, id);
        }
    }
}

/// Ends a texture's registration with egui-wgpu at the start of the next pass ([`RetiredTextures`]).
fn retire(ctx: &egui::Context, render_state: &RenderState, id: egui::TextureId) {
    let retired = ctx.plugin_or_default::<RetiredTextures>();
    retired.lock().pending.push((render_state.renderer.clone(), id));
}

/// Video textures to unregister from egui-wgpu at the start of the next pass, once the pass
/// that last drew them has been painted. Unregistering at once left that paint without the
/// texture ("Missing texture" in egui-wgpu's log, and a frame without the video).
#[derive(Default)]
struct RetiredTextures {
    pending: Vec<(Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>, egui::TextureId)>,
}

impl egui::Plugin for RetiredTextures {
    fn debug_name(&self) -> &'static str {
        "retired video textures"
    }

    fn on_begin_pass(&mut self, _ui: &mut egui::Ui) {
        for (renderer, id) in self.pending.drain(..) {
            renderer.write().free_texture(&id);
        }
    }
}

/// The largest rect with the frame's shape that fits in `rect`, centred. mpv renders square
/// pixels, so the frame's pixel size gives its shape.
fn contain(rect: egui::Rect, [width, height]: [u32; 2]) -> egui::Rect {
    if width == 0 || height == 0 {
        return rect;
    }
    let frame = egui::vec2(width as f32, height as f32);
    let scale = (rect.width() / frame.x).min(rect.height() / frame.y);
    egui::Rect::from_center_size(rect.center(), frame * scale)
}

/// What the player should send for this window: what [`adapter_import`] finds the window's
/// device can import. That's Vulkan memory where wgpu runs on Vulkan and can import it (Linux),
/// D3D11 textures where it runs on DX12 (Windows), and pixels otherwise: on wgpu's GL fallback,
/// or `WGPU_BACKEND` picking another API, that's pixels by itself.
///
/// The player still decides whether its GPU path is worth it: it renders pixels where its
/// OpenGL is a software rasterizer (llvmpipe), unless `TABBYFIN_VIDEO=gpu`.
pub fn import(render_state: Option<&RenderState>) -> VideoImport {
    let import = render_state.map_or(VideoImport::Pixels, adapter_import);
    tracing::info!(?import, "video: what the player is asked for");
    import
}

/// What the renderer's device could import: Vulkan memory where wgpu runs on Vulkan and the
/// device has `VULKAN_EXTERNAL_MEMORY_FD` (`main.rs` asks for it), D3D11 textures on DX12 (not
/// on WARP, the CPU adapter, where the player's GPU path would cost more than pixels), and
/// pixels otherwise (wgpu on GL; Metal for now).
pub fn adapter_import(render_state: &RenderState) -> VideoImport {
    let adapter = &render_state.adapter;
    match adapter.get_info().backend {
        #[cfg(target_os = "linux")]
        wgpu::Backend::Vulkan
            if render_state
                .device
                .features()
                .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_FD) =>
        {
            vulkan_device_uuid(adapter).map_or(VideoImport::Pixels, |device_uuid| VideoImport::VulkanMemory {
                device_uuid,
            })
        }
        #[cfg(windows)]
        wgpu::Backend::Dx12 if adapter.get_info().device_type != wgpu::DeviceType::Cpu => dx12_adapter_luid(adapter)
            .map_or(VideoImport::Pixels, |adapter_luid| VideoImport::D3d11Textures {
                adapter_luid,
            }),
        _ => VideoImport::Pixels,
    }
}

/// The Vulkan device UUID of wgpu's adapter, which the player's exporting device must match.
#[cfg(target_os = "linux")]
fn vulkan_device_uuid(adapter: &wgpu::Adapter) -> Option<[u8; 16]> {
    use ash::{khr, vk};

    // SAFETY: only reads the physical device's properties; nothing is created or destroyed.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let shared = hal.shared_instance();
    let mut ids = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut ids);
    // Core in Vulkan 1.1; an older instance needs the extension.
    if shared.instance_api_version() >= vk::API_VERSION_1_1 {
        // SAFETY: a physical device of this instance, and a valid output chain.
        unsafe {
            shared
                .raw_instance()
                .get_physical_device_properties2(hal.raw_physical_device(), &mut properties)
        };
    } else if shared
        .extensions()
        .contains(&khr::get_physical_device_properties2::NAME)
    {
        let loader = khr::get_physical_device_properties2::Instance::new(shared.entry(), shared.raw_instance());
        // SAFETY: as above, with the extension enabled on the instance.
        unsafe { loader.get_physical_device_properties2(hal.raw_physical_device(), &mut properties) };
    } else {
        return None;
    }
    Some(ids.device_uuid)
}

/// The LUID of wgpu's DX12 adapter, which the player's D3D11 device must be made on.
#[cfg(windows)]
fn dx12_adapter_luid(adapter: &wgpu::Adapter) -> Option<player::libmpv::Luid> {
    // SAFETY: only reads the adapter's description.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }?;
    // SAFETY: a valid DXGI adapter.
    let desc = unsafe { hal.raw_adapter().GetDesc1() }.ok()?;
    Some(player::libmpv::Luid {
        low: desc.AdapterLuid.LowPart,
        high: desc.AdapterLuid.HighPart,
    })
}

#[cfg(test)]
mod tests {
    use egui::{Rect, pos2, vec2};

    use super::contain;

    #[test]
    fn a_wide_frame_gets_bars_above_and_below() {
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(800., 800.));
        let drawn = contain(rect, [1600, 900]);
        assert_eq!(drawn, Rect::from_min_size(pos2(0., 175.), vec2(800., 450.)));
    }

    #[test]
    fn a_tall_frame_gets_bars_left_and_right() {
        let rect = Rect::from_min_size(pos2(10., 20.), vec2(1000., 500.));
        let drawn = contain(rect, [400, 500]);
        assert_eq!(drawn, Rect::from_min_size(pos2(310., 20.), vec2(400., 500.)));
    }

    #[test]
    fn a_smaller_frame_is_scaled_up_to_fit() {
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(1920., 1080.));
        assert_eq!(contain(rect, [1280, 720]), rect);
    }

    #[test]
    fn an_empty_frame_takes_the_whole_rect() {
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(300., 200.));
        assert_eq!(contain(rect, [0, 0]), rect);
    }
}
