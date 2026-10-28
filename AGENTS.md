# AGENTS.md

Guidance for coding agents working in this repository.

A native desktop Jellyfin client: egui (through eframe) for the UI, libmpv for in-window video. Linux, Windows and macOS; Jellyfin 10.11 minimum, developed against 12.2. `docs/PLAN.md` is the architecture reference, `docs/ROADMAP.md` tracks what's done and open, `docs/TESTING.md` lists what still needs testing on real hardware, and `docs/in-app-video.md` explains the video paths per OS.

## Commands

Tooling is pinned in `mise.toml` (Rust 1.99, `gh`, …); `mise tasks` lists everything.

```sh
mise run run          # debug build of the app
mise run ci           # what CI checks on every OS: fmt:check, clippy -D warnings, tests
mise run fmt          # rustfmt (max_width = 120); CI fails on unformatted code
cargo test -p jellyfin --test fixtures          # one integration test file
cargo test -p tabbyfin browse                  # tests whose name matches
cargo test -p tabbyfin ui_tests                # the headless UI tests (egui_kittest)
mise run dev:setup    # Jellyfin 12.2 dev server on :8096 with generated media (login dev/dev)
mise run dev:min      # Jellyfin 10.11 on :8097, plus the API smoke test against it
mise run smoke        # cargo run -p jellyfin --example smoke against the dev server
mise run ci:build     # build packages for every OS on GitHub Actions into dist/
mise run flatpak:sources # after any Cargo.lock change: regenerate the Flatpak's offline crate list
mise run deps:mirror  # after changing packaging/*.lock: copy the pinned files to the repo's deps release
dev/win-run.sh [secs] # cross-build (mingw) and run on the Windows host from WSL
```

GitHub CI also runs an `integration` job: Jellyfin 12.2 and 10.11.11 in Docker, set up with `dev/setup-server.py`, then the `smoke` and `socket` examples. `crates/jellyfin/examples/` has those live checks: `smoke`, `socket` (websocket, clock, keep-alive), and `syncplay` (a scripted second SyncPlay member). `dev/capture-fixtures.py <url> <version>` refreshes the JSON fixtures in `crates/jellyfin/tests/fixtures/` from a dev server.

## Workspace

