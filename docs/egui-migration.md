# Moving Tabbyfin to egui: plan

**Status (2026-10-25):** F1 (the foundation) is done; the other tasks are open. Work happens on the `egui` branch; `main` stays the GPUI app until the switch is done.

**Approach:** replace in place. The first task (F1) deletes the GPUI code from `crates/app` and sets up the egui app. Every later task ports one piece back. The old code stays readable on `main`:

```sh
git show main:crates/app/src/views/item.rs     # the GPUI version of a file
git grep -n 'fn on_key_down' main -- crates/app # search the old code
```

The bar is the same features as `main`, not the same look: egui's default widgets and styling first, restyling later (see [Restyle backlog](#restyle-backlog)).

---

## 1. Decisions

| Concern | Choice | Why |
|---|---|---|
| UI | `eframe` / `egui` **0.37** | Current release. Its text shaping (harfrust/skrifa) and on-demand system font fallback (`egui_system_font_provider`, the `system_fonts` feature, on by default) cover CJK and other scripts without bundling fonts. |
| Renderer | **wgpu** (eframe's default), wgpu 30 | Native API per OS (DX12, Vulkan, Metal), so mpv's frames can be shown without a copy everywhere. egui-wgpu takes outside textures (`Renderer::register_native_texture`), so no patched UI framework. `third_party/gpui/` goes away. |
| Backends | DX12 on Windows (forced; wgpu might otherwise pick Vulkan), Vulkan on Linux with GL as fallback, Metal on macOS | Windows imports D3D11 textures into D3D12; Linux imports Vulkan memory. On a GL fallback only the software video path works. |
| Look | egui's default look | Reach the same features first. |
| Icons | `egui_material_icons` 0.9 (egui 0.37) | `egui-phosphor` is still on egui 0.36. Fallback: the Lucide SVGs through `egui_extras`' SVG loader. |
| Toasts | Our own small module | `egui-notify` is still on egui 0.36. |
| UI tests | `egui_kittest` 0.37 (dev dependency) | Drives the UI headless through AccessKit, without a GPU when no snapshots are taken. |
| Unchanged | `crates/jellyfin`, the player host process, `run_session`'s logic, config file format, all env knobs (`TABBYFIN_ROUTE`, `TABBYFIN_KEYS`, …) | Users keep their config and accounts; dev workflows keep working. |

Facts checked against the 0.37 / wgpu 30 sources:
- `eframe::App` has `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)` (required), `fn logic(&mut self, ctx, frame)` (runs before each `ui`, and also while the window is hidden whenever a repaint was requested), and `fn raw_input_hook(&mut self, ctx, raw_input)`.
- `frame.wgpu_render_state()` gives the device, queue, adapter and `Arc<RwLock<egui_wgpu::Renderer>>`.
- `egui_wgpu::Renderer`: `register_native_texture(device, &TextureView, FilterMode) -> TextureId`, `update_egui_texture_from_wgpu_texture(...)`, `free_texture(&TextureId)`.
- wgpu-hal 30: `dx12::Device::texture_from_raw(ID3D12Resource, format, dimension, size, mips, samples)`, `dx12::Device::raw_device()`; `vulkan::Device::texture_from_raw(image, desc, drop_callback, memory: TextureMemory)` (the `memory` argument is new since the wgpu 29 the GPUI patch used). wgpu-hal 30 uses `windows` 0.62 and `ash` 0.38, the same versions the player crate already uses.
- wgpu 30 has `Features::VULKAN_EXTERNAL_MEMORY_FD`: the device must be requested with it to import fds.
- epaint fills **convex** paths only. The mascot's outlines are concave, so porting it needs a tessellator (`lyon_tessellation`) into an `egui::Mesh` (restyle task R3).

---

## 2. Model tiers

Every task below has a tier:

- **S** — smart model: Opus 5.5, at medium to xhigh effort. Architecture, anything other tasks copy, concurrency and timing, GPU interop.
- **C** — cheap model: DeepSeek Flash 4.1 on pi ([below](#how-the-cheap-tasks-run)), at thinking level high to max. A clear spec plus the old code to port from.
- **C+R** — cheap model implements, smart model reviews the diff with the [review checklist](#5-review-checklist-for-cr-tasks) before the task counts as done, Opus at medium to xhigh. Used where the port is clear but easy to get subtly wrong.

Each task's levels are in the [overview](#overview). Opus effort is set per session (`/effort`) or per subagent; pi's with `--thinking`.

A task is done when `mise run ci` passes (fmt, clippy `-D warnings`, tests), its acceptance checks pass, and (for C+R) the review is through.

**Running tasks in parallel:** give each task its own worktree branched off `egui`, and merge back in wave order. F1 declares every module up front, so later tasks rarely touch the same files. Shared files that do get touched (`services.rs`, `keys.rs`) should only gain lines.

### How the cheap tasks run

The smart model (the coordinator) starts each cheap task as a headless pi run with DeepSeek Flash 4.1, one worktree per task, then reviews the result. No Orca or other orchestrator is needed.

```sh
mkdir -p ../tabbyfin-tasks/{logs,reports,sessions}
git worktree add ../tabbyfin-tasks/L1 -b egui-L1 egui
cd ../tabbyfin-tasks/L1 && mise trust
pi -p --model opencode-go/deepseek-v4.1-flash --thinking high \
   --session-dir ../sessions/L1 --name L1 \
   "Do task L1 in docs/egui-migration.md. Read §3 and 'How to brief a task' first. When done, write what you did, what you dropped and why, and what's unverified to ../reports/L1.md." \
   < /dev/null > ../logs/L1.log 2>&1
```

- **`< /dev/null` is required.** In print mode pi reads piped stdin before starting, and a shell's open stdin (a socket or pipe) never ends, so pi waits forever without output.
- pi picks up `AGENTS.md` by itself, and reads the plan with its own tools, so the prompt only names the task.
- Long tasks: run the command in the background and wait for it to exit. A trivial edit takes seconds; a port takes minutes.
- `--session-dir` keeps the transcript, so the reviewer can see what was run and tried, not just the diff. `pi --export <session file>` turns one into HTML.
- Always pass the task's level from the overview with `--thinking` (high, xhigh or max). Without it pi uses its configured default, which is `max` on this machine.
- pi can't ask questions in print mode. A task that hits an open decision should stop and say so in its report rather than guess; the coordinator answers and starts it again on the same worktree (`--continue` with the task's own `--session-dir` keeps its context; one folder per task, so it can't pick up another task's session).
- Each worktree builds its own `target/`: parallel builds don't wait on each other, but each costs several GB. A shared `CARGO_TARGET_DIR` saves the space and makes concurrent builds take turns.
- The coordinator reviews (C+R tasks against §5), runs `mise run ci` itself rather than trusting the report, and merges the branch back into `egui` once the work is accepted.

---

## 3. Target architecture

F1 built this; every other task follows it. The patterns below exist in code: `views/home.rs` is the exemplar page, and each module's doc comment says which task fills it.

### Layout

```
crates/app/src/
├── main.rs          logging (unchanged), player host check, wgpu setup, eframe::run_native; declares every module
├── app.rs           eframe::App: screens (restoring, sign-in, signed in), account switching (was app_view.rs)
├── services.rs      Services: everything per signed-in session, AppEvent, Services::tick
├── task.rs          Task<T>: a tokio future whose result the UI polls; channel: repaint-on-send; Deadline: a timer
├── runtime.rs       the tokio runtime (unchanged, plus `start` returning the JoinHandle)
├── keys.rs          every key binding, and who gets a key: egui or the app
├── toasts.rs        notifications
├── nav.rs           Route, NavRequest (was router.rs)
├── debug.rs         TABBYFIN_KEYS replay, TABBYFIN_ROUTE parsing (compiled in debug builds only)
├── browse.rs        the keyboard cursor (main's logic and tests) and its egui wiring
├── config.rs, credentials.rs, session.rs, update.rs, markers.rs   unchanged
├── item_menu.rs, tracks.rs                                        pure logic (L1)
├── playback.rs, syncplay.rs, remote.rs, live.rs                   services
├── images.rs, trickplay.rs                                        textures
├── theme.rs         appearance: theme, text size, zoom, animations, over_video (A1; `theme::apply` is called at launch)
├── window.rs        the window as it opens (title, icon, app id, sizes) and its saved placement (A1)
├── widgets/         text (Load<T>), card, image, shelf, menu, state, video
└── views/           mod (Page, PageCx, open, StubPage), shell, home, login, search, library, value_filters, item, settings, player, dialogs
```

### State: no entities, one owner

- The app owns `Option<Services>` while signed in (`Screen::SignedIn`), and the toasts. Pages get a `&mut PageCx` each frame: `ctx`, `services`, a `nav: Vec<NavRequest>` to push into, and `render_state: Option<&egui_wgpu::RenderState>` (the player needs it; `None` in tests). No `Rc<RefCell<…>>`, no globals.
- `Services` holds the session (client, user, server), `user_data: u64` and `events: Vec<AppEvent>`; `images` (I1), `playback` (P1), `syncplay` (P2), `remote` and `live` (P3) join it.
- Cross-service calls that GPUI did through globals (`NowPlaying::control` asking SyncPlay, `PlaybackManager::play` handing over to a group) become methods on `Services`, which can reach every part: `services.control(UiCommand)`, `services.play(&item, options)`.
- GPUI events become `AppEvent`s pushed to `services.events`; the app drains them every frame after the shell has drawn. F1 has `Toast(Toast)` and `SessionExpired`; tasks add theirs (player started or ended, toggle fullscreen…). Shortcuts: `services.toast(…)`, and `services.handle_error(&err) -> String` (the user-facing message; `Unauthorized` also pushes `SessionExpired`, as `Navigator::handle_error` did).
- The old `Changes` global becomes `services.user_data`, a counter bumped on `UserDataChanged`.
- S1's screens are `Restoring`, `SignIn(Box<Login>)` and `SignedIn(Box<SignedIn>)`, with `App::parked` holding the signed-in screen while *Add account* is showing: the parked session keeps ticking (playback and the websocket carry on), its toasts still show, its screen-changing events (`SessionExpired`, `SignOut`, `SwitchAccount`, `AddAccount`) are dropped — an expiry there is ignored while the form is up, as on `main` — and anything else waits in `services.events` for the shell, which handles it after *Back to your account* (event kinds added later default to kept). The form owns one `Task` at a time (replacing it cancels the request) and gets what a request reports before it ends — the server still starting up, the Quick Connect code — over a `task::channel` made fresh for each request. Signing out, switching accounts or expiring calls `Services::end()` first (P1 stops playback there, P2 leaves a SyncPlay group) and then drops the screen, which cancels whatever it still had running. Restores and the form's requests are polled from `App::logic`/`Login::tick`, not `ui`, so they finish while the window is minimised.

### Pages and navigation

- A page is a struct implementing `views::Page`: `title()`, `scope()` (its key bindings, see Keyboard), `on_key(action, cx) -> bool`, `show(ui, cx)`. The shell keeps a stack of `Box<dyn Page>` (pages stay alive while stacked, so Back keeps their state) and draws the top one in a `CentralPanel` under the top bar (`egui::Panel::top`).
- `views::open(route, cx)` makes the page for a route. Routes whose page isn't ported yet open a `StubPage` naming the task; each page task replaces its arm there.
- Pages navigate by pushing `NavRequest::Push(route)` / `NavRequest::Back` into `cx.nav`; the shell applies them after drawing and requests a repaint so the new page shows at once.
- Loading (the exemplar, `views/home.rs`): the page keeps `Load<T>` (`widgets/text.rs`), an `Option<Task<…>>`, and `loaded_at: Option<u64>`. In `show` it starts a load when `loaded_at != Some(services.user_data)`, which covers the first load, Retry (set it to `None`) and watch data changing elsewhere; then polls the task. What's on screen stays until new data arrives. Unlike `main`, a page under others doesn't reload in the background: it catches up when it's back on top.

### Async

- **Rule kept from GPUI:** UI code never awaits `jellyfin` futures. Work runs on the tokio runtime (`runtime.rs`).
- `Task<T>`: `Task::spawn(ctx, future)` runs the future on tokio and requests a repaint once its result is ready. `task.poll() -> Option<T>` hands the result over once; a panic in the future is raised again on the UI thread (as `runtime::spawn` did). Dropping the task aborts the future, like dropping a GPUI `Task`. Owners keep `Option<Task<T>>`: replacing it cancels the previous one; they set it back to `None` after `poll` returned the result:
  ```rust
  if let Some(result) = self.loading.as_mut().and_then(Task::poll) {
      self.loading = None;
      …
  }
  ```
- Long-lived streams (a playback session's updates, the websocket) are `task::channel(ctx)`: `Sender::send` requests a repaint after each send (and returns the value if the receiver is gone, the sign to stop; `Sender::closed().await` for async code), `Receiver::drain()` takes everything without waiting. `Services::tick` drains them. It's called from `App::logic`, which eframe also calls while the window is hidden whenever a repaint was requested, so playback keeps reporting while the window is minimised.
- Timers (controls hiding, search debounce, SyncPlay's scheduled commands, saving the window placement): a `task::Deadline`. `set(after)` starts or restarts it, `cancel()`, and `fire(ctx)` returns true once when it's due and otherwise calls `ctx.request_repaint_after(remaining)`. Checked in `tick` or `show`. No thread sleeps on the UI side.
- Stale results: as on `main`, owners keep a generation counter (library pages, playback sessions) and drop results for an older one.
- Idle CPU must stay near zero. Nothing calls `request_repaint()` every frame except while something moves (video, an animation, a spinner). F1 measured 0 ticks over 10 s idle on Home.

### Keyboard

**Decided in F1: egui or the app owns the keyboard, per frame.** egui moves focus by itself (Tab from nothing; the arrows only once a widget has focus) and presses the focused widget with Space or Enter; buttons don't take focus on a click (only text fields and drag values do). On a cursor page Tab would start egui focus and Enter would press the focused widget as well as open the card. So `App::raw_input_hook` decides before egui sees the frame's input, from the state the last frame left (as egui's own focus does):

- **egui keeps every key** while it's busy with keys (`keys::egui_wants_keys`): a widget has focus (a text field, or anything reached with Tab on a page without a scope), or a modal (`top_modal_layer`) or a menu, combo box or popup (`Popup::is_any_open`) is open. egui then works as egui does: Tab and the arrows move focus, Space and Enter press, Esc closes the popup or drops the focus. The code that owns such a widget reads the keys it needs itself through `ui.input_mut(|i| i.consume_key(…))`: the search box's Down and Enter (U1), a dialog's Up/Down/Enter (U8), the SyncPlay panel's keys (U2). A single-line text field that uses Up/Down sets `Memory::set_focus_lock_filter` so egui doesn't also move focus with them.
- **Otherwise the app takes the keys bound in the active scopes** (`keys::take`): the press events are removed from egui's input, so egui never sees them, queued, and routed by the shell before anything is drawn. Unbound keys (and all key releases) stay with egui.
- `keys.rs` has one table, `BINDINGS`: `(Scope, modifiers, key) → Action`, holding everything `main.rs` and `browse::key_bindings` had. Scopes replace GPUI's key contexts: `Shell` (Back on Esc, Backspace and Alt+Left; search on Ctrl/Cmd+F), `Browse` (arrows, Tab, Shift+Tab, Enter), `Player`. The active scopes are the top page's `Page::scope()` and `Shell`. Modifiers match exactly (Alt+Left isn't Left), except on punctuation (`[`, `]`, `+`, `-`, `=`), which some layouts type with Shift or AltGr. Tasks that need more keys (W on the item page, Alt+Up/Alt+Down/Delete on playlists) add bindings and actions there.
- A page without the cursor (Settings, U9) returns `None` from `scope()`: Tab, the arrows, Space and Enter aren't bound in `Shell`, so they stay with egui and its focus works there, while Esc and Backspace still go back when nothing has focus.
- Routing order: open dialog, menu or focused text field (through egui, above) → top bar while the keyboard is in it (U1) → the page (`Page::on_key` with the action for its scope; returns whether it used it) → the shell's own keys (Back, search). An Up the page didn't use (`Browse::at_edge`) is where U1 moves into the top bar.
- The browse `Cursor` (pure logic, `main`'s tests kept, ported in F1) stays. A page implements `browse::Browse` (`cursor`, `sections`, `activate`, `adjust`, `at_edge`) and makes `browse::on_key` its `Page::on_key`. While drawing, the selected card calls `browse::reveal(cursor, &response)`, which calls `response.scroll_to_me` once after each move (replacing `browse::reveal` and its scroll handles), and the page calls `browse::hide_on_click(cursor, ui)`.
- The cursor stays hidden until a key is pressed and hides again on a click, as now.

### Images and video

- `ImageCache` keeps its budget, queue and blurhash placeholders, but holds `egui::TextureHandle`s. Dropping a handle frees the texture, so GPUI's atlas workaround goes. Decoding stays off the UI thread; `egui::Context` can upload textures from any thread.
- Video is a widget that owns its textures in egui-wgpu's renderer (`register_native_texture`): software frames are written with `queue.write_texture` into one reused texture (not through egui's texture deltas, which would copy every frame twice). GPU frames are imported textures, cached per ring slot.
- The app tells the player host what it can import when it starts it (V1): pixels only, Vulkan memory from a given device, or D3D11 textures on a given DX12 adapter. The host then picks a matching path, so it never produces frames the UI can't show.
- wgpu setup (`main.rs`): backends `DX12` on Windows, `METAL` on macOS, `VULKAN | GL` elsewhere (`WGPU_BACKEND` still overrides). wgpu already prefers Vulkan over GL for the same kind of device, so no adapter selector is needed. The device is requested with `VULKAN_EXTERNAL_MEMORY_FD` when the adapter has it. On WSL it picks Vulkan on llvmpipe.

### Toasts

`toasts.rs`: `Toast::info/success/error(text)`, `.title(…)`, `.action(label, ToastAction)`. They stay 5 s (errors 8 s), longer while the pointer is on them, at most four at once, and have a close button. The app owns `Toasts` and draws them over everything; a clicked action comes back from `Toasts::show` for the app to carry out (`ToastAction::OpenUrl` for the update notice). Signed-in code raises them with `services.toast(…)`. Each toast is logged.

### Dev knobs, kept as they are

`TABBYFIN_ROUTE` (pages and `play:`), `TABBYFIN_KEYS`, `TABBYFIN_SYNCPLAY`, `TABBYFIN_SYNCPLAY_SKEW_MS`, `TABBYFIN_MPV_OPTIONS`, `TABBYFIN_VIDEO`, `TABBYFIN_MPV_LOG`, `TABBYFIN_UPDATE_URL`, `TABBYFIN_CONFIG_DIR`, `TABBYFIN_PLAYER=embedded`. Screenshots as before: no `WAYLAND_DISPLAY` (X11), `import -window`.

- `TABBYFIN_KEYS`: same syntax and timing as `main` (first key after 3 s, `wait` 1 s, `key*N` 30 ms apart, 250 ms after each key; keys named `down`, `shift-tab`, `ctrl-f`, `alt-left`, `pageup`, `]`, `a`…). `debug::KeyScript` injects each press as `egui::Event::Key` (press and release, plus `Event::Text` for printable keys, so typing reaches a focused text field) in `App::raw_input_hook`, before `keys::take`, so scripted keys take the same path as real ones. `App::logic` requests a repaint for when the next one is due.
- `TABBYFIN_ROUTE`: parsed by `debug::Start` (`settings`, `item:<id>`, `library:<id>[:movies|tvshows]`, `search:<term>`, `play:<id>[:audio[:sub[:source]]]`); the shell opens it over Home. Until U1 and P1, `search:` opens the Search stub and `play:` opens the item without playing.

### During the migration

- `main.rs` has a temporary `#![allow(dead_code)]`, since code arrives before its callers. X3 removes it.
- Each task updates the docs its change touches (`docs/ROADMAP.md`, `docs/PLAN.md`, `docs/TESTING.md`), as AGENTS.md asks; X2 does the full rewrite at the end.
- Hooks already in place so tasks don't need `main.rs`: `window::viewport(&config)` (the launch size, A1 adds the saved placement), `window::Placement` (`tick` from `App::logic`, `save_now` from `App::on_exit`), `theme::apply(ctx, &config)` (from `App::new`).

---

## 4. Tasks

### Overview

*Does it* is the implementer and its effort: Opus effort for S tasks, pi's `--thinking` level for DeepSeek Flash. *Review* is the Opus effort for the C+R review.

| ID | Task | Tier | Does it | Review | Needs | Wave |
|---|---|---|---|---|---|---|
| F1 | Foundation: remove GPUI, eframe skeleton, core patterns | **S** | Opus xhigh | — | — | 0 |
| L1 | Carry over the GPUI-free logic and its tests | C *(mechanical)* | pi high | — | F1 | 1 |
| A1 | Theme, text size, zoom, fonts, window placement | C+R | pi high | Opus medium | F1 | 1 |
| I1 | Image cache and trickplay thumbnails | C+R | pi xhigh | Opus high | F1 | 1 |
| S1 | Sign-in, saved accounts, session restore and switching | C+R | pi xhigh | Opus high | F1 | 1 |
| P1 | Playback sessions (PlaybackManager, NowPlaying) | **S** | Opus high | — | F1, L1 | 1 |
| P2 | SyncPlay | **S** | Opus xhigh | — | P1 | 2 |
| V1 | Video widget, software frames, import handshake | **S** | Opus high | — | P1 | 2 |
| W1 | Shared widgets: cards, shelves, images, states, item menu | C+R | pi xhigh | Opus high | L1, I1 | 2 |
| P3 | Session websocket, remote control, user-data refresh | C+R | pi xhigh | Opus high | P1, P2 | 3 |
| U1 | Shell: top bar, page stack, search box, account menu | C+R | pi max | Opus high | W1, S1 | 3 |
| U8 | Dialogs: add to playlist, edit playlist, confirm delete | C | pi high | — | W1 | 3 |
| V2 | Linux GPU frames (Vulkan import) | **S** | Opus high | — | V1 | 3 |
| V3 | Windows GPU frames (DX12 import, NT handles) | **S** | Opus xhigh | — | V1 | 3 |
| X1 | Packaging and CI | C+R | pi xhigh | Opus medium | F1 | 3 |
| U2 | SyncPlay panel in the top bar | C+R | pi xhigh | Opus medium | U1, P2 | 4 |
| U3 | Home | C | pi high | — | U1 | 4 |
| U4 | Search | C | pi high | — | U1 | 4 |
| U5 | Library grid, sort and filters | C+R | pi max | Opus high | U1 | 4 |
| U6 | Item page: header, actions, pickers, shelves | C+R | pi xhigh | Opus medium | U1, P1 | 4 |
| U9 | Settings | C | pi high | — | U1, A1, I1, P1 | 4 |
| U10 | Player page | C+R | pi max | Opus xhigh | V1, W1, I1 | 4 |
| U7 | Item page: seasons, episodes, playlist entries | C+R | pi max | Opus high | U6, U8 | 5 |
| X4 | UI tests with egui_kittest | C | pi xhigh | — | U1–U5 | 5 |
| X3 | Parity sweep and cleanup | C runs it, **S** triages | pi high | Opus medium (triage) | everything | 6 |
| X2 | Docs | C *(mechanical)*, R for `in-app-video.md` | pi high | Opus medium | X3 | 6 |

**Why these levels:** Opus xhigh where a mistake spreads or is hard to see (F1's patterns copied everywhere, SyncPlay timing, the untested Windows interop, the player's many interactions); medium where the review is mostly checking a list. pi max where one page holds intricate state and keyboard handling (shell, library, playlist editing, player); high where the port is near line for line.

**Waves** (tasks in a wave can run at the same time):

```
0  F1
1  L1  A1  I1  S1  P1
2  P2  V1  W1
3  P3  U1  U8  V2  V3  X1
4  U2  U3  U4  U5  U6  U9  U10
5  U7  X4
6  X3  X2
```

Smart-model work: F1, P1, P2, V1, V2, V3, X3 triage, and every C+R review. V3 needs the Windows machine (`dev/win-run.sh`, RX 6600); V2 is only partly testable in WSL.

### How to brief a task

Give the agent: this file, `AGENTS.md`, the task's section below, and the files it names on `main`. Every brief implies:

1. Read the old file(s) on `main` in full first. Each doc comment there describes a behavior; list them, port each, and say in the task's final message which ones were dropped or changed and why.
2. Keep tests that exist for the code being ported, rewritten for egui types where needed.
3. Check against the dev server (`mise run dev:setup`; a copy of the config dir via `TABBYFIN_CONFIG_DIR`), with `TABBYFIN_ROUTE` / `TABBYFIN_KEYS` and a screenshot where the task has UI.
4. Don't change `crates/jellyfin`, and don't change `crates/player` unless the task says so.

---

### F1 — Foundation **[S]**

**Goal:** the egui app runs, signs in with a saved account and shows one real page, and the patterns in §3 exist in code for everyone else to copy.

**Do:**
- Workspace: remove `gpui-kit`, the `[patch.crates-io]` section, `third_party/gpui/`, and the Windows `rust-embed` workaround. Add `eframe`/`egui`/`egui-wgpu` 0.37 and `egui_material_icons`. Keep `image`, `blurhash`; drop `smallvec` if unused.
- `main.rs`: logging unchanged, `player::is_host_process()` first, then `eframe::run_native` with: title "Tabbyfin", a Wayland app id matching the installed desktop file (`tabbyfin` for the `.deb` and AppImage; the Flatpak installs it as `io.github.lgpy.tabbyfin.desktop`, so use `FLATPAK_ID` when it's set), the window icon from `packaging/tabbyfin.png`, minimum size 800×500, the default size 1280×800, and a wgpu setup that forces DX12 on Windows and requests `VULKAN_EXTERNAL_MEMORY_FD` where the adapter has it.
- Delete every GPUI module in `crates/app/src` except `config.rs`, `credentials.rs`, `session.rs`, `runtime.rs`, `update.rs`, `markers.rs` (GPUI-free; fix doc comments that mention GPUI). Create the §3 layout with every module declared, each with a doc comment saying which task fills it, so later tasks don't edit `main.rs`.
- Build `services.rs`, `task.rs` (`Task<T>`, `Deadline`, repaint-on-send senders), `keys.rs` (routing order, the `raw_input_hook` decision about egui's own focus keys, written into §3), `toasts.rs` (info, success, error; auto-dismiss; an optional action button, which the update notice needs), `nav.rs`, `debug.rs` (`TABBYFIN_KEYS`, `TABBYFIN_ROUTE` parsing).
- `app.rs`: restore the saved account (token from `credentials`, off the UI thread; `public_info` + `current_user`), then a minimal shell (title, Back) with **one exemplar page**: the user's libraries as a list loaded through `Task`, reloading when `user_data` changes, a click opening a stub page via `nav`, Esc going back, and the browse cursor working on it (Left/Right/Enter, hidden until a key is pressed). Without a saved account show "Sign-in arrives with S1".
- Add a banner at the top of `AGENTS.md`: on the `egui` branch the GPUI sections are out of date, see this file. Agents must not load the gpui-kit skills on this branch.

**Acceptance:** `mise run ci` green. With a copy of a signed-in config, `TABBYFIN_KEYS="wait right right enter wait escape"` moves the cursor, opens the stub page and comes back, which a screenshot shows. The process is near 0% CPU when idle. §3 matches what was built.

---

### L1 — Carry over the GPUI-free logic **[C, mechanical]**

**Goal:** the pure functions (and their tests) the views need, back in their old modules, free of GPUI types.

**From `main`, keep the module names:**
- `browse.rs`: done in F1 (the cursor and its tests came over with the egui wiring).
- `tracks.rs`: `SUBTITLES_OFF`, `Choice` (label as `String`), `track_choices`. Not the select helpers.
- `item_menu.rs`: `Mark`, `is_played`, `is_favorite`, `resume_point`, `is_playable`, `first_unwatched`, `episodes_of`, `can_add_to_playlist`, `can_mark`, their tests.
- `widgets/text.rs` (from `components.rs`): `card_text`, `episode_label`, `timestamp`, `runtime_text`, `poster_image`, `landscape_image`, `kind_label`, `is_placeholder_episode_name`, the size constants, tests. (`Load<T>`, with `String` errors, is there already from F1.)
- `views/library.rs`: `SORTS`, `FILTERS`, `RESOLUTIONS`, `year_choices`, `years_of`, `Jump`, `jump_target`, `chosen`, tests.
- `views/home.rs`: `HomeSection`, `DEFAULT_SECTIONS`, `home_sections`, tests.
- `views/item.rs`: `shuffle`, `collection_items`, `meta_line`, `chapters`, `item_actions`, `more_actions`, `ItemAction`.
- `remote.rs`: `in_order` and its test. `views/player.rs`: `scroll_steps`, its constants and tests.

**Acceptance:** all moved tests pass; no `gpui` in the crate; nothing else changed.

---

### A1 — Appearance and window **[C+R]**

**From `main`:** `theme.rs`, `zoom.rs`, `window.rs`, `motion.rs` (only the setting), the theme/text-size parts of `app_view.rs`, and `main.rs`.

**Do:**
- Theme: dark, light, or following the system (egui follows the OS theme natively), applied at launch and when the setting changes.
- Text size: the setting (90–150 %, or the system's, read as on `main`: Windows registry, GNOME `gsettings`, KDE `kdeglobals`; re-read when the window regains focus) scales egui's text styles, not the layout.
- Zoom: Ctrl+= / Ctrl+- / Ctrl+0 (Cmd on macOS) through `ctx.set_zoom_factor` on the old levels, saved as `ui_zoom`. Turn off egui's own zoom keys if they'd double-apply.
- Animations setting: off sets `style.animation_time = 0` and turns off scroll animation.
- `theme::over_video`: the same tokens as `Color32` constants (the player stays dark in both themes).
- System font fallback: confirm CJK and Arabic titles render (the provider loads them on demand); add it explicitly if eframe's default doesn't.
- Window placement: restore size, position and maximized state at launch; save 500 ms after the last change and on close; fullscreen leaves the saved placement alone. Port `remember`, `placed_bounds`, `lands_on_display` and their tests onto `egui::Rect`. eframe doesn't list monitors before the window opens: decide how to keep a window saved on a now-disconnected monitor from opening off screen (e.g. check the outer rect against the monitor size reported after the first frame and recentre), and note what was chosen in `docs/PLAN.md`.

**Acceptance:** each setting round-trips through the config; `docs/TESTING.md` §9 (window placement) and §14 (theme) pass where WSL can check them; tests ported.

---

### I1 — Image cache and trickplay **[C+R]**

**From `main`:** `images.rs`, `trickplay.rs`.

**Do:**
- `ImageCache::get(&ImageRequest) -> Option<(TextureId, Vec2)>` (or a handle), `placeholder(blurhash)`, `clear_memory()`; the same disk cache paths, `MAX_LOADS`, the most-recent-first queue that drops stale entries after a frame, `MEMORY_BUDGET` by decoded bytes, `KEEP_RECENT`, the delayed trim. Decode RGBA (no BGRA swap any more). Mipmaps on (`TextureOptions` with a mipmap mode), since posters are drawn smaller than their pixels.
- The queue is pumped in `Services::tick` after the frame that asked, the same as the GPUI `defer` did.
- Trickplay: the same sheet loading, cropping, `KEEP_SHEETS` and `locate`, as textures.
- Keep all tests (`Lru`, `next_load`, `locate`).

**Acceptance:** a test page (or the F1 exemplar) shows posters with blurhash placeholders. With `dev/fake-library.py` and a fast scroll (`TABBYFIN_KEYS="… down*150"` once U5 exists, or a scripted scroll now), memory stays under the budget and stale loads are skipped.

---

### S1 — Sign-in and accounts **[C+R]**

**From `main`:** `views/login.rs`, `app_view.rs`.

**Do:** port every flow in both files into `app.rs` and `views/login.rs`: probing `https://` then `http://`, waiting while the server starts, the minimum version check, username and password (password field masked with a show toggle; Enter moves from username to password, then signs in), Quick Connect with polling and expiry, saved accounts (continue as, remove), the pre-filled form after sign-out or expiry, waiting for an unreachable server and signing back in by itself, *Add account* (keeping the signed-in shell parked, with *Back to your account*), switching accounts, signing out (revoking the token), session expiry (from `NavRequest`/`AppEvent`), the restoring screen (a spinner for now; the mascot is R3), `report_unfinished` on sign-in. Token storage stays off the UI thread.

**Acceptance:** each flow works against the dev server (`dev`/`dev`, `dev2`/`dev2`); Quick Connect approved from the web client; stopping the server during restore shows the waiting message and signs back in when it's back; `docs/TESTING.md` §24 (several accounts).

---

### P1 — Playback sessions **[S]**

**From `main`:** `playback.rs` (all of it), the parts of `views/shell.rs` and `views/player.rs` that react to playback events.

**Do:**
- `PlaybackManager` and `NowPlaying` become plain structs in `Services`. `run_session`, `Reporting`, `apply_command`, bitrate measuring, subtitle look, `save_unfinished` / `report_unfinished`: unchanged.
- Replace `cx.spawn` and entity updates with the session update channel drained in `Services::tick`, keeping the generation rule (a replaced session's end doesn't close its successor), reconnects after `Ended::Lost` with the same delays (a `Deadline`, cancelled by stop), the queue, autoplay, previous/next details, adjustments carried to the next item, volume and subtitle size saved at the end.
- Methods that needed SyncPlay go on `Services` (`control`, `play`, `play_all_with`, `enqueue`, `reload`, `play_next`, `play_previous`, `start_over`, `change_audio`, `change_subtitle`, `adjust`). Until P2 lands, the SyncPlay branches call a stub that reports "not in a group".
- Events: `Started`, `Ended`, `Failed`, `Notice` as `AppEvent`s.
- Keep every test.

**Acceptance:** `TABBYFIN_ROUTE=play:<id>` starts a session (no video widget yet: log the frames arriving) and reports start, progress and stop to the dev server (`GET /Sessions`); killing the player host restarts at the same position; a network drop (`dev/README.md`) reconnects; autoplay moves to the next episode; the non-UI parts of `docs/TESTING.md` §3, §5b and §7b.

---

### P2 — SyncPlay **[S]**

**From `main`:** `syncplay.rs`.

**Do:** port it onto `Services`: group state and listing, requests (`call` logging failures as toasts), queue updates loading items paused, `Buffering`/`Ready` reports, scheduled Unpause/Pause/Seek/Stop at the server's `When` (a `Deadline` checked in `tick`; replacing it cancels the previous), the clock task (4 samples, then 1 a minute, ping), drift correction every second with the same thresholds, `Ready` after the player's next restart (watch `NowPlaying.restarts` in `tick` instead of GPUI's observe), `reload` for a version or track switch, `TABBYFIN_SYNCPLAY` and `TABBYFIN_SYNCPLAY_SKEW_MS`. Replace P1's stub.

Scheduled commands are only as precise as `tick` runs: `request_repaint_after` wakes the UI loop late by up to a frame, which is fine for drift correction but not for the Unpause start. If that's too late, schedule the command on tokio (sending the `UiCommand` straight to the session's command channel at the deadline) and only report back through `tick`.

**Acceptance:** the scripted member (`cargo run -p jellyfin --example syncplay`) and the app, both ways round, as in `docs/ROADMAP.md` §2.4.4; with `TABBYFIN_SYNCPLAY_SKEW_MS=800` drift is back under 50 ms within about 20 s; `docs/TESTING.md` §6.

---

### V1 — Video widget, software frames, import handshake **[S]**

**From `main`:** the frame handling in `views/player.rs`; `crates/player/src/libmpv/video.rs`, `host.rs`, `remote.rs`, `wire.rs`.

**Do:**
- A `widgets/video.rs` widget given the `VideoOutput` and the `RenderState`: keeps the frame receiver, wakes the UI on new frames (a tokio task on `frames.changed()` that requests a repaint), reports the target size in physical pixels (`set_target_size`), draws the latest frame letterboxed (contain), and frees its textures when dropped.
- Software frames: one texture per size, written with `queue.write_texture`, registered once with `register_native_texture`. Switch the player's software output to RGBA (`rgb0`, alpha forced to 255) so the texture is `Rgba8Unorm` on every backend, and update the wire comments.
- The handshake: `Player::spawn(import)`, where `import` is what the UI can take: pixels only, Vulkan memory from the device with this UUID, or D3D11 textures for the DX12 adapter with this LUID (filled from the wgpu adapter's hal info). Pass it to the host (`--player-host` arguments or the first wire message) and on to `render_thread`, which skips GPU paths that don't match. For now the app always asks for pixels; V2 and V3 turn their paths on.

**Acceptance:** software video plays in the window on Linux (WSL) and Windows (`TABBYFIN_VIDEO=software`); resizing re-renders at the new size; CPU use is no worse than `main`'s software path (`docs/in-app-video.md` has the numbers); `TABBYFIN_PLAYER=embedded` works too.

---

### V2 — Linux GPU frames **[S]**

**From `main`:** `third_party/gpui/gpui-pre-wgpu/src/shared_texture.rs` and its renderer changes, `crates/player/src/libmpv/egl.rs`.

**Do:** port the fd import to wgpu 30 (the new `TextureMemory` argument; a drop callback freeing the image and memory), cache one imported texture and egui texture id per ring image id, free them when the ring changes. Only when wgpu runs on Vulkan with `VULKAN_EXTERNAL_MEMORY_FD`: then ask the host for Vulkan frames from that device's UUID, and have `egl.rs` pick its exporter device by that UUID. Keep the software-rasterizer rule (`TABBYFIN_VIDEO=gpu` forces it).

**Acceptance:** with `TABBYFIN_VIDEO=gpu` on WSL (lavapipe), frames show and aren't black; with wgpu on GL the app falls back to software frames by itself; `docs/TESTING.md` lists the real-GPU checks (AMD, Intel, NVIDIA; X11 and Wayland) still to do.

---

### V3 — Windows GPU frames **[S]**

**From `main`:** `crates/player/src/libmpv/angle.rs`, `wgl.rs`, `opengl.rs`, `third_party/gpui/gpui-pre-windows` (the surface drawing), `examples/dx_interop_probe.rs`.

**Do:**
- D3D12 can't open the legacy share handles the host makes now. Create the ring textures with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED` and share them with `IDXGIResource1::CreateSharedHandle`: named handles (opened in the app with `ID3D12Device1::OpenSharedHandleByName`) avoid duplicating handles across processes. Change the `SharedHandle` wire message to match.
- In the app: open each texture on wgpu's DX12 device (`raw_device()`, `OpenSharedHandle` → `ID3D12Resource`), wrap it (`texture_from_raw`, `create_texture_from_hal`), cache per ring slot, register with egui-wgpu. Check the resource state wgpu assumes for an imported texture.
- Same adapter on both sides: pass the wgpu adapter's LUID to the host; ANGLE takes it as a display attribute (`EGL_PLATFORM_ANGLE_D3D_LUID_HIGH_ANGLE` / `_LOW_ANGLE`), the WGL path creates its D3D11 device on the DXGI adapter with that LUID. This also covers hybrid laptops.
- Sync stays `glFinish` plus the 3-texture ring, as on `main`; if frames tear, add shared fences (ROADMAP 2.3.7).
- Order and fallback as now: ANGLE, then WGL interop, then software.

**Acceptance:** on the RX 6600 through `dev/win-run.sh`: ANGLE with `d3d11va` and the WGL path (ANGLE hidden) both show video; CPU numbers in `docs/in-app-video.md` hold (≈11 % for 4K HEVC 10-bit with ANGLE); killing the host mid-play recovers.

---

### W1 — Shared widgets **[C+R]**

**From `main`:** `components.rs`, `item_menu.rs` (the menu), `rail_drag.rs`, `smooth_scroll.rs`, `motion.rs` (only to know what's dropped).

**Do:**
- Image widget: cover fit (crop through the UV rect, keeping the aspect ratio), rounded corners, the blurhash underneath while loading.
- Cards: poster, landscape, episode, chapter and person cards with the old sizes and scales, title and subtitle lines (truncated), the played badge, the progress bar, hover dimming, the keyboard selection ring, scroll-into-view when selected after a key.
- Shelf: titled horizontal row (`ScrollArea::horizontal`, drag to scroll on, a drag never counts as a click) with the previous/next page buttons, hidden when everything fits and disabled at the ends.
- States: centred spinner, message, empty state, error with *Retry*.
- The item context menu (`response.context_menu`): play or resume, play from beginning, Play for a series or season (its next episode), Watched and Favorite (checked), Add to playlist… (opens U8's dialog). Kinds without these get no menu.
- Watched and Favorite icons: filled, green and red, as on `main`.

`rail_drag.rs` and `smooth_scroll.rs` aren't needed: egui's scroll areas do both. Note that in the task's final message.

**Acceptance:** the F1 exemplar page shows its libraries as landscape cards in a shelf; right-click gives the menu on an item card.

---

### P3 — Websocket, remote control, refresh **[C+R]**

**From `main`:** `live.rs`, `remote.rs`, `changes.rs`.

**Do:** the socket task (capabilities reported on each connect), `SyncPlayGroupUpdate`/`SyncPlayCommand` to SyncPlay, `Play`/`Playstate`/`GeneralCommand` to remote control, `UserDataChanged` coalesced over 500 ms and ignored during playback (bumps `services.user_data`). Remote control: every command in `remote.rs` through the same `Services` paths as the player's controls; messages as toasts; `ToggleFullscreen` as an `AppEvent`; unknown commands ignored.

**Acceptance:** `docs/TESTING.md` §5, §11 and §12 (driving the session over REST, `GET /Sessions`), including play next/last and a message.

---

### U1 — Shell **[C+R]**

**From `main`:** `views/shell.rs` (all but the SyncPlay panel), `router.rs`.

**Do:** the top bar (logo, Back when there's somewhere to go, the page title, the search box, SyncPlay button, Settings that toggles, the account button with its initial), the page stack (pages stay alive while stacked), the player page replacing the bar while it plays, leaving the player stopping playback, closing the player leaving fullscreen. Search box: typing opens the results page (or returns to it), clearing it closes them, Enter or Down moves into the results, Esc in it goes back to the page; no open/close animation yet (R4). Keyboard: Up past a page's first row into the bar, Left/Right along it, Enter presses, Down or Esc back to the page with its cursor shown where it was; the account menu opens with a keyboard highlight. Account menu: saved accounts (current checked), *Add account…*, *Sign out*. Esc, Backspace and Alt+Left go back; Ctrl+F (Cmd+F) opens search. Toasts from `services.events` (the app already shows them). The update notice with *Download* (F1 already shows it). All of `debug_start_route` / `debug_search_term` (F1 parses them in `debug::Start`; U1 types the search term).

**Acceptance:** `TABBYFIN_KEYS` walks into the top bar and back; account switching works from the menu; `TABBYFIN_ROUTE=search:test` opens results.

---

### U2 — SyncPlay panel **[C+R]**

**From `main`:** `views/shell.rs` (`render_syncplay`, `group_queue`, `syncplay_panel`, `MenuEntry`, the menu keys).

**Do:** the popup: groups to join and *New group* when not in one; the group, its state, members, the queue (playing entry bold; play, move up/down, remove; shuffle and repeat toggles) and *Leave group* when in one. Keyboard highlight with Up/Down, Enter, Alt+Up/Alt+Down moving an entry (the highlight follows), Delete removing; shuffle and repeat keep the panel open. The SyncPlay button shows pressed while in a group.

**Acceptance:** with the scripted member queueing items, the panel follows; every keyboard action works.

---

### U3 — Home **[C]**

**From `main`:** `views/home.rs`. Shelves in the user's web-client section order (L1's `home_sections`), Favorites before the Latest shelves, Home's bigger cards and gaps, the cursor across shelves, reload on `user_data`, error with Retry, "Your server has no libraries yet." No staggered entry animation yet (R2). **Acceptance:** `docs/TESTING.md` §21, and the Home part of §2.

---

### U4 — Search **[C]**

**From `main`:** `views/search.rs`. Debounced 300 ms search as the box changes, Movies / Shows / Episodes groups in grids sized to the window, the cursor (Up from the first row back to the box), empty and failed states. **Acceptance:** `TABBYFIN_ROUTE=search:test` and typing through `TABBYFIN_KEYS`; the search part of `docs/TESTING.md` §2.

---

### U5 — Library grid and filters **[C+R]**

**From `main`:** `views/library.rs`, `views/value_filters.rs`.

**Do:** virtualised rows (`ScrollArea::show_rows`), pages of 100 fetched as their rows come into view, placeholders for rows not loaded, the generation rule. Toolbar: Sort and Show pickers (`ComboBox`), order toggle (tooltip saying what a click does; "Shuffle again" for random), the genres/tags/ratings dialog (with the summary label), year and resolution pickers where they apply, *Clear filters*, the item count; all remembered per library (`LibraryView`, `upgrade`). Filter values from the server with remembered ones kept. The letter bar while sorted by name, typed letters jumping (highlighting from the keyboard). Keyboard: toolbar row then grid; Enter on a picker steps it (pages own the selected index, no select widget state); refresh on `user_data` keeping items on screen; person pages (`Source::Person`). The dialog: checkboxes by kind, Up/Down/Space, every tick reloading the grid behind it.

**Acceptance:** with `dev/fake-library.py`, scrolling 10,000 items stays smooth and loads only what's on screen; every filter matches `docs/PLAN.md` §4.3's notes on the server; `docs/TESTING.md` §2b and §20.

---

### U6 — Item page: header and shelves **[C+R]**

**From `main`:** `views/item.rs` (everything except the children rows and playlist editing, which are U7).

**Do:** the backdrop with its fade into the background, poster, series line, title with the actions beside it (moving under it in a narrow window), the meta line, genres, studios, overview, tags. Actions in order (L1's `item_actions`): Resume / Play from beginning, Play (naming the episode for a series or season), Play all and Shuffle, Trailer (local file plays, remote opens the browser), Watched and Favorite toggles (shown at once, reverted if the server refuses), More (Add to playlist…, Edit…, Delete playlist…). The version picker (Left/Right change it on the cursor). Chapters, Cast & crew (opening person pages), Extras and More like this shelves. Cursor sections as on `main`; reload on `user_data`.

**Acceptance:** `docs/TESTING.md` §22 and §25; the dev server's three-version movie shows the version picker; a refused mark (stop the server) reverts with an error toast.

---

### U7 — Item page: seasons, episodes, playlists **[C+R]**

**From `main`:** `views/item.rs` (`season_rows`, `episode_rows`, `toggle_season`, `load_episodes`, `edit_entry`, `send_edits`, `on_key_down`, `confirm_delete`).

**Do:** a series' seasons grid with the open season's episodes in a band under it (clicking the open season closes it), a collection's items, a season's or playlist's rows with image, heading, runtime, three lines of overview and the Watched toggle (not opening the row), W toggling watched on the highlighted row. Playlists: move up/down and remove buttons, Alt+Up/Alt+Down/Delete, edits shown at once and sent one at a time in order, reload when they're all sent, a refusal (someone else's playlist) shown and the page reloaded. *Delete playlist…* with U8's confirmation, then back.

**Acceptance:** fast Alt+Up/Alt+Down presses on a playlist (`TABBYFIN_KEYS="… alt-up*5"`) end in the order shown, checked with a reload; a `dev2` playlist shared with `dev` refuses edits with the right message.

---

### U8 — Dialogs **[C]**

**From `main`:** `views/add_to_playlist.rs`, `views/edit_playlist.rs`, the delete confirmation in `views/item.rs`.

**Do:** with `egui::Modal`: *Add to playlist* (filter as you type, Up/Down highlight, Enter adds or makes "New playlist “name”", the duplicate check for movies and episodes, the outcomes as toasts, closing when done); *Edit playlist* (name, the sharing switch read from the server, Down/Up between them, Space/Left/Right flip it, Enter saves only what changed, other users' playlists explained); the delete confirmation with its note. Each takes the services and an item, and bumps `user_data` on success. **Acceptance:** each dialog works with the keyboard alone; adding a movie that's already in a playlist says so instead of adding it again (12.2).

---

### U9 — Settings **[C]**

**From `main`:** `views/settings.rs`.

**Do:** the same sections (Appearance, Playback, Subtitles, Storage, Account, About) as headed groups on one scrolling page or with a side list; every option with its description, the config key, the default when unset and the effect (theme and text size apply at once, subtitle size goes to the playback manager, bitrate's *Automatic*); *Clear* for the image cache (memory and disk, with the outcome as a toast); *Open folder* for the logs (`open`/`opener` crate); account and version info. Not on the browse cursor, as on `main` (egui's own Tab focus is fine here). **Acceptance:** every option writes the same config key and value as `main` does (compare the TOML after changing each); `docs/TESTING.md` §19.

---

### U10 — Player page **[C+R]**

**From `main`:** `views/player.rs` (all of it except what P1 moved).

**Do:** V1's video widget full window; the top bar (back, title, SyncPlay and Transcoding badges) and the control bar over gradients; the seek bar with chapter marks, the hover time and trickplay thumbnail, scrubbing without fighting playback; play/pause, ±10 s, mute and the volume slider, the clock (click toggles time left, saved), the speed badge, previous/next item with their tooltips, the version picker, the chapters list (current one highlighted, click seeks), the settings menu (Audio, Subtitles, General tabs: tracks, delays, subtitle size, speeds; "Normal in SyncPlay"). The skip button, the up-next card (countdown, Play now, Cancel) and the resume card, above the controls when they show. Controls and cursor hide after 3 s without mouse movement unless paused or a menu is open. Click toggles pause; double-click toggles fullscreen and puts pause back (`paused_before_click`); the wheel changes the volume (`scroll_steps`). Every key from `main.rs`'s player bindings, through `keys.rs`. Auto-skip (setting) once per segment, not in a group. Esc: fullscreen, then the up-next card, then stop.

**Acceptance:** `docs/TESTING.md` §4, §10, §13, §15, §17 and §18, on Linux and Windows.

---

### X1 — Packaging and CI **[C+R]**

**Do:** update `.github/workflows/ci.yml` (system packages eframe/winit need on Linux; drop GPUI's), `packaging/` (deb dependencies, now `libmpv2, libvulkan1, libwayland-client0`: winit loads libxkbcommon, libwayland-client and the X11 libraries (libX11, libXcursor, libXrandr, libXi) at runtime, so `$auto` misses them; Vulkan stays for video), the Flatpak (`mise run flatpak:sources` after the `Cargo.lock` change), AppImage, the Windows installer and the mingw cross-build (`dev/win-run.sh`), macOS (`Info.plist`), `packaging/THIRD_PARTY.md` (egui and the new crates' licences; GPUI's out). Drop `build.rs`'s comment about GPUI using the icon resource if eframe doesn't, but keep the resource for Explorer and the taskbar. Run `mise run ci:build`.

**Acceptance:** every CI job green; `mise run ci:build` packages for every OS; the `.deb` installs and starts in a clean Ubuntu 24.04 container as before.

---

### X4 — UI tests **[C]**

With `egui_kittest`, without GPU snapshots: the browse cursor on a page built from the JSON fixtures (`crates/jellyfin/tests/fixtures/`), the sign-in form's steps against a `wiremock` server (as `crates/jellyfin/tests/http.rs` does), toasts, the library toolbar's keyboard stepping. These are new; `main` had no UI tests.

---

### X3 — Parity sweep and cleanup **[C runs it, S triages]**

**Do (cheap model):** walk every ✅ item in `docs/ROADMAP.md` and every check in `docs/TESTING.md` against the egui build on the dev server (12.2 and 10.11), and write down what works, what's missing and what's different, with screenshots. Remove the temporary `#![allow(dead_code)]` and fix what that turns up. Measure idle CPU, CPU while playing (both video paths), and memory on the big fake library, against `main`.

**Do (smart model):** turn the list into fix tasks (tier each the same way), decide which differences are acceptable for now and record them in `docs/ROADMAP.md`.

---

### X2 — Docs **[C, mechanical; smart review for `in-app-video.md`]**

Rewrite for egui: `AGENTS.md` (workspace, app architecture, GPUI ↔ tokio rule becomes §3's async rule, keyboard, theme; drop the `gpui` dependency warning, the vendored crates and the "no `use super::*` in tests" note, which was a GPUI quirk), `docs/PLAN.md` (§2 stack, §3 layout, §5 playback, §6 app architecture), `docs/in-app-video.md` (wgpu import per OS, the handshake, NT handles), `docs/ROADMAP.md` (a migration entry, the restyle backlog, what X3 found), `docs/TESTING.md` (everything with UI needs re-checking; hardware checks unchanged), `dev/README.md` (how `TABBYFIN_KEYS` is injected now), `README.md`. Delete this file's task list once the migration is merged, keeping §1 and §3 in `docs/PLAN.md`.

---

## 5. Review checklist for C+R tasks

The reviewer reads the old file(s) on `main` next to the diff:

1. **Behavior:** every doc comment in the old code describes something; each one is ported, or the task says why not. Keyboard and mouse behaviors especially.
2. **Threads:** no network, keychain or heavy decoding on the UI thread; no `jellyfin` future awaited outside tokio.
3. **Repaints:** every background completion requests a repaint; nothing repaints every frame while idle (check the process's CPU).
4. **Cancellation:** async work belongs to a `Task` dropped with its owner; results for an older generation are ignored; deadlines are cancelled when their reason goes.
5. **Errors:** API errors go through `describe_error`; `Unauthorized` leads to the session-expired path.
6. **Textures:** none created per frame (except video), none kept after their page or player is gone.
7. **Config:** loaded fresh before a change and saved right after, as on `main` (other code saves to it too).
8. **Tests:** the old tests are there and pass; new pure logic has tests.
9. **Style:** comments say why, in plain English, matching the surrounding style; `mise run ci` green.
10. **Docs:** ROADMAP/PLAN/TESTING updated where behavior changed.

---

## 6. Risks and open questions

- ~~**Keyboard vs egui focus** (F1)~~: settled in F1, see §3 Keyboard. egui keeps every key while a widget has focus or a modal, menu or popup is open; otherwise the app takes the keys bound in the active scopes before egui sees them.
- **Settled in F1, besides the keyboard:** `egui_material_icons` with both its `filled` and `outline` fonts (`main`'s Lucide icons are outlines; Watched and Favorite are filled); no wgpu adapter selector (wgpu ranks by device type and lists Vulkan before GL); the window's launch options live in `window.rs` and `theme::apply` is called from `App::new`, so A1 needs neither `main.rs` nor much of `app.rs`; pages under others reload when they're back on top rather than in the background (§3 Pages); the update notice moved into F1's shell, since it was the toasts' first user.
- **Settled in S1, besides the keyboard:** the top bar has the account menu early — a menu button with the saved accounts, *Add account…* and *Sign out* — since the flows need a way in; U1 replaces the button with the user's initial and gives the bar its keyboard (egui 0.37 menus don't take keys, so the menu is pointer-only until then). `playback::report_unfinished` belongs to P1 (it runs from `PlaybackManager::new`, which P1 hooks into `Services::new`); S1 left `playback.rs` as it found it. Session expiry is `AppEvent::SessionExpired`, and sign-out, switching and adding an account are the new `AppEvent`s the shell pushes.
- **Windows DX12 import** (V3): untested combination of NT-handle textures with `WGL_NV_DX_interop2` and ANGLE's client buffers; only one test GPU (AMD). Fallback stays software, so this can't break playback, only make it more expensive.
- **Linux on GL** (V2): where wgpu falls back to GL (some VMs, old drivers), GPU frames can't be imported; the handshake sends pixels instead.
- **Off-screen windows** (A1): eframe can't check the saved position against the monitors before opening; needs a check after the first frame.
- **SyncPlay timing** (P2): a UI-loop deadline can be a frame late; P2 may need to schedule commands on tokio.
- **Look:** egui's default font and widgets will look plainer than `main` until the restyle; users may notice.
- **Mascot** (R3): needs tessellation; until then loaders are spinners.

---

## Restyle backlog

After X3, once the egui app matches `main`'s features. Not started before then.

| ID | Item | Tier |
|---|---|---|
| R1 | Theme tokens: colours, radii, spacing, fonts like `main` (gpui-kit's look) | C+R |
| R2 | Animations: page entry, staggered shelves, image fade-in, toggle pop, prompt slide-in, controls fade | C |
| R3 | The mascot loader (`main`'s `mascot.rs`): tessellate its paths with `lyon_tessellation` into `egui::Mesh`es; the three faces, blink and flick timing | C+R |
| R4 | Search box opening out of its button and closing back | C |
| R5 | Scroll feel: wheel smoothing and shelf glide like `smooth_scroll.rs` | C |
| R6 | Player overlay polish: gradients, button sizes, icon sizes as on `main` | C |
