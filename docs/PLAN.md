# jelly-gpui: Foundation Plan

A native, cross-platform desktop Jellyfin client: the UI is built with [GPUI](https://www.gpui.rs/) through [GPUI Kit](https://github.com/longbridge/gpui-kit), and playback runs in mpv.

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
| UI | **`gpui-kit` 0.7.x** | Formerly `gpui-component`, renamed to GPUI Kit. It pins an exact GPUI snapshot (`gpui-pre =0.3.9` today) and re-exports it. `use gpui_kit::*` gives us GPUI. **We never add `gpui` as a separate dependency.** The crates.io `gpui` crate (0.2.2, 2025) is stale, and mixing in a second copy would break the build. |
| Widgets | `gpui_kit::component` | More than 75 components, including text input, virtual lists, scroll areas, theming, dialogs, and sidebar. `gpui_kit::base` is there if we later want our own look on top of its unstyled behavior. |
| HTTP | `reqwest` (rustls) + `serde`/`serde_json` | Runs on our own tokio runtime on a background thread. GPUI's executor isn't tokio (§6). |
| Player | **libmpv, embedded**, loaded at runtime | Video renders inside the GPUI window (§5, `docs/in-app-video.md`). |
| Persistence | `directories` + TOML config. `keyring` (v4) holds the access token. | `keyring` uses the OS keychain on all three platforms. Where no credential store exists (WSL, or Linux without a Secret Service daemon), the token goes in a file only the user can read (`<config>/tokens/`, 0600), and a warning is logged. The config is saved to a temporary file and renamed into place, so a crash mid-save can't truncate it; one that doesn't parse is kept as `config.toml.unreadable` before the defaults replace it. |
| Image cache | On-disk cache in the platform cache dir, keyed by `(item id, image type, tag, width)`; in memory, up to 256 MB of decoded images, least recently used dropped first (also from GPUI's texture atlas) | Jellyfin's `ImageTags` act as cache busters. The app decodes images itself: GPUI's own image loading caches every image for good. |
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
│   ├── player/               # mpv via libmpv: no gpui
│   │   ├── lib.rs            #   Player: load, set_property, command, add_subtitle, quit
│   │   ├── events.rs         #   PlayerEvent enum and event translation
│   │   └── libmpv/           #   runtime-loaded libmpv: core, event thread, video output
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

- **Playlist entries (10.11 and 12.2):** an entry's `PlaylistItemId` is the item's own id, and 12.2 adds an item already in a playlist a second time (10.11 skips it), so the copies can't be removed separately. *Add to playlist* checks for the item first. Other users' shared playlists report `CanDelete: true` but answer 403 to changes.
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
- Library grids load page by page with `StartIndex`/`Limit` as the user scrolls, using GPUI Kit's virtual list.

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
- **mpv's log** is forwarded to ours at `warn` level. `JELLY_GPUI_MPV_LOG=v` (or any mpv level) shows more.

### 5.1b The player host process

libmpv doesn't run inside the app. `Player::spawn` starts this same executable again as `jelly-gpui --player-host` (`crates/player/src/host.rs`). That process runs libmpv and the video output described above, and the app talks to it over its stdin and stdout (`remote.rs`, `wire.rs`). An mpv or GPU driver crash then only kills the player process.

- **Messages:** length-prefixed JSON headers with an optional binary payload.
  - App to host: mpv commands, properties (each answered with a reply) and the on-screen size.
  - Host to app: player events, replies and frames.
- **Frames:**
  - **Software:** the BGRA pixels go as the payload. Only the newest frame is sent, so a slow app skips frames instead of queueing them.
  - **Windows:** only the D3D11 texture's legacy share handle is sent; any process can open it.
  - **Linux:** the Vulkan memory fd of each image is passed once over a socket pair (`SCM_RIGHTS`; the host inherits its end as `JELLY_GPUI_PLAYER_FD`). The app keeps the newest 8 open, since GPUI duplicates an fd when it first draws it.
- **Logging:** the host logs plain lines to stderr. The app forwards them into its own log under the `player` target.
- **Crashes:** when the host dies without having finished, the app gets `EndFile(Error(HOST_CRASHED))` and `Exited`. The session ends as *lost*, the manager starts a new session (and host) at the same position, and the user sees "The player stopped unexpectedly. Restarting it…". After three tries the error says the player keeps crashing.
- **Lifetime:** the host exits when its stdin closes, which happens when the app drops the player or exits. It's also killed when the app drops it.
- **`JELLY_GPUI_PLAYER=embedded`** runs libmpv inside the app as before, for debugging.

Tested: software video on Linux and ANGLE/D3D11 with d3d11va on Windows (AMD) through the host. Killing the host mid-play on both resumes playback in a new one. Linux GPU frames through the host are confirmed on WSL's llvmpipe (forced GPU path), in-process too.

### 5.2 Threading and reporting

1. The session runs on tokio. It forwards `PlayerEvent`s to a `NowPlaying` GPUI entity, which the player page draws, and it receives `UiCommand`s from that page: pause, seek, volume, tracks and stop.
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
     - Subtitles the server delivers as files (`DeliveryMethod: External`, embedded text ones included) are added to mpv; image subtitles are burned in (`Encode`).
     - A transcode carries only the chosen audio track, so changing audio, or changing to or from a burned-in subtitle, restarts the session at the current position. The server picks the default audio by the user's language preference and names it in the URL (`AudioStreamIndex`), which the picker follows.
2. It reports to the server:
   - `/Sessions/Playing` on start.
   - `/Sessions/Playing/Progress` about every 10 s, plus immediately on pause, seek or track change.
   - `/Sessions/Playing/Stopped` when playback ends, using the last known position.
3. When playback ends, pages that show watched state refresh.

### 5.3 Shipping libmpv

| Platform | libmpv |
|---|---|
| Linux | The system's `libmpv2`. The `.deb` depends on it. |
| Windows | `libmpv-2.dll` next to `jelly-gpui.exe` in the zip. It's a shinchiro build, pinned with its checksum in `packaging/mpv.lock` and fetched by `packaging/fetch-libmpv.sh`. The zip also carries ANGLE (`libEGL.dll`, `libGLESv2.dll`) and its MinGW runtime DLLs, MSYS2 builds pinned in `packaging/angle.lock` and fetched by `packaging/fetch-angle.sh`, for zero-copy video. |
| macOS | Not bundled yet. The app looks in Homebrew's folders, so `brew install mpv` makes playback work. Bundling `libmpv.dylib` is still to do. The app is ad-hoc signed, not notarized. |

`packaging/THIRD_PARTY.md` holds mpv's license (GPL-2.0-or-later) and source information, and ships in the packages.

**Builds:** `.github/workflows/ci.yml` runs clippy and the tests on all three OSes. On a `v*` tag, or when started by hand, it also packages all four targets:
- Linux `.tar.gz` and `.deb`
- Windows `.zip`
- macOS arm64 and x86_64 `.zip`

A tag also creates a draft GitHub release with these files. Locally, `mise run package` packages Linux, and `mise run ci:build` builds every OS on CI.

---

### 5.4 SyncPlay

`syncplay.rs` holds the group this session is in. The protocol is in `docs/ROADMAP.md` §2.4; the flow here:

1. **Joining:** the top-bar popover lists groups (`/SyncPlay/List`), creates one or joins one. The server confirms with a `GroupJoined` update on the websocket. While in a group, the server clock is sampled (4 samples, then one a minute) and the ping is reported.
2. **Starting something** while in a group (Play on an item page, next episode…) doesn't play locally: `PlaybackManager::play` hands it to SyncPlay, which sends `SetNewQueue` (the item, plus the next episode for episodes).
3. **Queue updates:** when the queue's current entry changes, every member (this one too) loads it paused at the group's position (`PlayOptions::start_paused`, `group_item`), reports `Buffering`, then `Ready` after mpv's first `playback-restart`.
4. **Commands** arrive with a `When` in server time:
   - `Unpause`: seek to the group's position if more than 0.4 s off, wait until `When`, then unpause. If it arrived late, seek ahead by the lateness and unpause now.
   - `Pause`: at `When`, pause and seek to the group's position.
   - `Seek`: pause, seek, report `Buffering`, and `Ready` after the restart; the server then unpauses everyone.
   - `Stop`: stop playback.
5. **Local controls** in a group session (`NowPlaying::control`): pause, seek and stop become `/SyncPlay/Pause`, `Unpause`, `Seek` and `Stop` requests, so they apply to everyone. Esc stops playback for the group, as in the web client. Volume and tracks stay local.
6. **End of an item:** each member sends `NextItem`; the server advances the queue once, and the next item arrives as a queue update. Local autoplay is off in a group.
7. **Drift correction:** after an Unpause, the group's timeline is "position P at local instant T". Once a second, the local position (mpv's last report plus the time since, at the current speed) is compared with it:
   - Under 80 ms off: normal speed.
   - Up to 1.5 s: play up to 5% faster or slower (0.25 speed change per second of drift); mpv keeps the pitch.
   - More: seek to the group's position.
   - In WSL with software rendering, a member drifts up to about 0.4 s while playback warms up, is back within 0.1 s after about 8 s, and stays within ±70 ms after that.

Not yet: track changes that need a new transcode, and version switches, don't stay in sync.

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
  - Define GPUI `actions!` with a keymap: Backspace, Esc or Alt+← to go back, Ctrl+F (⌘F) to search.
  - Browsing pages (`browse.rs`) share one keyboard cursor over sections of cards: Home shelves, the library grid, search result groups, and an item page's buttons followed by its seasons or episodes.
    - The arrow keys move it, Tab and Shift+Tab jump between sections, and Enter opens the card or presses the button.
    - It's hidden until a key is pressed and hides again on a click, so mouse users never see it.
    - Each page keeps its cursor while it's on the stack, so Back returns to the same card.
    - In search, Enter in the box moves to the first result, and Up from the top row returns to the box.
    - An item's version and track pickers are a column of the cursor: Up/Down move between them, Left/Right step through the choices and Enter moves to the next one, wrapping. gpui-kit's Select only opens its menu with a click, so the keys set it directly and send the same `Confirm` a menu pick would (`tracks::step_choice`).
    - Settings is a cursor over its controls: pickers as above, switches (Left off, Right on, Enter flips) and buttons (Enter).
    - On an item page, W toggles watched on the highlighted episode, season or entry; in a playlist, Alt+Up/Alt+Down move the entry and Delete removes it.
  - In the player: Space pauses, ←/→ seek 10 s, ↑/↓ change the volume, M mutes, Shift+N / Shift+P play the next / previous episode (or movie of its collection), PgUp/PgDn go to the previous/next chapter, Enter or S skips the intro (or plays the next episode from the up-next card, or starts over after an automatic resume), [ and ] change the speed, Z and X the subtitle delay, F toggles fullscreen, and Esc leaves fullscreen, then dismisses the up-next card, then leaves the player.
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

**Tools and tasks:** `mise install` installs the pinned Rust toolchain (with clippy and rustfmt) and `cargo-deb`. `mise tasks` lists the shortcuts: `run`, `ci`, `smoke`, `dev:setup`, `package`, and so on. System libraries still come from the OS:

**Ubuntu / WSL2 (current machine):**
```sh
sudo apt install build-essential pkg-config libmpv2 \
  libvulkan-dev mesa-vulkan-drivers vulkan-tools \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev \
  libfontconfig-dev libfreetype-dev libssl-dev
```
- `libmpv2` is the runtime library the player loads. No `-dev` package is needed, because libmpv is loaded at runtime.
- `mingw-w64` adds the Windows test loop: `dev/win-run.sh` cross-builds, copies to `C:\Temp\jelly-dev` and runs on the Windows host.
  - It uses its own config folder (`JELLY_GPUI_CONFIG_DIR=C:\Temp\jelly-dev\config`), so your real sign-in isn't touched. Set it to an empty string to use the normal one.
  - `JELLY_GPUI_VIDEO=software` forces software video and `=gpu` forces the GPU path, even on software OpenGL, to compare the two. Both work on Linux too.
  - The script also fetches ANGLE next to the exe. Delete `libEGL.dll` there to try the WGL path.
  - `JELLY_GPUI_ROUTE=play:<id>:::<media source id>` plays a specific version, e.g. the 4K HEVC 10-bit one that `dev/make-media.sh` adds to Test Movie.
- `JELLY_GPUI_KEYS="down right enter wait escape"` (debug builds) presses keys inside the app 3 s after sign-in, 250 ms apart; `wait` pauses for a second. Together with a screenshot (`import -window <id>` under X11), it tests keyboard navigation without touching the desktop's real keyboard or mouse.
- GPUI on WSL2 runs through WSLg (`WAYLAND_DISPLAY=wayland-0` is set). It needs a working Vulkan driver, such as mesa `dzn` or llvmpipe.
- Run `vulkaninfo --summary` before debugging rendering problems.
- If WSLg rendering is poor, use a native Windows build for daily UI work. GPUI supports Windows directly.

**Test server:** see `dev/README.md`. It's a Jellyfin 12.2 container with a synthetic library generated by ffmpeg, set up by a script.
- A second container runs Jellyfin 10.11, the minimum version, on port 8097: `mise run dev:min` sets it up and runs the smoke test against it.

---

## 9. Open questions

1. ~~**Quick Connect on 10.11:** check whether `POST /QuickConnect/Initiate` exists there.~~ **Answered on 2026-10-14:** it does. The smoke test passes against 10.11.11 with legacy authorization off (`mise run dev:min`), Quick Connect included, so no version-based fallback is needed.
