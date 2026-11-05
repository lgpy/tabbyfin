# Tabbyfin: Roadmap

What's still open. Finished work is listed in a line per area; its details are in the code, `docs/PLAN.md` and git history. What has been built but still needs checking on real hardware, servers or with a second person is in [`TESTING.md`](TESTING.md).

- **Priority:** **P1** is needed before calling it v1, **P2** is expected by most users soon after, **P3** is nice to have or a later scope.
- **Effort:** **S** is under a day, **M** a few days, **L** a week or more, or blocked on hardware we don't have.

## Where things stand

v1 scope is done: sign-in (password and Quick Connect, several accounts), Home, libraries, search and item pages, playback inside the window with server reporting, SyncPlay, remote control, Settings, and packages for Linux, Windows and macOS from CI. The whole app is in the cozy look (`docs/DESIGN.md`).

The big gaps:
- **Release:** no tagged release yet, and no signed macOS or Windows build.
- **macOS:** libmpv isn't bundled and video is drawn in software.
- **Testing** on hardware and servers we haven't touched (NVIDIA, Intel, real Linux GPUs, macOS), the app UI on Jellyfin 10.11, and a real mouse and keyboard (`TESTING.md`).

## 1. Release blockers (before tagging v0.1.0)

Done: license (GPL-3.0-or-later), package metadata, log files in release builds, `cargo fmt` in CI, the README.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 1.5 | **Smoke-test the CI artifacts** | P1 | S | The Windows zip runs on the dev machine and the `.deb` installs and starts in a clean Ubuntu 24.04 container. Still to do: a real Linux desktop with a GPU, and the macOS zip on a Mac. |
| 1.6 | **Screenshots in the README** | P1 | S | The README has no screenshots yet. |

## 2. Playback

