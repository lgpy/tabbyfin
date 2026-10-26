//! The video: the player's frames, drawn letterboxed in the area the widget is given.
//!
//! The widget owns its textures in egui-wgpu's renderer rather than going through egui's
//! texture manager. Software frames are written with `queue.write_texture` into one texture per
//! frame size, registered once (egui's texture deltas would copy each frame twice more on the
//! way). V2 and V3 add the imported GPU frames.
//!
//! [`import`] is what the app tells the player it can draw when it starts it, so the player
//! only sets up a video output whose frames this widget can show.

use std::sync::Arc;
use std::time::Duration;

use egui::emath::GuiRounding as _;
use egui_wgpu::{RenderState, wgpu};
use player::libmpv::{Frame, VideoFrame, VideoImport, VideoOutput};
use tokio::sync::watch;

use crate::task::Task;

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
    /// Repaints for each new frame. Dropping the widget stops it.
    _wake: Task<()>,
}

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
            _wake: wake,
        }
    }

    /// Whether a frame has arrived to draw.
    pub fn has_frame(&self) -> bool {
        self.texture.is_some()
    }

    /// Fills the space left in `ui` with the latest frame, letterboxed on black (contain). The
    /// player renders frames at this size in physical pixels (never larger than the video), so
    /// they're drawn one to one unless the video is smaller. The response senses clicks, for
    /// the player's controls.
    pub fn show(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
        let pixels_per_point = ui.ctx().pixels_per_point();
        let target = rect.size() * pixels_per_point;
        self.output
            .set_target_size(target.x.round() as u32, target.y.round() as u32);

        if self.frames.has_changed().unwrap_or(false) {
            let frame = self.frames.borrow_and_update().clone();
            match frame {
                Some(Frame::Pixels(frame)) => self.upload(&frame),
                // Only sent when asked for (`import`), which V2 and V3 do once they draw them.
                Some(Frame::Shared(_)) => tracing::warn!("a GPU video frame, which this build can't draw yet"),
                None => {}
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0., egui::Color32::BLACK);
        if let Some(texture) = &self.texture {
            // On whole pixels, so a frame rendered at the window's size isn't blurred by
            // sampling between them.
            let drawn = contain(rect, texture.size).round_to_pixels(pixels_per_point);
            let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
            painter.image(texture.id, drawn, uv, egui::Color32::WHITE);
        }
        response
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
        // The texture itself goes once the GPU is done with it; its registration is ours to end,
        // but not yet: the pass dropping the widget may have drawn it, and is painted after.
        if let Some(texture) = self.texture.take() {
            let renderer = self.render_state.renderer.clone();
            let retired = self.ctx.plugin_or_default::<RetiredTextures>();
            retired.lock().pending.push((renderer, texture.id));
        }
    }
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

/// What the player should send for this window: pixels for now. V2 (Vulkan memory) and V3
/// (D3D11 textures) ask for what [`adapter_import`] finds, once the widget draws their frames.
pub fn import(render_state: Option<&RenderState>) -> VideoImport {
    let possible = render_state.map_or(VideoImport::Pixels, adapter_import);
    tracing::info!(?possible, "video: asking the player for pixels");
    VideoImport::Pixels
}

/// What the renderer's device could import: Vulkan memory where wgpu runs on Vulkan and the
/// device has `VULKAN_EXTERNAL_MEMORY_FD` (`main.rs` asks for it), D3D11 textures on DX12, and
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
        wgpu::Backend::Dx12 => dx12_adapter_luid(adapter).map_or(VideoImport::Pixels, |adapter_luid| {
            VideoImport::D3d11Textures { adapter_luid }
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
