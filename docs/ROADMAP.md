# jelly-gpui: Roadmap

What's missing and what still needs work, as of 2026-10-14. Done items are marked ✅ and kept for context.

- `docs/PLAN.md` describes the architecture, and `docs/in-app-video.md` the video pipeline. This document only lists gaps.
- **Priority:**
  - **P1:** needed before calling it v1.
  - **P2:** expected by most users soon after.
  - **P3:** nice to have, or a later scope.
- **Effort:**
  - **S:** under a day.
  - **M:** a few days.
  - **L:** a week or more, or blocked on hardware we don't have.

## Where things stand

v1 scope is mostly done:
- **Sign-in:** password and Quick Connect, with the session kept between launches.
- **Browsing:** Home shelves, library grids, and series, season, episode and movie pages, all with images and a disk image cache.
- **Search** and **mark as watched**.
- **Playback inside the window:** libmpv with GPU texture sharing on Windows and Linux, and zero-copy decoding on Windows through ANGLE.
- **Server reporting:** start, progress and stop.
- **Player controls:** tracks, versions, volume, next episode and autoplay.
- **CI:** builds packages for all four targets.

The big gaps:
- **Release hygiene:** a signed macOS build.
- **macOS playback quality.**
- **Testing** on hardware and servers we haven't touched (NVIDIA, Intel, real Linux GPUs, macOS), and the app UI on Jellyfin 10.11.

---