- `crates/jellyfin`: the API client, no UI dependencies. `Client` (reqwest/rustls, header auth only — legacy auth is off in 12.x), typed models (`wire_enum!` maps unknown strings to `Other`), playback negotiation, the session websocket (`Client::socket()`, upgraded through reqwest), remote-control messages and session capabilities (`remote.rs`), SyncPlay requests and messages, and `ServerClock` (`/GetUtcTime`).
- `crates/player`: mpv playback. `libmpv/` loads libmpv at runtime (`dlopen`) and renders through `video.rs`: ANGLE (`angle.rs`, zero-copy d3d11va) or WGL (`wgl.rs`) into shared D3D11 textures on Windows, OpenGL into exported Vulkan memory on Linux (`egl.rs`), or software RGBA pixels. The three GPU paths are `opengl::Backend`s: `opengl.rs` holds what they share (mpv's render context, the 3-texture ring, the render loop), each backend only its GL context, shared textures and frame sync. By default libmpv runs in a separate **player host process** — the same executable started with `--player-host` (`host.rs`, `remote.rs`, `wire.rs`; `TABBYFIN_PLAYER=embedded` keeps it in-process). `main` must check `::player::is_host_process()` before starting any UI.
- `crates/app` (binary `tabbyfin`): the eframe/egui app. `main.rs` sets up wgpu (DX12 on Windows, Vulkan with GL as fallback on Linux, Metal on macOS) and opens the window; `app.rs` restores the session and holds its `Services`, which pages are lent each frame.

## App architecture

- **Async:** UI code never awaits `jellyfin` futures. `task::Task` runs a future on the tokio runtime (`runtime.rs`, 2 threads) and requests a repaint when the result is ready; the owner polls it each frame and keeps it in an `Option`, so replacing it cancels the work. Streams (a session's updates, the websocket) are a `task::channel` drained in `Services::tick`; timers are a `task::Deadline` checked in `tick` or `show`. Nothing repaints every frame while idle (the video widget wakes the UI per frame; a `Deadline` asks for the frame it's due in).
- **State:** no globals, no `Rc<RefCell<…>>`. `App` owns the toasts and, while signed in, `Option<Services>`; `Services` holds the session, the image cache, `user_data` (a counter pages watch to reload watched state), `playback`, `syncplay`, `live`, `remote` and the item page's playlist edit queue. Pages get a `&mut PageCx` each frame (`ctx`, `services`, `nav: &mut Vec<NavRequest>`, `render_state`). Cross-service calls (a control SyncPlay may take over, playback handing over to a group) are methods on `Services` (`services.control(UiCommand)`, `services.play(…)`), added by each part's own `impl Services` block. Things the app or the shell must act on become `AppEvent`s in `services.events`, drained every frame.
- **Navigation:** `Shell` keeps a stack of `Box<dyn Page>` (`nav::Route`); pages stay alive while stacked, so Back keeps their state. A page implements `views::Page` (`title`, `scope`, `on_key`, `show`, plus the search hooks); it navigates by pushing `NavRequest::Push` / `Back` into its `PageCx`. The top bar (Back, title, search box, SyncPlay, Settings, account menu) sits above the pages; while something plays, `views::player::PlayerPage` takes the window instead.
- **Keyboard:** `keys.rs` has every binding in one table of scopes (`App`, `Shell`, `Browse`, `Player`). `App::raw_input_hook` decides per frame whether egui keeps the keys (a widget has focus, or a modal, menu or popup is open) or the app takes the bound ones before egui sees them; the shell routes them. Browsing pages implement `browse::Browse` (a cursor over sections of cards, hidden until a key is pressed) and make `browse::on_key` their `Page::on_key`. Pages with no cursor (Settings) return `None` from `scope`, so egui's own focus works there.
- **Theme:** `theme.rs` applies the config's dark, light or system preference, the text size, zoom and animations; colours come from `theme.rs`'s tokens through egui's `Visuals` and `Spacing`, except the player, which stays dark over the video and takes its colours from `theme::over_video`. Settings raises `AppEvent::AppearanceChanged` instead of applying it, since the appearance is the app's (and on Linux the desktop's colour scheme is followed through the XDG portal). The restyle (`docs/ROADMAP.md`, R1–R6) is where the look becomes the old app's: R1 landed the tokens (`theme::Tokens`, one palette per theme in `theme.rs`, with egui's `Visuals` and `Spacing` built from them) and the app's own text-style table (`theme::text`, `main`'s `text_xs` … `text_2xl`, which the text size setting scales); widgets read `theme::tokens_of(ui)` rather than literal colours.
- **Toasts** (`toasts.rs`): info, success and error notices, at most four, gone after a few seconds or by their close button, with one optional action. Signed-in code raises them with `services.toast(…)`; the app owns `Toasts` and draws them over everything.
- **Playback** (`playback.rs`): `Services::play` → `run_session` on tokio negotiates `PlaybackInfo` (direct play, or the HLS `TranscodingUrl` with its `ApiKey` stripped), drives the `Player`, reports progress, and updates `NowPlaying`, the state the player page draws and controls. Sessions are numbered (`session`), so a replaced session's end doesn't close its successor. A session that fails after starting (`Ended::Lost`) is restarted at its position (network drops and player-process crashes). Speed, delays and subtitle size are `Adjustments` (speed and size carry to the next item, delays don't). There's a local play queue (Play all, remote play next/last), autoplay of the next episode, and in a SyncPlay group `play` is redirected to the group's queue and pause/seek/stop become group requests (`Services::control`).
- **SyncPlay, remote control and the websocket:** `live.rs` keeps the session websocket and hands `SyncPlayGroupUpdate` / `SyncPlayCommand` to `Services::on_syncplay_*` and remote messages to `Services::on_remote_*` (both in `tick_live`); `UserDataChanged` coalesces into one bump of `services.user_data`, half a second later, and none during playback. Remote control goes through the same paths as the player's own controls, so a group gets it too; unknown general commands are logged and ignored.
- **Images and video:** `images.rs` keeps the disk cache and a 256 MB LRU of decoded images as egui textures, with blurhash placeholders; a page asks for the size it draws (`ImageRequest::fill_width`, twice the drawn width), and that size is part of the cache key. `widgets/video.rs` draws the player's frames: software pixels written into an `Rgba8Unorm` texture, or the player's own GPU textures imported into wgpu (`widgets/video/vulkan.rs`, `dx12.rs`). `widgets::video::import` tells the player what this device can take when the session starts it; a failed import falls back to software frames for the rest of the sign-in (`Services::fall_back_to_pixels`).
- **Config:** `config.rs` (TOML in the platform config dir; `TABBYFIN_CONFIG_DIR` overrides it). Tokens live in the system keychain (`credentials.rs`), never in the config; without a keychain (WSL) they fall back to `tokens/` inside the config dir. The config lists the saved accounts (`accounts`, keyed by `server_id/user_id`) and the one restored at launch (`server`); `App` switches between them, parking the signed-in screen while another account is added. Logs go to stderr and a daily file (`config::log_dir()`).

## Testing in this environment

Development happens in WSL2 without a GPU (Linux video falls back to software). Debug builds have knobs for driving the app without touching the desktop's mouse or keyboard — never simulate real input: `TABBYFIN_ROUTE` (open a page or `play:<id>[:audio[:sub[:source]]]`), `TABBYFIN_KEYS` (replay keystrokes inside the app, on any screen), `TABBYFIN_SYNCPLAY` / `TABBYFIN_SYNCPLAY_SKEW_MS`, `TABBYFIN_MPV_OPTIONS` (extra mpv options, e.g. `ao=null;cache=no` for network-drop tests), `TABBYFIN_VIDEO=software|gpu`, `TABBYFIN_MPV_LOG`, `TABBYFIN_UPDATE_URL` (the update check against a local fake release, every launch). Screenshots: run without `WAYLAND_DISPLAY` (X11) and use `import -window`. Only use the local dev servers for playback tests. `dev/README.md` describes all of these.

- The UI tests (`crates/app/src/ui_tests/`) drive the real pages headless with `egui_kittest`, no window, GPU or network: the captured fixtures are served by `wiremock`, and what's checked is behaviour (which card the cursor lands on, what a key saves to the config), not images. A test that reads the config takes a `ConfigDir` first, since `TABBYFIN_CONFIG_DIR` is process-wide.
- To try a config change without touching the user's, copy the config dir (including `tokens/`) to the scratchpad and point `TABBYFIN_CONFIG_DIR` at it. Test runs save the volume and window placement, so put back anything a run changed in the real config.
- Remote control is tested by driving the app's session over REST the way the dashboard does (`POST /Sessions/{id}/Playing…`, `/Command`, `/Message`, finding the session by the config's `device_id`) and reading `GET /Sessions`.
- The menus that take the pointer only (the item menu, the player's menus, the Settings pickers' lists) and hover can't be reached with `TABBYFIN_KEYS`; for a screenshot, open them by default temporarily and revert before committing.

## Conventions

- Match the surrounding comment style: doc comments explain why, plain English, no filler.
- Keep `docs/ROADMAP.md` (✅ markers), `docs/PLAN.md` and `docs/TESTING.md` in step with behavior changes.
