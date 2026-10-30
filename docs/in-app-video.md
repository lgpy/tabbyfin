# In-app video: how mpv's picture gets into the window

**Goal:** play video inside the Tabbyfin window, with the app's own controls on top. There's no separate mpv window and no fallback to one; that was removed on 2026-10-14. Without libmpv, playback shows an install hint instead.

**Decision (2026-10-14):** **GPU texture sharing.** mpv renders on the GPU and the app imports its frames into wgpu's device, so egui draws them without a CPU copy. That keeps mpv's full quality (`gpu-next`) and 4K; HDR is tone-mapped to SDR, since frames are 8-bit RGBA. Software frames are the fallback everywhere.

- **Order:** it was built Windows first, then Linux (see History).
- **macOS:** software frames for now. There's no Mac to test with, so no Metal import has been attempted.

## Pipeline

```
player::libmpv  (the player host process, or in-process with TABBYFIN_PLAYER=embedded)
  render thread: mpv draws through OpenGL into one of 3 shared textures (the ring)
      │ "frame N ready" → the app's frame receiver
      ▼
platform interop (GL → native texture sharing)
  Windows: ANGLE (d3d11va, zero-copy) or WGL + WGL_NV_DX_interop2 → D3D11 texture
  Linux:   EGL + GL_EXT_memory_object_fd → Vulkan image (exported opaque fd)
  macOS:   (software frames)
      │
      ▼
crates/app/src/widgets/video.rs
  vulkan.rs / dx12.rs: import the texture into wgpu's device once per ring image,
  register it with egui-wgpu (Renderer::register_native_texture), draw it letterboxed
```

Files: `crates/player/src/libmpv/{video,opengl,angle,wgl,egl,d3d11}.rs`, `crates/player/src/{host,remote,wire}.rs`, `crates/app/src/widgets/video.rs`, `widgets/video/{vulkan,dx12}.rs`, `crates/app/src/main.rs` (wgpu setup), `crates/app/src/app.rs` and `playback.rs` (the import is set per sign-in), `views/player.rs` (a failed import falls back).

## Constraints

- **libmpv's render API only offers `opengl` and `sw`.** There's no Vulkan, D3D or Metal render API (checked in mpv v0.41.0 `render.h`). So mpv always draws through OpenGL, and every platform needs OpenGL-to-native-API texture sharing.
- **The textures must be made on the same GPU** whose device the window draws with: opaque Vulkan fds only import on the device that made them, and D3D12 only opens textures made on its own adapter. Hence the handshake below, which tells the player which GPU to put everything on.
- **wgpu's import goes through `wgpu-hal`.** `texture_from_raw` wraps a Vulkan image or a D3D12 resource as a `wgpu::Texture` (`create_texture_from_hal`); egui-wgpu registers that texture under an egui texture id. Both imports live in the app crate, not in the UI framework: egui-wgpu has `register_native_texture`, so no patched dependency is needed. (The GPUI app this replaced had to vendor patched crates under `third_party/gpui/`; they're gone.)

## The handshake: what the app can draw

The app tells the player, before libmpv starts, which frames it can show. **`player::libmpv::VideoImport`** (`crates/player/src/libmpv/video.rs`) is the answer:

| `VideoImport` | Meaning |
|---|---|
| `Pixels` | Software frames only (`VideoFrame`: RGBA, opaque alpha, tightly packed). The default. |
| `VulkanMemory { device_uuid }` | Linux: Vulkan memory exported as opaque fds, imported by the Vulkan device with this `VkPhysicalDeviceIDProperties::deviceUUID`. |
| `D3d11Textures { adapter_luid }` | Windows: D3D11 textures shared through named NT handles, opened by a D3D12 device on the DXGI adapter with this `LUID`. |

**The app side** (`crates/app/src/widgets/video.rs`):

- `adapter_import(render_state)` reads what wgpu's adapter and device can take, through wgpu-hal: on Vulkan with `Features::VULKAN_EXTERNAL_MEMORY_FD`, the physical device's `deviceUUID` (`vulkan_device_uuid`, using `ash`); on DX12 on a hardware adapter, the adapter's LUID (`dx12_adapter_luid`, not on WARP, where the player's GPU path would cost more than pixels); `Pixels` otherwise, which includes wgpu on GL and macOS.
- `import(render_state)` is what the app asks for: `adapter_import`'s answer as it is (or `Pixels` when there's no render state, as in tests), logged as `video: what the player is asked for`.
- `App` sets it on the session's `PlaybackManager` at sign-in (`set_video_import`), and each session passes it to `Player::spawn(import)` (`crates/app/src/playback.rs`).
- wgpu is set up for this in `main.rs`: backends `DX12` on Windows, `METAL` on macOS, `VULKAN | GL` elsewhere (`WGPU_BACKEND` still overrides); and the device is requested with `VULKAN_EXTERNAL_MEMORY_FD` when the adapter has it, because a device must be created with that feature to import fds.

