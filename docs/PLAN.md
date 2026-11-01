# Tabbyfin: Foundation Plan

A native, cross-platform desktop Jellyfin client: the UI is built with [egui](https://github.com/emilk/egui) through [eframe](https://github.com/emilk/egui/tree/main/crates/eframe) and drawn with wgpu, and playback runs in mpv.

**Targets:** Linux, Windows, and macOS.
**Server:** developed and tested against Jellyfin **12.x** (12.2 at the time of writing). The minimum supported version is **10.11** (see §4.4).

---

## 1. Goals for v1

**In scope**
- Sign in to a server with a username and password, or with Quick Connect. The session is kept between launches. Several accounts (servers or users) can be saved and switched between.
- Browse libraries:
  - Home: Continue Watching, Next Up, and Latest.
  - Library grids.
  - Detail pages for movies, series, seasons, and episodes.
- Play with mpv. Direct play is the default because mpv decodes nearly everything. Server transcoding is only a fallback.
- Report playback to the server (start, progress, stop) so watched state and resume points stay in sync.
- Choose audio and subtitle tracks, including external subtitles from Jellyfin. Pick between versions when an item has more than one (12.0 added multiple versions for episodes).
- Navigate mostly by keyboard.

**Not in v1 (later)**
- Music, Live TV, books, and photos.
- Downloads and offline mode.
- Multiple servers at once, and admin features.
- SyncPlay was here too, but is now wanted: see `docs/ROADMAP.md` §2.4.
- Casting, and remote-controlling other sessions.

---

## 2. Tech stack

| Concern | Choice | Notes |
|---|---|---|
| UI | **egui / eframe 0.37** | Current release. Its text shaping (harfrust/skrifa) and on-demand system font fallback (`egui_system_font_provider`, on by default) cover CJK and other scripts without bundling fonts. |
| Renderer | **wgpu** (eframe's default), wgpu 30 | One native API per OS (DX12, Vulkan, Metal), so mpv's frames can be shown without a copy everywhere: egui-wgpu takes outside textures (`Renderer::register_native_texture`). |
| Backends | DX12 on Windows (forced, or wgpu might pick Vulkan), Vulkan on Linux with GL as fallback, Metal on macOS | Windows imports D3D11 textures into D3D12, Linux imports Vulkan memory. On wgpu's GL fallback only the software video path works. |
| Widgets | egui's own | First reach the same features as before the migration; the look comes later (`docs/ROADMAP.md`, R1–R6). Icons are `egui_material_icons` 0.9 (built for one egui version), with both its filled and outline fonts. |
| HTTP | `reqwest` (rustls) + `serde`/`serde_json` | Runs on our own tokio runtime on a background thread; egui never awaits (§6). |
| Player | **libmpv, embedded**, loaded at runtime | Video renders inside the egui window (§5, `docs/in-app-video.md`). |
| Persistence | `directories` + TOML config. `keyring` (v4) holds the access token. | `keyring` uses the OS keychain on all three platforms. Where no credential store exists (WSL, or Linux without a Secret Service daemon), the token goes in a file only the user can read (`<config>/tokens/`, 0600), and a warning is logged. The config is saved to a temporary file and renamed into place, so a crash mid-save can't truncate it; one that doesn't parse is kept as `config.toml.unreadable` before the defaults replace it. |
| Image cache | On-disk cache in the platform cache dir, keyed by `(item id, image type, tag, width)`; in memory, up to 256 MB of decoded RGBA images held as egui textures, least recently used dropped first | Jellyfin's `ImageTags` act as cache busters. The app decodes images itself, off the UI thread: egui's texture loaders have no budget, no queue that drops what scrolled past, and no blurhash placeholders. |
| Toasts | Our own small module | `egui-notify` is still on egui 0.36. |
| UI tests | `egui_kittest` 0.37 (a dev dependency) | Drives the pages headless through AccessKit, without a GPU when no snapshots are taken. |
| Errors / logs | `thiserror` in the library crates, `anyhow` in the app, plus `tracing` | |

**Pinning:** eframe, egui and egui-wgpu are released together and pinned in the workspace; bump them together, along with `egui_material_icons`. wgpu comes in through eframe, but the app calls its types directly (video import, textures), so its version is named in `Cargo.toml` too.

---

## 3. Workspace layout

```
tabbyfin/
├── Cargo.toml                # workspace; pins the UI stack
├── crates/
│   ├── jellyfin/             # Pure API client: no UI, no mpv
│   │   ├── client.rs         #   base URL, auth header, request helpers, error mapping
│   │   ├── auth.rs           #   AuthenticateByName, Quick Connect, logout
│   │   ├── items.rs          #   views, items, resume, next up, latest, seasons/episodes
│   │   ├── playback.rs       #   PlaybackInfo, stream/subtitle URLs, session reporting
│   │   ├── images.rs         #   image URL builder
│   │   ├── ticks.rs          #   Ticks newtype (1 tick = 100 ns) <-> Duration
│   │   └── models.rs         #   hand-written DTOs, only the fields we use
│   ├── player/               # mpv via libmpv: no UI
│   │   ├── lib.rs            #   Player: load, set_property, command, add_subtitle, quit
│   │   ├── events.rs         #   PlayerEvent enum and event translation
│   │   └── libmpv/           #   runtime-loaded libmpv: core, event thread, video output
│   └── app/                  # the eframe binary, `tabbyfin` (§3.1)
└── docs/
```

**Why this split:** `jellyfin` and `player` can be built and tested on their own. For example, `cargo run -p jellyfin --example smoke` can run against a real server. That keeps UI changes confined to `app`.

### 3.1 The app crate

```
crates/app/src/
├── main.rs          logging (unchanged), player host check, wgpu setup, eframe::run_native; declares every module
├── app.rs           eframe::App: screens (restoring, sign-in, signed in), account switching; owns Services and the toasts
├── services.rs      Services: everything per signed-in session, AppEvent, Services::tick
├── task.rs          Task<T>: a tokio future whose result the UI polls; channel: repaint-on-send; Deadline: a timer
├── runtime.rs       the tokio runtime, plus `start` returning the JoinHandle
├── keys.rs          every key binding, and who gets a key: egui or the app
├── toasts.rs        notifications
├── nav.rs           Route, NavRequest
├── debug.rs         TABBYFIN_KEYS replay, TABBYFIN_ROUTE parsing (compiled in debug builds only)
├── browse.rs        the keyboard cursor and its egui wiring
├── smooth_scroll.rs the shelves' glide, and the wheel's instant step with animations off
├── motion.rs        `main`'s animations: page entry, the shelves' stagger, image fades, toggle pops, the player's prompt and controls
├── config.rs, credentials.rs, session.rs, update.rs, markers.rs
├── item_menu.rs, tracks.rs   pure logic (marks, episode lookup, track choices) and the playlist edit queue
├── playback.rs, syncplay.rs, remote.rs, live.rs   services
├── images.rs, trickplay.rs   textures
├── theme.rs         appearance: theme, text size, zoom, animations, over_video; `theme::apply` is called at launch
├── window.rs        the window as it opens (title, icon, app id, sizes) and its saved placement
├── ui_tests/        the headless UI tests (egui_kittest)
├── widgets/         text (Load<T>), card, image, shelf, menu, popup, select, dialog, button, state, mascot, video
└── views/           mod (Page, PageCx, open), shell, home, login, search, library, value_filters, item, settings, player, dialogs
```

**Models:** we write the serde DTOs by hand and keep them to the fields we need. Every field is `Option` or `#[serde(default)]`, and unknown fields are ignored. Generating code from the full OpenAPI spec would create a huge, churny surface. Swashbuckle v10 in 12.0 already changed the generated document once. If we later want generated code, we generate it from the 12.x spec.

---

## 4. Jellyfin integration

### 4.1 Authentication (changed in 12.0, important)

Jellyfin 12.0 **disables legacy authorization by default**, and existing installs are migrated to it too (PR #15559). The plan is to remove the old methods completely in a later release. Only two methods are accepted:

- The `Authorization` header with the `MediaBrowser` scheme:
  ```
  Authorization: MediaBrowser Client="tabbyfin", Device="<hostname>", DeviceId="<stable uuid>", Version="<crate version>", Token="<token>"
  ```
- The `ApiKey` query parameter, for cases where we can't set a header.

**Never use any of these:**
- The `X-Emby-Authorization`, `X-Emby-Token`, or `X-MediaBrowser-Token` headers.
- The `api_key` query parameter. It's lowercase and legacy, and many older client examples still use it.
- The `Emby` scheme.

We generate `DeviceId` once and save it to the config. Jellyfin uses it to identify the session.

The `/emby/*` and `/mediabrowser/*` route prefixes were also **removed** in 12.0. We only use bare paths.

### 4.2 Endpoints for v1

All of these exist in the 12.2 OpenAPI spec and none are marked obsolete (checked against `jellyfin-openapi-stable.json`, version 12.2.0).

| Purpose | Endpoint |
|---|---|
| Probe the server | `GET /System/Info/Public` |
| Log in | `POST /Users/AuthenticateByName` `{Username, Pw}` returns `AccessToken` and `User` |
| Quick Connect | `GET /QuickConnect/Enabled`, then **`POST`** `/QuickConnect/Initiate` (the `GET` form was **removed in 12.0**), then poll `GET /QuickConnect/Connect?Secret=`, then `POST /Users/AuthenticateWithQuickConnect` |
| Log out | `POST /Sessions/Logout` |
| Current user | `GET /Users/Me` |
| Libraries | `GET /UserViews` |
| Library contents | `GET /Items?ParentId=&IncludeItemTypes=&Recursive=&SortBy=&StartIndex=&Limit=&Fields=` |
| Continue watching | `GET /UserItems/Resume` |
| Next up | `GET /Shows/NextUp` |
| Latest | `GET /Items/Latest?ParentId=` |
| Item detail | `GET /Items/{itemId}` |
| Seasons / episodes | `GET /Shows/{seriesId}/Seasons`, `GET /Shows/{seriesId}/Episodes?SeasonId=` |
| Images | `GET /Items/{itemId}/Images/{imageType}?tag=&fillWidth=&quality=` |
| Playback info | `POST /Items/{itemId}/PlaybackInfo`, sending a DeviceProfile that allows direct play of everything, plus an HLS H.264/AAC transcoding profile for when the server won't direct play |
| Stream | `GET /Videos/{itemId}/stream?static=true&MediaSourceId=` (mpv opens this URL) |
| Transcode | The source's `TranscodingUrl` (`/videos/{itemId}/master.m3u8?…`), without its `ApiKey`: the `Authorization` header covers the playlists and segments. `DELETE /Videos/ActiveEncodings?deviceId=&playSessionId=` after the stop report |
| External subtitles | `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{index}/Stream.{format}` |
| Reporting | `POST /Sessions/Playing`, `/Sessions/Playing/Progress`, `/Sessions/Playing/Stopped` |
| Watched state | `POST` / `DELETE /UserPlayedItems/{itemId}` |
| Session websocket | `GET /socket` upgraded to a websocket, with the same `Authorization` header (works on 10.11 and 12.2). Answer `ForceKeepAlive` with a `KeepAlive` every half interval; reconnect with backoff. `POST /Sessions/Capabilities/Full` after each connect. Pushes `UserDataChanged`, `SyncPlay*` and remote-control messages |
| Remote control | Capabilities with `SupportsMediaControl` and the general commands we handle in `SupportedCommands`. The server then forwards `Play` (item ids, already expanded from series and seasons; `PlayNow`, `PlayNext`, `PlayLast`, `PlayShuffle`), `Playstate` (pause, seek, stop, next…) and `GeneralCommand` (volume, mute, tracks, fullscreen, `DisplayMessage`; every argument a string) messages. Unadvertised general commands still arrive and are ignored. `remote.rs` in both crates |
| Server clock | `GET /GetUtcTime`: request reception and response transmission times, for an NTP-style offset (`ServerClock`) |

We use the user-scoped routes without a userId, such as `/UserViews` and `/Items`. The old `/Users/{userId}/Items` and `/Users/{userId}/Views` routes are **not in the 12.x spec**.

### 4.3 Other 12.x behavior changes to handle

- **Playlist entries (10.11 and 12.2):** an entry's `PlaylistItemId` is the item's own id, and 12.2 adds an item already in a playlist a second time (10.11 skips it), so the copies can't be removed separately. *Add to playlist* checks for the item first. Other users' shared playlists report `CanDelete: true` but answer 403 to added items and 404 to renames. `POST /Playlists` without `IsPublic` makes the playlist visible to every user, so the app sends `false`; *Edit* shares it.
- **Resolution filters (10.11 and 12.2):** `GET /Items` with `is4K=false` matches nothing on either version, and 12.2's `isHd=true` also returns the hidden items of a movie's alternate versions. The library filters use `minWidth` / `maxWidth` instead, which 12.2 applies to an item's widest version and 10.11 to its first. Found while building the library filters.
- **Direct play needs subtitle profiles (12.2):** a `PlaybackInfo` device profile without `SubtitleProfiles` gets `SupportsDirectPlay: false` on every source in 12.2 (10.11 still allows direct play). The app's profile always lists them. Found while capturing test fixtures.

Source: the 12.0 release notes, plus 12.1 and 12.2, which only contained bug fixes.

| Change | What we do |
|---|---|
| `GET /Items` now applies `Recursive` on its own when filters are combined with `IncludeItemTypes`. The same query can return different results than on 10.11. | Always pass `Recursive` explicitly so behavior doesn't depend on the server version. |
| `PlaybackPositionTicks` was **removed from `MediaSourceInfo`**. | Read the resume position from `item.UserData.PlaybackPositionTicks`. |
| Episodes can have several versions now, not only movies. | Treat `MediaSources.len() > 1` as normal for any video, and show a version picker. |
| Image endpoints **no longer upscale** beyond the source resolution. | Don't assume we get the size we asked for. Scale on the client and keep the aspect ratio. |
| Name sorting now uses `SortName` / `CleanName`. | Don't re-sort on the client. Trust the server order. |
| Endpoints missing from the OpenAPI spec, or marked obsolete there, may be removed in any major release. | Only use endpoints that are in the spec and not obsolete. Re-check this list against the spec on every major Jellyfin release. |
| `UserDto.HasPassword` is obsolete. | Don't use it to decide whether to show a password field. Always show the field. |
| While starting up, `/System/Info/Public` answers with plain text ("Jellyfin Server is loading") or a partial object. Seen on the dev server. | Show a "server is starting" state rather than "not a Jellyfin server", and retry. |

**Checked on the dev server (12.2):**
- `api_key` and `X-Emby-Token` return 401. `ApiKey` and the `MediaBrowser` header work.
- `GET /QuickConnect/Initiate` returns 405, while `POST` works.
- The static stream endpoint also answers without auth. We still always send the header.

### 4.4 Minimum server version: 10.11

- Every endpoint in §4.2 exists in 10.11, including `POST /QuickConnect/Initiate` (checked against 10.11.11).
- 10.11 introduced the `EnableLegacyAuthorization` switch. So 10.11 is the oldest version where we can turn off legacy auth on a test server and prove we don't depend on it.
- The 12.0 release notes support direct upgrades from 10.10.7 and 10.11.x, so 10.10 users have an easy path forward.
- We read `Version` from `/System/Info/Public` at connect time. Servers older than 10.11 get a clear "please upgrade" message instead of failing in odd ways.

### 4.5 Units and paging

- Positions are measured in **ticks**, where 1 tick = 100 ns. One `Ticks` newtype handles conversion to and from `Duration`, so units are never mixed by hand.
- Library grids load page by page with `StartIndex`/`Limit` as the user scrolls, drawing only the rows on screen (`ScrollArea::show_rows`).

---

## 5. Playback architecture

Video plays **inside the app** through libmpv. There's no separate mpv window and no fallback to one; that was removed on 2026-10-14. See `docs/in-app-video.md` for the rendering plan, which moves from software rendering to GPU texture sharing.

### 5.1 libmpv embedding

- **Loading:**
  - `player::libmpv` loads libmpv at runtime (`dlopen`), so building needs no libmpv.
  - When libmpv is missing, playback shows an error saying how to install it.
- **Threads:**
  - mpv runs on its own threads.
  - An event thread turns libmpv events into `PlayerEvent`s.
  - A render thread draws frames for the UI.
- **Isolation from the user's mpv setup:** the embedded mpv never loads `mpv.conf`, scripts or key bindings (`config=no`, `load-scripts=no`, `osc=no`, `input-default-bindings=no`). The app draws its own controls, and user scripts such as uosc would otherwise draw theirs into the video.
- **Loading a file:** every option is set as a property, because the argument list of `loadfile` changed in mpv 0.38.
  1. `http-header-fields` carries the `Authorization` header, so the stream URL contains no token.
  2. Then `force-media-title` and `start`, followed by `loadfile`.
  3. After `file-loaded`, `sub-add` for each external subtitle. These requests are authenticated by the same header.
- **Observed properties:** `time-pos` (throttled to about once a second), `duration`, `pause`, `volume` and `mute`.
- **Hardware decoding:** `hwdec=auto-safe` on the GPU video paths, so frames stay on the GPU where mpv has an interop for the render context (`d3d11va` through ANGLE on Windows, `nvdec` anywhere), and `auto-copy-safe` on the software path. The log says which decoder each file got (`video decoding: …`).
- **mpv's log** is forwarded to ours at `warn` level. `TABBYFIN_MPV_LOG=v` (or any mpv level) shows more.

### 5.1b The player host process

libmpv doesn't run inside the app. `Player::spawn` starts this same executable again as `tabbyfin --player-host` (`crates/player/src/host.rs`). That process runs libmpv and the video output described above, and the app talks to it over its stdin and stdout (`remote.rs`, `wire.rs`). An mpv or GPU driver crash then only kills the player process.

- **Messages:** length-prefixed JSON headers with an optional binary payload.
  - App to host: first `Start` with what the app's renderer can draw (`VideoImport`: pixels only, Vulkan memory for a device UUID, or D3D11 textures for a DX12 adapter LUID), which picks the host's video output; then mpv commands, properties (each answered with a reply) and the on-screen size.
  - Host to app: player events, replies and frames.
- **Frames:**
  - **Software:** the RGBA pixels (opaque, tightly packed) go as the payload. Only the newest frame is sent, so a slow app skips frames instead of queueing them.
  - **Windows:** only the D3D11 texture's id and the name of its NT handle are sent; the app opens it by name on its D3D12 device. The host keeps the handles of the last 6 frames it sent, so a texture's name still opens after a resize has retired its ring.
  - **Linux:** the Vulkan memory fd of each image is passed once over a socket pair (`SCM_RIGHTS`; the host inherits its end as `TABBYFIN_PLAYER_FD`). The app keeps the newest 8 for frames to come, and each frame holds its image's fd (`Arc<OwnedFd>`) until the UI has imported it, so it can't be closed (or its number reused) under an import.
- **Logging:** the host logs plain lines to stderr. The app forwards them into its own log under the `player` target.
- **Crashes:** when the host dies without having finished, the app gets `EndFile(Error(HOST_CRASHED))` and `Exited`. The session ends as *lost*, the manager starts a new session (and host) at the same position, and the user sees "The player stopped unexpectedly. Restarting it…". After three tries the error says the player keeps crashing.
- **Lifetime:** the host exits when its stdin closes, which happens when the app drops the player or exits. It's also killed when the app drops it.
- **`TABBYFIN_PLAYER=embedded`** runs libmpv inside the app as before, for debugging.

Tested: software video on Linux and ANGLE/D3D11 with d3d11va on Windows (AMD) through the host, with GPU frames on both and the WGL path on Windows. Killing the host mid-play on both resumes playback in a new one. Linux GPU frames through the host are confirmed on WSL's llvmpipe (forced GPU path), in-process too.

### 5.2 Threading and reporting

1. The session runs on tokio. It forwards `PlayerEvent`s into `NowPlaying` — the state the player page draws and controls (`services.playback`) — and it receives `UiCommand`s from that page: pause, seek, volume, tracks and stop.
   - After playback starts, it also looks up the item's versions and, for episodes, the next episode.
   - **Switching version** starts a new session of the same item at the current position.
   - **Next episode** (button, Shift+N, or automatically when an episode plays to the end) starts a session for the next episode from its resume point.
   - **Chapters and segments** (`markers.rs`) are fetched with the details: chapters from the item (`Fields=Chapters`), segments from `GET /MediaSegments/{id}`.
     - The server only returns segments from provider plugins (e.g. Intro Skipper) that are enabled for the library. Without any, segments are guessed from chapter names: Intro/Opening/OP, Recap/Previously, Preview/Next Episode, Credits/Ending/ED/Outro/End Credits.
     - A "Skip intro" (recap, preview, ad) button shows during those segments.
     - From the Outro segment, or 20 s before the end without one, an up-next card offers the next episode with *Play now* and *Cancel*. Cancel (or Esc) means the session ends at the end of the file instead of autoplaying. *Settings → Playback* turns autoplay off altogether.
     - PgUp/PgDn seek to chapter starts from our own list, so PgDn in the last chapter does nothing (mpv's `add chapter 1` would end the file).
   - Sessions are numbered, so when one replaces another, the old one's end doesn't close the new player.
   - The volume carries over between sessions and is saved in the config.
   - **Network drops:** mpv's HTTP input reconnects by itself (`stream-lavf-o=reconnect=1,…`), which covers a server restart. If the stream still fails, or mpv hits EOF more than 10 s before the duration, the session ends as *lost* and the manager starts it again from that position after 2, 5 and 10 s ("Reconnecting…"), keeping the player up. Esc during the wait cancels it. Errors in the last 10 s aren't retried.
   - **Transcoding:** the server transcodes only when it won't direct play: above the *Settings → Playback → Max streaming bitrate* (sent as `MaxStreamingBitrate`), or when the user's policy forbids it.
     - The session then plays the HLS `TranscodingUrl` and reports `PlayMethod: Transcode`, and the player shows a "Transcoding" label.
     - The transcoding profile asks for H.264 in MPEG-TS segments, or with *Ask for HEVC* for `hevc,h264` in fragmented MP4 (HEVC in HLS needs it); servers that don't allow HEVC encoding fall back to H.264.
     - With the *Automatic* bitrate, the cap comes from timing `/Playback/BitrateTest` downloads before playing (80% of the measurement, none above 200 Mbit/s, reused for 10 minutes; on failure the fixed setting).
     - Subtitles the server delivers as files (`DeliveryMethod: External`, embedded text ones included) are added to mpv; image subtitles are burned in (`Encode`).
     - A transcode carries only the chosen audio track, so changing audio, or changing to or from a burned-in subtitle, restarts the session at the current position. The server picks the default audio by the user's language preference and names it in the URL (`AudioStreamIndex`), which the picker follows.
2. It reports to the server:
   - `/Sessions/Playing` on start.
   - `/Sessions/Playing/Progress` about every 10 s, plus immediately on pause, seek or track change.
   - `/Sessions/Playing/Stopped` when playback ends, using the last known position.
3. When playback ends, pages that show watched state refresh: the stop bumps `services.user_data`, the counter pages watch (§6).

**Ownership and events:** each session sends its updates through a `task::channel` that `Services::tick` drains into `NowPlaying`, and handles the end there (autoplay, reconnect, the events below). A session is detached on tokio rather than a `Task`, so it outlives the screen that started it and still reports its stop after sign-out: `Services::end` stops the current one, dropping the manager stops any left and saves the volume, and quitting (`Services::shut_down`) waits up to 3 s for the stop reports. `AppEvent::PlaybackStarted` and `PlaybackEnded` open and close the player page; failures and notices are toasts. Reconnecting after `Ended::Lost` waits on a `task::Deadline` (2, 5, 10 s), cancelled by stopping playback.

### 5.3 Shipping libmpv

| Platform | libmpv |
|---|---|
| Linux | The system's `libmpv2` for the `.deb` (which depends on it, on `libvulkan1`, and on the Wayland, xkbcommon and X11 libraries winit loads at runtime) and the tarball. The AppImage bundles it with the libraries it needs (`packaging/appimage.sh`, with linuxdeploy pinned in `packaging/linuxdeploy.lock`), leaving out what has to match the system (glibc, GL, X11/Wayland, the Vulkan loader) and forcing in `libjack` and `libusb`, which linuxdeploy's exclude list skips but plenty of desktops lack. The app looks for libmpv in `../lib` next to its executable (the AppImage's `usr/lib`), any version, before the system's. The AppImage is built in an Ubuntu 22.04 container (`packaging/appimage-docker.sh`, `mise run package:appimage`, a CI job of its own), so it needs glibc 2.35 and carries that release's mpv 0.34 (`libmpv.so.1`). The Flatpak (`packaging/flatpak/`, app id `io.github.lgpy.tabbyfin`, freedesktop 26.08 runtime) builds mpv 0.41 itself, with libass and libplacebo, on the runtime's FFmpeg; the runtime's codecs-extra extension supplies H.264, HEVC and the rest. Its Rust crates are listed in `packaging/flatpak/cargo-sources.json` because the build is offline: regenerate it with `mise run flatpak:sources` whenever `Cargo.lock` changes. It builds with the SDK's Rust (1.98 for 26.08), so the code must not need anything newer. |
| Windows | `libmpv-2.dll` next to `tabbyfin.exe` in the zip. It's a shinchiro build, pinned with its checksum in `packaging/mpv.lock` and fetched by `packaging/fetch-libmpv.sh`. The zip also carries ANGLE (`libEGL.dll`, `libGLESv2.dll`) and its MinGW runtime DLLs, MSYS2 builds pinned in `packaging/angle.lock` and fetched by `packaging/fetch-angle.sh`, for zero-copy video. |
| macOS | Not bundled yet. The app looks in Homebrew's folders, so `brew install mpv` makes playback work. Bundling `libmpv.dylib` is still to do. The app is ad-hoc signed, not notarized. |

**App icon:** the mascot traced from the drawing, as a vector: `packaging/tabbyfin.svg` is the source (the rest pose from `packaging/mascot/jelly-logo.svg`, cropped square), and the PNGs and the `.ico` are rendered from it. `packaging/tabbyfin.png` (512×512) is what's shipped, and `packaging/tabbyfin-artwork.png` is a 1024×1024 render for anywhere that wants it bigger. The top bar shows `crates/app/assets/logo.png`, a 128×128 render embedded in the binary (the image cache uploads one mip level, so the 512 px one would alias at 40 px). Linux installs the PNG as the 512×512 hicolor icon (the `.deb`, the AppImage, the Flatpak; the tarball carries it). Windows uses `packaging/tabbyfin.ico`, embedded in `tabbyfin.exe` as icon resource 1 by `crates/app/build.rs` (Explorer uses it; the window itself gets `packaging/tabbyfin.png` through eframe, and the installer's shortcuts use the `.ico` file). macOS gets `tabbyfin.icns`, made from the PNG with `sips` and `iconutil` when `package.sh` builds the bundle. After changing the SVG, render them again with resvg (`cargo install resvg`), each `.ico` size drawn at its own size so the small ones stay sharp:

```sh
resvg -w 512 -h 512 packaging/tabbyfin.svg packaging/tabbyfin.png
resvg -w 1024 -h 1024 packaging/tabbyfin.svg packaging/tabbyfin-artwork.png
resvg -w 128 -h 128 packaging/tabbyfin.svg crates/app/assets/logo.png
for z in 256 128 64 48 32 16; do resvg -w $z -h $z packaging/tabbyfin.svg /tmp/icon-$z.png; done
convert /tmp/icon-{256,128,64,48,32,16}.png packaging/tabbyfin.ico
```

**Pinned downloads** (`packaging/mpv.lock`, `angle.lock`, `linuxdeploy.lock`) go through `packaging/download.sh`, which tries our copy first (the repo's `deps` release, one asset per file, named by its SHA256; fetched with `gh` and CI's token while the repo is private), then the original URL, and checks the checksum either way. After changing a lock file, `mise run deps:mirror` uploads the new file. The Flatpak's sources (libass, libplacebo, mpv, crates) aren't mirrored: Flathub-style builds fetch them from their projects.

`packaging/THIRD_PARTY.md` holds the licenses and source information of the bundled third-party software (mpv, ANGLE, and the egui stack's fonts and icons), and ships in the packages.

**Builds:** `.github/workflows/ci.yml` runs clippy and the tests on all three OSes. On a `v*` tag, or when started by hand, it also packages all four targets:
- Linux `.tar.gz` and `.deb`, the AppImage (built in Ubuntu 22.04), and a Flatpak bundle (`packaging/flatpak/build.sh`)
- Windows `.zip`, and an installer from the same files (`packaging/tabbyfin.iss`, Inno Setup 6, built by `packaging/installer.sh`): per-user install without admin rights, Start-menu and optional desktop shortcuts, an uninstaller, upgrades in place
- macOS arm64 and x86_64 `.zip`

A tag also creates a draft GitHub release with these files. Locally, `mise run package` packages Linux (`package:appimage` and `package:flatpak` build those two), and `mise run ci:build` builds every OS on CI.

---

### 5.4 SyncPlay

`syncplay.rs` holds the group this session is in. The protocol is in `docs/ROADMAP.md` §2.4; the flow here:

1. **Joining:** the SyncPlay panel in the top bar lists groups (`/SyncPlay/List`), creates one or joins one. The server confirms with a `GroupJoined` update on the websocket. While in a group, the server clock is sampled (4 samples, then one a minute) and the ping is reported. Requests go out one at a time, in the order they're made, so a `Ready` never overtakes its `Buffering`.
2. **Starting something** while in a group (Play on an item page, next episode…) doesn't play locally: `Services::play` hands it to SyncPlay, which sends `SetNewQueue` (the item, plus the next episode for episodes).
3. **Queue updates:** when the queue's current entry changes, every member (this one too) loads it paused at the group's position (`PlayOptions::start_paused`, `group_item`), reports `Buffering`, then `Ready` after mpv's first `playback-restart`.
4. **Commands** arrive with a `When` in server time. The app waits for it on tokio, not on the UI's frames (which can be 16 ms apart, more on a slow renderer), and sends the command straight to the player from there:
   - `Unpause`: seek to the group's position if more than 0.4 s off, wait until `When`, then unpause. If it arrived late, seek ahead by the lateness and unpause now.
   - `Pause`: at `When`, pause and seek to the group's position.
   - `Seek`: pause, seek, report `Buffering`, and `Ready` after the restart; the server then unpauses everyone.
   - `Stop`: stop playback.
5. **Local controls** in a group session (`Services::control`): pause, seek and stop become `/SyncPlay/Pause`, `Unpause`, `Seek` and `Stop` requests, so they apply to everyone. Esc stops playback for the group, as in the web client. Volume and tracks stay local.
6. **End of an item:** each member sends `NextItem`; the server advances the queue once, and the next item arrives as a queue update. Local autoplay is off in a group.
7. **Drift correction:** after an Unpause, the group's timeline is "position P at local instant T". Once a second, the local position (mpv's last report plus the time since, at the current speed) is compared with it:
   - Under 80 ms off: normal speed.
   - Up to 1.5 s: play up to 5% faster or slower (0.25 speed change per second of drift); mpv keeps the pitch.
   - More: seek to the group's position.
   - In WSL with software rendering, a member drifts up to about 0.4 s while playback warms up, is back within 0.1 s after about 8 s, and stays within ±70 ms after that.

8. **Local reloads:** another version, or a transcode restarted for other tracks, reloads the item here only (`NowPlaying::reload` → `SyncPlay::reload`): report `Buffering` (the server pauses everyone and keeps the group's position), load paused as the same queue entry, report `Ready` once loaded (the server resumes everyone, or first has this member seek if it landed elsewhere). The old session stops being watched first, or it would answer `Ready` at once.
9. **Leaving:** signing out, switching accounts and quitting leave the group first.
10. **The queue:** the SyncPlay menu lists the group's queue (names fetched by id) with the playing entry marked: `SetPlaylistItem` plays an entry, `MovePlaylistItem` and `RemoveFromPlaylist` edit it, `SetShuffleMode` and `SetRepeatMode` set the modes. The changes come back as queue updates to everyone.

## 6. App architecture

### State: one owner, no entities

- `App` (`app.rs`) is the `eframe::App`. It owns the screens (`Restoring`, `SignIn(Box<Login>)`, `SignedIn(Box<SignedIn>)`), the toasts, the window's `Placement`, the zoom, and the keys `raw_input_hook` took from egui for the app.
- While signed in, `SignedIn` holds the session's `Services` and the `Shell`. Pages get a `&mut PageCx` each frame: `ctx`, `services`, a `nav: Vec<NavRequest>` to push into, and `render_state` (the player's video needs the wgpu device; `None` in tests). There are no globals and no `Rc<RefCell<…>>`.
- `Services` (`services.rs`) is everything per signed-in session: the session (client, user, server), `user_data`, `events: Vec<AppEvent>`, `images`, `playback`, `syncplay`, `live`, `remote`, the item page's playlist edit queue, and the marks and fetches that outlive a frame (a watched change, a series' next episode, a shuffle). `Services::new(session, ctx)` takes the `egui::Context`, which the services keep to wake the UI. `Services::tick(ctx)` runs each part's background work: images, the item menu's finished requests, playlist edits, then playback's, the websocket's, remote control's and SyncPlay's.
- Cross-service calls that would otherwise be globals are methods on `Services` (`control(UiCommand)`, `play(&item, options)`, `play_all_with`, `enqueue`, `reload`, `mark`, `play_up_next`); each part's module adds its own `impl Services` block, so `services.rs` stays small.
- `AppEvent`s are what the app or the shell must act on: `Toast`, `AppearanceChanged`, `SessionExpired`, `SignOut`, `SwitchAccount`, `AddAccount`, `PlaybackStarted`, `PlaybackEnded`, `ToggleFullscreen`. The app drains them after the shell has drawn.
- Adding an account parks the signed-in screen in `App::parked`: it keeps ticking (playback, the websocket), its toasts still show, and its screen-changing events (`SessionExpired`, `SignOut`, `SwitchAccount`, `AddAccount`, `ToggleFullscreen`) are dropped, while anything else waits in its queue for the shell after *Back to your account*.
- Ending a session (sign-out, expiry, switching, removing the parked account) calls `Services::end()` (stops playback, leaves a SyncPlay group) and then drops the screen. Quitting saves the window placement and calls `Services::shut_down` on the signed-in session and a parked one, waiting up to 3 s.
- `user_data` counts changes to the user's watch data: bumped when a session ends, when the server says it changed elsewhere (coalesced, half a second later, and not during playback), and by the app's own edits.

### Pages and navigation

- A page is a struct implementing `views::Page`: `title()`, `scope()` (its key bindings), `on_key(action, cx) -> bool`, `on_text` (the library's letter jump), the search hooks (`set_search_term`, `enter_results`, `hide_cursor`), and `show(ui, cx)`.
- The shell keeps a stack of `Box<dyn Page>` and each page's `Route` next to it (`StackEntry`), so Settings' toggle and the results page are found by route rather than by downcasting. Pages stay alive while stacked, so Back keeps their state. The top one is drawn in a `CentralPanel` under the top bar; both panels are frameless over the backdrop's gradient (`backdrop::fill`), and a page scrolled under the bar marks its top edge with `backdrop::scroll_edge`'s hairline.
- `views::open(route, cx)` makes the page for a route. Pages navigate by pushing `NavRequest::Push(route)` / `Back` into `cx.nav`; the shell applies them after drawing and requests a repaint, so the new page shows at once.
- Loading (see `views/home.rs`): a page keeps a `Load<T>` (`widgets/text.rs`), an `Option<Task<…>>` and `loaded_at: Option<u64>`. `show` starts a load when `loaded_at != Some(services.user_data)` — the first load, Retry (`None`) and watch data changing elsewhere — then polls the task; what's on screen stays until new data arrives. A page under others doesn't reload in the background: it catches up when it's back on top. Owners keep a generation counter (library pages, playback sessions) and drop results for an older one.
- While a page loads it shows the mascot (`widgets/mascot.rs`): `main`'s jellycat, every outline tessellated once with `lyon_tessellation` into an `egui::Mesh` and posed per frame (egui fills convex paths only, and these are concave).
- The sign-in form (`views/login.rs`) is one panel under the mascot (`mascot::watching`), which reacts to the form through a `Watch` filled in as the fields draw: it looks toward the focused field's caret (its eyes move, its mouth and cheeks half as far), shuts its eyes while a hidden password has the keyboard, and wears the confused face while an error shows. Its place is allocated before the panel and it's painted after, so it follows this frame's caret; it floats a little above the panel with no shadow.
- **The cozy look** is where the app's look is heading, one screen at a time; the sign-in and restoring screens are the first. Its colours are `theme::Tokens` (the backdrop's gradient and glow, `surface`, `inset`, `ink`, `brand`, the avatar pastels), and since C1 they colour the whole app (since C2 the backdrop's gradient is under the whole shell too): `theme::visuals` is built from them, the classic token names are aliases of them, and only the player keeps the classic dark set (`theme::over_video::TOKENS`). Its pieces are shared widgets so the next screens reuse them: `widgets/backdrop.rs` (the window's gradient and the glow behind the mascot, as meshes), `surface.rs` (the raised panel and its centred heading), `field.rs` (a labelled field, with an optional button inside its right end), `button.rs`'s `brand`, `quiet` and `subtle`, `avatar.rs` (a name's initial on a pastel picked by the name), `avatar_row.rs` (avatars, title, subtitle and an optional ✕ or `brand` action, one row; and `person`, a member of a list) and `round_button.rs` (a round icon or avatar button with a hover circle, the top bar's and the SyncPlay panel's). The form checks focus by the focused widget's id, since `Response::has_focus` is false while the window lacks the OS's focus.
- Scrolling feels as on `main` (`smooth_scroll.rs`): egui already eases a wheel step over frames and leaves trackpad deltas alone; with the animations setting off, `App::raw_input_hook` splits a wheel step into the deltas egui applies at once. A shelf's arrows glide a page over (`smooth_scroll::Glide`, `main`'s `glide_by`) and its drag follows the pointer 1:1 and stops with it, taking no wheel, as `main`'s rail drag did.

- Settings (`views/settings.rs`) is a menu beside a scrolling column of every section. The menu has a search box and then the sections in two groups, by where their settings are kept: *This device* (the config file: Appearance, Playback, Subtitles, Storage, About) and *Jellyfin server* (the signed-in user's `UserConfiguration`, which the web client and other apps share: Account, Languages, Home). Each store's sections are under a banner naming it (the server's names the user and server, and says while a save is out). A menu entry clears the search and scrolls its section's start to the top (`ui.scroll_to_rect`, eased like any scroll), with room left under the last section for it to get there too; the menu marks the section at the top of the column, or the one an entry asked for until the scroll arrives (at most a second). The search filters every row by its store, section, label, description and key, and shows the sections with matches under their banners, their match counts in the menu; Esc in the box clears it. The rows are `Row` consts listed in `ROWS`, which the search reads, so a new row goes in both.
- The server's sections read `/Users/Me` and `/Localization/Cultures` when the page opens. A change shows at once and is saved by `Client::update_user_configuration`, which reads the whole configuration, changes only the given fields and posts it back (`POST /Users/Configuration`, 10.9+: the server replaces it wholesale, so fields the client doesn't model must survive). One save is out at a time, later changes merged behind it; a failed save toasts and reloads what the server has. *Leave watched items out of Latest* bumps `user_data` once saved, so Home reloads.

### Top bar

- The bar is the app icon, Back while there's a page to go back to, the top page's title, the search box in the middle, and on the right SyncPlay (its panel), Settings (opens Settings, or closes it when it's on top) and the account menu. The buttons are `widgets::round_button::RoundButton`s (C7): a muted icon with no frame, an `inset_hover` circle under the pointer, and SyncPlay on (circle kept, icon in `brand`) while in a group; the account button is the user's avatar (`avatar::paint`). The search box is always open: sign-in's field (`base` text, 12 × 9 margin, `inset` fill) with the search icon inside its left end. Its width shrinks with the bar (at least 120 px) so the icon, Back and a title fit on its left and the buttons on its right at any window size or zoom. While the field has text, a round clear (×) button at its right end (R7, `field::trailing_button`) clears the text, closes the results as clearing by keyboard does, and keeps the keyboard in the box; it skips focus (`Focus::Skip`) and has an AccessKit name, so Tab still belongs to the field.
- **Search:** the box is the bar's only text field. Typing opens the results page (or comes back to it from a page opened from it); clearing the box closes them; Enter or Down moves into them; Up or Shift+Tab the results leave unused at their first row go back into the box. Esc gives the keyboard back to the page (over the results it closes them and clears the box).
- The SyncPlay panel hangs under its button in the menus' frame, placement and opening motion (`popup::panel`), 340 wide with 16 points of room, and is laid out like a small surface (C7). Without a group: a *Watch together* heading, the server's groups as `AvatarRow`s (their members' avatars overlapping, the name, who's in it and the state, a `brand` *Join* at the right end; the whole row joins), then *New group*, which is `quiet` beside those rows and `brand` when there are none (the groups icon over "No one is watching together right now."). In one: the group's name with its state as a coloured dot (green playing, amber waiting, grey paused or idle), *Watching* with each member as an `avatar_row::person` (the user's marked "you"), then the queue in one inset block (the playing entry marked ▶ in `brand`, the others numbered, round shuffle and repeat toggles beside its label, and round move and remove buttons only on the row the keyboard or pointer is on) and *Leave group* as a `button::quiet_danger`. Everything in it is clicks only (`Focus::Skip`): the panel reads its own keys, and the keyboard's place is a ring (on a toggle or button) or the row's highlight. The account menu is a `PopupMenu` (`widgets/popup.rs`): the saved accounts with their server in muted text and the current one checked, then *Add account…* and *Sign out*.

### Async

- **The rule:** UI code never awaits `jellyfin` futures. Work runs on the tokio runtime (`runtime.rs`), and the UI polls for it.
- `Task<T>`: `Task::spawn(ctx, future)` runs the future on tokio and requests a repaint once its result is ready. `task.poll()` hands the result over once; a panic in the future is raised again on the UI thread. Dropping the task aborts the future. Owners keep `Option<Task<T>>`: replacing it cancels the previous one, and they set it back to `None` after `poll` returned the result.
- Long-lived streams (a playback session's updates, the websocket) are a `task::channel`: `Sender::send` requests a repaint after each send, `Receiver::drain` takes everything without waiting. `Services::tick` drains them.
- Timers (controls hiding, the search debounce, SyncPlay's drift checks, saving the window placement, reconnects) are a `task::Deadline`: `set(after)` starts or restarts it, `cancel()` stops it, `fire(ctx)` returns true once when it's due and otherwise asks for a repaint for the remaining time. No thread sleeps on the UI side. A deadline is only as punctual as the frames, so what must happen at an exact time (SyncPlay's scheduled commands) waits on tokio instead.
- `Services::tick` runs from `App::logic`, which eframe also calls while the window is hidden whenever a repaint was requested, so playback and the sign-in form's requests carry on while the window is minimised.
- Idle CPU stays near zero: nothing requests a repaint every frame except while something moves (video, an animation, a loader). An animation (`motion.rs`) asks for frames only while it runs: once a page's entry is over, a repaint for a hover or a key draws it at its end state and asks for no more. A loader is one of the moving things: it asks for a frame every 33 ms for as long as it's on screen, as `main`'s animation frames did (the spinner while a page loads, a season's episodes band, the player's loader, the restoring screen). The confused mascot over an empty state moves the same way, for as long as the page is up (it used to hold still after 8 s to keep an empty page idle, which looked like a bug).

### Keyboard

- **egui or the app, per frame.** egui moves focus by itself (Tab from nothing; the arrows once a widget has focus) and presses the focused widget with Space or Enter. On a cursor page that would fight the cursor, so `App::raw_input_hook` decides before egui sees the frame's input, from the state the last frame left:
  - **egui keeps every key** while it's busy with keys (`keys::egui_wants_keys`): a widget has focus (a text field, or anything reached with Tab on a page without a scope), or a modal, menu, combo box or popup is open. egui then works as egui does, and the code that owns such a widget reads the keys it needs itself (`consume_key`).
  - **Otherwise the app takes the keys bound in the active scopes** (`keys::take`): their press events are removed from egui's input, queued, and routed by the shell before anything is drawn. Unbound keys and all releases stay with egui.
- `keys.rs` has one table, `BINDINGS`: `(Scope, modifiers, key) → Action`. The scopes are `App` (the zoom, on every screen, sign-in included), `Shell` (Back on Esc, Backspace and Alt+Left; search on Ctrl/Cmd+F), `Browse` (arrows, Tab, Shift+Tab, Enter; the item page's rows add W, Alt+Up, Alt+Down and Delete) and `Player`. The active scopes are the top page's `Page::scope()`, `Shell` and `App`. Modifiers match exactly, except on punctuation, which some layouts type with Shift or AltGr.
- A page without the cursor (Settings) returns `None` from `scope()`: Tab, arrows, Space and Enter stay with egui there, while Esc and Backspace still go back when nothing has focus.
- Routing order: open dialog, menu or focused text field (through egui) → the top bar while it has the keyboard → the page (`Page::on_key`) → the shell's own keys (Back, search). Up past a page's first row moves into the top bar; Left and Right move along it, Enter presses, Down or Esc goes back to the page with its cursor where it was.
- The browsing `Cursor` (`browse.rs`) is pure logic with unit tests. A page implements `browse::Browse` (`cursor`, `sections`, `activate`, `adjust`, `at_edge`) and makes `browse::on_key` its `Page::on_key`. While drawing it asks `cursor.take_reveal()` once for the move and scrolls the selected card into view itself (`scroll_to_me`, or its own scroll offset where the row isn't drawn, as with `ScrollArea::show_rows`), and calls `browse::hide_on_click`. The cursor stays hidden until a key is pressed and hides again on a click.
- Menus: every list of choices that pops up is a `widgets::popup::PopupMenu` — the item menu (a card's right-click), the item page's *More* and version menus, the account menu and the player's version menu. It is opened with `PopupMenu::open`/`toggle` (saying whether the keyboard or the pointer opened it) and drawn each frame with `show`, which returns the index picked; whether it's open is egui's popup memory, and its highlight is kept under its id, so pages hold no menu state. The pointer and the keyboard share one highlight, a filled `accent` row (a menu opened from the keyboard starts on its first row): Up/Down wrap, Home/End, Enter or Space pick, Esc, Backspace, Alt+Left or a click outside close, Ctrl/Cmd+F closes and reports `search`, and Tab and Left/Right stay in the menu. It reads those keys while it draws (`keys::take_exact`, so Alt+Up/Down stay the page's) and cancels egui's queued focus move, which egui only applies at the end of the pass, so the keys can't reach a button behind it. Rows are painted, non-focusable and tell AccessKit their role (`MenuItem`, `MenuItemRadio` for a checked choice, `MenuItemCheckBox` for Watched and Favorite). Lists of choices that aren't menus draw the same rows with `popup::row` and move their highlight with `popup::step`, so they look and walk like one: *Add to playlist*'s playlists, the genres, tags and ratings dialog (`Entry::ticked`, any number at once) and the player's track lists; there the roles are `Button`, `RadioButton` and `CheckBox`, and the pointer takes the highlight as it does in a menu. The SyncPlay panel and the player's settings and chapters menus aren't lists of rows and draw their own popups, reading their keys the same way. Selects are `widgets::select`: a painted box (a `ComboBox` to AccessKit, its value the chosen name) whose list is a `PopupMenu` with the chosen row checked — the library toolbar's, the Settings pickers and the item page's and player's version pickers. On pages with a cursor the box takes no focus (the page's keys step it); in Settings it takes egui's focus, Enter or Space opens its list on the first row, and Left/Right still step it.
- In the player: Space pauses, ←/→ seek 10 s, ↑/↓ change the volume, M mutes, Shift+N / Shift+P play the next / previous episode (or movie of its collection), PgUp/PgDn go to the previous/next chapter, Enter or S skips the intro (or plays the next episode from the up-next card, or starts over after an automatic resume), [ and ] change the speed, Z and X the subtitle delay, F toggles fullscreen, and Esc leaves fullscreen, then dismisses the up-next card, then leaves the player.

### Theme and appearance

- `theme.rs` applies the config's theme (dark, light, or the system's), text size (90–150 % of egui's text styles, or the desktop's: the Windows registry, GNOME's `gsettings`, KDE's `kdeglobals`, read again when the window regains the focus), zoom (Ctrl/Cmd `=`, `-`, `0`, saved as `ui_zoom`), and animations (`animation_time` 0 and no scroll animation when off). `theme::animations` is how `motion.rs` and the shelf glide read the setting.
- `motion.rs` has `main`'s times and curves for what appears: a page rises and fades in, Home's shelves come in one after another, a loaded image fades in over its blurhash, the Watched and Favorite icons pop, a season's band rises in, and the player's prompt slides in while its bars fade in. An animation plays when what it wraps appears (egui memory keyed by pass adjacency, so a frame's several passes count as one appearance); with the setting off everything is drawn at its end state at once.
- On Linux the desktop's colour scheme comes from the XDG portal (`theme::SystemTheme`), read on tokio; Windows and macOS let egui/winit follow the system.
- Colours come from egui's style. The player stays dark over the video in both themes and takes its colours from `theme::over_video` (`Color32` constants): the bars' scrims, the sliders' track and thumb, chapter marks and the cards. Its menus sit on an opaque surface, so they use the dark theme's tokens like `main`'s popovers did (`popover`, `tab_bar`, `tab_foreground`, and `Tokens::input_background` for the version picker), and the bars, sliders, buttons, badges and menus are measured against `main`'s player (`docs/TESTING.md` §35). Its widgets are painted by the page rather than built from egui's, so each registers its own AccessKit info (`Response::widget_info`: a button's name, a tab's or row's selected state, a slider's value) while staying non-focusable — egui focus would take every key from the player, so they are clicked, never tabbed to.
- Settings raises `AppEvent::AppearanceChanged` after saving an appearance setting; the app calls `theme::apply` (the appearance is the app's, so it applies on the sign-in screen too).

### Toasts

`toasts.rs` draws info, success and error notices in the window's bottom-right corner: 5 s (errors 8 s), longer while the pointer is on them, at most four at once, with a close button and optionally one action (`ToastAction::OpenUrl` for the update notice). The app owns `Toasts` and draws them over everything, on every screen; signed-in code raises them through `services.toast(…)`, and API errors through `services.handle_error`. Each toast is logged.

### Images and video

- `ImageCache` (`images.rs`) keeps the disk cache, a queue with a budget and blurhash placeholders, and hands out `egui::TextureHandle`s: `get(&ImageRequest)` queues a miss and asks for a frame, `Services::tick` uploads what finished. Every request carries a size (`ImageRequest::fill_width`, twice the drawn width), so the download stays small and the cache key covers it. Images bigger than the device's largest texture are scaled down as they're decoded. Loads stop with their owner (the load task selects on the completion channel closing). Blurhash previews are their own textures, with an LRU capped by bytes and entries. Trickplay (`trickplay.rs`) is a plain `Trickplay` the player page owns: `thumbnail(position)` and a `poll()` each frame, keeping the two most recently used sheets' tiles.
- **Video** is `widgets::video::Video`, one per playback session. It keeps the session's frame receiver, wakes the UI for each new frame, reports the drawn size to the player (`set_target_size`, so mpv renders at the size drawn, never above the video's), uploads the newest frame and draws it letterboxed on black. Software frames are RGBA written with `queue.write_texture` into one `Rgba8Unorm` texture per size, registered once with `register_native_texture` (a new size swaps the texture under the same id). GPU frames are the player's own textures, imported once per image of its ring and registered with egui-wgpu (`docs/in-app-video.md`). Dropping the widget frees its registration and stops the wake task.
- `widgets::video::import(render_state)` is what the app tells the player it can draw when a session starts it (`PlaybackManager::set_video_import` at sign-in): Vulkan memory on wgpu's Vulkan, D3D11 textures on a hardware DX12 adapter, pixels otherwise. A failed import falls back to software frames at the same position and stays there for the rest of the sign-in (`Services::fall_back_to_pixels`).

---

## 7. Milestones

1. **Skeleton:**
   - Cargo workspace with the UI stack pinned and a window that opens with eframe.
   - `tracing` set up.
   - Verified on WSLg, and on native Windows if possible.
2. **API client:**
   - Auth, views, items, and images in the `jellyfin` crate.
   - A `smoke` example that runs against a real server.
   - Test against a server **with legacy auth disabled**, which is the 12.x default.
3. **Login:**
   - Server URL screen, username and password, and Quick Connect.
   - Token stored in the keyring, version gate (§4.4), and auto-login on launch.
4. **Browsing:**
   - Home shelves.
   - Library grid with paging, drawing only the rows on screen.
   - Series, season, episode, and movie pages.
   - Image cache.
5. **Playback:**
   - The `player` crate with the IPC backend.
   - PlaybackInfo, stream URL, and resume position.
   - Session reporting, including recovery after an mpv crash.
6. **Polish:**
   - Audio and subtitle track selection and the version picker.
   - Mark as watched, search, settings (mpv path and config mode), and error states.
7. **Packaging:**
   - CI builds for all three platforms.
   - Bundle mpv on Windows and macOS (§5.5).
8. **Spike:**
   - In-window video using libmpv's software render, then GPU texture sharing (`docs/in-app-video.md`).

---

## 8. Dev environment

**Tools and tasks:** `mise install` installs the pinned Rust toolchain (with clippy and rustfmt) and `cargo-deb`. `mise tasks` lists the shortcuts: `run`, `ci`, `smoke`, `dev:setup`, `package`, and so on. System libraries still come from the OS:

**Ubuntu / WSL2 (current machine):**
```sh
sudo apt install build-essential pkg-config libmpv2 \
  libvulkan1 mesa-vulkan-drivers vulkan-tools \
  libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 \
  libx11-6 libx11-xcb1 libxcursor1 libxi6
```
- `libmpv2` is the runtime library the player loads, and `libvulkan1` the loader wgpu renders through; neither needs a `-dev` package, because both are loaded at runtime.
- winit loads xkbcommon, the Wayland client and cursor libraries and the X11 libraries at runtime too, so building needs no `-dev` packages for them. The `.deb` names them as dependencies (`crates/app/Cargo.toml`).
- `mingw-w64` adds the Windows test loop: `dev/win-run.sh` cross-builds, copies to `C:\Temp\jelly-dev` and runs on the Windows host.
  - It uses its own config folder (`TABBYFIN_CONFIG_DIR=C:\Temp\jelly-dev\config`), so your real sign-in isn't touched. Set it to an empty string to use the normal one.
  - `TABBYFIN_VIDEO=software` forces software video and `=gpu` forces the GPU path, even on software OpenGL, to compare the two. Both work on Linux too.
  - The script also fetches ANGLE next to the exe. Delete `libEGL.dll` there to try the WGL path.
  - `TABBYFIN_ROUTE=play:<id>:::<media source id>` plays a specific version, e.g. the 4K HEVC 10-bit one that `dev/make-media.sh` adds to Test Movie.
- `TABBYFIN_KEYS="down right enter wait escape"` (debug builds) presses keys inside the app 3 s after sign-in, 250 ms apart; `wait` pauses for a second. Together with a screenshot (`import -window <id>` under X11), it tests keyboard navigation without touching the desktop's real keyboard or mouse.
- The app on WSL2 runs through WSLg (`WAYLAND_DISPLAY=wayland-0` is set). It needs a working Vulkan driver, such as mesa `dzn` or llvmpipe.
- Run `vulkaninfo --summary` before debugging rendering problems.
- If WSLg rendering is poor, use a native Windows build for daily UI work. The app runs on Windows directly.

**Test server:** see `dev/README.md`. It's a Jellyfin 12.2 container with a synthetic library generated by ffmpeg, set up by a script.
- A second container runs Jellyfin 10.11, the minimum version, on port 8097: `mise run dev:min` sets it up and runs the smoke test against it.

---

## 9. Open questions

1. ~~**Quick Connect on 10.11:** check whether `POST /QuickConnect/Initiate` exists there.~~ **Answered on 2026-10-14:** it does. The smoke test passes against 10.11.11 with legacy authorization off (`mise run dev:min`), Quick Connect included, so no version-based fallback is needed.
2. ~~**A window saved on a display that's gone:**~~ **Settled in A1:** eframe lists no displays before the window opens and afterwards names only the monitor the window is on, as a size without a position, so the question is settled before the window exists, where the OS can be asked: on Windows the process is made DPI-aware first (winit would only do it when its event loop starts, and until then Windows reports monitors scaled to 96 DPI), then the saved rectangle's centre is looked up among the monitors (`EnumDisplayMonitors`), and a spot on none of them opens the window centred on the primary. Everywhere else the window manager (or AppKit) keeps a window on screen, and Wayland doesn't let an app place its window at all, so the saved spot is used as saved and the app neither guesses nor moves the window after it's up. The placement is stored as physical pixels for the frame's top-left plus the scale factor they were measured at, with the content size in OS points — what eframe's own window storage keeps — so the UI zoom and a monitor of another scale both reopen right. On Windows winit converts the launch position with the scale factor of the monitor the window is created on, so when the window doesn't open on its saved spot (a mixed-DPI setup) the physical position is set again after the first frame, and a maximized window maximized again after that; otherwise nothing is sent. Configs from before the scale factor (`window_bounds` without `scale`) keep their position in points and are saved anew on the next move. Verified on the hidden X11 display; Windows, HiDPI and real multi-monitor desktops are in `docs/TESTING.md` §9.
