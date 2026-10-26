# In-app video: plan

**Goal:** play video inside the Tabbyfin window, with GPUI controls on top. This replaces the separate mpv window.

**Decision (2026-10-14):** use **GPU texture sharing** (plan §5.4 option D). mpv renders on the GPU and GPUI draws the result directly, so there's no CPU copy. That keeps mpv's full quality: `gpu-next`, HDR and 4K.
- **Order:** Windows first, then Linux.
- **macOS:** stays on the software-render fallback for now. We have no Mac to test with.

## Constraints

- **libmpv render API:** it only offers `opengl` and `sw`; there's no Vulkan, D3D or Metal API (checked in mpv v0.41.0 `render.h`). So mpv always draws through OpenGL, and every platform needs OpenGL-to-native-API texture sharing.
- **GPUI 0.3.9 support for external surfaces:**

| OS | Renderer | Today | Needed |
|---|---|---|---|
| macOS | Metal | `surface(CVPixelBuffer)` works | Nothing in GPUI. mpv's OpenGL draws into an IOSurface-backed `CVPixelBuffer`. |
| Windows | D3D11 (`gpui-pre-windows`) | `draw_surfaces` is an empty stub | Patch: draw a shared D3D11 texture |
| Linux | wgpu/Vulkan (`gpui-pre-wgpu`) | Surfaces are skipped | Patch: draw a `wgpu::Texture` imported from external Vulkan memory |

## Architecture

```
player::libmpv  (dlopen libmpv; core, events, OpenGL render context on its own thread)
      │ renders each frame into one of N shared textures (ring), at the element's pixel size
      ▼
platform interop
  Windows: WGL context + WGL_NV_DX_interop2 ↔ D3D11 texture (shared NT handle + keyed mutex)
  Linux:   EGL context + GL_EXT_memory_object_fd ↔ Vulkan image (exported fd), imported by wgpu
  macOS:   CGL context + IOSurface ↔ CVPixelBuffer (no GPUI patch)
      │ "frame N ready" → GPUI repaints
      ▼
GPUI VideoSurface element (PaintSurface with a per-platform source) + GPUI controls overlay
```

**Fallback:** GPU sharing where it works, otherwise software rendering. The choice is automatic and logged. There's no separate mpv window anymore; it was removed on 2026-10-14. Without libmpv, playback shows an install hint.

**GPUI fork:**
- The crates we change (`gpui-pre`, `gpui-pre-windows`, later `gpui-pre-wgpu`) are vendored under `third_party/gpui/` and applied with `[patch.crates-io]`.
- Each change is kept small and listed in `third_party/gpui/PATCHES.md`, so it can be re-applied on a gpui-kit upgrade and offered upstream.

## Phases

The three GPU renderers (ANGLE, WGL, Linux EGL) share `player::libmpv::opengl`: mpv's `opengl` render context, the ring of 3 shared textures and the render loop. Each is a `Backend` that only sets up its GL context, makes the shared textures and syncs a frame for GPUI's device.

1. ✅ **libmpv backend and in-app player view, on the software path.** Done on 2026-10-14. Verified on Linux (WSL) and on Windows (RX 6600) with `dev/win-run.sh`. The Windows zip now bundles `libmpv-2.dll`, and the `.deb` depends on `libmpv2`.
   - `player::libmpv` (core and event loop) produces `PlayerEvent`s that the session turns into progress reports.
   - A full-window player page: seek bar, play/pause, ±10 s, volume and mute, version, audio and subtitle pickers, next episode (also automatic at the end of an episode), fullscreen, keyboard shortcuts.
   - Rendering uses the software path at first, so in-app playback works on every OS straight away.
