# Spike: video inside the GPUI window (milestone 8)

> **Historical:** this spike measured the GPUI app, which the egui migration replaced (see `docs/in-app-video.md` for the current video paths). The numbers are kept as a record.

**Question:** can we show mpv's video inside our window, with GPUI drawing the controls on top? This is option C in plan §5.4: libmpv's software render API, with frames uploaded as GPUI images.

**Answer:** yes. It plays at full frame rate up to 1080p, with a GPUI overlay drawn on the video. Almost all the measured cost here comes from this machine having no GPU, not from the approach. We still need a run on real hardware before shipping it.

## What was built

(The spike code was later folded into `player::libmpv`, the in-app player; see `docs/in-app-video.md`. The original example is in commit `ef28e48`.)

- **`player::software`** (`crates/player/src/software.rs`), behind the `software-render` feature:
  - Loads libmpv at runtime with `dlopen`. Builds don't need libmpv, and the app keeps working without it.
  - Sets up the software render context.
  - On a render thread, it renders each new frame as `bgr0` into a buffer aligned to 64 bytes, copies it out with opaque alpha, and publishes it on a `watch` channel. Only the latest frame is kept.
- **`crates/app/examples/inline_video.rs`:**
  - A GPUI window that turns each frame into a `RenderImage` and frees the previous frame's texture.
  - Overlays: stats, pause, and seek ±10 s.
  - Run it with `cargo run --release -p tabbyfin --example inline_video -- <url> [header] [WxH]`. Set `SPIKE_THREADS=1` to print the per-thread CPU breakdown.

## Measurements

| Setup | Value |
|---|---|
| Machine | WSL2, 12 threads |
| GPU | **none**: GPUI and mpv both render through Mesa llvmpipe, on the CPU |
| Media | Synthetic 24 fps H.264 from the dev server, played in software |

| | 720p | 1080p |
|---|---|---|
| Frames rendered by mpv | 23–24.5 fps (all of them) | 23.5–24.5 fps (all of them) |
| Frames shown by GPUI | same as rendered, after a warm-up of about 2 s | same as rendered |
| Our frame copy | 1.0 ms/frame | 1.4–1.5 ms/frame |
| Total CPU, in-window spike | ~190% of one core | ~210% |
| Total CPU, plain mpv in its own window (baseline) | ~16% | ~31% |

Where the in-window CPU goes, measured per thread (`/proc/self/task`):

| Threads | 720p | 1080p |
|---|---|---|
| llvmpipe (software GPU drawing GPUI's window) | **164%** | **161%** |
| GPUI main thread | 12% | 13% |
| mpv decode, audio, demux | 4% | 6% |
| mpv software render plus our copy | 4% | 6% |
| other | 9% | 21% |

**How to read this:**
- **llvmpipe:** the 7× gap to the baseline is almost entirely llvmpipe repainting the whole GPUI window 24 times a second on the CPU. On a machine with a GPU that work moves to the GPU.
- **Real cost of the approach:** what this approach really adds is mpv's software scaling plus our copy (~5%) and the texture upload. That's small at 1080p.
- **Unexplained:** the "other" 21% at 1080p isn't explained yet. It's probably wgpu and driver threads.

These are numbers from a machine without a GPU. They show the approach works and roughly how its cost splits up. They are not a performance verdict.

## Limitations of option C (from the API and this spike)

- **No zero-copy hardware decoding.** Hardware decoding has to copy frames back to the CPU (`hwdec=auto-copy-safe`), and every frame goes through CPU memory twice: mpv's render, then the GPU upload.
- **Bandwidth grows with resolution.** A 1080p BGRA frame is 8 MB, which is about 200 MB/s at 24 fps. 4K at 60 fps would be about 2 GB/s of copies plus uploads. That's not realistic.
- **8-bit RGB output.** No HDR passthrough, and none of mpv's GPU shaders: no `gpu-next` tone mapping and no user shaders. Picture quality is lower than mpv's own window for HDR and high-end setups.
- **A new texture every frame.** GPUI caches images by id, so each frame is a new texture in the atlas. The spike frees the previous one with `window.drop_image`. GPUI has no API for updating a texture in place.
- **Not tested yet:** subtitles (mpv draws them into the frame, so they should work), window resizing (the frame size is fixed at startup), audio/video sync under load, and Windows or macOS.

## Recommendation

1. **Keep option A** (mpv in its own window) as the default. It has full quality, hardware decoding and HDR, and costs the least.
2. **Measure on real hardware next:** run the spike on a machine with a GPU, ideally a native Windows build, and with real HEVC/10-bit 1080p and 4K files. If 1080p stays well under a core there, option C is worth shipping as an **opt-in "play inside the window" mode for content up to 1080p**. Its main benefit is a single window with our own GPUI controls.
3. **Treat option D as the long-term answer** for 4K and HDR inside the window: mpv's GPU render API with the texture shared into GPUI's renderer. It needs changes to GPUI's wgpu renderer for each platform, so it's a separate project. This spike's frame-delivery plumbing (render thread, latest-frame channel, image swap) carries over.

**Turning C into a product mode would need:**
- Render at the element's real pixel size, tracking resizes.
- A `Player` backend trait shared with the IPC backend, so progress reporting is reused.
- Keyboard controls and a seek bar.
- Install guidance for libmpv. On Windows, `libmpv-2.dll` ships in the mpv-dev builds. On macOS it isn't in `mpv.app`, so we'd have to bundle `libmpv.dylib`.
