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

**Fallbacks**, chosen automatically and logged:
1. GPU sharing.
2. Software render: the spike, `player::software`.
3. A separate mpv window over IPC, today's behaviour, used when libmpv can't be loaded.

**GPUI fork:**
- The crates we change (`gpui-pre`, `gpui-pre-windows`, later `gpui-pre-wgpu`) are vendored under `third_party/gpui/` and applied with `[patch.crates-io]`.
- Each change is kept small and listed in `third_party/gpui/PATCHES.md`, so it can be re-applied on a gpui-kit upgrade and offered upstream.

## Phases

1. ✅ **libmpv backend and in-app player view, on the software path.** Done on 2026-10-14. Verified on Linux (WSL) and on Windows (RX 6600) with `dev/win-run.sh`. The Windows zip now bundles `libmpv-2.dll`, and the `.deb` depends on `libmpv2`.
   - `player::libmpv` (core and event loop) produces the same `PlayerEvent`s as the IPC backend, so progress reporting is reused.
   - A full-window player page: seek bar, play/pause, ±10 s, audio and subtitle tracks, fullscreen, keyboard shortcuts. Not done yet: volume control, version switching during playback, a "next episode" action.
   - Rendering uses the software path at first, so in-app playback works on every OS straight away.
2. **Windows GPU sharing.**
   - Patch `gpui-pre` and `gpui-pre-windows` for a D3D11 shared-texture surface.
   - In the player, a WGL context and `WGL_NV_DX_interop2` render into that texture.
   - Fall back to software if the extension is missing.
3. **Linux GPU sharing.** Vulkan external memory, plus the `gpui-pre-wgpu` patch. When wgpu runs on its GL backend, fall back to software.
4. **Packaging libmpv.**
   - Windows: `libmpv-2.dll`.
   - macOS: `libmpv.dylib`.
   - Linux: a `.deb` dependency on `libmpv2`.
   - Then remove the bundled `mpv.exe` and `mpv.app` once the IPC fallback is no longer the default.
5. **macOS GPU sharing** (IOSurface), when a Mac is available for testing.

## Test loop for Windows

- Build a debug cross-build from WSL with mingw. Debug GPUI compiles its shaders at runtime, so it doesn't need `fxc.exe`.
- Copy it to `C:\Temp`, launch it through WSL interop, and read its log file.
- CI produces the MSVC release builds.