## 1. Release blockers (before tagging v0.1.0)

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 1.1 | **Choose a project license** | P1 | S | ✅ Done: GPL-3.0-or-later (`LICENSE`, `license` in every crate, shipped in every package). |
| 1.2 | **Package metadata** | P1 | S | ✅ Done: maintainer `lgpy <lgpy17@gmail.com>`, bundle id `io.github.lgpy.jelly-gpui`. |
| 1.3 | **Log to a file in release builds** | P1 | S | ✅ Done: daily log files, 7 kept, in the local data dir (`<config dir>/logs` with `JELLY_GPUI_CONFIG_DIR`), plus *Settings → Logs → Open log folder*. |
| 1.4 | **One-time `cargo fmt`, then a CI check** | P1 | S | ✅ Done: formatted at 120 columns (`rustfmt.toml`); CI and `mise run ci` run `cargo fmt --check`. |
| 1.5 | **Smoke-test the CI artifacts** | P1 | S | Partly done: the Windows zip runs on the dev machine, and the `.deb` installs on a clean Ubuntu 24.04 container (dependencies resolve, the app starts and logs). Still to do: a real Linux desktop with a GPU, and the macOS zip on a Mac (someone else's). |
| 1.6 | **Release notes / README** | P1 | S | ✅ Done: `README.md`. Screenshots are still to add. |

## 2. Playback

### 2.1 Player features

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 2.1.1 | **Skip intro / outro** | P2 | M | ✅ Done: a skip button during Intro, Recap, Preview and Commercial segments, from `/MediaSegments` or, without a segment plugin, from chapter names. The up-next card starts at the Outro. Untested against a real segment plugin (the dev servers have none); the chapter fallback is tested. |
| 2.1.2 | **"Up next" countdown** | P2 | S | ✅ Done: a card from the Outro (or 20 s before the end) with the next episode, "Starts in N s", *Play now* and *Cancel*; Esc also cancels. Autoplay is a setting. |
| 2.1.3 | **Chapters** | P2 | S | ✅ Done: marks on the seek bar, the chapter name next to the time, PgUp/PgDn. Not yet: chapter image thumbnails. |
| 2.1.4 | **Seek-bar preview thumbnails (trickplay)** | P3 | M | Jellyfin 10.9+ serves trickplay tile sheets (`/Videos/{id}/Trickplay/{width}/{index}.jpg`). Show a hover preview on the seek bar. |
| 2.1.5 | **Playback speed** | P3 | S | mpv `speed`. A small menu (0.5×–2×) plus `[` and `]`. |
| 2.1.6 | **Subtitle and audio delay, subtitle size** | P3 | S | mpv `sub-delay`, `audio-delay`, `sub-scale`. Useful for badly timed external subtitles. |
| 2.1.7 | **Show the current time on hover and the remaining time** | P3 | S | The seek bar shows no time where the mouse is. Option to show "-12:34" remaining. |
| 2.1.8 | **Mouse niceties** | P3 | S | Double-click toggles fullscreen, the scroll wheel changes volume, and the cursor hides along with the controls. |
| 2.1.9 | **Previous episode, and next for movies in a collection** | P3 | S | Mirror of 2.1.2. |
| 2.1.10 | **Resume prompt** | P3 | S | The item page has Resume / Play from start buttons, but autoplayed episodes always resume. Consider "Resume from 12:34?" when starting from a list. |

### 2.2 Playback robustness

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 2.2.1 | **Server transcoding fallback** | P1 | M | ✅ Done: an HLS H.264/AAC transcoding profile, the `TranscodingUrl` when the server won't direct play, and *Settings → Playback → Max streaming bitrate*. Tested on 12.2 (playback, seeking, subtitles, audio choice, stop cleanup) and the HLS requests on 10.11. Not yet: HEVC or AV1 transcodes (cheaper bandwidth, but costly for servers), and automatic bitrate from a speed test. |
| 2.2.2 | **Recover from network drops** | P2 | M | ✅ Done: ffmpeg's HTTP reconnect rides out short drops (tested with a server restart mid-play). For longer outages a session that fails or hits EOF more than 10 s early counts as lost and restarts from its position after 2, 5 and 10 s, with a notice; then a clear error. Progress reports already just log failures. Not in SyncPlay groups. |
| 2.2.3 | **Recover from an mpv crash** | P2 | M | Partly done: the running playback is saved in the config every 10 s and its stop is reported on the next launch if the app didn't exit cleanly (tested with `kill -9` mid-play). A crash still takes the app down; running libmpv in a child process would prevent that but is a big change. |
| 2.2.4 | **Report volume and mute** | P3 | S | `PlaybackProgressInfo` has `VolumeLevel` and `IsMuted`; other Jellyfin clients' remote-control UIs show them. |
| 2.2.5 | **Remote control from other clients** | P3 | L | Jellyfin's websocket (`/socket`) sends Play, Pause, Seek and similar commands from the web dashboard and phone apps. It needs `SupportsMediaControl` in session capabilities, a websocket client and command handling. |

### 2.3 Video output

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 2.3.1 | **macOS libmpv bundling** (in-app-video phase 4) | P1 | M | macOS users must `brew install mpv` today. Bundle `libmpv.dylib` plus its dependencies (e.g. from an IINA/mpv build, or built on CI), fix install names with `install_name_tool`, and sign them. Needs a Mac to verify. |
| 2.3.2 | **macOS GPU video** (phase 5) | P2 | L | macOS uses the software renderer, which costs a lot of CPU at 1080p and won't hold 4K. Plan: CGL context, IOSurface-backed `CVPixelBuffer` ring, `surface(CVPixelBuffer)` (GPUI already supports this). VideoToolbox zero-copy should then follow for free. Blocked on Mac access. |
| 2.3.3 | **Linux zero-copy decoding (VA-API)** | P2 | M | Linux GPU paths use `hwdec=auto-safe`, which ends up as `vaapi-copy` (mpv's `vaapi` interop gets no display from the render API). Pass a DRM render node with `MPV_RENDER_PARAM_DRM_DISPLAY_V2` so mpv maps VA surfaces into our EGL context through dma-buf. NVIDIA should already be zero-copy through `nvdec` (untested). |
| 2.3.4 | **Test on NVIDIA and Intel (Windows)** | P1 | S | Only the AMD RX 6600 has been tested. ANGLE, `WGL_NV_DX_interop2` and legacy share handles all have per-vendor quirks. Also test a hybrid laptop: ANGLE and GPUI must land on the same adapter for share handles to work. |
| 2.3.5 | **Test on real Linux GPUs** | P1 | M | Only Mesa llvmpipe/lavapipe (WSL) has been tested. Test AMD and Intel (Mesa radeonsi/iris, RADV/ANV) and NVIDIA's proprietary driver. Linear-tiled export is the likely problem area. Also X11 vs Wayland. |
| 2.3.6 | **HDR** | P3 | L | Frames are 8-bit RGBA, so HDR is tone-mapped to SDR by mpv (correct but not HDR). Real HDR output needs a 10-bit/FP16 shared texture and an HDR swapchain in GPUI, which GPUI doesn't do today. |
| 2.3.7 | **Frame pacing** | P3 | M | Sync between mpv's device and GPUI's relies on `glFinish` plus a 3-texture ring, with no keyed mutex or fences. It looks fine at 24–60 fps; check judder at high refresh rates, and add DXGI keyed mutexes or Vulkan semaphores if needed. GPUI also redraws on mpv's frame timer, not the display's vsync. |
| 2.3.8 | **Share code between the Windows renderers** | P3 | S | The WGL renderer and the ANGLE renderer share much of their slot/ring code; fold it together once both have been tested on more hardware. |

### 2.4 SyncPlay (watch together)

Wanted, so moved out of the plan's "not in v1" list. All the endpoints below are in the 12.2 OpenAPI spec and none are obsolete (checked against the dev server). Effort: **L** overall, but it splits into the steps below. Priority: **P2**.

**How Jellyfin's SyncPlay works:**
- The server keeps a group: its members, a shared play queue, and a state (idle, waiting, paused, playing).
- Clients send requests over HTTP:
  - Group: `POST /SyncPlay/New`, `Join`, `Leave`, `GET /SyncPlay/List`.
  - Playback: `Pause`, `Unpause`, `Seek`, `Stop`, `NextItem`, `PreviousItem`.
  - Queue: `SetNewQueue`, `Queue`, `SetPlaylistItem`, `RemoveFromPlaylist`, `MovePlaylistItem`, `SetRepeatMode`, `SetShuffleMode`.
  - Readiness: `Buffering`, `Ready`, `SetIgnoreWait`, `Ping`.
- The server answers through the session **websocket** (`/socket`):
  - `SyncPlayGroupUpdate` messages: joined, left, state changes, queue changes.
  - `SyncPlayCommand` messages: Unpause, Pause, Seek or Stop, each with a `When` timestamp in server time and a `PositionTicks`.
- Clients estimate the offset between their clock and the server's with `GET /GetUtcTime`, repeated a few times (NTP-style, keeping the lowest-latency sample). They then execute each command at the given moment. The web client corrects drift during playback by briefly changing the playback speed, or by skipping when the drift is large.

| # | Step | Effort | Notes |
|---|---|---|---|
| 2.4.1 | **Session websocket** | M | ✅ Done: `Client::socket()` in the `jellyfin` crate (header auth, keep-alive, reconnect with backoff, typed `ServerMessage`s), capabilities reported on each connect. The app refreshes pages on `UserDataChanged` (except during playback). Checked on 12.2 and 10.11 with `cargo run -p jellyfin --example socket`, including a 150 s hold. |
| 2.4.2 | **Server time sync** | S | ✅ Done in the `jellyfin` crate: `Client::clock_sample()` and `ServerClock` (keeps the lowest-latency of the last 8 samples, converts server times to local). Re-sampling every minute belongs to the SyncPlay session, once it exists. |
| 2.4.3 | **Group UI** | M | ✅ Done: a top-bar popover (list, join, new group; or the group, its state and members, and Leave), notifications for joins and leaves, and a "SyncPlay" label in the player. |
| 2.4.4 | **Playback under group control** | L | ✅ Done (see the plan §5.4): `SetNewQueue` when starting anything in a group, loading queue items paused with `Buffering`/`Ready`, scheduled Unpause/Pause/Seek/Stop, local controls sent as requests, `NextItem` at the end. Not yet: track changes that restart a transcode, version switches, shuffle/repeat and queue editing. |
| 2.4.5 | **Drift correction** | M | ✅ Done: ±5% `speed` nudges under 1.5 s of drift, a seek above. With an artificial 0.8 s lag (`JELLY_GPUI_SYNCPLAY_SKEW_MS=800`) it's back within 50 ms after about 18 s. Tune on real hardware; the thresholds are in `syncplay.rs`. |
| 2.4.6 | **Testing** | M | Partly done: `cargo run -p jellyfin --example syncplay` is a scripted second member, and the app was tested against it both ways (following and leading the group). Still to do: two app instances on different machines, and the Jellyfin web client as a member. |

**Order:** 2.4.1 and 2.4.2 first; they're also the base for remote control. Then group UI with play/pause/seek. Then queues (next episode in a group), then drift correction.

## 3. Browsing and library

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 3.1 | **Keyboard navigation in grids and shelves** | P1 | M | ✅ Done: a shared cursor (`browse.rs`) on Home, libraries, search and item pages. Arrow keys move it, Tab jumps between sections and Enter opens. It's hidden until a key is pressed. Left: version and track pickers, the episode rows' watched toggle and Settings still need the mouse. |
| 3.2 | **Sorting and filters in libraries** | P2 | M | ✅ Done: sort by name, date added, release date, rating, runtime or random (ascending/descending), show all, unwatched, watched, favorites or in progress; remembered per library. Not yet: genre, year and resolution filters. |
| 3.3 | **Favorites** | P2 | S | ✅ Done: a Favorite button on item pages (also on the keyboard cursor), a Favorites shelf on Home, and a Favorites filter in libraries. |
| 3.4 | **Cast and crew** | P2 | M | ✅ Done: a "Cast & crew" shelf on item pages (photo or initials, role or job) and person pages listing their movies and shows, with the library sort and filter. |
| 3.5 | **Collections and playlists** | P2 | M | ✅ Done: item pages for both (collections list their movies by release date, playlists their entries in order), with *Play all* and *Shuffle* through a local play queue that the up-next card and autoplay follow (in a SyncPlay group it becomes the group's queue). Not yet: editing playlists (add, remove, reorder) and creating them. |
| 3.6 | **Image placeholders** | P2 | S | ✅ Done: cards, posters, backdrops and episode stills show their blurhash (decoded at 32×32 and scaled up) while the image loads. |
| 3.7 | **Item page extras** | P3 | M | Trailers (local and `RemoteTrailers`), "More like this" (`/Items/{id}/Similar`), special features, studio and tags. |
| 3.8 | **Home customisation** | P3 | S | Respect the user's Home screen settings from the server (section order, hidden libraries), or offer our own toggles. |
| 3.9 | **Alphabet jump in long libraries** | P3 | S | A side bar of letters using `NameStartsWith`. |
| 3.10 | **Other media types** | P3 | L | Music, Live TV, books and photos are out of v1 scope. Music would be the first candidate, since mpv already plays audio. |

## 4. Accounts and settings

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 4.1 | **Player preferences** | P2 | S | ✅ Done: *Settings → Playback* has autoplay, skip intros automatically, and the max bitrate. Tracks not chosen explicitly (autoplay, next episode, SyncPlay) now follow the server's defaults, which apply the user's profile (language preferences, subtitle mode, remembered selections) instead of the file's default flags. |
| 4.2 | **Subtitle appearance** | P3 | S | Font size, colour, background (mpv `sub-*` options) for text subtitles. |
| 4.3 | **Theme** | P3 | S | Dark only. Add light and follow-system, from GPUI Kit's theme system. |
| 4.4 | **Remember window size and position** | P3 | S | The window always opens centred at 1280×800. |
| 4.5 | **Multiple servers or users** | P3 | M | One saved server at a time. A server/user switcher is out of v1 scope but often requested. |
| 4.6 | **Minimum-version gate on real 10.11** | P1 | S | ✅ Done: `mise run dev:min` (Jellyfin 10.11.11, legacy auth off): the smoke test passes, Quick Connect included. The app UI itself hasn't been clicked through on 10.11 yet. |

## 5. Platform and packaging

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 5.1 | **macOS signing and notarisation** | P2 | M | Builds are ad-hoc signed, so Gatekeeper warns on first launch. Needs an Apple Developer account. |
| 5.2 | **Windows installer and signing** | P2 | M | It's a zip today. An MSIX or Inno Setup installer gives Start-menu entries and uninstall; code signing avoids SmartScreen warnings. |
| 5.3 | **Linux formats** | P2 | M | `.deb` and `.tar.gz` only. Flatpak (with libmpv from the runtime or bundled) reaches most distros; an AppImage is the simpler alternative. Also add the `.desktop` file and icon to the tarball install instructions. |
| 5.4 | **ARM builds** | P3 | M | Windows on ARM and Linux aarch64 aren't built. The libmpv and ANGLE pins are x86_64 only. |
| 5.5 | **Self-update notice** | P3 | S | Check GitHub releases on launch and show "a new version is available". |
| 5.6 | **Pinned downloads are mirrors of third parties** | P3 | S | `packaging/mpv.lock` points at shinchiro's releases (permanent tags), and `packaging/angle.lock` at repo.msys2.org, which may prune old packages. Consider attaching both to our own GitHub release once per update, so CI doesn't depend on them. |
| 5.7 | **GPUI patch upkeep** | P2 | M | We vendor and patch `gpui-pre`, `gpui-pre-windows` and `gpui-pre-wgpu` 0.3.9 (`third_party/gpui/PATCHES.md`). Every gpui-kit upgrade means re-applying them. Offer the shared-texture surface API upstream to gpui-kit / GPUI to shrink that burden. |

## 6. Quality and engineering

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 6.1 | **Tests** | P2 | M | Partly done: JSON fixtures captured from 12.2 and 10.11 (`dev/capture-fixtures.py`) that every model must parse, with checks on versions, chapters and direct play vs transcode; unit tests for the keyboard cursor, chapters and segments, the server clock and time formats, websocket and SyncPlay messages. Still to do: a mocked HTTP test of the playback report flow, and GPUI `TestAppContext` tests for navigation and the player page. |
| 6.2 | **Integration test in CI** | P3 | M | Spin up the Jellyfin container in the Linux CI job and run `examples/smoke.rs` against it. |
| 6.3 | **Error messages** | P2 | S | ✅ Done: starting with the server down or still booting keeps the saved session, says it will sign back in automatically, and does so when the server answers (checked with the dev server stopped and restarted); a dropped connection reads "may be restarting"; wrong password, expired session and missing libmpv have their own wording; playback reconnects (2.2.2). Not rechecked here: an expired token while browsing. |
| 6.4 | **Accessibility** | P3 | L | GPUI's screen-reader support is limited. At least ensure good contrast, keyboard reachability (3.1), and text that scales with the system setting. |
| 6.5 | **Localisation** | P3 | M | All strings are English and inline. Extract them before the UI grows much more. |
| 6.6 | **Performance pass** | P3 | M | Profile large libraries (10k+ items) for grid scrolling and image decode, and check memory over long sessions (the image cache has no size limit in memory). |

---

## Suggested order

What has been built but still needs testing on real hardware, servers and with a second person is tracked in [`TESTING.md`](TESTING.md).

1. **Release blockers (§1)** and **10.11 testing (4.6):** small items that make a v0.1.0 tag responsible.
2. **Keyboard navigation (3.1)** and **transcoding fallback (2.2.1):** the two biggest functional gaps for v1.
3. **Hardware testing (2.3.4, 2.3.5):** borrow or rent an NVIDIA/Intel machine; ask testers with Linux desktops.
4. **Skip intro, up-next card, chapters (2.1.1–2.1.3):** most noticeable everyday player features.
5. **SyncPlay (2.4)**, starting with the websocket, which remote control (2.2.5) also needs.
6. **Library sorting/filters, favorites, cast (3.2–3.4).**
7. **macOS (2.3.1, 2.3.2, 5.1)** once a Mac is available.
