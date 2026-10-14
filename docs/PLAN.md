# jelly-gpui: Foundation Plan

A native, cross-platform desktop Jellyfin client: the UI is built with [GPUI](https://www.gpui.rs/) through [GPUI Kit](https://github.com/longbridge/gpui-kit), and playback runs in mpv.

**Targets:** Linux, Windows, and macOS.
**Server:** developed and tested against Jellyfin **12.x** (12.2 at the time of writing). The minimum supported version is **10.11** (see §4.4).

---

## 1. Goals for v1

**In scope**
- Sign in to a server with a username and password, or with Quick Connect. The session is kept between launches.
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
- SyncPlay, multiple servers at once, and admin features.
- Casting, and remote-controlling other sessions.

---

## 2. Tech stack

| Concern | Choice | Notes |
|---|---|---|
| UI | **`gpui-kit` 0.7.x** | Formerly `gpui-component`, renamed to GPUI Kit. It pins an exact GPUI snapshot (`gpui-pre =0.3.9` today) and re-exports it. `use gpui_kit::*` gives us GPUI. **We never add `gpui` as a separate dependency.** The crates.io `gpui` crate (0.2.2, 2025) is stale, and mixing in a second copy would break the build. |
| Widgets | `gpui_kit::component` | More than 75 components, including text input, virtual lists, scroll areas, theming, dialogs, and sidebar. `gpui_kit::base` is there if we later want our own look on top of its unstyled behavior. |
| HTTP | `reqwest` (rustls) + `serde`/`serde_json` | Runs on our own tokio runtime on a background thread. GPUI's executor isn't tokio (§6). |
| Player | **The `mpv` binary as a child process, controlled over JSON IPC** | §5 explains why we use the binary rather than libmpv. |
| Persistence | `directories` + TOML config. `keyring` holds the access token. | `keyring` uses the OS keychain on all three platforms. |
| Image cache | On-disk cache in the platform cache dir, keyed by `(item id, image type, tag, width)` | Jellyfin's `ImageTags` act as cache busters. |
| Errors / logs | `thiserror` in the library crates, `anyhow` in the app, plus `tracing` | |

**GPUI churn:** GPUI Kit warns that any GPUI snapshot can change its API. We upgrade `gpui-kit` on purpose, in its own commit, and only after reading its changelog. All GPUI code lives in the `app` crate.

---

## 3. Workspace layout

```
jelly-gpui/
├── Cargo.toml                # workspace; pins gpui-kit
├── crates/
│   ├── jellyfin/             # Pure API client: no gpui, no mpv
│   │   ├── client.rs         #   base URL, auth header, request helpers, error mapping
│   │   ├── auth.rs           #   AuthenticateByName, Quick Connect, logout
│   │   ├── items.rs          #   views, items, resume, next up, latest, seasons/episodes
│   │   ├── playback.rs       #   PlaybackInfo, stream/subtitle URLs, session reporting
│   │   ├── images.rs         #   image URL builder
│   │   ├── ticks.rs          #   Ticks newtype (1 tick = 100 ns) <-> Duration
│   │   └── models.rs         #   hand-written DTOs, only the fields we use
│   ├── player/               # mpv control: no gpui
│   │   ├── lib.rs            #   Player trait: load, pause, seek, set_track, stop, events()
│   │   ├── ipc.rs            #   JSON IPC transport (Unix socket / Windows named pipe)
│   │   ├── process.rs        #   spawn/locate the mpv binary, lifecycle, crash handling
│   │   └── events.rs         #   PlayerEvent enum
│   └── app/                  # gpui-kit binary
│       ├── main.rs
│       ├── state.rs          #   AppState entity: session, user, router
│       ├── router.rs         #   navigation stack (Route enum)
│       ├── runtime.rs        #   tokio bridge
│       ├── images.rs         #   async image load + disk cache -> gpui image source
│       ├── playback.rs       #   PlaybackSession: ties Player events to server reporting
│       ├── views/            #   login, home, library, item_detail, settings
│       └── components/       #   poster card, shelf row, progress bar, etc.
└── docs/
```

**Why this split:** `jellyfin` and `player` can be built and tested on their own. For example, `cargo run -p jellyfin --example smoke` can run against a real server. That keeps GPUI changes confined to `app`.

**Models:** we write the serde DTOs by hand and keep them to the fields we need. Every field is `Option` or `#[serde(default)]`, and unknown fields are ignored. Generating code from the full OpenAPI spec would create a huge, churny surface. Swashbuckle v10 in 12.0 already changed the generated document once. If we later want generated code, we generate it from the 12.x spec.

---

## 4. Jellyfin integration

### 4.1 Authentication (changed in 12.0, important)

Jellyfin 12.0 **disables legacy authorization by default**, and existing installs are migrated to it too (PR #15559). The plan is to remove the old methods completely in a later release. Only two methods are accepted:

- The `Authorization` header with the `MediaBrowser` scheme:
  ```
  Authorization: MediaBrowser Client="jelly-gpui", Device="<hostname>", DeviceId="<stable uuid>", Version="<crate version>", Token="<token>"
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
| Playback info | `POST /Items/{itemId}/PlaybackInfo`, sending a DeviceProfile that allows direct play of everything |
| Stream | `GET /Videos/{itemId}/stream?static=true&MediaSourceId=` (mpv opens this URL) |
| External subtitles | `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{index}/Stream.{format}` |
| Reporting | `POST /Sessions/Playing`, `/Sessions/Playing/Progress`, `/Sessions/Playing/Stopped` |
| Watched state | `POST` / `DELETE /UserPlayedItems/{itemId}` |

We use the user-scoped routes without a userId, such as `/UserViews` and `/Items`. The old `/Users/{userId}/Items` and `/Users/{userId}/Views` routes are **not in the 12.x spec**.

### 4.3 Other 12.x behavior changes to handle

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

### 4.4 Minimum server version: 10.11

- Every endpoint in §4.2 exists in 10.11. The only exception is `POST /QuickConnect/Initiate`, which needs a quick check on 10.11. If it's missing there, fall back to `GET` only for servers older than 12.
- 10.11 introduced the `EnableLegacyAuthorization` switch. So 10.11 is the oldest version where we can turn off legacy auth on a test server and prove we don't depend on it.
- The 12.0 release notes support direct upgrades from 10.10.7 and 10.11.x, so 10.10 users have an easy path forward.
- We read `Version` from `/System/Info/Public` at connect time. Servers older than 10.11 get a clear "please upgrade" message instead of failing in odd ways.

### 4.5 Units and paging

- Positions are measured in **ticks**, where 1 tick = 100 ns. One `Ticks` newtype handles conversion to and from `Duration`, so units are never mixed by hand.
- Library grids load page by page with `StartIndex`/`Limit` as the user scrolls, using GPUI Kit's virtual list.

---

## 5. Playback architecture

### 5.1 Why we use the `mpv` binary over IPC instead of libmpv

We agreed to start with **option A**: mpv opens its own window. Doing that by spawning the `mpv` binary and controlling it through `--input-ipc-server` (JSON IPC) is the most portable approach:

- **The same mechanism works on every platform:**
  - Linux and macOS use a Unix socket.
  - Windows uses a named pipe (`\\.\pipe\jelly-gpui-<pid>`).
- **libmpv opening its own window is the weak spot on macOS.** mpv's macOS backends assume the mpv app owns the Cocoa run loop. Here, GPUI owns the main thread and NSApplication, and the mpv docs point embedders to the render API instead. A separate process avoids that conflict.
- **Crash isolation.** If mpv crashes or gets stuck, the app keeps running. We detect it, report playback as stopped, and recover.
- **Nothing native to link at build time.** We don't need `libmpv-dev` on build machines and avoid ABI mismatches. On Linux, users can keep their own mpv and its config.
- **The cost:** IPC is JSON lines, and we have to manage the process. `player/ipc.rs` contains both.

libmpv returns later for in-window video (§5.4). The `Player` trait keeps that swap internal to the `player` crate.

### 5.2 Launching mpv

```
mpv --idle=once --force-window=immediate --input-ipc-server=<path>
    --hwdec=auto-safe --vo=gpu-next --keep-open=no
    --title="<item name> — jelly-gpui"
```

After connecting over IPC, the app sends this sequence:
1. `set_property http-header-fields ["Authorization: MediaBrowser ... Token=..."]`. **The token goes over IPC, not on the command line**, because other local users can read process arguments with `ps`.
2. `loadfile <stream url> replace` with `start=<resume seconds>`.
3. `sub-add <subtitle url>` for each external subtitle. These requests are authenticated by the same header fields.
4. `observe_property` on `time-pos`, `pause`, `duration`, `track-list`, `eof-reached`, and `idle-active`.

**User config:** we load the user's normal `mpv.conf` by default. A setting can switch to `--no-config` or a config dir that belongs to the app.

### 5.3 Threading and reporting

1. An IPC reader thread parses JSON events and turns them into `PlayerEvent`s. It sends them over an `async-channel`.
2. A GPUI task on the `app` side receives those events and updates a `PlaybackSession` entity.
3. `PlaybackSession` reports to the server:
   - `/Sessions/Playing` on start.
   - `/Sessions/Playing/Progress` about every 10 s, plus immediately on pause, seek, or track change.
   - `/Sessions/Playing/Stopped` when mpv exits or reaches end of file. This uses the last known position, so a crash still records progress.
4. When playback ends, we refresh the item and Continue Watching so the UI shows the new watched state.

### 5.4 Later: video inside the app window

| Option | How it works | Status |
|---|---|---|
| B. `--wid` embed | Give mpv a native child window handle | Works on X11, Win32, and macOS only. It **does not work on Wayland**, and GPUI exposes no child surfaces. Unlikely. |
| C. Software render API | libmpv `mpv_render_context` with the `sw` backend, copying frames into a GPUI image | Easiest way to get video in the window, with GPUI drawing the overlay. CPU-heavy, so likely capped around 1080p. |
| D. GL render plus texture sharing | Share mpv's FBO with GPUI's renderer through dmabuf, IOSurface, or DXGI | The best result. Each platform needs changes to GPUI's renderer. This is a long-term project. |

We plan a time-boxed spike on C after v1. The `player` crate gets a `libmpv` backend next to the `ipc` backend, behind the same trait.

### 5.5 Shipping mpv

| Platform | Plan |
|---|---|
| Linux | **Decided:** use the user's installed `mpv` (`PATH`, overridable in settings). We don't bundle it, including in distro packages, which declare `mpv` as a dependency. A Flatpak would get mpv through the sandbox runtime or an extension if we ever ship one. |
| Windows | Bundle `mpv.exe` next to our executable. Use the shinchiro / mpv-winbuild builds and record the version. |
| macOS | Bundle the `mpv` binary inside the `.app` (`Contents/MacOS/` or `Resources/`). It must be signed along with the app. |

Lookup order: path set in settings, then bundled, then `PATH`. If none is found, the app shows a clear error that links to install instructions.

---

## 6. App architecture (GPUI side)

- **`AppState` (Entity):**
  - Server URL, the authenticated session, and the current user.
  - The `Router`.
- **`Router`:**
  - A stack of `Route` values: `Login`, `Home`, `Library{id}`, `Item{id}`, `Season{id}`, `Settings`.
  - The root view renders whatever is on top of the stack. Going back pops it.
- **Views load their own data:**
  - They spawn a fetch when created and move through `Loading`, then `Loaded(T)` or `Error`.
  - DTOs are cached in memory by id, so going back is instant.
- **Async bridge (`runtime.rs`):**
  - One global multi-threaded tokio `Runtime`.
  - A helper (an extension trait, for example `cx.spawn_tokio(fut)`) runs the future on tokio and returns the result to GPUI's foreground executor.
  - GPUI code never awaits reqwest directly.
- **Images:**
  - Fetch through the bridge and save to the disk cache.
  - Decode off the main thread and hand the result to GPUI.
  - Show a placeholder from `ImageBlurHashes` while the real image loads.
- **Keyboard:**
  - Define GPUI `actions!` with a keymap: arrow keys and Enter to navigate, Backspace or Esc to go back, `/` to search.
- **Theme:**
  - GPUI Kit's theme system, defaulting to dark.

---

## 7. Milestones

1. **Skeleton:**
   - Cargo workspace with `gpui-kit` pinned and a window that opens with GPUI Kit initialized.
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
   - Library grid with paging and a virtual list.
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
   - In-window video using libmpv's software render (§5.4 C).

---

## 8. Dev environment

**Ubuntu / WSL2 (current machine):**
```sh
sudo apt install build-essential pkg-config mpv \
  libvulkan-dev mesa-vulkan-drivers vulkan-tools \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev \
  libfontconfig-dev libfreetype-dev libssl-dev
```
- The IPC backend doesn't need `libmpv-dev`. Add it later for the §5.4 spike.
- GPUI on WSL2 runs through WSLg (`WAYLAND_DISPLAY=wayland-0` is set). It needs a working Vulkan driver, such as mesa `dzn` or llvmpipe.
- Run `vulkaninfo --summary` before debugging rendering problems.
- If WSLg rendering is poor, use a native Windows build for daily UI work. GPUI supports Windows directly.

**Test server:**
- A local Jellyfin 12.x in Docker (`jellyfin/jellyfin:12.2`) with a small test library.
- Keep a second 10.11 container to test against the minimum version.

---

## 9. Open questions

1. **Quick Connect on 10.11:** check whether `POST /QuickConnect/Initiate` exists there. The answer decides whether we need a version-based fallback. We'll check this during milestone 3.