**The player side:**

- **Host process:** the import is the first wire message, `Request::Start { import }` (`crates/player/src/remote.rs`, read by `host.rs`), sent before the app waits for `Message::Started`; the host starts libmpv only after reading it, so it can pick the video output with the import. Being a typed, versioned message, it needs no extra `--player-host` argument.
- **Embedded** (`TABBYFIN_PLAYER=embedded`): `Player::spawn_embedded(import)` gets it directly.
- **`render_thread`** (`crates/player/src/libmpv/video.rs`) tries only the GPU paths matching the import, in order, and falls back to the software renderer: for `D3d11Textures`, ANGLE (`angle.rs`) and then WGL (`wgl.rs`); for `VulkanMemory`, the EGL/Vulkan exporter (`egl.rs`); nothing for `Pixels`, or for an import from the other OS. A GPU path that fails to set up is logged and software rendering takes over, so the player never produces frames the app can't show. `TABBYFIN_VIDEO=software` forces software; `=gpu` forces the GPU path even on a software rasterizer.
- `hwdec=auto-safe` on the GPU paths, so frames can stay on the GPU where mpv has an interop (d3d11va with ANGLE, nvdec anywhere); `auto-copy-safe` on software. The log says which decoder each file got.

**Fallback when an import still fails** (a driver quirk, another GPU): the widget's `import_failed()` turns true (`crates/app/src/widgets/video.rs`), and the player page calls `Services::fall_back_to_pixels` (`crates/app/src/views/player.rs` → `playback.rs`), which sets the import to `Pixels` and restarts the session at its position. The sign-in keeps to pixels from then on, so it can't loop.

## Linux GPU frames

`crates/app/src/widgets/video/vulkan.rs` (import), `crates/player/src/libmpv/egl.rs` (renderer), `crates/app/src/main.rs` (device features).

- **When:** wgpu runs on Vulkan and its device was made with `VULKAN_EXTERNAL_MEMORY_FD`. On wgpu's GL fallback (`WGPU_BACKEND=gl`, or no Vulkan) `adapter_import` finds nothing to import and the app asks for pixels.
- **Same GPU:** the app sends its Vulkan `deviceUUID`, and the player puts everything there (`egl.rs`): the exporting Vulkan device is the one with that UUID, and mpv's OpenGL context is made on the EGL display whose `GL_DEVICE_UUID_EXT` matches — the default display first, then each EGL device (`EGL_EXT_device_enumeration` + `EGL_EXT_platform_device`) — which covers laptops whose default GL is the other GPU. Nothing matching means software frames. The context asks for no surface type, since a device's display has no window configs.
- **Software rasterizer rule:** mpv's OpenGL on llvmpipe/softpipe still means software frames unless `TABBYFIN_VIDEO=gpu` (the app on lavapipe asks for Vulkan memory regardless; the player decides). Forced onto a software rasterizer, mpv's scalers are set to bilinear: mpv 0.37's defaults draw black frames on llvmpipe whenever the video is scaled.
- **Import:** the frame's fd is duplicated and imported into a `VkImage` described exactly as the player made it (linear RGBA8, dedicated allocation, the same memory type rule), wrapped with wgpu-hal's `texture_from_raw(…, drop_callback, TextureMemory::External)` and `create_texture_from_hal(…, TextureUses::UNINITIALIZED)`. The drop callback destroys the image and frees the memory once wgpu is done with the texture. wgpu's first barrier takes each image out of `UNDEFINED` once; linear images keep their contents through it on Mesa, which is one of the things to watch on real drivers.
- **Cache:** one texture and egui texture id per ring image (`SharedFrame::id`), so each of the three images is imported once and then only switched between. A frame of a new size means a new ring: the old images' registrations end at the start of the next pass (the same deferred free as dropping the widget), and their textures go with them.
- **fd lifetime:** `SharedFrame::fd` is an `Arc<OwnedFd>` shared by the frames of one image (embedded: the render thread's slot; host: the app's fd table), so the fd a frame names stays open, and its memory alive, until the UI has imported it, even after a resize replaced the ring. The fds travel over a socket pair (`SCM_RIGHTS`; the host inherits its end as `TABBYFIN_PLAYER_FD`, `crates/player/src/remote.rs`).
- **Sync, as before the migration:** `glFinish` after mpv draws, before the frame is published, so the UI never samples a frame GL is still writing; and the ring of three (`opengl.rs`), so mpv writes an image again only two frames after it was sent, long after the UI (which always draws the newest frame) has sampled it. No fences or semaphores yet. wgpu-hal can add semaphores to the next submit (`vulkan::Queue::add_wait_semaphore` / `add_signal_semaphore`) and GL can share them (`GL_EXT_semaphore_fd`), but GL only has binary semaphores, which need exactly one wait per signal, while the UI skips frames; and the other direction would need a message back to the player per frame, with mpv waiting on the UI. `glFinish` costs the render thread the GPU's time per frame, which it would wait for anyway. If real drivers show tearing or stale frames (the import has no acquire barrier from the external queue family either), that's the next step (ROADMAP 2.3.7).
- **Measured on WSL** (debug, 1080p H.264 in a 1280×800 window, `TABBYFIN_VIDEO=gpu`, everything on the CPU): app ~135%, host ~200%, against ~143% and ~63% for software frames. On a software rasterizer the GPU path costs more, hence the rule; on a real GPU both sides should drop to a few percent, which is still to be measured.
- **Not tested yet:** real Mesa GPUs (AMD/Intel) and NVIDIA's proprietary driver. The extensions are standard there, but driver quirks with linear tiling are possible (`docs/TESTING.md` §7).

