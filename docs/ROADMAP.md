# jelly-gpui: Roadmap

What's missing and what still needs work, as of 2026-10-14 (commit `23b7c57`).

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
- **Keyboard navigation** outside the player.
- **Release hygiene:** a log file, a license, a signed macOS build.
- **macOS playback quality.**
- **Testing** on hardware and servers we haven't touched (NVIDIA, Intel, real Linux GPUs, Jellyfin 10.11).

---

## 1. Release blockers (before tagging v0.1.0)

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 1.1 | **Choose a project license** | P1 | S | Nothing declared yet. It has to be compatible with shipping GPL-2.0+ libmpv (Windows zip) and GCC runtime DLLs (runtime exception, fine). GPL-3.0 or MPL-2.0 would both work; MIT/Apache work for our code but the Windows bundle as a whole is GPL-governed. Add `LICENSE`, `license =` in `Cargo.toml`, and a line in `THIRD_PARTY.md`. |
| 1.2 | **Package metadata** | P1 | S | The `.deb` maintainer is a placeholder (`crates/app/Cargo.toml`). The macOS `CFBundleIdentifier` needs a real reverse-DNS id (`packaging/Info.plist`). |
| 1.3 | **Log to a file in release builds** | P1 | S | Release Windows builds have no console (`windows_subsystem = "windows"`), so logs go nowhere and bug reports have nothing to attach. Write a rolling log to the cache/data dir (e.g. `tracing-appender`), and add an "Open log folder" button to Settings. |
| 1.4 | **One-time `cargo fmt`, then a CI check** | P1 | S | The code hasn't been formatted yet, so CI can't check formatting. Do one formatting commit, then add `cargo fmt --check` to the `check` job. |
| 1.5 | **Smoke-test the CI artifacts** | P1 | S | So far only the Windows zip has been launched. Install the `.deb` on a clean Ubuntu, and open the macOS zip on a Mac (someone else's). |
| 1.6 | **Release notes / README** | P1 | S | There's no README with install steps, system requirements (Vulkan on Linux, `brew install mpv` on macOS) or screenshots. |

## 2. Playback

### 2.1 Player features

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 2.1.1 | **Skip intro / outro** | P2 | M | Jellyfin 10.10+ has media segments (`GET /MediaSegments/{itemId}`). Show a "Skip intro" button during Intro segments, and offer the next episode early at an Outro. |
| 2.1.2 | **"Up next" countdown** | P2 | S | Autoplay currently starts the next episode right away at EOF. Show a card in the last ~20 s (or at the Outro segment) with "Play now" and "Cancel". Make autoplay a setting. |
| 2.1.3 | **Chapters** | P2 | S | mpv already reads chapters from the file. Add chapter marks on the seek bar and previous/next chapter keys (PgUp/PgDn). The `Chapters` model field exists; Jellyfin chapter images could serve as thumbnails. |
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
| 2.2.1 | **Server transcoding fallback** | P1 | M | `PlaybackInfo` is sent with direct play only (`TranscodingProfiles: []`). That fails when direct play isn't allowed (the user lacks the direct-play permission, a bitrate limit, remote access over a slow link). Add a transcoding profile (HLS, H.264/AAC) and use `TranscodingUrl` when the server returns it. Also expose a "max bitrate" setting for remote use. |
| 2.2.2 | **Recover from network drops** | P2 | M | If the stream stalls or the server goes away mid-play, mpv eventually ends with an error. Detect `end-file` errors after start, retry from the last position a couple of times, then show a clear message. Progress reports also need to survive a temporary 401/5xx. |
| 2.2.3 | **Recover from an mpv crash** | P2 | M | libmpv now runs in our process, so a crash in mpv or a driver takes the whole app down. Options: run libmpv in a child process (big change), or at least report the stop on next launch from the last saved position. The old IPC design had this; the embedded one doesn't. |
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
| 2.4.1 | **Session websocket** | M | Connect to `/socket` with the same `Authorization: MediaBrowser` header as HTTP (a native websocket client can set headers; `ApiKey` in the query also works, legacy `api_key` doesn't in 12.x). Check that 10.11 accepts the header on the socket too. Answer `KeepAlive`, reconnect with backoff, and report capabilities (`POST /Sessions/Capabilities/Full`). This is shared with remote control (2.2.5) and lets other pages refresh on `UserDataChanged`. Lives in the `jellyfin` crate as a stream of typed messages. |
| 2.4.2 | **Server time sync** | S | Sample `/GetUtcTime` (request and response times) to estimate the offset and round-trip time. Re-sample every minute or so. Convert server `When` times to local `Instant`s. |
| 2.4.3 | **Group UI** | M | A SyncPlay button in the top bar or player: list groups, create one, join or leave. Show members and the group state, and handle "user joined/left" messages as notifications. |
| 2.4.4 | **Playback under group control** | L | While in a group, local controls send requests instead of acting directly. Play, pause and seek go to `/SyncPlay/*`, and the player waits for the server's command. Then: <br>• Scheduled commands: pause or seek mpv at the given moment, then unpause exactly at `When`. <br>• Buffering: report `Buffering`/`Ready` around loads and seeks (mpv's `paused-for-cache` and `seeking`). <br>• Queue: starting an item in a group means `SetNewQueue`. The server then tells every member which item to play, and each member negotiates its own `PlaybackInfo` and stream. |
| 2.4.5 | **Drift correction** | M | Compare mpv's `time-pos` with the expected group position. Nudge `speed` (e.g. ±5%) for small drift, seek for large drift. This is the part that makes it feel synchronised; mpv's precise `speed` control makes it easier than in browsers. |
| 2.4.6 | **Testing** | M | Needs two clients. Use the Jellyfin web client in a browser against the dev server as the second member, plus two instances of jelly-gpui (separate `JELLY_GPUI_CONFIG_DIR`s so they get different device ids). |

**Order:** 2.4.1 and 2.4.2 first; they're also the base for remote control. Then group UI with play/pause/seek. Then queues (next episode in a group), then drift correction.

## 3. Browsing and library

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 3.1 | **Keyboard navigation in grids and shelves** | P1 | M | The plan promises keyboard-first navigation, but only the player, search (`/`) and Back have keys. Add focus rings, arrow-key movement in grids and shelves, Enter to open, and Tab between sections. This is the biggest v1 gap. |
| 3.2 | **Sorting and filters in libraries** | P2 | M | Libraries are always sorted by name. Add sort (name, date added, release date, rating, runtime, random) and filters (played/unplayed, favorites, genre, year, resolution). Remember the choice per library. |
| 3.3 | **Favorites** | P2 | S | `UserData.IsFavorite` is in the model but unused. Add a heart toggle on item pages (`POST/DELETE /UserFavoriteItems/{id}`) and a Favorites shelf or filter. |
| 3.4 | **Cast and crew** | P2 | M | Item pages don't show people. Add a cast row (`People` field, person images) and person pages listing their items. |
| 3.5 | **Collections and playlists** | P2 | M | BoxSets and playlists route to the generic library grid. Give them their own pages (collection overview, playlist order, "play all"). |
| 3.6 | **Image placeholders** | P2 | S | `ImageBlurHashes` is parsed but unused. Draw the blurhash while the real image loads (§6 of the plan). |
| 3.7 | **Item page extras** | P3 | M | Trailers (local and `RemoteTrailers`), "More like this" (`/Items/{id}/Similar`), special features, studio and tags. |
| 3.8 | **Home customisation** | P3 | S | Respect the user's Home screen settings from the server (section order, hidden libraries), or offer our own toggles. |
| 3.9 | **Alphabet jump in long libraries** | P3 | S | A side bar of letters using `NameStartsWith`. |
| 3.10 | **Other media types** | P3 | L | Music, Live TV, books and photos are out of v1 scope. Music would be the first candidate, since mpv already plays audio. |

## 4. Accounts and settings

| # | Item | Pri | Effort | Notes |
|---|---|---|---|---|
| 4.1 | **Player preferences** | P2 | S | Settings only has Storage, Account and About. Add: preferred audio and subtitle languages (or follow the user's server-side preferences: `AudioLanguagePreference`, `SubtitleMode`), autoplay next episode, skip-intro behaviour, and "max streaming bitrate" (with 2.2.1). |
| 4.2 | **Subtitle appearance** | P3 | S | Font size, colour, background (mpv `sub-*` options) for text subtitles. |
| 4.3 | **Theme** | P3 | S | Dark only. Add light and follow-system, from GPUI Kit's theme system. |
| 4.4 | **Remember window size and position** | P3 | S | The window always opens centred at 1280×800. |
| 4.5 | **Multiple servers or users** | P3 | M | One saved server at a time. A server/user switcher is out of v1 scope but often requested. |
| 4.6 | **Minimum-version gate on real 10.11** | P1 | S | The version gate exists, but nothing has run against 10.11 yet (plan §4.4 and open question 1: does `POST /QuickConnect/Initiate` exist on 10.11?). Add a 10.11 container to `dev/compose.yaml` and run the smoke example against it. |

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
| 6.1 | **Tests** | P2 | M | Unit tests cover only small pieces: ticks, models, the client and system info in `jellyfin`, event translation in `player`, and track ids in the app. Add: JSON fixtures for the models (from the 12.2 and 10.11 servers), a mocked HTTP test for the playback report flow, and GPUI `TestAppContext` tests for navigation and the player page state. |
| 6.2 | **Integration test in CI** | P3 | M | Spin up the Jellyfin container in the Linux CI job and run `examples/smoke.rs` against it. |
| 6.3 | **Error messages** | P2 | S | Check the offline, wrong-password, server-starting, expired-token and libmpv-missing paths for clear wording and a retry where useful. Several were done in milestone 6; network loss during playback (2.2.2) isn't covered. |
| 6.4 | **Accessibility** | P3 | L | GPUI's screen-reader support is limited. At least ensure good contrast, keyboard reachability (3.1), and text that scales with the system setting. |
| 6.5 | **Localisation** | P3 | M | All strings are English and inline. Extract them before the UI grows much more. |
| 6.6 | **Performance pass** | P3 | M | Profile large libraries (10k+ items) for grid scrolling and image decode, and check memory over long sessions (the image cache has no size limit in memory). |

---

## Suggested order

1. **Release blockers (§1)** and **10.11 testing (4.6):** small items that make a v0.1.0 tag responsible.
2. **Keyboard navigation (3.1)** and **transcoding fallback (2.2.1):** the two biggest functional gaps for v1.
3. **Hardware testing (2.3.4, 2.3.5):** borrow or rent an NVIDIA/Intel machine; ask testers with Linux desktops.
4. **Skip intro, up-next card, chapters (2.1.1–2.1.3):** most noticeable everyday player features.
5. **SyncPlay (2.4)**, starting with the websocket, which remote control (2.2.5) also needs.
6. **Library sorting/filters, favorites, cast (3.2–3.4).**
7. **macOS (2.3.1, 2.3.2, 5.1)** once a Mac is available.
