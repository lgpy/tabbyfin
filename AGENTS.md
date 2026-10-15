# AGENTS.md

Guidance for coding agents working in this repository.

A native desktop Jellyfin client: GPUI (through gpui-kit) for the UI, libmpv for in-window video. Linux, Windows and macOS; Jellyfin 10.11 minimum, developed against 12.2. `docs/PLAN.md` is the architecture reference, `docs/ROADMAP.md` tracks what's done and open, `docs/TESTING.md` lists what still needs testing on real hardware, and `docs/in-app-video.md` explains the video paths per OS.

## Commands

Tooling is pinned in `mise.toml` (Rust 1.99, `gh`, …); `mise tasks` lists everything.

```sh
mise run run          # debug build of the app
mise run ci           # what CI checks on every OS: fmt:check, clippy -D warnings, tests
mise run fmt          # rustfmt (max_width = 120); CI fails on unformatted code
cargo test -p jellyfin --test fixtures          # one integration test file
cargo test -p jelly-gpui browse                  # tests whose name matches
mise run dev:setup    # Jellyfin 12.2 dev server on :8096 with generated media (login dev/dev)
mise run dev:min      # Jellyfin 10.11 on :8097, plus the API smoke test against it
mise run smoke        # cargo run -p jellyfin --example smoke against the dev server
mise run ci:build     # build packages for every OS on GitHub Actions into dist/
dev/win-run.sh [secs] # cross-build (mingw) and run on the Windows host from WSL
```

GitHub CI also runs an `integration` job: Jellyfin 12.2 and 10.11.11 in Docker, set up with `dev/setup-server.py`, then the `smoke` and `socket` examples. `crates/jellyfin/examples/` has those live checks: `smoke`, `socket` (websocket, clock, keep-alive), and `syncplay` (a scripted second SyncPlay member). `dev/capture-fixtures.py <url> <version>` refreshes the JSON fixtures in `crates/jellyfin/tests/fixtures/` from a dev server.

## Workspace

- `crates/jellyfin`: the API client, no UI dependencies. `Client` (reqwest/rustls, header auth only — legacy auth is off in 12.x), typed models (`wire_enum!` maps unknown strings to `Other`), playback negotiation, the session websocket (`Client::socket()`, upgraded through reqwest), remote-control messages and session capabilities (`remote.rs`), SyncPlay requests and messages, and `ServerClock` (`/GetUtcTime`).
- `crates/player`: mpv playback. `libmpv/` loads libmpv at runtime (`dlopen`) and renders through `video.rs`: ANGLE (`angle.rs`, zero-copy d3d11va) or WGL (`wgl.rs`) into shared D3D11 textures on Windows, OpenGL into exported Vulkan memory on Linux (`egl.rs`), or software BGRA pixels. By default libmpv runs in a separate **player host process** — the same executable started with `--player-host` (`host.rs`, `remote.rs`, `wire.rs`; `JELLY_GPUI_PLAYER=embedded` keeps it in-process). `main` must check `::player::is_host_process()` before starting any UI.
- `crates/app` (binary `jelly-gpui`): GPUI views and app state.
- `third_party/gpui/`: vendored, patched `gpui-pre*` crates for shared-texture surfaces (`[patch.crates-io]`); see its `PATCHES.md` before bumping gpui-kit. Never depend on `gpui` directly; gpui-kit pins and re-exports it.

## App architecture

