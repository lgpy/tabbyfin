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
- `Services` holds the session (client, user, server), `user_data: u64`, `events: Vec<AppEvent>`, `images` (I1), `playback` (P1), `syncplay` (P2), `live` and `remote` (P3). `Services::new(session, ctx)` takes the `egui::Context`, which services keep to wake the UI; `tick` runs each part's background work (`images.tick()`, the item menu's finished requests, then playback's, the websocket's, remote control's and SyncPlay's).
- Cross-service calls that GPUI did through globals (`NowPlaying::control` asking SyncPlay, `PlaybackManager::play` handing over to a group) become methods on `Services`, which can reach every part: `services.control(UiCommand)`, `services.play(&item, options)`. Each part's module adds its own `impl Services` block (playback's are in `playback.rs`), so `services.rs` stays small.
- GPUI events become `AppEvent`s pushed to `services.events`; the app drains them every frame after the shell has drawn. F1 has `Toast(Toast)` and `SessionExpired`; S1 added `SignOut`, `SwitchAccount` and `AddAccount`, P1 `PlaybackStarted` and `PlaybackEnded` (the app hands those to the shell), A1 `AppearanceChanged` (Settings saved an appearance setting; the app applies it, on every screen, since the appearance is the app's), P3 `ToggleFullscreen` (a remote command; the app owns the window, so it sends the viewport command). Shortcuts: `services.toast(…)`, and `services.handle_error(&err) -> String` (the user-facing message; `Unauthorized` also pushes `SessionExpired`, as `Navigator::handle_error` did).
- Ending a session (sign-out, an expired session, switching accounts, removing the parked account) calls `Services::end()` and then drops the `Services`: `end` stops playback, and the stopped session reports its stop once, in the background, after the drop. Quitting can't wait for a background task, so `App::on_exit` saves the window placement first (`Placement::save_now`) and then calls `Services::shut_down` on the signed-in session and a parked one together, waiting up to 3 s in all.
- The old `Changes` global becomes `services.user_data`, a counter bumped when a session ends and when the server says watch data changed elsewhere (coalesced, P3).
- S1's screens are `Restoring`, `SignIn(Box<Login>)` and `SignedIn(Box<SignedIn>)`, with `App::parked` holding the signed-in screen while *Add account* is showing: the parked session keeps ticking (playback and the websocket carry on), its toasts still show, its screen-changing events (`SessionExpired`, `SignOut`, `SwitchAccount`, `AddAccount`) and fullscreen requests (`ToggleFullscreen`, about the window at the moment they arrive) are dropped — an expiry there is ignored while the form is up, as on `main` — and anything else waits in `services.events` for the shell, which handles it after *Back to your account* (event kinds added later default to kept). The form owns one `Task` at a time (replacing it cancels the request) and gets what a request reports before it ends — the server still starting up, the Quick Connect code — over a `task::channel` made fresh for each request. Signing out, switching accounts or expiring calls `Services::end()` first (P1 stops playback there, P2 leaves a SyncPlay group) and then drops the screen, which cancels whatever it still had running. Restores and the form's requests are polled from `App::logic`/`Login::tick`, not `ui`, so they finish while the window is minimised.

### Pages and navigation

- A page is a struct implementing `views::Page`: `title()`, `scope()` (its key bindings, see Keyboard), `on_key(action, cx) -> bool`, `show(ui, cx)`. The shell keeps a stack of `Box<dyn Page>` (pages stay alive while stacked, so Back keeps their state) and draws the top one in a `CentralPanel` under the top bar (`egui::Panel::top`).
- `views::open(route, cx)` makes the page for a route. Routes whose page isn't ported yet open a `StubPage` naming the task; each page task replaces its arm there.
- Pages navigate by pushing `NavRequest::Push(route)` / `NavRequest::Back` into `cx.nav`; the shell applies them after drawing and requests a repaint so the new page shows at once.
- Loading (the exemplar, `views/home.rs`): the page keeps `Load<T>` (`widgets/text.rs`), an `Option<Task<…>>`, and `loaded_at: Option<u64>`. In `show` it starts a load when `loaded_at != Some(services.user_data)`, which covers the first load, Retry (set it to `None`) and watch data changing elsewhere; then polls the task. What's on screen stays until new data arrives. Unlike `main`, a page under others doesn't reload in the background: it catches up when it's back on top.

### Top bar (U1)

- The bar is the app icon, Back while there's a page to go back to, the top page's title, and on the right the search box, SyncPlay (U2 fills the panel; until then it toasts), Settings (opens Settings, or closes it when it's on top) and the account menu. The open search box takes the middle of the bar, with animation left to R4. The player hides the bar while it plays.
- The box's width shrinks with the bar (at least 120 px) so that the icon, Back and a title fit on its left and the buttons on its right at any window size or zoom.
- **Search** (U1): the box is the bar's only text field. Typing opens the results page (`Route::Search`), or comes back to it from a page opened from the results; clearing the box closes them; Enter or Down moves into them, and Up or Shift+Tab the results leave unused at their first row go back into the box, as do returns to the results from one of their pages. Esc gives the keyboard back to the page (over the results it closes them and clears the box, as `main`'s Back did). The shell hands the term to the page with `Page::set_search_term` and asks it to take the keyboard with `Page::enter_results` (returns whether it had something to put the cursor on), and takes the page's cursor off screen with `Page::hide_cursor` while the box has the keys; U4's `views/search.rs` is the results page.
- The shell keeps each page's `Route` next to it (`StackEntry`), so Settings' toggle and the search page are found by route rather than by downcasting `dyn Page`.

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
- Timers (controls hiding, search debounce, SyncPlay's drift checks, saving the window placement): a `task::Deadline`. `set(after)` starts or restarts it, `cancel()`, and `fire(ctx)` returns true once when it's due and otherwise calls `ctx.request_repaint_after(remaining)`. Checked in `tick` or `show`. No thread sleeps on the UI side. A deadline is only as punctual as the frames, so what must happen at an exact time (SyncPlay's scheduled commands) waits on tokio instead and sends from there (§3 SyncPlay).
- Stale results: as on `main`, owners keep a generation counter (library pages, playback sessions) and drop results for an older one.
- Idle CPU must stay near zero. Nothing calls `request_repaint()` every frame except while something moves (video, an animation, a spinner). F1 measured 0 ticks over 10 s idle on Home.

### Keyboard

**Decided in F1: egui or the app owns the keyboard, per frame.** egui moves focus by itself (Tab from nothing; the arrows only once a widget has focus) and presses the focused widget with Space or Enter; buttons don't take focus on a click (only text fields and drag values do). On a cursor page Tab would start egui focus and Enter would press the focused widget as well as open the card. So `App::raw_input_hook` decides before egui sees the frame's input, from the state the last frame left (as egui's own focus does):

- **egui keeps every key** while it's busy with keys (`keys::egui_wants_keys`): a widget has focus (a text field, or anything reached with Tab on a page without a scope), or a modal (`top_modal_layer`) or a menu, combo box or popup (`Popup::is_any_open`) is open. egui then works as egui does: Tab and the arrows move focus, Space and Enter press, Esc closes the popup or drops the focus. The code that owns such a widget reads the keys it needs itself through `ui.input_mut(|i| i.consume_key(…))`: the search box's Esc, Down and Enter (U1), a dialog's Up/Down/Enter (U8), the SyncPlay panel's keys (U2). A single-line text field that uses Up/Down sets `Memory::set_focus_lock_filter` so egui doesn't also move focus with them.
- **Otherwise the app takes the keys bound in the active scopes** (`keys::take`): the press events are removed from egui's input, so egui never sees them, queued, and routed by the shell before anything is drawn. Unbound keys (and all key releases) stay with egui.
- `keys.rs` has one table, `BINDINGS`: `(Scope, modifiers, key) → Action`, holding everything `main.rs` and `browse::key_bindings` had. Scopes replace GPUI's key contexts: `App` (whatever is on screen, sign-in included: the zoom on Ctrl/Cmd+=, `-` and `0`), `Shell` (Back on Esc, Backspace and Alt+Left; search on Ctrl/Cmd+F), `Browse` (arrows, Tab, Shift+Tab, Enter), `Player`. The active scopes are the top page's `Page::scope()`, `Shell` and `App`; the app's own keys are carried out before anything is drawn. Modifiers match exactly (Alt+Left isn't Left), except on punctuation (`[`, `]`, `+`, `-`, `=`), which some layouts type with Shift or AltGr. Tasks that need more keys (W on the item page, Alt+Up/Alt+Down/Delete on playlists) add bindings and actions there.
- A page without the cursor (Settings, U9) returns `None` from `scope()`: Tab, the arrows, Space and Enter aren't bound in `Shell`, so they stay with egui and its focus works there, while Esc and Backspace still go back when nothing has focus.
- Routing order: open dialog, menu or focused text field (through egui, above) → top bar while the keyboard is in it (U1) → the page (`Page::on_key` with the action for its scope; returns whether it used it) → the shell's own keys (Back, search). An Up the page didn't use (`Browse::at_edge`) is where U1 moves into the top bar.
- The bar borrows the `Browse` actions (Up past a page's first row, then Left and Right along the bar, Enter presses, Down or Esc back to the page with its cursor shown where it was; the other `Browse` actions are swallowed so the page's cursor never moves behind the bar's ring). It has no bindings of its own, so a page without a scope (Settings) keeps the arrows and Tab for egui's focus.
- The account menu is a `Popup` with the shell's own rows, because egui 0.37's buttons take focus only through egui, not for the menu itself: while it's open the shell reads its keys out of egui's input before anything in the bar draws (`keys::take_exact`, so Alt+Up/Down stay the page's), wrapping the highlight round; its rows only answer the pointer. Esc and clicks close the popup as egui's popups do, and so do Backspace and Alt+Left; Ctrl/Cmd+F closes it and opens the search box. Tab stays in the menu (egui would otherwise focus a button or card behind it, and Enter would press that too); for that the shell cancels the focus move egui queued for the key. Choosing the account already signed in just closes it, and a menu opened from the bar hands the keyboard back to the bar when it closes.
- The search box's `TextEdit` locks Esc, Tab and the arrows (`EventFilter`), so egui's focus pass neither drops the box's focus on Esc before the box reads it nor moves the keyboard on Tab; the box reads Esc, Down, Enter and Tab itself and swallows Tab. Keys the boxes and menus read match modifiers exactly (`keys::take_exact`), so Shift/Alt with a key isn't the plain key (U2's panel needs the same for Alt+Up/Down).
- The browse `Cursor` (pure logic, `main`'s tests kept, ported in F1) stays. A page implements `browse::Browse` (`cursor`, `sections`, `activate`, `adjust`, `at_edge`) and makes `browse::on_key` its `Page::on_key`. While drawing, the selected card calls `browse::reveal(cursor, &response)`, which calls `response.scroll_to_me` once after each move (replacing `browse::reveal` and its scroll handles), and the page calls `browse::hide_on_click(cursor, ui)`. A card inside a shelf can only scroll the shelf's own row: its target is taken by that row's horizontal `ScrollArea`, so Home scrolls its page itself, to the whole shelf `widgets::shelf` returns (§6, settled in U3).
- The cursor stays hidden until a key is pressed and hides again on a click, as now.

### Playback (P1)

`playback.rs` holds `main`'s session logic unchanged (`run_session`, `Reporting`, `apply_command`, the bitrate measurement, subtitle look, `save_unfinished` / `report_unfinished`); what changed is how the UI side hears from it.

- **`services.playback: PlaybackManager`**, the part that needs nothing else: `now_playing() -> Option<&NowPlaying>` (the player's state: position, tracks, versions, chapters, segments, next/previous…; `None` while loading, between sessions or reconnecting), `now_playing_mut()` (what's the player's own, e.g. `next_cancelled` for the up-next card), `is_active()` (P3: no user-data refresh during playback), `set_subtitle_scale()` (U9), `client()`. `NowPlaying::send(UiCommand)` goes straight to the player, bypassing SyncPlay (P2's scheduled commands and drift correction); `NowPlaying.session` numbers the session, so P2 can tell a new one from the one it watches.
- **On `Services`**, since SyncPlay may take them over: `control(UiCommand)` (every control from the player page or remote control; pause, seek and stop become group requests in a group; Stop with no session to send it to cancels a pending reconnect), `stop_playback()` (leaving the player), `play(&item, options)`, `play_from_resume(&item)`, `play_all(items)`, `play_all_with(items, options)`, `enqueue(items, next)`, `reload(options)` (another version, same position), `play_next()`, `play_previous()`, `start_over()`, `change_audio(index)`, `change_subtitle(choice)`, `adjust(adjustments)`.
- **Updates:** each session runs on tokio and sends its updates through a `task::channel`; `Services::tick` drains them into `NowPlaying` and handles a session's end (autoplay, reconnect, `AppEvent::PlaybackEnded`). Sessions are numbered: a replaced session's updates only reach its own state, its `Started` is dropped (it's stopping), and its end only saves the volume and bumps `user_data`. A reconnect after `Ended::Lost` waits on a `Deadline` (2, 5, 10 s), cancelled by `stop_playback`.
- **Sessions aren't `Task`s:** a session outlives the `Services` that started it, so after sign-out it still reports its stop. `Services::end` stops the current session; dropping `PlaybackManager` stops any that are left and lets them finish on tokio, then saves the volume. Quitting (`Services::shut_down`) does the same and waits up to 3 s for the stop reports.
- **SyncPlay seam:** playback reaches SyncPlay only through the `impl Services` block in `syncplay.rs` (`in_group`, `group_request`, `group_start_items`, `group_start_queue`, `group_enqueue`, `group_reload`, `group_item_ended`, `group_playback_started`), whose bodies P2 filled (§3 SyncPlay). `group_playback_started` is called from `tick` when a session starts (`main`'s SyncPlay subscribed to `PlaybackEvent::Started`); SyncPlay watches `now_playing().restarts` from its own `tick` work.
- **The player page (U10):** from `PlaybackStarted` to `PlaybackEnded` the shell draws `views::player::PlayerPage` over the whole window instead of the top bar and the page stack, and routes the `Player` scope's keys and the shell's Back to it (Back stops playback, as leaving the player did). The page is kept across sessions: it follows `now_playing` (V1's video keyed by `NowPlaying::session`, the last frame up while the next loads) and starts over what belongs to one item. A session parked behind *Add account* keeps playing; its `PlaybackStarted`/`PlaybackEnded` wait in its queue, and the shell applies them in order after *Back to your account*, so the player is open exactly when something is playing.

### SyncPlay (P2)

`syncplay.rs` holds `main`'s SyncPlay logic; what changed is where it runs and how it hears from playback and the server.

- **`services.syncplay: SyncPlay`**, the part that needs nothing else, for U2's panel: `group`, `groups`, `queue` and `names` (pub fields, as on `main`), and the requests `refresh_groups()`, `create(name)`, `join(id)`, `leave()`, `play_entry(id)`, `remove_entry(id)`, `move_entry(id, by)`, `toggle_shuffle()`, `cycle_repeat()`, plus `start_items`, `start_queue`, `enqueue` and `request` for playback. The panel's default group name (`"{user}'s group"`) is U2's.
- **Requests** go through one queue on tokio that runs them in order (a `Ready` can't overtake its `Buffering`, nor an unpause the pause before it). It's detached, not a `Task`, so leaving the group on sign-out still goes out after the services are dropped. A failure comes back through a `task::channel` and `tick` turns it into an error toast via `handle_error` ("SyncPlay: …"; `Unauthorized` expires the session). Results arrive as websocket updates.
- **Entry points** for the session websocket: `Services::on_syncplay_update(GroupUpdate)` and `Services::on_syncplay_command(Command)`. P2 brought the minimal `live.rs` that calls them: `Live::connect` (in `Services::new`) forwards `Client::socket()` into a `task::channel`, and `Services::tick_live` hands `SyncPlayGroupUpdate`/`SyncPlayCommand` over. P3 adds capabilities on `Connected`, remote control and `UserDataChanged` there, in the same `match`.
- **The seam** (`in_group`, `group_request`, …, `group_playback_started`) now calls into `services.syncplay`. The group session is recognised by `NowPlaying.session`: `group_playback_started` records it when its `group_item` is the loaded queue entry. `tick_syncplay` (after `tick_playback` and `tick_live`) drains what requests and the clock task sent back, polls the fetches (groups, queue names, the queue entry's item, each a `Task` replaced by the next), reports `Ready` once the watched session's `restarts` passes the count seen when it was due, and checks drift once a second (a `Deadline`). When the watched session's `now_playing` is gone (ended, or replaced), the watch and the drift checks stop, so nothing wakes the UI after group playback.
- **Scheduled commands wait on tokio** (§6): `on_syncplay_command` turns the server's `When` into a local `Instant` (`ServerClock`, as on `main`), and a `Task` sleeps until then and sends the `UiCommand`s straight to the session's command channel (`NowPlaying::commands()`). Replacing the task cancels the previous command. A command already due is sent at once. `tick` only hears that it ran, and how late (debug log).
- **Ending:** `Services::end` stops playback and leaves the group. Quitting (`shut_down`) leaves too, in the same 3 s wait as the stop reports (`main` didn't; the server also drops a member whose websocket closes, but a clean leave is immediate).
- **Dev knobs:** `TABBYFIN_SYNCPLAY` joins once the websocket is connected (the server confirms over it, so an earlier join's `GroupJoined` could be missed); with `TABBYFIN_ROUTE=play:` the item waits for the join, so it plays for the group. `TABBYFIN_SYNCPLAY_SKEW_MS` as on `main`.

### Remote control and watch data (P3)

`live.rs` keeps the session websocket (P2 brought the connection) and hands what the server pushes over; `remote.rs` carries out remote control. `changes.rs` is gone: a watch-data refresh is a bump of `Services::user_data`, the counter the pages watch.

- **Capabilities** are reported on every `SocketEvent::Connected` (`Client::socket` reconnects by itself, with backoff): `SupportsMediaControl` and `GeneralCommand::SUPPORTED`. Without them the dashboard has no remote for the session. The report is a `Task` owned by `Live`, replaced by the next connect, polled and dropped once done in `tick_live`, and cancelled by signing out (`main` used a detached task). A failure is logged; `Unauthorized` ends the session through `handle_error`, like every other request.
- **Watch data** (`UserDataChanged`): one refresh per burst, 500 ms after its first message, and none while `playback.is_active()` — a session changes the resume point every few seconds, and its end bumps `user_data` anyway. A burst that lands during playback is dropped, not postponed, as on `main`. The rule is `Refresh` in `live.rs` (a `Deadline` checked in `tick_live`, so it fires while the app idles), with a unit test for the burst and the playback rule.
- **Remote control:** `Services::on_remote_play`, `on_remote_playstate` and `on_remote_command` (in `remote.rs`), called from `tick_live`'s `match`. Transport controls go through `Services::control`, so a SyncPlay group gets pause, seek and stop as group requests like the player's own buttons; tracks through `change_audio`/`change_subtitle`; next and previous through `play_next`/`play_previous` (previous only within 5 s of the start when there's an item before it, else it seeks to the start). `Play` fetches the ids first: `remote.rs`'s `Remote` queues the requests and `tick_remote` places each one's items as soon as its fetch is done, oldest first, so a play next sent while a play now is still fetching lands in the new session's queue — then plays now, shuffled, next or last through the same `play_all_with`/`enqueue` the pages use (in a group: its queue). A failed fetch is logged and, if the token expired, ends the session; `DisplayMessage` is an info toast; `ToggleFullscreen` is `AppEvent::ToggleFullscreen`, carried out by the app for the signed-in session (a parked session's is dropped with its other screen-changing events); unadvertised or unknown commands are logged and ignored.

### Images and video

- `ImageCache` (I1) keeps its budget, queue and blurhash placeholders, but holds `egui::TextureHandle`s. `get(&ImageRequest) -> Option<TextureHandle>` queues a miss and asks for a frame; `Services::tick` uploads what finished (decoding stays off the UI thread, and `tick` creates every texture). The queue is looked at in `tick`, after the frame that asked, and again as each load finishes, as GPUI's deferred pump and per-load pump did. A trim that can't get under the budget yet arms a deadline and asks for that frame itself, so an idle app frees the memory instead of holding it until the next input. Dropping the last handle frees the texture (egui-wgpu frees it after the frame's submit), so GPUI's atlas workaround goes, and evictions need no special timing. `TextureOptions` asks for mipmaps, but egui-wgpu 0.37 uploads one level per texture (only egui_glow builds the chain), so posters are filtered linearly on our renderer.
- **Every image request carries a size.** `ImageRequest::fill_width` (twice the drawn width, for high-DPI screens) keeps the download small and is part of the cache key, so pages must set it rather than ask for the original; the check page does, and W1's image widget and U5's grid must too.
- Images (and trickplay tiles) bigger than the device's largest texture are scaled down as they're decoded: `start_load` reads `InputState::max_texture_side` on the UI thread, and `decode`/`crop` fit the pixels into it. wgpu refuses an oversized texture, and egui only has a debug assertion for it, so a big source image (or trickplay sheet width) would otherwise crash release builds.
- Loads stop with their owner: each `start_load` (here and in trickplay) `select!`s the work against the completion channel closing, so signing out or leaving the player cancels the download; a decode already running on a blocking thread finishes and is thrown away. A panicking loader reports a failed load instead of leaking one of the `MAX_LOADS` slots.
- The blurhash previews are their own GPU textures (4 KB of pixels, but some drivers pad an allocation far more), so their LRU is capped at 2048 entries as well as by bytes (well above a page's worth: past the cap, previews on screen would be decoded again every frame).
- Trickplay (I1) is a plain `Trickplay` the player page owns: `thumbnail(position)` and a `poll()` the page calls each frame (thumbnail calls it too). It keeps the two most recently used sheets' thumbnails as one texture per tile, which always fits the device's limit.
- **Video (V1)** is `widgets::video::Video`, one per playback session: `Video::new(ctx, output, render_state)` keeps the session's frame receiver and starts a task on tokio that wakes the UI for each new frame (`request_repaint_after(1 ms)`: a zero-delay `request_repaint` makes egui draw two passes, which doubled the CPU on llvmpipe). `show(ui) -> Response` takes the space left in the `Ui`, reports it to the player in physical pixels (`set_target_size`, so mpv renders at the size drawn, never above the video's), uploads the newest frame if there is one, and draws it letterboxed on black (contain, snapped to whole pixels); the response senses clicks for U10. `has_frame()` says whether anything has arrived (the shell shows a spinner until then). Dropping the widget frees its texture registration and stops the wake task.
- Software frames are RGBA with opaque alpha (the player renders `rgb0`), written with `queue.write_texture` into one `Rgba8Unorm` texture per frame size, registered once with `register_native_texture` (a new size makes a new texture under the same id, `update_egui_texture_from_wgpu_texture`), not through egui's texture deltas, which would copy every frame twice more. GPU frames are the player's textures imported into wgpu's device, one texture and egui id per image of the player's ring (`SharedFrame::id`), so each is imported once and then only switched between; a frame of a new size (a new ring) retires the old ones, a pass later as on drop. Linux (V2) imports Vulkan memory (`widgets/video/vulkan.rs`), Windows (V3) D3D11 textures opened on wgpu's D3D12 device by the name of their NT handle (`widgets/video/dx12.rs`); a Windows frame whose texture is already gone (`ImportError::Gone`) is skipped with the last frame left up, and only ten in a row count as a failed import. If an import fails, `Video::import_failed()` turns true and the shell calls `Services::fall_back_to_pixels`, which plays the session on from its position with software frames and keeps the import at `Pixels` until the next sign-in. U10's player page must keep that check.
- **The handshake (V1):** `Player::spawn(import)` with a `player::libmpv::VideoImport`: `Pixels`, `VulkanMemory { device_uuid }` or `D3d11Textures { adapter_luid: Luid { low, high } }`. `widgets::video::adapter_import(render_state)` reads what wgpu's device could take through its hal (the Vulkan `deviceUUID` when the device has `VULKAN_EXTERNAL_MEMORY_FD`; the DXGI adapter's LUID on DX12; pixels on GL or Metal), and `widgets::video::import` is what the app asks for: `adapter_import`'s answer as it is (`D3d11Textures` only on a hardware DX12 adapter, not WARP). `app.rs` sets it on the session's `PlaybackManager` at sign-in (`set_video_import`), and each session passes it to `Player::spawn`. The host gets it as the first wire message (`Request::Start { import }`, before it starts libmpv), the embedded player directly, and `render_thread` tries only the GPU paths matching it (ANGLE then WGL for `D3d11Textures`, EGL/Vulkan for `VulkanMemory`), falling back to software, so it never produces frames the UI can't show. On Linux (V2) `egl.rs` puts mpv's OpenGL context and its exporting Vulkan device on the GPU with `device_uuid` (the EGL display whose GL reports that UUID: the default one, else each EGL device), or falls back to software. On Windows (V3) ANGLE (as display attributes) and WGL (its D3D11 device) are put on the adapter with `adapter_luid`, and ANGLE's device is checked to be there; the ring textures are shared through named NT handles (`player::libmpv::d3d11`, `docs/in-app-video.md`).
- wgpu setup (`main.rs`): backends `DX12` on Windows, `METAL` on macOS, `VULKAN | GL` elsewhere (`WGPU_BACKEND` still overrides). wgpu already prefers Vulkan over GL for the same kind of device, so no adapter selector is needed. The device is requested with `VULKAN_EXTERNAL_MEMORY_FD` when the adapter has it. On WSL it picks Vulkan on llvmpipe.

### Toasts

`toasts.rs`: `Toast::info/success/error(text)`, `.title(…)`, `.action(label, ToastAction)`. They stay 5 s (errors 8 s), longer while the pointer is on them, at most four at once, and have a close button. The app owns `Toasts` and draws them over everything; a clicked action comes back from `Toasts::show` for the app to carry out (`ToastAction::OpenUrl` for the update notice). Signed-in code raises them with `services.toast(…)`. Each toast is logged.

### Dev knobs, kept as they are

`TABBYFIN_ROUTE` (pages and `play:`), `TABBYFIN_KEYS`, `TABBYFIN_SYNCPLAY`, `TABBYFIN_SYNCPLAY_SKEW_MS`, `TABBYFIN_MPV_OPTIONS`, `TABBYFIN_VIDEO`, `TABBYFIN_MPV_LOG`, `TABBYFIN_UPDATE_URL`, `TABBYFIN_CONFIG_DIR`, `TABBYFIN_PLAYER=embedded`. Screenshots as before: no `WAYLAND_DISPLAY` (X11), `import -window`.

- `TABBYFIN_KEYS`: same syntax and timing as `main` (first key after 3 s, `wait` 1 s, `key*N` 30 ms apart, 250 ms after each key; keys named `down`, `shift-tab`, `ctrl-f`, `alt-left`, `pageup`, `]`, `a`…). `debug::KeyScript` injects each press as `egui::Event::Key` (press and release, plus `Event::Text` for printable keys, so typing reaches a focused text field) in `App::raw_input_hook`, before `keys::take`, so scripted keys take the same path as real ones. `App::logic` requests a repaint for when the next one is due.
- `TABBYFIN_ROUTE`: parsed by `debug::Start` (`settings`, `item:<id>`, `library:<id>[:movies|tvshows]`, `search:<term>`, `play:<id>[:audio[:sub[:source]]]`, and I1's `posters:<library id>` check page); the shell opens it over Home. `play:` opens the item and plays it from its resume point with the tracks given, as on `main`. `search:` types the term into the top bar's box, which opens the results page.

### During the migration

- `main.rs` has a temporary `#![allow(dead_code)]`, since code arrives before its callers. X3 removes it.
- Each task updates the docs its change touches (`docs/ROADMAP.md`, `docs/PLAN.md`, `docs/TESTING.md`), as AGENTS.md asks; X2 does the full rewrite at the end.
- Hooks in `main.rs` and `app.rs` so tasks don't need to touch them: `window::launch(&config)` (the saved placement, read once in `main`), `window::viewport(launch)` (the viewport options: size, saved placement, title, icon, app id; `NativeOptions::centered` when there's no launch placement), `window::Placement::new(launch)` (handed to `App::new`; `tick` from `App::logic`, `save_now` from `App::on_exit`), `theme::apply(ctx, &config)` (only through `App::apply_appearance`, which also starts or stops following the desktop's colour scheme and puts it back after `apply`). Settings raises `AppEvent::AppearanceChanged` after saving an appearance setting, rather than calling `theme::apply` itself.

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

**Do:** the socket task (capabilities reported on each connect), `SyncPlayGroupUpdate`/`SyncPlayCommand` to SyncPlay (P2 already did the connection and this part in `live.rs`: extend its `match`), `Play`/`Playstate`/`GeneralCommand` to remote control, `UserDataChanged` coalesced over 500 ms and ignored during playback (bumps `services.user_data`). Remote control: every command in `remote.rs` through the same `Services` paths as the player's controls; messages as toasts; `ToggleFullscreen` as an `AppEvent`; unknown commands ignored.

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
- **Settled in L1:** the helpers `main` kept private are `pub` here, since later tasks need them from other modules (`kind_label`, `is_placeholder_episode_name`, `can_mark`, `SORTS`/`FILTERS`/`RESOLUTIONS`); `Mark` came over with its pure methods (`shown_on`, `apply`, `undone`), the optimistic update before the reload, but not `mark`'s GPUI task and notification (the services own that now); `tracks::Choice::label` is a `String` instead of a `SharedString`; `views/player.rs` keeps the wheel's line constants as well as `SEEK_STEP`/`VOLUME_STEP`, which P3 and U10 both use, and drops `KEY_CONTEXT` (`keys::Scope::Player` replaces it).
- **Settled in S1, besides the keyboard:** the top bar has the account menu early — a menu button with the saved accounts, *Add account…* and *Sign out* — since the flows need a way in; U1 replaces the button with the user's initial and gives the bar its keyboard (egui 0.37 menus don't take keys, so the menu is pointer-only until then). `playback::report_unfinished` belongs to P1 (it runs from `PlaybackManager::new`, which P1 hooks into `Services::new`); S1 left `playback.rs` as it found it. Session expiry is `AppEvent::SessionExpired`, and sign-out, switching and adding an account are the new `AppEvent`s the shell pushes.
- **Settled in I1:** egui-wgpu 0.37 creates every texture with one mip level, so a mipmap mode has nothing to filter there (only egui_glow builds the chain); posters are therefore drawn with linear minification, and the image cache keeps `main`'s request sizes rather than asking the server for smaller images. `views/posters.rs` is a debug-only check page (`TABBYFIN_ROUTE=posters:<library id>`, §4 I1) that U5 supersedes and X3 deletes. U5's virtualised grid has to bring the cursor's row on screen itself: `ScrollArea::show_rows` draws only the visible rows, so a card below them cannot `scroll_to_me`; the check page keeps its own scroll offset for that reason.
- **Settled in W1:** the item menu returns a `MenuAction`, and `Services::carry_out` (in `widgets/menu.rs`) plays the item, plays a series' or season's next episode (`play_up_next`) or marks it watched or favorite; only *Add to playlist…* comes back for the page to open U8's dialog. `mark` and `play_up_next` are the one exception to §5.4's "async work belongs to a `Task` dropped with its owner": they're fire-and-forget like `main`'s `.detach()`ed tasks, reporting through a channel `Services::tick` drains, so the request outlives the frame that asked (and a mark still lands when the user navigates away mid-click). The menu is pointer-only (egui's popups take no keys), as the account menu was until U1 gave it its own keys. `rail_drag.rs` and `smooth_scroll.rs` are dropped rather than ported: `ScrollArea` drags its contents with the mouse, and egui suppresses the click when the press moved past its click distance, so a press that moves more than 4 px (`max_click_dist`, set in `theme::apply` to `main`'s threshold) doesn't open a card; within those 4 px the row already scrolls along with the pointer, as egui's drag-only scroll areas do (`main` didn't scroll before the threshold; R5). `play_up_next` has no generation check, as on `main`: a slow episode fetch can start after the user played something else. Its wheel only scrolls a row sideways with Shift, as `main` had it. R5 covers the glide the arrows don't do yet and one difference in feel: a drag-scrollable `ScrollArea` shows the grabbing cursor on press and coasts or rubber-bands on release, where `main` only followed the pointer. The shelf hides the scroll bar (`ScrollBarVisibility::AlwaysHidden`) since its arrows are the affordance, as on `main`. A card reserves its text block with `main`'s `CARD_TEXT_H` (`4 + 20 + 4 + 16`, the gaps included) as the floor, so a grid's virtualised rows keep their height at any text size. The progress bar takes A1's `theme::over_video::TRACK`/`TRACK_OUTLINE`, and the played badge's circle and tick are `strong_text_color`/`panel_fill` so they read in both themes; the filled watched and favorite icons still keep `main`'s green and red as constants until R1 (which also owns the badge's exact colours, the card's hover dim and the person card's initials size). Every page draws under its own id scope (`Shell`'s `StackEntry::serial`), so stacked pages with the same shelf names keep their own scroll state; U1 keeps that.

- **Settled in P1:** `Services::new(session, ctx)` takes the `egui::Context` (I1's order); sessions run detached on tokio (not as `Task`s) so they report their stop after sign-out, and quitting waits up to 3 s for that (`main` didn't stop playback on quit; the next launch reported it through `report_unfinished`, which still covers a quit that runs out of time); `report_unfinished` runs in `PlaybackManager::new` (so S1 doesn't call it on sign-in); `Services::end` (S1's hook) stops playback, and dropping the services lets the stopped session report its stop once, in the background; quitting shuts down the signed-in session and a parked one together; playback's failures and notices are plain `AppEvent::Toast`s (error and info), since the shell only showed them, so only `PlaybackStarted` and `PlaybackEnded` are new events; a replaced session's `Started` no longer opens the player (on `main` a session replaced while loading could still push its page over its successor's); `Services::control(Stop)` with no session to reach (waiting to reconnect, or the next item loading) stops playback instead of being lost, so Esc while "Reconnecting…" works (`docs/TESTING.md` §5b); the SyncPlay seam is the `impl Services` block in `syncplay.rs`; the shell is the player page's stand-in until U10 (see §3 Playback).
- **Settled in U1:** the results page interface is `Page::set_search_term`, `Page::enter_results` and `Page::hide_cursor`: the shell focuses the box whenever the results come back on top, sends an Up or Shift+Tab the results didn't use back into the box (as `main`'s `at_edge` did), and hides a page's cursor the box's keys would otherwise leave on screen (`hide_cursor`, implemented by `browse::Browse`'s pages), so Ctrl+F and Esc leave the cursor where it was instead of moving it a row; U4 replaces `views/search.rs` and keeps the interface, its page returning `true` from `enter_results` once it has a first row. The bar borrows `keys::Scope::Browse`'s actions rather than adding a `TopBar` scope, since Up, Left, Right, Down, Enter and Tab are exactly what it needs and it only has the keyboard while a browsing page's scope is active (Up past the page's first row is what gets it there); the shell swallows the bar's unused actions (Up, Tab) so the page's cursor can't move behind the ring. The search box reads Esc, Down, Enter and Tab itself in `top_bar` before the `TextEdit` draws (`keys::take_exact`, exact modifiers, so Alt+Down isn't Down) because egui owns the keys while a text field has focus; the `TextEdit` locks Esc, Tab and the arrows (`EventFilter`) so egui's focus pass can neither drop the box's focus on Esc before the box reads it nor move the keyboard on Tab, and Tab stays in the box. It stays open while it has text and closes on blur once empty, as `main`'s did, and loses its focus (and egui's keyboard) when the player opens. The account menu is a `Popup` whose rows the shell draws and whose highlight it moves itself: egui's buttons are focusable only through egui's focus pass, so the shell reads the menu's keys in `account_menu_keys` before the bar draws (Tab stays in the menu, Backspace and Alt+Left close it, Ctrl/Cmd+F closes it and opens the box; the focus move egui queued for Tab or an arrow is cancelled), while Esc and clicks close it as egui's popups do. A click on any bar button clears the bar's keyboard (going back left the page with it, as on `main`), and a `bar_key` the bar no longer has (Back on the first page) is dropped rather than pressed invisibly. The logo is the 128 px copy of `crates/app/assets/logo.png` `main` used, resized once to 64 px: egui-wgpu uploads one mip level (I1), so a 128 px texture drawn at 32 px aliases. The account button is a 36 px slot with the user's initial, drawn by the shell, since it can't be a button's label (a circle). `main`'s `Menu`/`menu_keys`/`render_syncplay` machinery is U2's; only the account half is here. `Shell::player_started` takes the `egui::Context` (it hides the bar, so `App` repaints, and it drops the search box's focus or egui would keep the player's keys). The box's `EventFilter` only holds from its second focused frame (egui applies a widget's filter to the frame after it draws), so a Tab, arrow or Esc arriving in the very frame the box takes focus can still move egui's focus; nobody types that fast, but a `TABBYFIN_KEYS` script should `wait` after opening the box.
- **Settled in V1:** the handshake is the first wire message (`Request::Start`), not a `--player-host` argument: it's typed and versioned with the rest of the protocol, and the host waits for it before starting libmpv. The import lives on `PlaybackManager` (set once per sign-in from the window's `RenderState`), not passed through `Services::new`, so the services' constructor stays as S1/P1 left it; `run_session` takes it as an argument. The video widget is per session (keyed by `NowPlaying.session`; U10's player page does the same), since each session has its own `VideoOutput`; the shell keeps the last session's widget between sessions, so its last frame stays up while the next one loads. Resizing swaps the texture under the same egui texture id rather than freeing and registering a new one. The app reads the adapter's Vulkan UUID and DX12 LUID already (and logs them), so V2/V3 only change `widgets::video::import`; the app crate depends on `ash` on Linux for that (the version wgpu uses), while the LUID needs no new dependency (wgpu-hal's `IDXGIAdapter3`). The bar over the video and the letterbox take A1's `theme::over_video` colours (`BACKDROP`, `SCRIM`, `FOREGROUND`, and `MUTED_FOREGROUND` for the spinner), with egui's dark button visuals until U10's controls. `NowPlaying`'s frame log is gone: the widget logs each new frame size instead.
- **Settled in U8:** the three dialogs are page-owned structs in `views/dialogs.rs` — `AddToPlaylist`, `EditPlaylist`, `DeletePlaylist` — each made with `new(cx, item)` and drawn with `show(cx)` until that returns `false`, which the page reads as "drop it". They're `egui::Modal`s, so the backdrop blocks the page and egui keeps the keyboard while one is open; each dialog reads its own keys (`Up`/`Down`/`Enter`; `Space`/`Left`/`Right` on the sharing switch), and its buttons and list rows are `Sense::CLICK` widgets, which egui doesn't focus, so a Space or Enter meant for the dialog can't land on a button it isn't showing as focused. The sharing switch is drawn (`switch_row`) for the same reason, with `main`'s selection ring. `Enter` is the confirm (`main`'s dialog OK) and is consumed before the text field draws so the field never swallows it; `Esc` and a click on the backdrop close through `ModalResponse::should_close`. *Delete playlist…* keeps its confirmation open while the request runs — `main` closed the alert as soon as Delete was pressed — so a refusal ("belongs to someone else", or a server error) is a toast with the dialog still there to cancel or retry; deleting toasts, bumps `user_data` and goes back, as `main` did. The list's rows put the entry count at the right end with `Atom::grow()`, and *New playlist “…”* is offered only when no playlist already has that name (ignoring case), as `main` had it. The list's height is worked out from the row height measured as the rows draw: a `ScrollArea` given a modal's leftover height would keep the first frame's (empty) height for ever and clip the last row. `views/home.rs` opens *Add to playlist…* from a card's menu; U6's *More* menu and U7's playlist page open these three the same way, keeping one `Option<…>` per dialog.
- **Settled in P3:** capabilities are reported from a `Task` in `Live` (replaced on the next connect, polled and dropped once done, cancelled by sign-out) rather than `main`'s detached task, so nothing outlives the session; its failure is logged, and `Unauthorized` goes through `handle_error` so an expired token ends the session (the remote item fetch does the same; no toast for other errors, as on `main`). The watch-data burst is a `Deadline` (`Refresh`) instead of a GPUI timer task, which keeps `main`'s timing (500 ms after the burst's first message, new messages join the pending refresh) and `main`'s drop of a refresh that lands during playback; a unit test covers the rule. Remote `Play`s apply in the order they were made, none dropped: `Remote` keeps a `VecDeque` of fetches, and `tick_remote` places the head once its items are there, then the next, so a play next sent while a play now is still fetching lands in the new session's queue (on `main` both fetches ran and whichever returned last won, which could reverse their order). That exposed a P1 gap it fixes: `NowPlaying.next` is now read from the play queue when the session starts rather than from the queue's front captured at `start_session`, so an item queued while the new session was still loading shows up as next — `NextTrack`, autoplay and the up-next card find it — instead of waiting in the queue unseen. The `RemoteEvent` enum is gone: `DisplayMessage` is an info toast and `ToggleFullscreen` is `AppEvent::ToggleFullscreen`, which `App::handle_events` turns into a `ViewportCommand`; a parked session's request is dropped with its screen-changing events rather than replayed after *Back to your account* (it's about the window at that moment), and `Shell::player_ended` asks for windowed unconditionally — `viewport().fullscreen` is a frame stale, and asking a windowed window to leave fullscreen does nothing. `GeneralCommand::SUPPORTED` is advertised as on `main`; unadvertised commands still arrive and are ignored (`GeneralCommand::Other`). The remote `previous` keeps `main`'s 5 s rule, and reads the position and the item before it at the moment the command arrives, as `main` did (a seek sent shortly before it may not have reached `NowPlaying.position` yet, so a second press can still play the previous item). Pause, seek and stop act at once, so one sent while an earlier remote play is still fetching its items reaches what's playing now, and the play starts afterwards, as on `main`.
- **Settled in U3:** Home scrolls the whole shelf into view itself when a key moves the cursor onto it, as `main`'s `browse::reveal` scrolled a whole section (title and cards together). A card's `scroll_to_me` can't do it: egui gives a scroll target to the next scroll area that ends, which is the shelf's own horizontal `ScrollArea`, and that ignores the other axis. So `widgets::shelf::shelf_with_gaps` returns the row's rectangle — its top measured before the title, its bottom from `inner_rect.top() + content_size.y` rather than `inner_rect.bottom()`, which is clipped to the window — and the page sets the target once every shelf is drawn, or the next shelf's row would take it. Home keeps `main`'s 1.2 card scale, 24 px gaps and 8 px title gap; the shelves' staggered entry waits for R2. The empty state keeps `main`'s rule exactly: "Your server has no libraries yet." shows whenever no shelf is left to draw, so an account that hid every section in the web client sees it too. Favorites' place before the Latest rows and the per-library Latest rows are `main`'s `shelves()` logic, now covered by unit tests. The 12.2 dev server answers `/Items/Latest` with `[]` for every library (the captured fixture does too), so the Latest rows were checked on 10.11 (`docs/TESTING.md` §21).
- **Settled in U4:** the results page searches 300 ms after the last change to the box, as `main` did, but the pause is a `Deadline` checked in `show` rather than a tokio sleep inside the search `Task` (§3's timers); the request itself is a `Task`, and both go when the term changes — the deadline is re-set, the task dropped — so a term can't land its results on a later term's page. The term changing puts the spinner up at once (and hides the cursor), as on `main`, and an unchanged term is ignored (trailing spaces, or Enter reporting a change). The page's width is taken from `show` and kept for the keys: the groups' column counts can only be worked out while drawing, so `sections` and `activate` use the layout the page last showed, as I1's check page did. `main`'s window width became `ui.available_width()` (the panel's inner width, the same 24 px padding subtracted on both sides), and its `smooth_wheel` is gone with the rest of the scroll helpers (W1). Each card's `scroll_to_me` is enough here, unlike Home's shelves: a row is a plain line of cards, so the page's own `ScrollArea` is the next one to end. The page's `ScrollArea` doesn't shrink to its content (`auto_shrink([false, true])`), so the scroll bar sits at the window's edge rather than at the end of the widest row. A failed search is a message only, with no *Retry*: that's what `main` showed, and the box is how a search is repeated. Results don't reload when watch data changes elsewhere (`main` didn't either).
- **Windows DX12 import** (V3): NT-handle textures work with `WGL_NV_DX_interop2` and with ANGLE's client buffers on the RX 6600; NVIDIA, Intel and hybrid laptops are untested (`docs/TESTING.md` §7). Fallback stays software, so this can't break playback, only make it more expensive.
- **Settled in V3:** named NT handles rather than duplicating handles into the app: the app opens each texture by name (`OpenSharedHandleByName`), which works the same from the host and embedded, and needs no process handle in the host. A name only opens while a handle to its texture is open, so the frame carries the texture's handle (`SharedFrame::_handle`, an `Arc<OwnedHandle>` like V2's fds) in-process, and the host keeps the handles of the last six frames it sent, since the app may not have opened a frame before a resize retires its ring. Names carry the process id and start time and a per-process counter, so a name is never reused. The imported texture is declared to wgpu as already a shader resource (`TextureUses::RESOURCE`): it's only sampled, so wgpu never transitions it, and D3D12's implicit promotion from and decay to `COMMON` around each submit leaves it in the state the player's device writes it in. The app asks for D3D11 textures only on a hardware adapter: on WARP the player's GPU path would cost more than pixels. ANGLE gets the LUID only when it lists `EGL_ANGLE_platform_angle_d3d_luid`, and its device's adapter is checked either way; WGL's D3D11 device is made on the adapter, its GL context stays where the driver puts it. 4K HEVC 10-bit with ANGLE costs 14–21% of a core across app and host in debug builds, against GPUI's ~11% (`docs/in-app-video.md` has the table). `dev/win-run.sh` no longer mirrors GPUI's shader sources.
- **Settled in U10:** `views/player.rs` is a `PlayerPage` the shell makes on `PlaybackStarted` and drops on `PlaybackEnded`; a new session keeps the page, and `PlayerPage::follow_session` starts over what belongs to one item (trickplay sheets, the auto-skipped segment, a scrub, the menus, the settings tab) and re-reads the player's config settings, so V1's video widget stays keyed by `NowPlaying::session` and the last frame is up while the next session loads (a fresh page per `PlaybackStarted`, as `main` had, would drop it). Every widget on the player is non-focusable (`Sense::CLICK` or `CLICK|DRAG`, and `Button::sense`), and `Video::show` senses `Sense::CLICK` instead of `Sense::click()`: otherwise a Tab that egui took could focus the video or a control and `egui_wants_keys` would stop the app taking the player's keys (U8's rule for its dialogs). For the same reason the seek bar and the volume slider are drawn by the page rather than egui's `Slider`, and the version picker is a `Popup` rather than a `ComboBox`; the seek bar also needs the chapter marks, the hover preview and a scrub that doesn't fight playback. `Shell::player_started` surrenders whatever egui focus is left (the search box's included), so nothing behind the player can hold the keyboard. The settings, chapters and versions menus are `Popup`s with `CloseOnClickOutside` (they stay up while a value is stepped, as `main`'s popovers did); while one is open egui keeps the keys, so the player's keys wait until it closes (Esc closes it). The bars are drawn in the panel and occlude the video with click-sensing rects (`main`'s `occlude`), and clicks and the wheel are also checked against their rects, since egui hit-tests from the previous frame's widgets; the prompt cards are an `Area` anchored bottom-right with `Sense::CLICK` for the same reason. The controls' auto-hide is a `task::Deadline` (3 s, restarted by any mouse movement and by the keys that showed the controls on `main`), the cursor is `CursorIcon::None` while they're hidden, and paused playback, an open menu, a seek-bar scrub or a volume drag keeps them up (a drag would otherwise be dropped under the user's hand). The wheel is counted in egui's units: one `Line` is one notch and one 5 % step, where `main`'s GPUI input gave three lines per notch, and a trackpad's pixels are 90 points per notch (`main`'s three lines of 30). The version picker is a 140×28 bordered select with a caret, shown only when the item has more than one version. A session that is reconnecting (no `now_playing`) keeps the last frame with the spinner and a Back bar saying "Starting playback…", as the shell's stand-in had. The menus are pointer-only (§3), and their and the bars' look is R6's.
- ~~**Linux on GL** (V2)~~: settled in V2: where wgpu runs on GL (some VMs, old drivers, `WGPU_BACKEND=gl`), `adapter_import` finds nothing to import, so the app asks for pixels and the player renders software frames (checked on WSL).
- **Settled in V2:** the import is `main`'s GPUI one on wgpu 30 (`TextureMemory::External`, a drop callback destroying the image and freeing the memory, `create_texture_from_hal` with `TextureUses::UNINITIALIZED`, which wgpu 30 now asks for). Sync stays `glFinish` plus the ring of three (`docs/in-app-video.md` says why not semaphores). Three things go beyond `main`: (1) the player's OpenGL is put on the app's GPU, not only its exporter: `main` matched the exporter to whatever GPU EGL's default display gave, which on a hybrid laptop could differ from the window's, and the import then fails or is undefined; now the default display is tried, then each EGL device. (2) `SharedFrame::fd` is an `Arc<OwnedFd>` instead of a raw number, so the fd a frame names can't be closed (and its number reused for another image's memory) before the UI imports it; the host's fd table hands out the same `Arc`s. (3) A failed import switches the session to software frames (`Services::fall_back_to_pixels`) instead of leaving the video black. The app asks for Vulkan memory even on lavapipe; the player's llvmpipe rule then decides (software unless `TABBYFIN_VIDEO=gpu`), so the rule stays in one place. `widgets/video/vulkan.rs` is a submodule of the video widget, so V3's DX12 import can sit beside it without touching the same lines.
- ~~**Off-screen windows** (A1)~~: settled in A1 (round 2, on review). The check happens before the window exists, where it can be exact: on Windows the saved rectangle's centre is looked up among the monitors from `EnumDisplayMonitors` (in physical pixels: the process is made DPI-aware first) and a spot on none of them opens the window centred; on Linux and macOS the window manager or AppKit keeps windows on screen (Wayland doesn't let an app place its window at all), so the saved spot is used as saved and nothing moves the window after launch. See `docs/PLAN.md` §9.
- **Settled in A1, besides the window:** the zoom keys are an always-active `App` scope taken even while egui has the keyboard (a text field with the focus, a menu, a dialog: Ctrl/Cmd `=`, `-`, `0` type and press nothing), egui's own zoom keys are off so they don't step by 10 % behind the saved levels, and the app keeps the zoom it chose because `ctx.zoom_factor` only changes at the start of the next pass.
- **Settled in A1, placement:** the frame's top-left is saved in physical pixels with the scale factor, and the content size in OS points, which the UI zoom doesn't touch (eframe's own window storage keeps the same). On Windows winit converts a launch position given in points with the scale factor of the monitor the window is created on, so if the window doesn't open within a pixel of its saved spot (or, maximized, opens on a monitor of another scale) the physical position is set once more after the first frame and the window maximized again after it, since moving a maximized window restores it; in the common single-scale case nothing is sent. The process is made DPI-aware at the top of `main` (`window::become_dpi_aware`, what winit does later anyway), so the pre-launch monitor check gets physical rectangles. Configs from before the scale factor keep their position in points. Each move or resize of the window gives a frame whose input already has the new rects, and `Placement` remembers every placed report: no repaints of its own, no two-frame confirmation. The rects are undone with the zoom they were measured with — egui-winit measures before the frame starts, and a newly chosen zoom only applies at the frame's first pass — and positions at or below −32000 (X11 while mapping, a minimized window on Windows) are ignored. A launch that asked for a maximized window and finds it not maximized asks once more (X11 window managers ignore a maximize before mapping) and doesn't remember that frame; after that the window's own report counts, so a platform that never maximizes saves the window as it is. With nothing saved (or an off-screen spot on Windows) the window opens centred.
- **Settled in A1, appearance:** the desktop's colour scheme on Linux comes from the XDG portal (`theme::SystemTheme`: subscribed to `SettingChanged` first, then `ReadOne`, or `Read` on portals before 1.15, whose value comes in one more variant; the signal is followed even when neither read answers; light when there's no portal or no preference, as `main` was), on Windows and macOS from egui/winit. `theme::apply` knows only light for "system" on Linux, so `App::apply_appearance` sets the scheme the portal last said again after it. The system text size is read on tokio (`reg`, `gsettings`, `kdeglobals`: no blocking read on the UI thread at launch or on focus), with 100 % until it answers; it's read the first time a setting follows it (at launch, or when Settings switches the text size to the system's) and again whenever the window gets the focus back. Text size scales an app-owned table of text styles (`theme::set_text_styles`), egui's own to start with, so a restyle's sizes and named styles (R1) scale too and re-applying never compounds. `theme::over_video` is a module of `Color32` constants. Animations off sets `animation_time` to 0 and `scroll_animation` to none.
- ~~**The system theme on Linux** (A1)~~: settled on review — follow the desktop, as `main` did, through the XDG desktop portal (`org.freedesktop.portal.Settings.ReadOne("org.freedesktop.appearance", "color-scheme")`) over `zbus` on tokio, then the portal's `SettingChanged` signal (round 3: subscribed before reading, with `Read` as the fallback for portals before 1.15). 1 is dark, 2 is light, and 0, an error or no portal at all mean light (what `main` showed without a signal). Windows and macOS still let egui/winit follow the system. `zbus` comes in through keyring with its `async-io` runtime enabled as well; the extra `tokio` feature is the one the app asks for, and its future is awaited on the app's runtime either way, never on the UI thread.
- **Settled in P2 — SyncPlay timing:** scheduled commands wait on tokio and send straight to the session's command channel; the UI loop only hears back. Measured with the scripted member, a UI `Deadline` armed for the same instant as each command beside the tokio task: with the UI idle (it repaints only for events), Unpause commands (~1 s lead) were sent 0.6–1.6 ms late from tokio and 1.0–1.7 ms from the UI loop; with the UI repainting continuously, as it will under V1's video, tokio stayed at 0.3–1.9 ms while the UI loop was 1.7–8.0 ms late (mean 4.1 ms) — and that was on Xvfb without vsync, at 70–220 frames a second with the worst frame gaps 22–201 ms in 5 s windows. On a 60 Hz screen a frame-aligned check is late by up to 16.7 ms, and by a whole frame's work on a slow one. Commands already due (Pause and Seek come with the server's current time) are sent at once: through tokio's timer they took 0.2–1.8 ms more. Drift correction stays on a 1 s UI `Deadline`: it measures, so a late check only shifts when it measures. What still depends on the frame: `NowPlaying.position_at` is stamped when `tick` drains the mpv event, up to a frame after it arrived; small against the 80 ms tolerance at 60 Hz, but if V1's software path shows noisy drift, stamp the events in `run_session`.
- **Settled in P2, besides timing:** requests run in order through one queue (above); failed requests are error toasts (`main` showed every SyncPlay notice as info); a group item that finishes loading after the group was left or stopped is stopped (on `main` it stayed paused, its controls sent to a group it wasn't in); the item fetch for the queue's entry is a `Task` cancelled by the next entry or leaving; quitting leaves the group; the minimal websocket is in `live.rs` for P3 to extend.
- **WSL's clocks** (found in P2): in this WSL VM (measured 2026-10-26), `CLOCK_MONOTONIC` (Rust's `Instant`) runs about 8% slow against the server's clock and `CLOCK_MONOTONIC_RAW` (measured over 30 s: server 32.66 s, monotonic 30.06 s, raw 32.78 s), and `CLOCK_REALTIME` is stepped forward ~2.6 s about every 30 s to catch up. mpv keeps true time, so against the app's timeline it plays ~8% fast, beyond drift correction's ±5%: drift grows ~40 ms/s at 0.95× until it seeks. `mpv --ao=null` (and `--no-audio`) runs 1.083× against Python's `time.monotonic()` here; `--ao-null-speed=0.923` slows it to the guest's clock for tests. The server runs in the same VM, so its `When` and the app's wall clock step together and `ServerClock` stays exact here; a converter pinned to `Instant` (tried) drifted 2.5 s within 14 s of a sample. Elsewhere it's the other way round: pinned would survive a wall-clock step (NTP, resume from sleep) between samples, which `ServerClock` takes until the next sample a minute later. P2 keeps `main`'s `ServerClock`. Timing measurements in this VM need care until it's restarted (`wsl --shutdown`).
- **Settled in X1:** CI's Linux jobs install only `pkg-config` (plus the runner's C toolchain): eframe/winit `dlopen` the X11, Wayland and xkbcommon libraries, and libmpv and Vulkan are loaded at runtime, so no `-dev` packages are needed (a `cargo check -p tabbyfin --locked` in a clean Ubuntu 24.04 container with just `build-essential` and `pkg-config` passes). The Linux release binary's only `DT_NEEDED` entries are `libc`, `libm` and `libgcc_s`, so `$auto` misses all the runtime ones; the `.deb`'s `depends` names `libmpv2`, `libvulkan1`, `libwayland-client0`, `libxkbcommon0`, `libxkbcommon-x11-0`, `libx11-6`, `libx11-xcb1` (winit opens libX11-xcb on every X11 start; `libx11-6` doesn't pull it in), `libxcursor1` and `libxi6` (wayland-cursor is pure Rust and winit uses x11rb's RandR, so no libwayland-cursor or libXrandr). `THIRD_PARTY.md` now lists the egui/eframe/egui-wgpu/wgpu licences and the fonts and Material Symbols embedded in the binary (GPUI wasn't listed, so there was nothing to remove), and it ships in the Linux tarball and `.deb` too (`/usr/share/doc/tabbyfin/`), as well as in the Flatpak's licences folder. The AppImage build container and the CI jobs use the same short package list; the old `-dev` packages were GPUI's. `build.rs`'s comment now says Explorer uses the icon resource while the window icon comes from `packaging/tabbyfin.png` through eframe, and `packaging/appimage.sh` no longer mentions GPUI. `Info.plist` needed no change: eframe has no extra bundle keys beyond what was already there. `mise run ci:build` couldn't run here: it builds what's pushed, and X1 leaves its branch uncommitted and unpushed. The same steps ran locally instead: the `.deb` and AppImage start in clean Ubuntu 24.04 containers, the Flatpak builds and installs, and `mise run flatpak:sources` leaves `cargo-sources.json` unchanged. The Flatpak wasn't run: the harness's Xvfb exposes an abstract X11 socket, which Flatpak can't share with the sandbox (not a packaging result).
- **Settled in U9:** `views/settings.rs` is one scrolling page of headed groups (Appearance, Playback, Subtitles, Storage, Account, About) instead of gpui-kit's searchable sidebar of pages: the sidebar, its keyword search and the per-page reset buttons were panel chrome, and what each row keeps is what `main`'s settings had — the same choices in the same order, the same config key and value (`Config::load` fresh, change, save, as `main`'s `save` did, so a save here can't write over what playback or the window saved meanwhile), and the value that applies while the key isn't in the file. Each row says what its change takes effect on: the theme, text size and animations go through `AppEvent::AppearanceChanged` (the app applies them, on every screen), the subtitle size straight to `PlaybackManager::set_subtitle_scale`, and the rest are read when what they configure next starts (a playback session, the player page, the update check at launch). The two Storage rows save nothing, so their smaller line is the folder instead of `key · unset: …`. Pickers are egui's `ComboBox`, flags a `Checkbox::without_text`; the page has no scope, so Tab reaches them and Space flips a switch (every flag was flipped that way). A 0.37 `ComboBox`'s list is pointer-only like the other popups (§3), so the pickers take Left and Right themselves while egui's focus is on one: `Pickers::begin_frame` takes the plain keys out of egui's input before anything draws (exact modifiers, so Alt+Left stays the shell's, and only while a picker had the focus at the end of the last frame), and the step comes back from `choose` as the same value a click gives (same save, same `AppearanceChanged`/`set_subtitle_scale`/bitrate handling), stopping at either end; the picker also tells egui not to move the focus sideways with its `EventFilter` (it holds from the frame after the picker gains the focus, as the search box's does), and while a list is open the keys stay egui's. Up and Down still move egui's focus as usual, and the page's scope stays `None`: the keys are only the picker's while it has the focus. *Clear* and *Open folder* run on a blocking thread through one `Task` on the page and report as toasts; dropping the page mid-clear still finishes the delete (a blocking job isn't cancelled) and only loses the toast. `open_folder` runs `xdg-open`, `open` or `explorer` from `std::process` instead of depending on the `opener` crate: one call site, no `Cargo.lock`/`mise run flatpak:sources`/`THIRD_PARTY.md` churn, and only a launcher that can't be spawned is an error — one that exits 0 without opening anything (no file manager on this harness) isn't, as `main`'s `open_with_system` wasn't. `Show remaining time` stays the player's clock toggle (not on `main`'s page either), and the account rows are the four informational ones `main` had (switching and signing out stay in the top bar's account menu).
- **Look:** egui's default font and widgets will look plainer than `main` until the restyle; users may notice.
- **Mascot** (R3): needs tessellation; until then loaders are spinners.

---

## Restyle backlog

After X3, once the egui app matches `main`'s features. Not started before then.

| ID | Item | Tier |
|---|---|---|
| R1 | Theme tokens: colours, radii, spacing, fonts like `main` (gpui-kit's look); the card's hover dim, the played badge's and the watched/favorite icons' colours, and the person card's initials size | C+R |
| R2 | Animations: page entry, staggered shelves, image fade-in, toggle pop, prompt slide-in, controls fade | C |
| R3 | The mascot loader (`main`'s `mascot.rs`): tessellate its paths with `lyon_tessellation` into `egui::Mesh`es; the three faces, blink and flick timing | C+R |
| R4 | Search box opening out of its button and closing back | C |
| R5 | Scroll feel: wheel smoothing and shelf glide like `smooth_scroll.rs`; the grabbing cursor and release coasting a drag-scrollable row has now | C |
| R6 | Player overlay polish: gradients, button sizes, icon sizes as on `main` | C |
