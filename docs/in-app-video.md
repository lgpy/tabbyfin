# In-app video: plan

**Goal:** play video inside the jelly-gpui window, with GPUI controls on top. This replaces the separate mpv window.

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

1. ✅ **libmpv backend and in-app player view, on the software path.** Done on 2026-10-14. Verified on Linux (WSL) and on Windows (RX 6600) with `dev/win-run.sh`. The Windows zip now bundles `libmpv-2.dll`, and the `.deb` depends on `libmpv2`.
   - `player::libmpv` (core and event loop) produces `PlayerEvent`s that the session turns into progress reports.
   - A full-window player page: seek bar, play/pause, ±10 s, audio and subtitle tracks, fullscreen, keyboard shortcuts. Not done yet: volume control, version switching during playback, a "next episode" action.
   - Rendering uses the software path at first, so in-app playback works on every OS straight away.
2. ✅ **Windows GPU sharing.** Done on 2026-10-14 and verified on the RX 6600.
   - **Probe:** `examples/dx_interop_probe.rs` confirmed `WGL_NV_DX_interop2` with D3D11 shared textures (and D3D9Ex as a fallback) on AMD driver 26.8.1.
   - **GPUI patches** (`third_party/gpui/PATCHES.md`): `SurfaceSource::SharedTexture`, and the DirectX renderer drawing surfaces with the polychrome sprite shaders.
   - **Player** (`player::libmpv::wgl`):
     - A render thread with a hidden WGL context.
     - A ring of 3 shared D3D11 BGRA textures, each registered with GL through the interop and attached to a framebuffer.
     - mpv renders with the `opengl` API into the next texture, then `glFinish` runs and the share handle is published. No Y flip is needed.
     - The player page draws the frame with `surface(SharedTexture)`.
   - **Fallback:** if GPU setup fails, the software path is used automatically. `JELLY_GPUI_VIDEO=software` forces it.
   - **Measured** (debug build, 1080p H.264, RX 6600):

     | Path | CPU |
     |---|---|
     | GPU | ~5% of one core |
     | Software | ~54% |

   - **Follow-ups:**
     - Decoding still copies frames back to the CPU (`hwdec=auto-copy-safe`). Zero-copy hardware decoding (dxva2 through the same interop) would remove that too.
     - Pacing between the 3 textures relies on `glFinish` plus the ring, with no keyed mutex.
     - Test on NVIDIA and Intel.
3. **Linux GPU sharing.** Vulkan external memory, plus the `gpui-pre-wgpu` patch. When wgpu runs on its GL backend, fall back to software.
4. **Packaging libmpv.**
   - Windows: `libmpv-2.dll`.
   - macOS: `libmpv.dylib`.
   - Linux: a `.deb` dependency on `libmpv2`.
   - Done for Windows and Linux. macOS uses Homebrew's libmpv for now.
5. **macOS GPU sharing** (IOSurface), when a Mac is available for testing.

## Test loop for Windows

- Build a debug cross-build from WSL with mingw. Debug GPUI compiles its shaders at runtime, so it doesn't need `fxc.exe`.
- Copy it to `C:\Temp`, launch it through WSL interop, and read its log file.
- CI produces the MSVC release builds.