2. ✅ **Windows GPU sharing.** Done on 2026-10-14 and verified on the RX 6600.
   - **Probe:** `examples/dx_interop_probe.rs` confirmed `WGL_NV_DX_interop2` with D3D11 shared textures (and D3D9Ex as a fallback) on AMD driver 26.8.1.
   - **GPUI patches** (`third_party/gpui/PATCHES.md`): `SurfaceSource::SharedTexture`, and the DirectX renderer drawing surfaces with the polychrome sprite shaders.
   - **Player** (`player::libmpv::wgl`):
     - A render thread with a hidden WGL context.
     - A ring of 3 shared D3D11 BGRA textures, each registered with GL through the interop and attached to a framebuffer.
     - mpv renders with the `opengl` API into the next texture, then `glFinish` runs and the share handle is published. No Y flip is needed.
     - The player page draws the frame with `surface(SharedTexture)`.
   - **Fallback:** if GPU setup fails, the software path is used automatically. `TABBYFIN_VIDEO=software` forces it.
   - **Measured** (debug build, 1080p H.264, RX 6600):

     | Path | CPU |
     |---|---|
     | GPU | ~5% of one core |
     | Software | ~54% |

   - **Follow-ups:**
     - ~~Decoding still copies frames back to the CPU.~~ Done with ANGLE, see 2b.
     - Pacing between the 3 textures relies on `glFinish` plus the ring, with no keyed mutex.
     - Test on NVIDIA and Intel.

   **2b. ✅ Zero-copy hardware decoding on Windows (ANGLE).** Done on 2026-10-14 and verified on the RX 6600.
   - **Why ANGLE:** with libmpv's `opengl` render API on a WGL context, mpv has no interop for `d3d11va` frames. Its `dxva2` interop needs mpv's own `dxinterop` context, so with WGL every hardware-decoded frame is copied back to the CPU (`d3d11va-copy`). On an OpenGL ES context from ANGLE's D3D11 backend, mpv's `d3d11egl` interop maps the decoder's D3D11 textures directly, for NV12 and P010 (10-bit).
   - **Player** (`player::libmpv::angle`):
     - Loads `libEGL.dll` from next to the exe. mpv's ANGLE loader then finds the same module.
     - An ES 3 context on a D3D11 ANGLE display.
     - A ring of 3 shared RGBA8 textures on ANGLE's own D3D11 device, each wrapped as a pbuffer (`EGL_ANGLE_d3d_texture_client_buffer`), bound as a GL texture and attached to a framebuffer. No Y flip is needed.
     - GPUI opens them by share handle, as with WGL.
   - **Order on Windows:** ANGLE, then WGL interop, then software. Any failure, such as ANGLE missing or running on WARP, falls through to the next.
   - **hwdec:** the GPU paths set `hwdec=auto-safe`. That gives `d3d11va` with ANGLE, and `d3d11va-copy` (or `nvdec` on NVIDIA) with WGL.
   - **Measured** (debug build, 4K HEVC 10-bit, RX 6600):

     | Path | Decoder | CPU |
     |---|---|---|
     | ANGLE | `d3d11va` | ~11% of one core |
     | WGL | `d3d11va-copy` | ~22% |

   - **Packaging:** ANGLE is MSYS2's ucrt64 build, about 12 MB with its runtime DLLs (`packaging/angle.lock`).
   - **Linux:** could get the same with VA-API, by passing mpv a DRM render node so its `vaapi` interop maps frames into our EGL context. Not done yet; Linux still uses `vaapi-copy` and friends through `auto-safe`.