Done: skip intro and outro, the up-next card and autoplay, chapters, trickplay thumbnails, speed, subtitle and audio delay, subtitle size, the time-left clock, mouse niceties, previous and next, the resume prompt; transcoding (H.264 and HEVC, automatic bitrate), recovery from network drops and player crashes, volume and mute in reports, remote control from other clients; SyncPlay with group UI, queue and drift correction; downloads of the original file, played from disk.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 2.1 | **Skip segments against a real plugin** | P3 | S | Segments come from `/MediaSegments`, or from chapter names without a plugin. Only the chapter fallback is tested; the dev servers have no segment plugin. |
| 2.2 | **AV1 transcodes** | P3 | S | Transcoding asks for H.264 or HEVC only. |
| 2.3 | **macOS libmpv bundling** | P1 | M | macOS users must `brew install mpv` today. Bundle `libmpv.dylib` and its dependencies, fix install names with `install_name_tool`, and sign them. Needs a Mac. |
| 2.4 | **macOS GPU video** | P2 | L | macOS draws video in software, which costs a lot of CPU at 1080p and won't hold 4K. Plan: a CGL context, an IOSurface-backed `CVPixelBuffer` ring, and a Metal import beside `widgets/video/{vulkan,dx12}.rs` (whether wgpu-hal exposes what that needs is untested). VideoToolbox zero-copy should follow. Blocked on Mac access. |
| 2.5 | **Linux zero-copy decoding (VA-API)** | P2 | M | Linux GPU paths use `hwdec=auto-safe`, which ends up as `vaapi-copy`. Pass a DRM render node with `MPV_RENDER_PARAM_DRM_DISPLAY_V2` so mpv maps VA surfaces into our EGL context through dma-buf. NVIDIA should already be zero-copy through `nvdec` (untested). |
| 2.6 | **Test on NVIDIA and Intel (Windows)** | P1 | S | Only an AMD RX 6600 has been tested. ANGLE, `WGL_NV_DX_interop2` and NT-handle shared textures opened by D3D12 have per-vendor quirks. Also a hybrid laptop: the player and the UI must land on the same adapter (the app passes the UI's adapter LUID to the player). |
| 2.7 | **Test on real Linux GPUs** | P1 | M | Only Mesa llvmpipe/lavapipe (WSL) has been tested. Test AMD and Intel (radeonsi/iris, RADV/ANV) and NVIDIA's proprietary driver, on X11 and Wayland. Linear-tiled export is the likely problem area. |
| 2.8 | **HDR** | P3 | L | Frames are 8-bit RGBA, so mpv tone-maps HDR to SDR. Real HDR needs a 10-bit/FP16 shared texture and an HDR swapchain. |
| 2.9 | **Frame pacing** | P3 | M | Sync between mpv's device and wgpu's relies on `glFinish` and a 3-texture ring, without keyed mutexes or fences, and the app redraws on mpv's frame timer rather than vsync. Fine at 24–60 fps; check judder at high refresh rates. |
| 2.10 | **SyncPlay with real members** | P2 | M | Tested against the scripted member (`cargo run -p jellyfin --example syncplay`). Still to do: two app instances on different machines, the Jellyfin web client as a member, and tuning the drift thresholds (`syncplay.rs`) on real hardware. |
| 2.11 | **Chromecast** | P3 | L | Cast to Jellyfin's own receiver app from the desktop and Android, through a CASTV2 sender of our own (`crates/cast`, not started). Needs a Chromecast or Google TV to test against. Plan in [`chromecast.md`](chromecast.md). |
| 2.12 | **Offline mode** | P2 | L | Downloads play from disk (`PLAN.md` §5.5), but only once the server has let the session in. Left: a signed-in screen with just the Downloads page when the server can't be reached (the user and server info kept with the downloads), the player's chapters and skip buttons from `item.json`, local watch state and resume points, and handing them to the server once it answers. |
| 2.13 | **Downloads on Android** | P3 | M | A foreground service so a long download survives the app going to the background, Wi-Fi only as a setting, the files in app-specific external storage, and Media3 playing the `file://` URL. |

## 3. Browsing and library

Done: keyboard navigation everywhere, library sort and filters, favorites, cast and crew, collections and playlists (with editing), blurhash placeholders, item page extras (trailers, extras, more like this), Home following the web client's settings, the alphabet jump, context menus, playing a series or season, loading an item's page ahead of the click.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 3.1 | **Playlist sharing and reordering** | P3 | S | Playlists are shared with everyone or no one; sharing with chosen users needs admin rights to list them. Entries move with buttons and keys, not by dragging. |
| 3.2 | **Remote trailers** | P3 | S | Untested: the dev server won't take `RemoteTrailers` through an item update, so the dev data has none. |
| 3.3 | **Other media types** | P3 | L | Music, Live TV, books and photos are out of v1 scope. Music would be first, since mpv already plays audio. |

## 4. Accounts and settings

Done: player preferences, subtitle appearance, theme, window size and position, several servers and users, Settings for this device and the server (languages, Home), a 10.11 smoke test.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 4.1 | **The app UI on real 10.11** | P1 | S | `mise run dev:min` passes the API smoke test on 10.11.11; the app itself hasn't been clicked through there. |

## 5. Platform and packaging

Done: an Inno Setup installer for Windows, an AppImage and a Flatpak bundle beside the `.deb` and `.tar.gz`, the update notice, mirrored pinned downloads.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 5.1 | **macOS signing and notarisation** | P2 | M | Builds are ad-hoc signed, so Gatekeeper warns on first launch. Needs an Apple Developer account. |
| 5.2 | **Windows code signing** | P2 | M | Avoids SmartScreen warnings on the installer and the exe. |
| 5.3 | **Flathub** | P2 | M | The Flatpak builds and plays from a local bundle; it isn't published. |
| 5.4 | **ARM builds** | P3 | M | Windows on ARM and Linux aarch64 aren't built. The libmpv and ANGLE pins are x86_64 only. |
| 5.5 | **Android (phones and tablets)** | P3 | L | Signs in, browses and plays by touch, in a phone layout (a bottom bar, compact pages, 48-point targets, long-press menus); Media3 (default) and libmpv (a setting) play on a phone, with a touch player page. Left: refining the touch UI, then the release (`build.sh --sign` makes the signed APKs; CI signing on tags and the key are left), predictive Back's animation, picture-in-picture and a `MediaSession`. Plan in [`android.md`](android.md). |

## 6. Quality and engineering

Done: an integration job in CI against 12.2 and 10.11 (not yet run on GitHub Actions), error messages for the common failures, bounded image memory and a loading queue.

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 6.1 | **UI tests** | P2 | M | API fixtures, a mocked server and `egui_kittest` UI tests (`crates/app/src/ui_tests/`) cover the client, reporting, the browse cursor, sign-in, dialogs, Settings, Home, the library, the item page and the widgets. The top bar and the player have little UI coverage. |
| 6.2 | **Screen readers** | P3 | L | Keyboard reach, contrast (WCAG AA) and text size are done, and every widget names itself to AccessKit, but no screen reader has been tried. |
| 6.3 | **Localisation** | P3 | M | All strings are English and inline. Extract them before the UI grows much more. |
| 6.4 | **Performance on real hardware** | P3 | M | Profiled only in WSL debug builds (10,000-item fake library: about 640 MB, posters in view 0.5 s after a fast scroll). Check a release build on real hardware. |
| 6.5 | **Expired token while browsing** | P3 | S | The error wording exists but hasn't been rechecked. |

## Suggested order

1. **Release (§1, 4.1):** screenshots, the artifact smoke tests and a 10.11 click-through, then tag v0.1.0.
2. **Hardware testing (2.6, 2.7):** borrow or rent an NVIDIA/Intel machine; ask testers with Linux desktops.
3. **Signing and Flathub (5.1–5.3).**
4. **macOS (2.3, 2.4, 5.1)** once a Mac is available.
5. **Android (5.5)**, following the milestones in `android.md`.