## Windows GPU frames

`crates/app/src/widgets/video/dx12.rs` (import), `crates/player/src/libmpv/{angle,wgl,d3d11}.rs` (renderers), `crates/player/src/wire.rs` (`SharedHandle`).

- **When:** wgpu runs on DX12 (`main.rs` forces it; `WGPU_BACKEND` can pick another API, which gets pixels) on a hardware adapter. The app sends the adapter's LUID.
- **Same GPU:** D3D12 only opens textures made on its own adapter, so the player makes its textures there (`d3d11.rs`). ANGLE gets the LUID as display attributes (`EGL_PLATFORM_ANGLE_D3D_LUID_HIGH_ANGLE` / `_LOW_ANGLE`, when ANGLE has `EGL_ANGLE_platform_angle_d3d_luid`), and the player then checks the adapter of the D3D11 device ANGLE made; another adapter is an error, so WGL is tried. The WGL path creates its D3D11 device on the DXGI adapter with that LUID (`EnumAdapterByLuid`, `D3D_DRIVER_TYPE_UNKNOWN`); its OpenGL runs wherever the driver puts it, and if that's the other GPU of a laptop the interop fails and playback uses software frames.
- **NT handles:** D3D12 can't open D3D11's legacy share handles. Each ring texture is made with `D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE` (no keyed mutex) and shared with `IDXGIResource1::CreateSharedHandle` under a name, `Local\tabbyfin-video-<pid>-<process start>-<id>`: unique per texture and never reused, so a name can only open the texture it was made for. The app opens it by name (`ID3D12Device1::OpenSharedHandleByName`, then `OpenSharedHandle` → `ID3D12Resource`), so nothing has to be duplicated into the app's process and the same frames work from the host and embedded. Duplicating the NT handle into the app (`DuplicateHandle`, which needs the app's process handle in the host) was the alternative; named handles were simpler and worked, so it wasn't needed. The wire message is `SharedHandle { id, name, width, height }` (`crates/player/src/wire.rs`).
- **Texture lifetime:** a name opens only while some handle to the texture is open. `SharedFrame` holds the texture's NT handle (`Arc<OwnedHandle>`, shared with its ring slot), so in-process a frame keeps its texture alive until the UI has drawn it, even after a resize replaced the ring. Frames from the host arrive without it; the host keeps the handles of the last 6 frames it sent (two rings). If a name doesn't open anyway (`ImportError::Gone`), the widget keeps the last frame up and waits for the next one; 10 in a row count as a failed import. Once the app has opened a texture, its `ID3D12Resource` keeps it alive on its own.
- **Import:** the resource is checked (2D, the frame's size, `R8G8B8A8_UNORM` from ANGLE or `B8G8R8A8_UNORM` from WGL), wrapped with wgpu-hal's `dx12::Device::texture_from_raw` and `create_texture_from_hal`, cached per ring image like Linux's, and registered with egui-wgpu. Dropping the `wgpu::Texture` releases the resource once the GPU is done with it.
- **Resource state:** the texture is handed to wgpu as already in `TextureUses::RESOURCE` (shader resource) and only ever sampled, so wgpu never puts a barrier on it. A texture opened from another device starts in `COMMON`; D3D12 promotes it to a shader resource implicitly on each submit that samples it and lets it decay back to `COMMON` when that submit finishes, which is the state the player's D3D11 device needs to draw into it again. Declaring it `UNINITIALIZED`, as on Linux, would have wgpu move it out of `COMMON` once with an explicit barrier and leave it there while the player writes it.
- **Sync:** `glFinish` before the frame is published, and the ring of three. No shared fences (ROADMAP 2.3.7); no tearing was seen on the RX 6600.
- **Fallback:** ANGLE, then WGL, then software, as before; a failed import switches the session to software frames (the handshake's fallback above).
- **Measured** (debug builds, RX 6600, Test Movie's 4K HEVC 10-bit version at 24 fps in a 1296×759 window, rendered at 1280×720; Windows-side CPU time over 8–10 s windows, after the first seconds; a temporary counter showed the UI drawing all 24 frames a second):

  | Path | App | Player host | Together | Embedded (`TABBYFIN_PLAYER=embedded`) |
  |---|---|---|---|---|
  | ANGLE, `d3d11va` | 3–14% | 5–17% | 14–26% (mostly 14–21%) | 18–19% (15–17% with `WGPU_VALIDATION=0`) |
  | WGL, `d3d11va-copy` | 4–12% | ~23% | 27–34% | — |
  | Software (`TABBYFIN_VIDEO=software`) | 13–20% | 100–137% | — | — |

  How the CPU splits between the app and the host varies from sample to sample, more than the sum does. For comparison, the GPUI app's ANGLE path in one process cost ~11% (see History) and its WGL host ~22%, so the egui app's ANGLE costs about one and a half times that. Release builds weren't measured.
- **Packaging:** on Windows the zip carries ANGLE (MSYS2's ucrt64 build, about 12 MB with its runtime DLLs, `packaging/angle.lock`, `packaging/fetch-angle.sh`). Delete `libEGL.dll` next to the exe to try the WGL path.

## Software frames

Every path ends here when no import can be used; it's also what macOS shows for now.

- mpv renders `rgb0`, the alpha byte is forced to 255, and the pixels go tightly packed as RGBA (`VideoFrame`), which is what a `Rgba8Unorm` texture takes as is.
- The video widget writes each frame with `queue.write_texture` into one texture per frame size, registered once with egui-wgpu (`register_native_texture`; a new size swaps the texture under the same id), and wakes the UI once per frame.
- Measured with debug builds, 1080p H.264 in a 1280×800 window (rendered at 1280×720), 25 s into playback (the first seconds also pay for mpv reading the file ahead):

  | | App | Player host | Embedded (`TABBYFIN_PLAYER=embedded`) | GPUI's software path |
  |---|---|---|---|---|
  | WSL (llvmpipe) | ~148% | ~65% | ~191% | ~280% |
  | Windows (RX 6600, `TABBYFIN_VIDEO=software`) | ~7% | ~29% | ~45% | ~54% |

  Paused: 0%, since mpv renders nothing new and nothing repaints.
- **Repaints:** `ctx.request_repaint()` gives two UI passes per call, so the widget wakes the UI with `request_repaint_after(1 ms)`, which gives one. With the zero delay the app took about twice the CPU (~300% on WSL), drawing every frame twice.
- **Dropping the widget** unregisters its texture at the start of the next egui pass (a small egui plugin), not at once: the pass that dropped it (playback ending) has usually drawn it already, and egui-wgpu paints that pass afterwards.
- **No HDR:** frames are 8-bit RGBA, so HDR is tone-mapped to SDR by mpv (correct, but not HDR). Real HDR output needs a 10-bit/FP16 shared texture and an HDR swapchain, which the app doesn't have (ROADMAP 2.3.6).

## History

The GPUI app that came before used the same pipeline with patched crates instead of egui-wgpu:

1. **libmpv backend and the software path** (2026-10-14), verified on Linux (WSL) and on Windows (RX 6600).
2. **Windows GPU sharing** (2026-10-14), verified on the RX 6600: ANGLE (`d3d11va`) and WGL (`WGL_NV_DX_interop2`), chosen in that order, with software as the fallback; `examples/dx_interop_probe.rs` had confirmed the interop on AMD driver 26.8.1. Measured (debug, 1080p H.264): GPU ~5% of a core against ~54% software; with ANGLE at 4K HEVC 10-bit, ~11% (`d3d11va`) against ~22% (`d3d11va-copy` through WGL).
3. **Linux GPU sharing** (2026-10-14), verified on Mesa llvmpipe/lavapipe, i.e. without a real GPU; `examples/vk_interop_probe.rs` checks export/import both ways.
4. **Packaging** `libmpv`: done for Windows and Linux; macOS still uses Homebrew's libmpv.
5. **macOS GPU sharing** (IOSurface) is open, when a Mac is available.

The migration to egui kept all of that and changed only where the import lives: `widgets/video/{vulkan,dx12}.rs` instead of the vendored GPUI renderers, and the `VideoImport` handshake instead of GPUI's patched surface API. The patches (`third_party/gpui/`, `[patch.crates-io]`) are gone.

## Test loop for Windows

- Build a cross-build from WSL with mingw (`dev/win-run.sh`, release; `--debug` for a debug build). wgpu's DX12 backend compiles its shaders at runtime, so it doesn't need `fxc.exe`.
- Copy it to `C:\Temp`, launch it through WSL interop, and read its log file.
- CI produces the MSVC release builds.