3. ✅ **Linux GPU sharing.** Done on 2026-10-14. Verified on Mesa llvmpipe/lavapipe, i.e. without a real GPU.
   - **Probe:** `examples/vk_interop_probe.rs` checks that a Vulkan device can export linear RGBA8 image memory as an opaque fd, OpenGL can import and render into it (`GL_EXT_memory_object_fd`), and a second Vulkan device can import and read it back.
   - **Player** (`player::libmpv::egl`):
     - Windowless EGL/OpenGL, plus a small exporter Vulkan device matched to OpenGL by device UUID.
     - A ring of 3 images, each exported twice: one fd stays open for GPUI, the other is imported into GL.
     - mpv renders with `FLIP_Y=0`, then `glFinish` runs.
   - **GPUI patch:** `gpui-pre-wgpu` imports the fd (see `third_party/gpui/PATCHES.md`).
   - **Software OpenGL:** the GPU path is skipped on CPU-emulated OpenGL (llvmpipe, softpipe, SwiftShader, Windows' "GDI Generic"). There it costs far more than the software renderer: about 11 cores against 2.8 on this WSL box for 1080p. `TABBYFIN_VIDEO=gpu` forces it anyway; `=software` forces software. Forced onto a software rasterizer, mpv's scalers are set to bilinear: mpv 0.37's defaults draw black frames on llvmpipe whenever the video is scaled (always with `hwdec=no`, otherwise depending on start-up timing). The memory sharing itself was fine; the frames were black in GL already.
   - **Not tested yet:** real Mesa GPUs (AMD/Intel) and NVIDIA's proprietary driver. The extensions are standard there, but driver quirks with linear tiling are possible.
4. **Packaging libmpv.**
   - Windows: `libmpv-2.dll`.
   - macOS: `libmpv.dylib`.
   - Linux: a `.deb` dependency on `libmpv2`.
   - Done for Windows and Linux. macOS uses Homebrew's libmpv for now.
5. **macOS GPU sharing** (IOSurface), when a Mac is available for testing.

## On egui (wgpu)

The egui app draws with wgpu (DX12 on Windows, Vulkan or GL on Linux), so the GPUI patches above go away; `docs/egui-migration.md` §3 *Images and video* describes the app side.

- **Handshake (V1):** the app tells the player what its renderer can draw when it starts it, as `player::libmpv::VideoImport`: `Pixels`, `VulkanMemory { device_uuid }` or `D3d11Textures { adapter_luid }`, read from wgpu's adapter through its hal (`widgets/video.rs`: `adapter_import`). `Player::spawn(import)` passes it to the player host as the first wire message (`Request::Start`), and the render thread only tries the GPU paths that match it: ANGLE and WGL for `D3d11Textures`, EGL/Vulkan for `VulkanMemory`, software otherwise. A GL fallback or a GPU path the app can't import yet therefore gets software frames instead of frames it can't show. `widgets::video::import` is what the app asks for: `VulkanMemory` on Linux since V2, `Pixels` elsewhere until V3; the log says both (`video: what the player is asked for import=… possible=…`).
- **Linux GPU frames (V2):**
  - **When:** only when wgpu runs on Vulkan and its device was made with `VULKAN_EXTERNAL_MEMORY_FD` (`main.rs` asks for it where the adapter has it). On wgpu's GL fallback (`WGPU_BACKEND=gl`, or no Vulkan) `adapter_import` finds nothing to import and the app asks for pixels.
  - **Same GPU:** opaque fds only import on the GPU that made them, so the app sends its Vulkan `deviceUUID` and the player puts everything there (`egl.rs`): the exporting Vulkan device is the one with that UUID, and mpv's OpenGL context is made on the EGL display whose `GL_DEVICE_UUID_EXT` matches, the default display first and then each EGL device (`EGL_EXT_device_enumeration` + `EGL_EXT_platform_device`), which covers laptops whose default GL is the other GPU. Nothing matching means software frames. The context asks for no surface type, since a device's display has no window configs.
  - **Software rasterizer rule kept:** mpv's OpenGL on llvmpipe/softpipe still means software frames unless `TABBYFIN_VIDEO=gpu` (the app itself on lavapipe asks for Vulkan memory regardless; the player decides).
  - **Import** (`widgets/video/vulkan.rs`, ported from GPUI's `gpui-pre-wgpu/src/shared_texture.rs`): the frame's fd is duplicated and imported into a `VkImage` described exactly as the player made it (linear RGBA8, dedicated allocation, the same memory type rule), wrapped with wgpu-hal's `texture_from_raw(…, drop_callback, TextureMemory::External)` and `create_texture_from_hal(…, TextureUses::UNINITIALIZED)`. The drop callback destroys the image and frees the memory once wgpu is done with the texture. wgpu's first barrier takes each image out of `UNDEFINED` once, as GPUI's renderer did; linear images keep their contents through it on Mesa, which is one of the things to watch on real drivers.
  - **Cache:** one texture and egui texture id per ring image (`SharedFrame::id`), so each of the three images is imported once and then only switched between. A frame of a new size means a new ring: the old images' registrations end at the start of the next pass (the same deferred free as dropping the widget), and their textures go with them.
  - **fd lifetime:** `SharedFrame::fd` is an `Arc<OwnedFd>` shared by the frames of one image (embedded: the render thread's slot; host: the app's fd table), so the fd a frame names stays open, and its memory alive, until the UI has imported it, even after a resize has replaced the ring. GPUI's raw fd numbers could be closed and reused in between.
  - **Fallback:** if an import fails anyway (a driver quirk), the widget says so (`import_failed`) and `Services::fall_back_to_pixels` restarts the session at its position with software frames, and keeps to pixels for the rest of the sign-in.
  - **Sync, as on `main`:** `glFinish` after mpv draws, before the frame is published, so the UI never samples a frame GL is still writing; and the ring of three, so mpv writes an image again only two frames after it was sent, long after the UI (which always draws the newest frame) has sampled it. No fences or semaphores yet. wgpu-hal can add semaphores to the next submit (`vulkan::Queue::add_wait_semaphore` / `add_signal_semaphore`) and GL can share them (`GL_EXT_semaphore_fd`), but GL only has binary semaphores, which need exactly one wait per signal, while the UI skips frames (it draws the newest); and the other direction would need a message back to the player per frame, with mpv waiting on the UI. `glFinish` costs the render thread the GPU's time per frame, which it would wait for anyway. If real drivers show tearing or stale frames (the import has no acquire barrier from the external queue family either), that's the next step (2.3.7).
  - **Measured on WSL** (debug, 1080p H.264 in a 1280×800 window, `TABBYFIN_VIDEO=gpu`, everything on the CPU): app ~135%, host ~200%, against ~143% and ~63% for software frames. On a software rasterizer the GPU path costs more, hence the rule; on a real GPU both sides should drop to a few percent, which is still to be measured.
- **Software frames:** mpv renders `rgb0`, the alpha byte is forced to 255, and the pixels go tightly packed as RGBA, which is what a `Rgba8Unorm` texture takes as is (GPUI wanted BGRA). The video widget writes each frame with `queue.write_texture` into one texture per frame size, registered once with egui-wgpu (`register_native_texture`; a new size swaps the texture under the same id), and wakes the UI once per frame. Measured with debug builds, 1080p H.264 in a 1280×800 window (rendered at 1280×720), 25 s into playback (the first seconds also pay for mpv reading the file ahead):

  | | App | Player host | Embedded (`TABBYFIN_PLAYER=embedded`) | GPUI's software path |
  |---|---|---|---|---|
  | WSL (llvmpipe) | ~148% | ~65% | ~191% | ~280% |
  | Windows (RX 6600, `TABBYFIN_VIDEO=software`) | ~7% | ~29% | ~45% | ~54% |

  Paused: 0%, since mpv renders nothing new and nothing repaints.
- **egui repaints:** `ctx.request_repaint()` gives two UI passes per call, so the widget wakes the UI with `request_repaint_after(1 ms)`, which gives one. With the zero delay the app took about twice the CPU (~300% on WSL), drawing every frame twice.
- **Dropping the widget** unregisters its texture at the start of the next egui pass (a small egui plugin), not at once: the pass that dropped it (playback ending) has usually drawn it already, and egui-wgpu paints that pass afterwards.

## Test loop for Windows

- Build a debug cross-build from WSL with mingw. Debug GPUI compiles its shaders at runtime, so it doesn't need `fxc.exe`.
- Copy it to `C:\Temp`, launch it through WSL interop, and read its log file.
- CI produces the MSVC release builds.