- **GPUI ↔ tokio:** GPUI code never awaits `jellyfin` futures directly; wrap them in `runtime::spawn` (a 2-thread tokio runtime) and await that from `cx.spawn`.
- **Navigation:** `ShellView` keeps a stack of pages (`router::Route`); pages stay alive while stacked, so Back keeps their state. The shell gives keyboard focus to the top page (`focus_top`).
- **Globals:** `ImageCache` (disk, plus a 256 MB LRU of decoded images that also frees GPUI's atlas; blurhash placeholders), `PlaybackManager`, `SyncPlay`, `Remote` and `Changes` (an event that makes pages reload watched state) are GPUI globals set up per signed-in session in `ShellView::new`. `live.rs` keeps the session websocket open and feeds `Changes`, `SyncPlay` and `Remote`.
- **Remote control** (`remote.rs`): `Play`, `Playstate` and `GeneralCommand` messages from other clients. They go through the same paths as the player's own controls (`NowPlaying::control`, `PlaybackManager::play_all_with` / `enqueue`), so SyncPlay groups get them as group requests. The server also sends general commands we don't advertise; ignore unknown ones.
- **Playback** (`playback.rs`): `PlaybackManager::play` → `run_session` on tokio negotiates `PlaybackInfo` (direct play, or the HLS `TranscodingUrl` with its `ApiKey` stripped), drives the `Player`, reports progress, and publishes a `NowPlaying` entity that `views/player.rs` draws and controls. Sessions are numbered (`generation`) so a replaced session's end doesn't close its successor. A session that fails after starting ends as `Ended::Lost` and is restarted at its position (network drops and player-process crashes). `PlaybackManager::now_playing()` is the running session's state. Speed, delays and subtitle size are `Adjustments` (speed and size carry to the next item, delays don't). There's a local play queue (Play all, remote play next/last), autoplay of the next episode, and in a SyncPlay group `play` is redirected to the group's queue and pause/seek/stop become group requests (`NowPlaying::control`).
- **Keyboard:** browsing pages implement `browse::Browse` (a cursor over sections of cards; hidden until a key is pressed); the player has its own key context. Key bindings are in `main.rs`.
- **Theme:** `theme.rs` applies the config's dark, light or system preference; colours come from `cx.theme()`, except the player, which stays dark over the video.
- **Config:** `config.rs` (TOML in the platform config dir; `JELLY_GPUI_CONFIG_DIR` overrides it). Tokens live in the system keychain (`credentials.rs`), never in the config; without a keychain (WSL) they fall back to `tokens/` inside the config dir. Logs go to stderr and a daily file (`config::log_dir()`).

## Testing in this environment

Development happens in WSL2 without a GPU (Linux video falls back to software). Debug builds have knobs for driving the app without touching the desktop's mouse or keyboard — never simulate real input: `JELLY_GPUI_ROUTE` (open a page or `play:<id>[:audio[:sub[:source]]]`), `JELLY_GPUI_KEYS` (replay keystrokes inside the app), `JELLY_GPUI_SYNCPLAY` / `JELLY_GPUI_SYNCPLAY_SKEW_MS`, `JELLY_GPUI_MPV_OPTIONS` (extra mpv options, e.g. `ao=null;cache=no` for network-drop tests), `JELLY_GPUI_VIDEO=software|gpu`, `JELLY_GPUI_MPV_LOG`. Screenshots: run without `WAYLAND_DISPLAY` (X11) and use `import -window`. Only use the local dev servers for playback tests. `dev/README.md` describes all of these.

- To try a config change without touching the user's, copy the config dir (including `tokens/`) to the scratchpad and point `JELLY_GPUI_CONFIG_DIR` at it. Test runs save the volume and window placement, so put back anything a run changed in the real config.
- Remote control is tested by driving the app's session over REST the way the dashboard does (`POST /Sessions/{id}/Playing…`, `/Command`, `/Message`, finding the session by the config's `device_id`) and reading `GET /Sessions`.
- Popovers and hover can't be opened with `JELLY_GPUI_KEYS`; for a screenshot, open them by default temporarily and revert before committing.

## Conventions

- Match the surrounding comment style: doc comments explain why, plain English, no filler.
- Keep `docs/ROADMAP.md` (✅ markers), `docs/PLAN.md` and `docs/TESTING.md` in step with behavior changes.
- Tests in the app crate use explicit imports instead of `use super::*`: GPUI's glob re-export shadows `#[test]`.
