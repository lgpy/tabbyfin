# Manual testing checklist

Things that work in development but haven't been tested where it matters: on real hardware, real servers, release builds, or with a real second person. Development happens in WSL2 without a GPU, against the local dev servers in `dev/` (see [`dev/README.md`](../dev/README.md)), with keys pressed from inside the app (`TABBYFIN_KEYS`) rather than a real mouse and keyboard.

Tick an item when it's been tried; note the hardware, OS and server version, and file anything that fails. Roadmap numbers refer to [`ROADMAP.md`](ROADMAP.md).

**Legend:** ✅ tested in development · ⬜ still to test

---

## 1. Release build on Windows

Everything below was built and run only as Linux debug builds. Use a CI package (`mise run ci:build`), unzipped over the old copy.

- ✅ **Player controls don't pause the video when clicked** (the `.occlude()` fix). Click the volume slider, the pickers, -10s/+10s and Fullscreen; playback must keep going.
- ⬜ **Log file:** *Settings → Storage → Logs → Open folder* opens the folder, today's `tabbyfin.*.log` exists and has no colour codes in it.
- ⬜ **Settings → Playback:** the autoplay switch and the bitrate picker are remembered after a restart.
- ⬜ **Settings panel** (gpui-kit's `Settings`): every sidebar page and sub-entry opens, the search box filters items, a page's reset button puts its switches back to their defaults, and *Clear* empties the image cache. Seen so far only in screenshots (Appearance, Playback, Storage, Account).
- ⬜ **Everything in §2–§6** at least once on Windows, since the GPU and ANGLE paths only exist there.

## 2. Keyboard navigation (3.1)

✅ Home shelves, library grid, search (the top bar's search button opens into a box, which opens the results as you type; Enter or Down into them, Up back to the box, Esc closes them), item page buttons, seasons and episodes, Back keeping the cursor; Enter on Resume plays and Esc returns focus to the item page.

✅ Item page pickers by keyboard (Test Movie: version to 720p rebuilt the track pickers, then audio and subtitles stepped), and W on a season's first episode (watched off and on again on the server).

- ⬜ With a **real keyboard**: the ring appears on the first arrow key, a mouse click hides it.
✅ Top bar and library toolbar by keyboard: Up from Home's first row into the top bar and back down (the cursor returns to its card), the account menu's highlight, a SyncPlay group created and the menu reopened with Leave highlighted, Esc closing the menu then the top bar; the library's Show stepped to Unwatched and *Clear filters*, the cursor staying in the toolbar.

✅ **egui (U3)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U3-scratch/`): Down walks Home's shelves and the page scrolls the whole shelf into view, downwards and back up (`home-keys4.png`, `home-keys-up.png`); Enter on the first card opened Collections and Esc came back to Home with the ring on the same card (`home-enter-stub.png`, `home-back.png`).

✅ **egui (U4)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U4-scratch/`): `TABBYFIN_ROUTE=search:test` shows the three groups — Movies and Shows as posters, Episodes as stills — two rows of four episodes in a 1280 px window, sorted by name (`u4-search-test2.png`); Enter in the box moved the cursor onto the first result and two Downs walked it Movies → Shows → Episodes and scrolled the Episodes group into view (`u4-search-episodes.png`); a fourth Down moved inside the episode grid to its second row, kept on screen (`u4-search-grid-row2.png`); Up and Shift+Tab from the first row went back into the box with typing continuing there (`test` → `testx`, `test` → `testy`: `u4-search-typed-x.png`, `u4-search-shifttab.png`), the debounced search then showing *No results.*; Enter on a result opened its item page and Esc came back to the results with the box focused and typing going in again (`u4-search-open-item.png`, `u4-search-back.png`); Enter with no results kept the keyboard in the box (`zzz` → `zzzq`, `u4-search-noresults-enter.png`); Ctrl+F then typing `test` from Home opened the results (`u4-search-typed-from-home.png`) and backspacing the box empty closed them again (`u4-search-cleared.png`). Idle CPU on the results page was 0 ticks over 10 s.

✅ The account menu is a gpui-kit `PopupMenu`: Enter on the user button opens it, Down highlights the accounts, *Add account…* and *Sign out*, Enter on *Add account…* shows sign-in, and Esc closes it with the keyboard back on the user button.

- ⬜ With a real keyboard: the top bar after mouse use (a click elsewhere hides its ring), and the account menu switching to another saved account.
- ⬜ Changing a picker by keyboard and then by mouse (or the other way) keeps them in step, and playing after a keyboard change uses the chosen version and tracks.
- ⬜ A **large library** (hundreds of items): the grid scrolls to keep the selection visible, including rows that are still loading.
- ⬜ **Window resize** while the cursor is in the library grid or search results (the column count changes).
- ⬜ Clicking a picker or the search box and then pressing arrow keys doesn't move the cursor unexpectedly.

## 2b. Library sort and filter, favorites, cast (3.2–3.4)

✅ Saved sort and filter views (date added + favorites, name + all), the Favorite button by keyboard and the Home shelf, the cast shelf with initials and roles, a person page.

- ⬜ **Picking a sort or filter with the mouse** reloads the grid from the top and is remembered when the library is opened again; the order button flips it (and reshuffles for Random).
✅ The genres, tags and ratings dialog by keyboard: Comedy, then Documentary too, then the multi-version tag (Second Movie; both; Test Movie only), remembered per library. The dev setup gives the two movies different genres, tags and ratings.

- ⬜ **Genre, year and resolution menus** picked with the mouse: each narrows the grid, they combine with Show and the letter bar, *Clear filters* resets them, and they're remembered per library. (Checked with values set in the config: Documentary + 2020s + 4K on 12.2.)
- ⬜ A library with **real metadata** (TMDb): sorting by rating, release date and runtime gives sensible orders; cast **photos** show.
- ⬜ A person page for someone in **many** movies and shows.
- ⬜ Favoriting from another device shows up in the Home shelf (via the live connection).
- ⬜ The player's chapter list with the mouse: a click seeks to the chapter (the list was only seen opened, with the current chapter highlighted).

✅ The *Chapters* shelf on 12.2: images on both versions' servers, and the second chapter by keyboard started playback at 0:10.

✅ Playlist editing on 12.2 with the keyboard: Alt+Down then Delete on the first entry (the server's order matched), *Add to playlist* to an existing playlist, again (says it's already there), to a new one, and *Delete playlist*. The API round trip is in the `smoke` example (both versions).

✅ *Edit* on a playlist by keyboard: renamed, shared with everyone (the server's `OpenAccess`), and the `smoke` example checks private creation, rename and sharing on both versions.

✅ **egui (U7, round 2)**, against 12.2 through a local proxy that holds playlist requests for 2.5 s: two Deletes on Test Playlist (S02E01, then S01E01) followed by Esc 1.5 s before the first request left — the page was gone, and both edits still reached the server in order (`/Playlists/…/Items` ended as [Second Movie]); the same with a `dev2`-owned playlist: the refused Delete answered after Back, Home showed "This playlist belongs to someone else; it can't be changed." and the server's order was unchanged (`r2-refused-after-back.png`). A refused move on an open page still reloads it to the server's order with the toast (`r2-refused.png`). Five Alt+Ups 30 ms apart still end in the order shown after the page's reload (`r2-playlist-order.png`). Down into a playlist's first entry or a collection's first card keeps the page's header in view (`r2-playlist-order.png`, `r2-collection.png`), and Tab into an open season's band scrolls just enough to show the band with the seasons above still there (`r2-band.png`).

✅ **egui (U7)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U7-scratch/`): fast repeated moves on a playlist end in the order shown, checked with a reload — five Alt+Up presses 30 ms apart on the third of three entries moved it to the top, the page reloaded to the server's order (`playlist-alt-up5.png`; `/Playlists/…/Items` gave S01E01, Second Movie, S02E01), and two Alt+Down presses on the first entry did the same downwards earlier (`playlist-after-move.png`); Delete removed the highlighted movie and left the cursor on the row that took its place (`playlist-delete.png`); a `dev2`-owned playlist shared with `dev` refused Alt+Down with "This playlist belongs to someone else; it can't be changed." and the page reloaded showing the server's order unchanged (`playlist-refused-toast.png`, the toast logged too). The rows carry the image, "S1:E2 · name" (or the movie's name with the year on the runtime line), three lines of overview and the Watched toggle; the entry buttons and tooltips are there but pointer-only.

✅ The item page's *More* menu by keyboard: Enter on *More* opens it (a `PopupMenu` in a controlled popover, since `DropdownMenu` only opens on a click); on a playlist it holds *Edit…* and, past a separator, *Delete playlist…*; on a movie *Add to playlist…*, whose dialog opens with the keyboard in its list. *Watched* and *Favorite* are `Toggle`s, pressed while on (a favorite movie shows *Favorite* filled).

✅ The context menu's contents and commands, built in place of the *More* menu by temporary code (the harness can't right-click): on a resumable movie *Resume from 0:29*, *Play from beginning*, *Watched*, *Favorite* (checked) and *Add to playlist…*; Enter on *Watched* marked it on the server and the page's toggle turned on. Home, the library grid and search lay out as before with the menus attached.

✅ State shown as pressed, not as `primary` or as a changing label: the SyncPlay button is selected while in a group; the group panel's *Shuffle* and *Repeat* are `Toggle`s (Repeat's tooltip says which mode), with the keyboard's ring on them; the player's speeds are a segmented `ToggleGroup` with the current one checked; an episode row's watched button is a `Toggle`, pressed for watched episodes. Seen with a scripted SyncPlay member's queue, a paused player with its settings open by temporary code, and Season 1.
- ⬜ With the mouse: clicking a speed (and the checked one, which stays), *Shuffle* and *Repeat* cycling off, all, one.
- ⬜ **Context menus with a real mouse**: right-click on a Home card, a library poster, a search result, a season and an episode row; *Add to playlist…* opens its dialog with the keyboard in it; a library or person card has no menu.

- ⬜ **Playlist editing with the mouse**: the entry buttons (move up, move down, remove) and clicking a playlist in the *Add to playlist* dialog. Quick repeated moves landing in the server's order were checked by keyboard (U7, below); as a second user (dev2), adding to dev's playlist or deleting it gives the "belongs to someone else" message (the U7 entry below checks the edit side).
- ⬜ **Collections and playlists** from a real library (TMDb collections, a long playlist): *Play all* and *Shuffle* go through the whole list, the up-next card names the next entry, and playing something else ends the queue.
- ⬜ *Play all* while in a SyncPlay group plays the list for the whole group.

## 3. Transcoding (2.2.1)

✅ On 12.2: playback at a 2 Mbit/s cap (`ContainerBitrateExceedsLimit`), seeking, an embedded subtitle delivered as a file, the English audio track picked before playing, cleanup of the server's encode on stop. On 10.11: the HLS requests with header auth.

- ⬜ **Playing through the app on 10.11** (`mise run dev:min`, sign in to port 8097).
- ⬜ **Changing audio during a transcode** (restarts at the same position) and **switching to or from an image subtitle** (PGS, burned in, also restarts).
- ⬜ A user whose **policy forbids direct play** or remuxing (Dashboard → Users → Playback), with no bitrate limit set.
- ⬜ **Remote access** over a real slow connection, e.g. a phone hotspot with the limit at 4 or 8 Mbit/s.
- ⬜ A server using **hardware transcoding** (NVENC, QSV, VAAPI).
- ⬜ Seeking far ahead in a long (feature-length) transcode.

## 4. Skip intro, up-next card, chapters (2.1.1–2.1.3)

✅ With the dev media's chapters (Intro / Episode / Credits): the skip button during the intro, Enter skipping it, PgDn to Credits without ending the file, the up-next card with its countdown, Esc dismissing it (no autoplay), autoplay into the next episode.

- ⬜ **Real media segments** from a plugin: install [Intro Skipper](https://github.com/intro-skipper/intro-skipper) on a server, let it analyse a series, and check "Skip intro" and the up-next card at the credits.
- ⬜ **Real chapter names**: anime releases (OP/ED chapters), Blu-ray rips with plain "Chapter 1…" names (no skip button should appear).
- ⬜ The skip button and up-next card **click** correctly (they sit above the controls when those are visible).
- ⬜ Autoplay off in Settings: the episode ends and the player closes.

✅ The egui build (U10), on 12.2: the skip button during the intro, Enter skipping it (0:01 → 0:12), `auto_skip_intro = true` skipping the 10 s intro by itself, the up-next card from the Outro with its countdown, Esc dismissing it (no autoplay) and Esc again stopping, autoplay into the next episode, and PgUp to the previous chapter. Screenshots and the gaps in §30.

## 5. Live connection (2.4.1)

✅ The websocket on 12.2 and 10.11 (`cargo run -p jellyfin --example socket`), a 150 s keep-alive hold; Home refreshing when an item is marked watched from elsewhere.

✅ The egui build (P3): `GET /Sessions` shows the session with `SupportsMediaControl: true` and the general commands reported on connect (so the dashboard offers a remote); unmarking an item as played from REST (as another device would) made Home drop its badge and progress bar about a second later; five episodes marked at once (a series) gave one refresh (`watch data changed elsewhere: pages refresh` once, ~0.5 s after the burst's first `UserDataChanged`); a change while something was playing gave none, and the page caught up when playback ended. Idle CPU stayed at 0 ticks over 10 s with the timer armed.

- ⬜ **Another real device**: mark something watched in the web client or a phone app; Tabbyfin's open page refreshes within a second or so.
- ⬜ **Reconnecting**: restart the server, or drop the network for a minute, while the app is open; the log shows "session websocket connected" again and refreshes still work.
- ⬜ Behind a **reverse proxy with HTTPS** (`wss://`), e.g. nginx or Caddy, including a base path like `/jellyfin`.
- ⬜ The app shows up in *Dashboard → Activity / Sessions* as a session that can play video.

## 5b. Network drops (2.2.2)

✅ In development: a server restart mid-play (ffmpeg reconnects, no interruption), and a 30 s outage with a tiny cache (EOF at 0:30, reconnect after 2 s, resumed at 0:30 to the end).

- ⬜ **Wi-Fi off for 10 s and for a minute** on a laptop, with the normal cache: playback continues, or reconnects with the "Reconnecting…" notice.
- ⬜ A **transcoded** stream (HLS) through an outage.
- ✅ Esc while "Reconnecting…" closes the player and doesn't start again (egui branch: Esc 0.9 s after killing the player host; no new session or host followed). On `main` Esc went to the lost session and was dropped.

- ⬜ **Automatic bitrate** over a real slow link (Wi-Fi, a phone hotspot, or a remote server): the measurement in the log is plausible and playback transcodes to fit. Only a mocked 4 Mbit/s and the local 1.1 Gbit/s were tried.
- ⬜ **HEVC transcodes** on a server with hardware HEVC encoding (only software x265 on the dev server).

- ⬜ The **AppImage** on a real Linux desktop (GNOME, KDE; Wayland and X11), from the file manager, on a distribution other than Ubuntu, and on a 2022-era one (Ubuntu 22.04, Debian 12) with a real GPU (in a 22.04 container the window was black: old software Vulkan).
- ⬜ The **Flatpak** on a real desktop with a GPU: GPU video and hardware decoding through `--device=dri`, sign-in kept in the keychain across restarts (Secret Service from the sandbox), audio through PulseAudio/PipeWire, and the menu entry and icon.
- ⬜ The **Windows installer from CI** (a release build inside): install, the Start-menu entry, the app running from it, upgrade from an older version while it's running, uninstall.

- ⬜ **Text size following the system** on Windows (Settings → Accessibility → Text size), GNOME (Large Text) and KDE (System Settings → Fonts), both at launch and when changed while the app runs (it applies on switching back to the app). Tried in WSL: fixed sizes (130%, 150%), and KDE's font size from a stand-in `kdeglobals`, at launch and changed after launch.

## 6. SyncPlay (2.4.3–2.4.6)

✅ Against the scripted member (`cargo run -p jellyfin --example syncplay`): following the group (load, Ready, Unpause, Pause, Unpause, Seek, Stop) and leading it (Play on an item page, Space, →, Esc); the popover in and out of a group; drift correction catching up 0.8 s.

✅ Also against it: an audio change during a transcode (remote command) reported `Buffering`, the member paused, and both resumed at 18.6 s after the reload; shuffle, repeat and moving an entry from the SyncPlay menu by keyboard reached it as queue updates.

✅ The egui build (P2) against the scripted member: following (load, Ready, Pause, Unpause, Seek, Stop), leading (`TABBYFIN_ROUTE=play:` with `TABBYFIN_SYNCPLAY`, then Space, Space, →, Esc), joining a group that's already playing, the next episode after an episode's end, a version switch through the reload path (from a temporary hook, since the player page is U10: `Buffering`, the server had the app seek to the group's position, both resumed), switching account and quitting while in a group (the member saw the app leave), and drift catching up 0.8 s in 13 s. In WSL, check the clocks first (`docs/egui-migration.md` §6): this VM's monotonic clock ran 8% slow, which makes mpv look 8% fast to drift correction.

✅ **egui (U2)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U2-scratch/`): the SyncPlay button's panel, out of a group, listing the server's group with its *Join* and *New group* (`u2-panel-groups3.png`) and Enter on *Join* joining it (the toast, the button pressed, the panel closed and the keyboard back on the page, `u2-join2.png`); in a group, its name, "Nothing playing", "Watching: dev" and *Leave group* (`u2-q1-panel.png` from the queue run), with Enter on *Leave group* leaving (the toast "Left u2 list group"). In a group playing a two-item queue (`TABBYFIN_ROUTE=play:` with `TABBYFIN_SYNCPLAY`, stopped so the bar comes back): the queue's two entries with their names, the playing one in bold and the other muted, shuffle and repeat above them and *Leave group* below (`u2-q1-panel.png`); the keyboard's highlight walking Shuffle → Repeat → the first entry (`u2-q5-altdown.png`), and following an entry moved with Alt+Up/Alt+Down (`u2-q7-up.png`, `u2-a2-altup.png`). The server saw each action as a queue update: RepeatAll and RepeatOne (`reason: "RepeatMode"`), MoveItem both ways, SetCurrentItem from Enter on an entry (the app then played it for the group, `u2-a3-playing.png`), and RemoveItems from Delete (`u2-a6-deleted.png` is the item the server moved to afterwards). Shuffle toggled on and its PlayQueue came back with `shuffle_mode: Shuffle`. Toggles keep the panel open (the next Enter still reached it). Idle CPU with the panel open was 0 ticks over 10 s.

✅ **egui (U2, round 2)**, against 12.2, keyboard only (`U2-scratch/r2-*.png`): plain Left and Right in the open SyncPlay panel, then Space, leave the panel open (`r2-c2-arrows.png`; before the fix Left gave egui's focus to the SyncPlay button and Space pressed it), and the same in the account menu (`r2-d1-account-left-space.png`); Ctrl/Cmd+F in the panel closes it and opens the search box, typing then searching (`r2-e1-ctrl-f.png`); in a group with a two-item queue, deleting the highlighted entry moved the highlight to the queue entry that took its place (`r2-f1-entry.png`, the server's `RemoveItems`), and deleting the last entry left no highlight at all so Enter did nothing rather than leaving the group (`r2-f2-after-first-delete.png`, no `Left …` toast); a keyboard choice (`Enter` on *Leave group*) still leaves the page's cursor where it was (`r2-g1-leave-keyboard.png`). The highlight is the entry, so a reordered queue keeps the ring on the same row.

- ⬜ The panel by **mouse**: a click on the button opening and closing it, on a row's name playing the entry, on ↑/↓/✕ moving and removing it, on Shuffle and Repeat (the panel staying up), and on *Join*, *New group*, *Leave group* and *New group*'s name — and that such a click closes the panel **without moving the page's cursor** (the round-2 change; the harness can't click).
- ⬜ A **long queue** (more than the 320 pt cap) scrolling with the pointer, and the keyboard's highlight being brought into view as Up/Down walk past the cap.
- ⬜ A **version switch** in a group, from the player's version menu (the same reload path as the audio change, not clicked yet), and removing a queue entry.

- ⬜ **With the Jellyfin web client** as the other member, in a browser against the same server:
  - ⬜ Create the group in Tabbyfin, join from the web, and the other way round.
  - ⬜ Start an item from each side; both load and start together.
  - ⬜ Pause, unpause and seek from each side.
  - ⬜ Stop from each side (Esc in Tabbyfin stops the group).
  - ⬜ Join a group that's **already playing**; Tabbyfin loads the item and joins at the right position.
- ⬜ **Two Tabbyfin instances on two machines** (or two users on one server). Check by ear that the audio stays in sync for 10+ minutes.
- ⬜ **Next episode in a group**: let an episode end; the group moves on to the next episode together.
- ⬜ **Notifications** for members joining and leaving, and Leave from the popover.
- ⬜ A member **with a slow connection** or a transcode (bitrate limit on one side): the group waits for it, and it stays in sync.
- ⬜ **Drift on real hardware**: `RUST_LOG=tabbyfin::syncplay=debug` logs the drift each second; check it stays under ~0.1 s, and tune the thresholds in `syncplay.rs` if needed.
- ⬜ Known gaps, check how badly they behave: changing audio during a transcode, switching version, and the next-episode button while in a group.

## 7. Video output and hardware (2.3.4, 2.3.5, 1.5)

✅ Windows with an AMD RX 6600: ANGLE zero-copy (`d3d11va`, 4K HEVC 10-bit at ~11% CPU) and the WGL fallback. Linux: only Mesa llvmpipe in WSL (software rendering).

- ⬜ **Windows, NVIDIA** and **Windows, Intel**: the log line `video decoding:` says `d3d11va` and playback is smooth with the 4K HEVC version of Test Movie.
- ⬜ A **hybrid laptop** (Intel + NVIDIA): video shows (ANGLE and GPUI must end up on the same GPU).
- ⬜ (egui branch, V3) **Windows GPU frames on NVIDIA and Intel**: the log says `video output: GPU (ANGLE/D3D11)` and `video: GPU frames`, the picture is right (colours not swapped, not black), resizing and pausing work, no `couldn't import a GPU frame` or `skipped a GPU frame` lines beyond a resize, no tearing in fast motion (sync is `glFinish` plus the ring, no fences); then the same with ANGLE hidden (`OpenGL/D3D11 interop`).
- ⬜ (egui branch, V3) A **hybrid laptop** on Windows: the window on either GPU (Windows' per-app graphics preference): `OpenGL ES (ANGLE) context for mpv renderer=` names the window's GPU (`graphics backend=… adapter=`), and with ANGLE hidden the WGL path either shows video or falls back to software frames rather than black.
- ⬜ (egui branch, V3) A **release build** on Windows: CPU for 4K HEVC 10-bit with ANGLE, app and host together (debug: 14–21%, `docs/in-app-video.md`).
- ⬜ **Linux, AMD or Intel** (Mesa) and **Linux, NVIDIA** (proprietary driver), on **X11 and Wayland**: GPU video output, not "using software rendering".
- ⬜ (egui branch, V2) **Linux GPU frames on real hardware**, each of **AMD, Intel and NVIDIA**, each on **X11 and Wayland** (six runs): the log says `video output: GPU (OpenGL/Vulkan interop)` and `video: GPU frames`, the picture is right (not black, not garbled, colours not swapped, not upside down) through the host and with `TABBYFIN_PLAYER=embedded`; resizing during play and pausing; no `couldn't import a GPU frame` / `switching to software frames` in the log (if there is, the fallback kept it playing, but the GPU path doesn't work there); no tearing or flicker in fast motion (sync is `glFinish` plus the 3-image ring, no fences); CPU well under the software path's; `video decoding:` names a hardware decoder (`vaapi-copy`, `nvdec`…) where the GPU has one.
- ⬜ (egui branch, V2) A **hybrid laptop** on Linux (Intel + AMD or NVIDIA): the log's `OpenGL context for mpv renderer=` names the GPU the window runs on (`graphics backend=… adapter=`), found through EGL's device list when the default display is the other GPU (on WSL that path was only reached by skipping the default display in a temporary build).
- ⬜ (egui branch, V2) The **Flatpak** with a real GPU: the same, from inside the sandbox (`--device=dri`).
- ⬜ The `.deb` on a **real Ubuntu desktop** (only tested in a container).
- ⬜ **macOS** on Apple silicon and Intel: the app starts, signs in, and plays with `brew install mpv` (software video), and the Dock and Finder show the app icon.
- ⬜ High refresh-rate displays (120/144 Hz): no judder at 24 fps (2.3.7).

## 7b. Player host process (2.2.3)

✅ Software video (Linux) and ANGLE/D3D11 + d3d11va (Windows, AMD) through the host; killing the host mid-play resumes in a new one on both; no leftover processes after quitting.

- ⬜ **Linux with a real GPU:** GPU frames through the host (Vulkan memory fds over the socket). Watch for "a frame's memory never arrived" in the log.
- ⬜ **macOS:** playback works through the host (software frames over the pipe), and CPU use is reasonable at 1080p.
- ⬜ **Windows on NVIDIA and Intel**, and a hybrid laptop: the host's ANGLE device and GPUI must be on the same adapter for shared handles (egui branch: §7's V3 entries).
- ⬜ **A release build on Windows** (no console): the host starts without a window flashing up, and its log reaches the app's log file.
- ⬜ Quitting the app during playback leaves no `tabbyfin --player-host` process behind (Task Manager / `ps`).
- ✅ (egui branch, V1) Software video in the egui window through the host and embedded: Linux (WSL, llvmpipe) and Windows (RX 6600, `TABBYFIN_VIDEO=software`); resizing re-renders at the new size on both; killing the host mid-play brings the video back in a new one (Linux). GPU frames came with V2 (Linux) and V3 (Windows); see below.
- ✅ (egui branch, V2) Linux GPU frames on WSL (wgpu on lavapipe, mpv's OpenGL on llvmpipe, `TABBYFIN_VIDEO=gpu`): the picture through the host and embedded, the right way up and in the right colours; resizing (1280×800 → 800×600 → 1600×900, and five more) brings a new ring each time with the app's fd count steady; killing the host mid-play comes back on GPU frames; paused and after leaving the player the app sits at ~0% CPU. Without `TABBYFIN_VIDEO=gpu` the llvmpipe rule keeps software frames, and with `WGPU_BACKEND=gl` the app asks for pixels by itself. A failed import (injected in a temporary build) switched the session to software frames within half a second. Real GPUs: §7.
- ✅ (egui branch, V3) Windows GPU frames on the RX 6600 (debug builds): ANGLE with `d3d11va` through the host and embedded (1080p H.264 and 4K HEVC 10-bit), and the WGL path with ANGLE hidden (`d3d11va-copy`), in the right colours; resizing (1296×759 → 900×600 → 1100×700 → 900×600 → back) brings a new ring each time with no skipped frames; killing the host mid-play comes back on ANGLE with GPU frames. CPU for 4K HEVC 10-bit with ANGLE: 14–21% of a core for app and host together, 18–19% embedded (GPUI: ~11%); `docs/in-app-video.md` has the table. Other GPUs: §7.
- ⬜ (egui branch) **macOS:** software video in the egui window (Metal), through the host.
- ✅ (egui branch, Linux) Quitting during playback stops it first: the server gets the stop report and the config's `unfinished_playback` is cleared, rather than the next launch reporting it. Closing the window waits up to 3 s for that.

## 8. Minimum server version (4.6)

✅ The API smoke test on 10.11.11 with legacy authorization off.

- ⬜ **Clicking through the app on 10.11**: sign-in (password and Quick Connect), Home, libraries, search, item pages, mark watched, playback with resume and reporting.

## 9. Window placement (4.4)

✅ In WSLg's X11 (no window manager): a resize and a move are saved to the config half a second later, and a restart reopens at the saved size. The position itself is approximated by the compositor there, so it says little about real desktops.

✅ Minimum size 800×500: a config saved at 600×400 reopens at 800×500. At that size the player's bar has every control on screen, and an item page's title keeps its line with the actions wrapped under it (they sit beside it when there's room; at 640 px wide the title squeezed to a letter a line).
- ⬜ On a real X11 desktop, whether the window drifts down on each launch: in WSLg the saved `y` grows by the title bar's height (27 px) every run, since the window is placed by its frame but saved by its client area.

✅ On Windows, two 1920×1080 monitors at 100%: a window moved to the second monitor is saved there and reopens on it at its size, and a maximized window reopens maximized on the same monitor at its last windowed size. (Before, the app did not name the display to place on; GPUI assumed the primary monitor and replaced the saved spot with the default 1536×1061 window on the primary.)

- ⬜ **macOS and Linux with a window manager**: the same round trip to a second monitor, through maximize.
- ⬜ **Mixed DPI** (e.g. a 125% monitor next to a 100% one): the saved spot and size survive.
- ⬜ **Unplug the second monitor** the window was on: the next launch opens centred at the default size instead of off-screen.
- ⬜ **Fullscreen playback**, then quit: the next launch opens windowed at the last normal bounds.
- ⬜ A move or resize followed by **closing the window within half a second**: the new bounds are saved (closing flushes the pending save).

**egui (A1)**, on the egui branch, on X11 without a window manager or compositor (the hidden `:99` display), moves and resizes made with `XMoveResizeWindow`:

✅ Moving and resizing a window to 1100×650 at (300, 120) is saved to the config half a second later — the frame's top-left in physical pixels, the content size in points and `scale = 1.0` — and a restart reopens at exactly that size and position.

✅ With no saved bounds the window opens centred at the default 1280×800 (on a 1920×1080 display: at (320, 140)).

✅ The UI zoom doesn't leak into the placement: at 125 % a resize to 1000×600 is saved as 1000×600, a launch at 125 % reopens at the saved size, and an idle launch leaves the config file untouched (0 CPU ticks over 10 s idle).

✅ A move and `WM_DELETE_WINDOW` sent in the same instant: the moved bounds are in the config after the app exits.

✅ A spot saved at (5000, 200), off the monitor, is used as saved on X11: the window opens there and the config keeps it. Moving a window off screen doesn't move the window back — the window manager's job on Linux, and `docs/PLAN.md` §9 says why.

✅ A launch with `window_maximized = true` on `Xvfb`: the app asks the platform to maximize once more after the window is up (once, logged at debug). `Xvfb` never maximizes, so the window is then saved as it really is, not maximized.

- ⬜ **Windows, one monitor at 100 %**: the round trip as on `main` (move, resize, close within half a second, reopen), and a maximized window reopening maximized with no flicker or restore-then-maximize at launch (nothing should be sent after the first frame: no "moving the window to its saved spot" in the log).
- ⬜ **Windows, HiDPI (one monitor at 150 %)**: the same round trip; the config's `scale` is 1.5 and `x`/`y` are physical pixels; reopening lands exactly on the saved spot without a visible move.
- ⬜ **Windows, mixed DPI** (e.g. a 150 % laptop and a 100 % monitor, or the reverse): a window saved on the monitor whose scale differs from the one the window is created on reopens on its saved spot (the log says "moving the window to its saved spot"; one visible jump right after launch is expected); the same maximized reopens maximized on that monitor, and un-maximizing it restores it to its saved spot.
- ⬜ **Windows, monitors changing**: a spot saved on an unplugged monitor opens the window centred ("the saved window placement is on no monitor"), one still on a second monitor reopens exactly there, one whose centre falls between monitors of different heights opens centred. Only the pure monitor rule is tested here (`cargo test -p tabbyfin window`); `become_dpi_aware` and the `EnumDisplayMonitors` path are only compiled (`cargo clippy -p tabbyfin --target x86_64-pc-windows-gnu`).
- ⬜ **Windows, maximized at 150 %**: un-maximizing after a maximized launch restores the saved windowed size, not a size scaled by 1.5.
- ⬜ **Linux with a real window manager (X11 and Wayland) and macOS**: a saved spot reopens as saved (Wayland: the size only), a maximized window reopens maximized (the second maximize after mapping is the fix for X11 window managers), and a window saved on a monitor that's gone still opens on screen.

## 10. Player mouse niceties (2.1.8)

The development harness cannot send mouse input. The cursor hiding uses `App::hide_cursor_until_mouse_moves` (the gpui-pre patch), which the platforms restore on the next mouse move.

- ✅ **Single click** on the video pauses; a **double click** toggles fullscreen and leaves the video playing (the first click's pause is undone), also in a SyncPlay group, where the group must not end up paused.
- ✅ **Wheel over the video**: one notch is 5% volume, scrolling down takes it away, and a fast spin follows the wheel rather than the event count.
- ⬜ **Trackpad** (macOS, or a Windows precision touchpad): small deltas add up instead of being ignored, and one flick does not slam the volume to 0 or 100.
- ✅ **Cursor and controls**: after 3 s without movement both disappear while playing; any movement brings both back; neither happens while paused.
- ✅ The wheel over the control bars does not change the volume.

✅ The egui build (U10): the controls hide after 3 s while playing, stay while paused, and come back with any key that acts on the player; the *hidden display*'s pointer went nowhere, so the click, wheel, trackpad and cursor rows above are unchecked there (§30).

## 11. Volume and mute in reports (2.2.4)

✅ Against the 12.2 dev server: three volume-down presses and `m` inside the app left `VolumeLevel = 85` and `IsMuted = true` in the session's `PlayState` (`GET /Sessions`). The 10.11 spec carries the same fields on `PlaybackProgressInfo` (checked), not run at runtime.

- ✅ The **web dashboard's session view** shows the app's volume and mute, and they follow a change made in the app (about a second later; the report is batched).
✅ Re-run after batching the reports: the same check gave `VolumeLevel = 85` and `IsMuted = true` about 3 s after the keys.

✅ The egui build (P3), with the dashboard's requests over REST against 12.2: `SetVolume 42`, `Mute`, `Unmute` and `VolumeUp` gave `VolumeLevel = 42`, `IsMuted = true`, `IsMuted = false` and `VolumeLevel = 47` in `/Sessions` (about 1–3 s later, the batch).

## 12. Remote control (2.2.5)

✅ Against the 12.2 dev server, driving the app's session over REST the way the dashboard does (`POST /Sessions/{id}/Playing…`, `/Command`, `/Message`): play now with a start position, pause, seek, unpause, set volume, mute, unmute, subtitles off, a message (shown as a notification), next and stop all show up in `/Sessions`. Play last plus play next with a series put the series' episodes (expanded by the server) ahead of the movie, and next went to S01E01. The 10.11.11 server sends the same `Play`, `Playstate` and `GeneralCommand` payloads (captured, not run against the app).

✅ The egui build (P3), same way: play now from 10 s, pause, seek, unpause, volume, mute, unmute, `SetSubtitleStreamIndex 3` and `SetAudioStreamIndex 2` (both in `PlayState`), `PreviousTrack` (back to the start, then to S01E01), a message (a toast, screenshot in the report), play next (series) plus play last (movie), next twice (S01E01, then S01E02), and stop. In a SyncPlay group with the scripted member as dev2, remote pause, unpause and seek reached the member as group commands, remote play next added the episode to the group's queue (`QueueNext` at the member), and remote stop stopped both. Round 2: two play requests sent 25 ms apart, both still fetching (play next Second Movie, then play last Test Movie), played Second Movie and then Test Movie on `NextTrack`, i.e. they applied in the order they were made.

- ✅ From the **web dashboard** (and a phone app): the session shows a remote control, its buttons work, and the volume and tracks it shows follow the app.
- ⬜ **Fullscreen** from the dashboard toggles the window (the harness can't check fullscreen; P3 verified the event reaches the app, and winit asks the window manager, which the hidden display doesn't have).
- ✅ In a **SyncPlay group**: remote pause and seek act on the whole group, and play next/last add to the group's queue (egui build, P3, with the scripted member).
- ⬜ Changing the **audio track** remotely during a transcode restarts the stream at the same position.

## 13. Speed, delays, subtitle size and the clock (2.1.5–2.1.7)

✅ With `TABBYFIN_KEYS`: `]` twice set 1.5× (the test pattern's timecode advanced about 1.5 s per second, and the badge showed "1.5×"), and `x` three times set the subtitle delay to +0.3 s, as the menu showed (opened by default for that screenshot only).

- ✅ **Clicking** through the menu: each speed, the delay and size buttons, Reset; the controls stay up while it's open.
- ✅ **Subtitle delay** and **size** on a real subtitle track (the dev media's text subtitles), and **audio delay** with sound.
- ✅ **Hovering the seek bar** shows the time under the mouse, close to where a click would seek.
- ⬜ **Clicking the clock** switches to the time left, and the choice survives a restart.
- ⬜ The next episode keeps the speed and size and drops the delays; a **SyncPlay** group shows "Normal in SyncPlay" and stays at 1×.

✅ The egui build (U10): `]` twice set 1.5× (the badge next to the clock), `x` twice the subtitle delay (+0.2 s, shown by the settings menu opened for the screenshot only), and `show_remaining_time = true` in the config showed "-0:31 / 1:00" after the chapter's name. The menu's buttons and the clock's click are unchecked (no pointer; §30).

## 14. Theme (4.3)

✅ In WSLg with a config set to light: Home, an item page (the backdrop fades into the light background) and Settings render with the light theme. Set to follow the system, the app came up light, as WSLg reports.

- ⬜ **Switching in Settings** applies at once, without a restart, on every page.
- ⬜ **Follow the system** on Windows, macOS and a Linux desktop, including switching the system between light and dark while the app runs.
- ⬜ Every page in light: login, libraries (sort and filter menus), search, seasons and episodes, the SyncPlay popover, notifications.

**egui (A1):**

✅ `theme = "light"` in the config renders light and `"dark"` dark (Home); `text_size = 130` makes text bigger without growing the layout's fixed sizes; `text_size` unset picked up GNOME's 1.5 text scaling factor (text 1.47× measured, read and applied after the window opened rather than before it).

✅ `theme = "system"` on Linux shows light when the desktop exports no `org.freedesktop.portal.Settings` (this one; the log says so), which is what `main` showed without a signal.

✅ Following the desktop, against a fake `org.freedesktop.portal.Desktop` on a private session bus (`dbus-run-session`): it said dark at launch and the app opened dark; switching it to light and back to dark switched the app within a second each time. With KDE's font size raised from 10 to 15 pt (`XDG_CURRENT_DESKTOP=KDE`, a scratch `kdeglobals`) and the focus moved away from the window and back (`XSetInputFocus`), the app read the system text size again ("system text size changed scale=1.5"), drew text 1.5× larger, and stayed dark: applying the appearance again keeps the portal's scheme. The portal reads themselves (`ReadOne`, `Read` on portals before 1.15, and following a portal that answers neither) are covered by tests against fake portals on the session bus (`cargo test -p tabbyfin theme::tests::portal`), which skip where there's no session bus.

✅ Zoom: Ctrl+= twice from 100 % saved `ui_zoom = 1.25`, the UI drew 1.25× larger, a restart opened at 1.25, and Ctrl+0 reset it and removed the key from the config.

✅ CJK (Japanese), Arabic and Greek text render through the system font provider.

Accepted on Linux: a launch with `theme = "system"` draws its first frames light until the portal answers (well under a second here), and one with the system's text size draws at 100 % until the background read lands (that read starts a process, `gsettings`, which took several seconds under load here).

- ⬜ `theme = "system"` follows the system on Windows and macOS (egui/winit), switching while the app runs included; on a real GNOME and KDE desktop through the real portal, including Ubuntu 22.04's portal 1.14 (`Read` only).
- ⬜ Settings (U9) changing the theme, the text size (to the system's included, which then reads it) and the animations while the app runs, through `AppEvent::AppearanceChanged`; nothing raises it yet.
- ⬜ A system text size changed while the app runs on Windows (`reg`) and GNOME (`gsettings`), applied when the window gets the focus back.
- ⬜ Animations off (`animations = false`) makes a page change, a fade or a scroll jump; only the style values are unit-tested.

## 15. Previous and next (2.1.9)

✅ With `TABBYFIN_KEYS` against 12.2: Shift+P on S01E02 played S01E01, Shift+N on Test Movie played Second Movie (the next in Test Collection), and Test Movie ending did not start Second Movie. The smoke test reads a movie's collections on 12.2 and gets none (no route) on 10.11.

- ⬜ The **buttons** and their tooltips, and the up-next card offering a collection's next movie.
- ⬜ A remote **previous** from the dashboard: restarts the item, or within 5 s goes to the previous one.
- ⬜ A movie in **several collections** uses the first the server lists.

✅ The egui build (U10): Shift+P on S02E02 played S02E01 and Shift+N came back (the buttons appear for an item that has them, with their tooltips); clicking them is unchecked (§30).

## 16. Image memory (6.6)

✅ With the budget lowered to 1 MB and the keep-recent window to 1 s (a local build only): leaving Home for an item page and a library, and coming back, dropped 10, 10 and 9 images, and Home's images reloaded from disk and showed correctly.

✅ I1 (the egui build's image cache, checked on its temporary posters page before U5 landed; X3 deleted that page): with the budget lowered to 32 MB (a local build only) and `dev/fake-library.py` on :8098 serving 10,000 posters 300 ms apart, `TABBYFIN_ROUTE=posters:<fake library id>` with `TABBYFIN_KEYS="wait down*200"` logged two trims: 39 images dropped but 281 MB left (the 10 s keep-recent rule held the rest), then 259 dropped and 32.6 MB left; the process's RSS went from 563 MB to 432 MB. Only the rows on screen asked for posters: at most 6 loads were in flight, and in a 12 s scroll past about 400 cards with a 300 ms delay only 165 were requested, the rest having scrolled out of the queue before a slot freed. After the scroll stopped, with no input at all, the trim ran on its own: 34 images dropped at 219.8 MB (the keep-recent window), 10 s later 195 more and 32.6 MB left, and the RSS fell from 501 MB to 369 MB.

- ⬜ A **10k-item library**: scroll the whole grid and check the process memory levels off instead of growing, and that scrolling back up doesn't flash placeholders for long.
- ⬜ **Long session** (an hour of browsing and playback): memory stays flat.

## 17. Seek-bar previews (2.1.4)

✅ Against 12.2 with tiles generated by the dev setup, a hover forced at 0:25 in a local build showed the thumbnail from 0:20 above the time.

- ⬜ **Real hovering** along a long film: thumbnails follow the mouse, appear quickly after the first hover over a sheet's range, and stay on screen at the bar's ends.
- ⬜ An item **without tiles** shows only the time; a version switch shows the new version's tiles.
- ⬜ **10.11** (the same endpoints) with tiles generated.

⬜ The egui build (U10) draws the seek bar's hover time and thumbnail from the same `Trickplay` (the page polls it every frame); the pointer can't be driven here, so hovering and scrubbing are unchecked (§30).

## 18. Resume prompt (2.1.10)

✅ With `TABBYFIN_KEYS` against 12.2: Shift+P from S01E02 resumed S01E01 at 0:25 with "Resumed from 0:25 · Start over", and Enter restarted it from 0:00.

- ⬜ **Autoplay** into an episode with a resume point shows the card, and it goes away by itself after 10 s.
- ⬜ Clicking **Start over**; in a **SyncPlay** group it's a group seek.

✅ The egui build (U10): Shift+N to Second Movie resumed at 0:25 with "Resumed from 0:25 · Start over", and Enter started over from 0:00 (§30).

## 19. Subtitle appearance (4.2)

✅ Linux, libmpv 0.37, with a config set to yellow and a dark box: the embedded SRT showed yellow on a translucent box. The settings page shows the three pickers, and each steps with Left and Right once Tab has focused it (U9 round 2: the size went 100 % → 125 % and back to 75 %, the colour White → Yellow and the style Outline → Dark box, each in the config), the list itself staying pointer-only.

- ⬜ **Windows** (the bundled 0.38+ libmpv, which takes `sub-border-style`): box and outline both look right.
- ⬜ Changing the **size** in Settings, then in the player's menu: both agree afterwards.
- ⬜ An **ASS** subtitle keeps its own colours and style.

## 20. Alphabet jump (3.9)

✅ In the dev Movies library, typing `t` moved the highlight to Test Movie; the letter bar shows while sorted by name. `nameLessThan` counts the same, case-insensitively, on 10.11 and 12.2.

- ⬜ A **large library**: clicking letters lands on the first item with that letter, ascending and descending, and with a filter (Unwatched…).
- ⬜ Names with **articles** ("The …"), accents and digits land where the grid sorts them (the server's sort name).

## 21. Home settings (3.8)

✅ Against 12.2, with Next Up, Continue Watching, My Media and Latest set in that order, Collections hidden from My Media and Shows left out of Latest (stored through the API the way the web client does): Home showed Continue Watching (Next Up was empty), My Media without Collections, Favorites, then Latest Movies and no Latest Shows.

- ⬜ Changing **Settings → Home** in the web client, then reloading Home here (a restart, or anything that refreshes Home).
- ⬜ **10.11**: the same preferences.

✅ **egui (U3)**, against 12.2 (`U3-scratch/home-default.png`) and 10.11 (`U3-scratch/home-1011.png`, `home-1011-favorites.png`): the default order drew My Media, Continue Watching (Next Up was empty, so it had no shelf) and then Favorites; with `homesection0`–`2` set through the display preferences API (and put back afterwards) to Next Up, Continue Watching and My Media, Home drew Continue Watching, My Media and Favorites last, without Latest (`home-prefs.png`); Home's cards are 1.2× the other pages' with 24 px gaps between them; on 10.11, with Test Movie favorited, Favorites came just before Latest Shows, and unfavoriting it through the API while the app ran made the shelf go away without a restart (the watch-data refresh, `U3-scratch/home-1011-reloaded.png`); idle CPU on Home was 0 ticks over 10 s.

- ⬜ The **Latest** shelves against 12.2: this dev server's `/Items/Latest` answers `[]` for every library (so does the captured fixture), so they were only seen on 10.11 (Latest Shows; Latest Movies was empty there and had no shelf). `LatestItemsExcludes` is untested for the same reason.
- ⬜ The empty state's line with an account that has **no libraries** (the same `state::empty` widget W1 checked).

## 22. Item page extras (3.7)

✅ Against 12.2: Test Movie's page shows its genres, studio and tag, a Trailer button, Extras (Trailer, Making of Test Movie) and, with Second Movie unwatched, More like this (Second Movie). Selecting the featurette with the keyboard played it. The setup's metadata step works on 10.11 too.

✅ **egui (U6)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U6-scratch/`): Test Movie's page with the backdrop fading into the background, the poster, its genres, studio and tags, and the actions (Play, Trailer, Watched filled green and pressed, Favorite filled red and pressed, More) beside the title (`item-movie.png`); the Version picker under the header stepping 1080p → 720p with Right (`item-version-720.png`) and cycling with Enter (`item-version-final.png`); the Chapters shelf (`item-scrolled-chapters.png`) and the cursor moving on to Extras (`item-scrolled-extras.png`); More opening with the keyboard's highlight and *Add to playlist…* opening U8's dialog (`item-more-menu.png`, `item-add-dialog.png`); a playlist's More menu with *Edit…* and the separated *Delete playlist…*, and *Edit…* opening its dialog (`item-playlist-more2.png`, `item-playlist-edit2.png`); More like this (Second Movie) as a 1.2× poster card on `dev2`'s page, where everything was unplayed and the toggles were unlit (`item-dev2-similar2.png`); Watched marked through the page and shown filled at once, then unmarked again (`item-dev2-top.png`, `item-dev2-unmarked.png`); and the refused mark: behind a local proxy that answers 500 for `/UserPlayedItems` and `/UserFavoriteItems` (not stopping the shared server), Watched showed the change at once and a frame later was back with "The server returned an error (500)." (`item-refused-mark2.png`). Choosing 4K HEVC on the page and pressing Play opened the player on that version (`item-play-4k.png`), and Enter on the "Episode" chapter started the session at 0:10 (checked with `GET /Sessions`: position 20 s ten seconds in); the page reloaded on watch data changed elsewhere, the heart filling in when the API favorited the movie while it was open (`item-userdata-before.png`, `item-userdata-after.png`); in an 820×600 window the actions moved under the title (`item-narrow2.png`). Idle CPU on the page was 0 ticks over 10 s.

✅ **egui (U6, round 2)**: the backdrop is asked for at a fixed `fillWidth=1280` (a proxy's request log; twice the 1280 px it was drawn at would have been 2560), so resizing doesn't refetch it; a Watched mark refused while another item's page was on top was taken back on Back — pressing Watched again sent a `POST` (`/UserPlayedItems`), not the `DELETE` a still-played page would have sent (the refusing proxy sat on the mark for 1.2 s so the refusal landed while the first page was off screen); Ctrl+F with the More menu open closed the menu and opened the top bar's search box (`r2-ctrl-f-debug.png`); *Shuffle* on Test Show started playing S02E01 after Esc had already gone Back, checked with `GET /Sessions`. Tab while the version list is open is covered by a unit test (the list needs a click to open).

- ⬜ The **Cast & crew** shelf: this dev server's items have no people (the captured 12.2 fixture does), so the shelf didn't draw. Person cards were checked in W1.
- ⬜ Right-clicking a season, *More like this* card or episode row, and clicking the version list or the More menu (the pointer paths).
- ⬜ The **remote trailer** and a series' extras (unchanged from above).
- ⬜ A **refusal with the server down** rather than a 500: the same `Services::refused_marks` path (the delayed proxy covered the off-screen case), but a real outage also fails the requests the page might otherwise make.

## 23. Update notice (5.5)

✅ With `TABBYFIN_UPDATE_URL` pointing at a local fake `v9.9.9` release: the notification showed "Version 9.9.9 is out (this is 0.1.0)" with Download, and the check time was saved.

- ⬜ After the **first published release**: an older build shows the notice once a day; the current one doesn't; *Download* opens the release page; the setting turns it off.

## 24. Several accounts (4.5)

✅ With `dev` on 12.2 and `dev2` on 10.11 saved (tokens from a scratch config): the sign-in screen lists both; launching with `dev2` current restores it on 10.11; the account menu lists the saved accounts with the current one checked, then *Add account…* and *Sign out*; switching restores the other account with a new websocket; *Add account* shows sign-in with *Back to your account*, which returns to the same session without reconnecting; signing out deletes and revokes the token and leaves `dev2` listed; an unfinished playback recorded for `dev` isn't reported by `dev2`. The menu, switch, add/cancel and sign-out were triggered through temporary test code, since the harness can't click and gpui-kit buttons don't take Enter or Space.

✅ **egui (S1)**, against 12.2 with `dev` and `dev2` on one server: restoring `dev`; the form's server address (HTTPS tried before HTTP), username and password with Enter moving between them and signing in; Quick Connect, approved through `POST /QuickConnect/Authorize` (the endpoint the web client uses); a saved account's *continue as*; *Add account* keeping the signed-in session, *Back to your account* returning to it, and signing in as `dev2` instead (the waiting session is dropped); *Sign out* revoking the token and dropping the account from the config while `dev2` stays listed; switching accounts; *Remove* on the sign-in screen; removing the *parked* account from the Add-account form, which ends its session and takes *Back to your account* away; an expired session, which keeps the account, deletes the token and prefills the form with the error; a server stopped during restore, which waits and signs back in when it answers. The account menu's choices and the expiry event were driven with temporary test code: egui 0.37 menus don't take the keyboard yet, so the button and menu are pointer-only until U1.

✅ **egui (X7)**, on the hidden `:99` display with an empty config dir and `TABBYFIN_KEYS` typing the dev server's address at launch (no click, no Tab): the address reached the field (`X7-scratch/X7-1-address-typed-at-launch.png`), Enter connected and the username field was focused for `dev` (`X7-2-credentials-username-typed.png`), Enter moved the keyboard to the password, and Enter there signed in (`X7-3-signed-in.png`). In kittest, a failed connect puts the keyboard back in the address field, and the field keeps it when a row above it goes.

- ⬜ **Clicking** through all of the above (the egui menu needs a pointer for now).
- ⬜ Signing in to a **new server** while another account waits.
- ⬜ **Sign out with no account left** (the form prefilled with the server and username).
- ⬜ **Windows and macOS keychains** with two accounts.
- ⬜ Switching **during playback** and **in a SyncPlay group** (the group is left first).

## 25. Playing a series or season (3.12)

✅ Against 12.2: Test Show (S1:E1 watched, S1:E2 partway) shows *Resume S1:E2*, *Play from beginning* and *Shuffle*; Enter on it resumed S01E02 at its position.

✅ **egui (U6)**, against 12.2: Test Show's page shows *Resume S1:E1* (this dev account's S1:E1 now has a resume point), *Play from beginning*, *Shuffle*, Watched and Favorite, named after the episode it plays (`item-series.png`). The children rows (seasons, episodes) are U7's.

✅ **egui (U7)**, against 12.2, keyboard only (`U7-scratch/`): Test Show's page shows "Seasons" as two 1.2× poster cards with their played badges (`series-top.png`); Enter on Season 1 (after Down, Tab) opens its episodes in the tinted band under the seasons, S1:E1 marked with the cursor (`series-open.png`); Tab moves into the band and Enter opens the episode's page (`episode-open.png`); Enter again on the open season closes the band (`season-closed.png`); the season's own page lists its episodes as rows with "S1:E1 · Test Show S01E01", the runtime and the Watched toggle (`season-rows.png`); W on its first row unmarked it watched on the server and the toggle and page button followed (`season-w.png`); unmarking S01E02 through the API (`/UserPlayedItems`, in place of another client) reloaded the page and its band, the season card then saying "1 unwatched" and the S01E02 badge gone (`series-band-after.png`); the overview clamps to three lines with an ellipsis (a temporary string in `entry_row`, reverted; `season-overview-clamp-crop.png`); a test collection lists its two movies under "In this collection" (`collection.png`). Idle CPU on the page with the band open was 0 ticks over 10 s.

✅ **egui (X5)**, against 12.2, keyboard only (`X5-scratch/`): Enter on Season 1 (two Tabs from the top) scrolls the page so that the season's row and its whole band are on screen, the seasons heading with them (`band-enter-final.png`); Right then Enter on Season 2 does the same for its two episodes (`band-second-season.png`); at the minimum 800×500 window the band alone comes into view, with the season cards' labels above it (`band-enter-small.png`); Down after Enter still moves into the band and reveals it as U7 had it (`band-then-down.png`). `ui_tests/item.rs` covers the Enter reveal without a window (the first episode card's rectangle inside the window).

✅ **egui (U6, round 2)**: *Shuffle* on Test Show (`dev2`) fetched all six episodes, then played one of them (S02E01) even though Esc had already gone Back to Home when it arrived; the episode's user data was put back afterwards.

- ⬜ **Clicking** the context menu's *Play* on a series and a season card.
- ⬜ *Shuffle* on a big series, and in a SyncPlay group.
- ⬜ A series with **specials** placed between seasons.
- ⬜ A **season's** page's Play and Shuffle (U6's actions); U7 draws its rows (above).

## 26. Cards, shelves and the item menu (W1)

✅ **egui (W1)**, against 12.2, on Home's two shelves (`TABBYFIN_KEYS` only, screenshots in `W1-scratch/`): libraries as landscape cards and the newest movies and shows as poster cards, with the played badge and the progress bar from the server's user data; the cursor showing with the first arrow key and the ring following Left/Right and Down between the shelves; Enter opening the card's page and Esc coming back with the cursor where it was; an overflowing row showing the page arrows with the left one greyed out at the start (checked with the cards temporarily at 1.4× so the row didn't fit); a movie's menu ("Resume from 0:13", "Play from beginning", *Watched* unchecked, *Favorite* checked, *Add to playlist…*), opened with temporary code since the harness can't right-click; *Favorite* marked from the menu reaching the server and the page reloading with it unticked; the error state with *Retry* and the empty state, both from temporary code. An episode card ("Episode 2 · name", its name in grey and cut short) was seen on the second shelf with the episodes temporarily in its query; I1's check page, since deleted by X3, drew the same cards.

✅ **egui (W1, round 2)**, against 12.2, driving the menu's dispatcher (`Services::carry_out`) from temporary code since the harness can't right-click: *Play* on Test Movie resumed it at 0:39 and played (frames logged, 249 in 10 s, the shell's line at the bottom); *Play* on Test Show fetched its episodes and played S1:E2 from 0:37, its resume point (the "Nothing here to play." path is out of reach: the dev server's show has episodes). Stacked pages keep their own ids: the posters page's cursor worked over Home and Esc came back to Home's own cursor.

- ⬜ **Right-click** opening the menu at the pointer, and *Play*, *Play for a series or season*, *Watched*, *Favorite* and *Add to playlist…* from it (U5 and U6 wire it for their pages; Home's cards open U8's dialog already).
- ⬜ Card text not stealing the card's click, right-click and drag (architecturally checked: with `style.interaction.selectable_labels = false` a label senses hover only, and egui's hit-test gives the click to the card under it).
- ⬜ **A drag** sideways scrolling a shelf and not counting as a click on the card it ends on, the wheel over a shelf scrolling the page, and a click hiding the cursor.
- ⬜ The page arrows paging by a whole row on a big shelf, and Shift+wheel scrolling one sideways.
- ⬜ Person cards; episode cards in a season's row were drawn with U7 (§25) but not clicked, and cards' hover dimming still needs a real pointer. Chapter cards and their cursor were checked in U6 (`TESTING.md` §22).

## 27. Playlist dialogs (U8)

✅ **egui (U8)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U8-scratch/`): *Add to playlist* on "Second Movie (2025)" with the dev server's three playlists, the field focused, each row with its entry count and the first one highlighted; typing "test" narrowing the list to "smoke test", "Test Playlist" and *New playlist “test”*; Down, Down, Enter on "Test Playlist" answering "Second Movie (2025) is already in Test Playlist." and closing (12.2 adds an item twice, so the dialog looks first); typing "u8 test" and Enter making that playlist with the movie in it (the server had it, private, with one entry); *Edit playlist* on it: the name filled in, the sharing read as off from the server, a longer name typed, Down to the switch (with the keyboard ring on it), Space to share, Enter saving both (the server then said `OpenAccess` true and the new name); reopening it showed the switch on; Down, Left, Enter unsharing it with the name untouched, and Down, Right, Enter sharing a second playlist; Down, Up, Space, x typing " x" into the name, which shows Up hands the keyboard back to the field; *Delete playlist…* showing the note, Esc closing it and leaving the playlist, then Enter deleting it ("Deleted u8 test renamed.", the item 404 after); adding to an existing playlist that didn't have the movie ("Added to u8 add.", the server having both entries); a second user's shared playlist answering "You can't change u8 dev2 list; it belongs to someone else." from the add dialog (which stays open) and "This playlist belongs to someone else; it can't be changed." from *Edit*. Home sat at 0 CPU ticks over 10 s once the dialogs were closed.

- ⬜ **A real pointer**: right-clicking a card, *Add to playlist…* opening the dialog, clicking a playlist row, the Save, Cancel and Delete buttons, the sharing switch, and a click on the backdrop closing the dialog (the same `ModalResponse::should_close` path as Esc).
- ⬜ A list longer than the 320 px cap scrolling, with the highlight kept on screen.
- ⬜ The dialogs with the **server down** (the playlist list fails to load) and for an account with **no playlists** ("No playlists yet. Type a name to make one.").
- ⬜ The dialogs from the **playlist page** (U7); the item page's *More* menu opened *Add to playlist…* on a movie and *Edit…* on a playlist with U6 (`TESTING.md` §22, screenshots in `U6-scratch/`). Until those pages landed, nothing on the keyboard opened *Edit* or *Delete*: the checks above used a temporary `TABBYFIN_DIALOG` hook in `views/home.rs`, which is not in the tree.

## 28. Top bar and search (U1)

✅ **egui (U1)**, screenshots in `U1-scratch/`. The first rounds ran against a local stand-in for the dev server (Docker was down after a restart; `U1-scratch/fake-jellyfin.py`), and the same checks were then redone against 12.2 once Docker came back: the bar with the app icon, the page title and the search, SyncPlay, Settings and account buttons (`u1-home.png`); Up past Home's first row into the bar (the second Up: the first shows the cursor, as F1's cursor does), Search ringed, then Left/Right to Settings and the account initial (`u1-bar-search2.png`, `u1-bar-settings.png`, `u1-bar-account.png`, and `u1-bar-122.png` on 12.2); Down and Esc back to the page with its cursor on the card it was on; Enter on Settings opening it, the Back button appearing and Esc coming back (`u1-settings-page.png`); the account menu from the bar, the current account checked and the first entry highlighted, Down walking it to *Add account…* and *Sign out* (`u1-account-menu.png`, `u1-account-menu-down2.png`, `u1-account-signout-hl.png`), Esc closing it with the ring back on the user button and Down then returning to the page, and Enter on the other saved account switching to it (`u1-account-switch-122.png`: the log says `switching account account=dev2 on tabbyfin dev` then `restored session user=dev2`, and Home shows dev2's unplayed cards); Ctrl+F, typing opening the results page and clearing the box closing them (`u1-search-typed2.png`, `u1-search-typed-122.png`, `u1-search-cleared.png`), Esc closing the results and the box (`u1-search-esc4.png`), Enter on the bar's Search button opening and focusing it (`u1-bar-search-enter.png`), `TABBYFIN_ROUTE=search:test` (`u1-search-route.png`, `u1-search-route-122.png`), the box narrowing at 175 % zoom (`u1-zoom-search.png`) and in an 800×500 window (`u1-narrow-search.png`); the SyncPlay button answering with a toast until U2 (`u1-syncplay-toast2.png`); playback on 12.2 (`TABBYFIN_ROUTE=play:<id>`, `ao=null`): the bar gone and the player's line with Stop in its place (`u1-playing.png`), and Esc stopping playback, the line going and the bar coming back (`u1-after-player.png`). Idle CPU on Home was 0 ticks over 10 s.

✅ **egui (U1, round 2)**, against 12.2 (`U1-scratch/r2-*.png`): Backspace and Alt+Left in the account menu close it like Esc (the ring back on the user button, the page below unchanged), Ctrl/Cmd+F closes it and opens the box (typing then searches); Tab in the menu keeps the keyboard there (on the posters page, Enter chose the current account instead of pressing the Back button it would otherwise have focused); Tab and Shift+Tab in the box stay in it (typing continued: "test" became "testx"); Alt+↓ in the menu doesn't move its highlight; Ctrl+F then Esc leaves the page's cursor on the card it was on. With a temporary `Browse` stand-in for the results page (not in the tree): Down from the box moved into the results; Up and Shift+Tab from the first row came back to the box (typing continued); Enter opened an item page from the results, and Esc back to them put the keyboard in the box again (typing continued). Round-1 checks re-run too: the bar walk, `search:test`, clearing the box, the menu (Esc then Down; Down/Enter switching to dev2), the SyncPlay toast, and playback with V1's video and player bar.

- ⬜ **With a mouse**: the bar's buttons, a click on the bar's Back leaving no keyboard in the bar, opening the account menu by clicking the user button and a click outside closing it, and a click on the page hiding the bar's ring.
- ⬜ **Fullscreen while playing** (Xvfb has no window manager, so fullscreen does nothing there): *closed again when the player closes by itself*.
- ⬜ U4's results page with a **mouse** (a click on a card opening the item, a right-click giving the item menu), the failed state (the search erroring while the page is up), and a search with the window **resized** while the cursor is in the results (the column count changes, and the cursor may land outside them). Esc over another page leaving the box open with its text.

## 29. Packaging and CI (X1)

✅ **egui (X1)**, on this machine and in Docker:

- The Linux release binary needs only `libc`, `libm` and `libgcc_s` at link time; the windowing libraries, the Vulkan loader and libmpv are all `dlopen`ed, so `dpkg-shlibdeps`'s `$auto` misses them and `crates/app/Cargo.toml` names them explicitly: `libmpv2`, `libvulkan1`, `libwayland-client0`, `libxkbcommon0`, `libxkbcommon-x11-0`, `libx11-6`, `libx11-xcb1`, `libxcursor1`, `libxi6`.
- A clean Ubuntu 24.04 container with only `build-essential` and `pkg-config` compiles the app (`cargo check -p tabbyfin --locked`): eframe/winit need no `-dev` packages, because those libraries are loaded at runtime. CI's Linux job installs the same two packages.
- `packaging/package.sh x86_64-unknown-linux-gnu dist` builds the `.tar.gz` and `.deb`; both now carry `THIRD_PARTY.md` (the `.deb` under `/usr/share/doc/tabbyfin/`).
- The `.deb` installs in a clean Ubuntu 24.04 container (pulling `libmpv2` 0.37, Mesa, and the windowing libraries) and starts under Xvfb with an empty config dir, choosing Mesa's llvmpipe Vulkan adapter.
- The AppImage builds in the Ubuntu 22.04 container (`mise run package:appimage`) and starts in a clean Ubuntu 24.04 container once the windowing libraries winit loads are installed; without `libxkbcommon-x11` it panics in `xkbcommon-dl`, which is expected for an AppImage that relies on the system's windowing stack.
- The Flatpak builds and installs (`mise run package:flatpak`). Running it under this harness's Xvfb fails before the window: Xvfb listens on an abstract X11 socket, which Flatpak can't share with the sandbox. That isn't a packaging result; its real-desktop checks stay in §7.
- `mise run flatpak:sources` regenerates `packaging/flatpak/cargo-sources.json` with no changes after the egui dependency changes.
- `cargo build -p tabbyfin --target x86_64-pc-windows-gnu` (what `dev/win-run.sh` builds) succeeds; the release MSVC package itself is CI's.

- ⬜ The **GitHub Actions jobs**: this branch isn't pushed (X1 must not push), so `mise run ci:build` couldn't run; the same steps passed locally.
- ⬜ The `.deb`, AppImage and Flatpak on a **real desktop with a GPU** (the list in §7), and the Windows and macOS packages from CI.
- ⬜ **Playback** from those packaged builds; the checks above only opened the sign-in screen.

## 30. Player page (U10)

**egui (U10)**, against the 12.2 dev server with `TABBYFIN_ROUTE=play:<id>` and `TABBYFIN_KEYS`; screenshots in `U10-scratch/`. Video was software frames (llvmpipe), as before.

✅ The page draws V1's video with `main`'s bars over it: the top bar (Back, "Series · S1:E1 · name", the SyncPlay and Transcoding badges) and the bottom bar (seek bar with chapter marks, play/pause, ±10 s, mute, volume slider, the clock with the chapter's name, a speed badge, previous/next, chapters, settings, fullscreen) (`play11-autoskip.png`, `play3-keys.png`, `play23-timeleft.png`). The controls go after 3 s without mouse movement while playing (`play1.png`, `play4-upnext.png`), stay while paused (`play3-keys.png`) and with a menu open (`play17-settings.png`), and the page keeps the last frame with the spinner while the next item's session loads (`play13-resume.png`). While a session waits to reconnect, a bar with Back says "Starting playback…" (not triggered here).

✅ Keys (`play3-keys.png`, `play10-prev.png`, `play10-next.png`): Space, `]`/`[`, `x`/`z`, PgUp/PgDn, Shift+P/Shift+N, Enter (skip / *Play now* / *Start over*), Esc (fullscreen → up-next card → stop). The up-next card (`play4-upnext.png`, `play7-card.png`), Esc cancelling it (`play7-cancelled.png`) and the player closing (`play7-closed.png`); the resume card (`play14-resume.png`, `play16-resume-paused.png`) and Enter starting over (`play16-startedover.png`); the skip button (`play8-skip.png`) and Enter after it (`play8-skipped.png`); auto-skip from the config (`play11-autoskip.png`); autoplay keeping the page into the next episode's resume point (`play25-autoplay.png`).

✅ Round 2, on Test Movie (three versions, chapters "Intro"/"Episode"/"Credits" from the dev media): the clock's chapter name is the *chapter's*, not the item's kind — "· Intro" at 0:01 and "· Credits" at 0:52 on the movie (`r2-movie-intro.png`, `r2-movie-episode.png`); `main` did the same (`render_controls` appends `current_chapter`'s name with no kind check), so a movie mid-chapter reads "· Episode" because that is what the dev media calls its middle chapter. The version picker appears because the movie has three versions, now as a 140×28 bordered select with a caret and a "Version" tooltip (`r2-movie-intro.png`) instead of the 40 px filled box it was. A remote `Message` (a toast) over the player kept its look, and Tab with the toast up didn't take the keyboard: Space still reached the group as a Pause (`r2-toast-tab.png`).

✅ The badges: SyncPlay in a group joined with `TABBYFIN_SYNCPLAY=u10test`, its pause reaching the group (`play22-syncplay.png`), and Transcoding with `max_streaming_bitrate = 400000` (`play21-transcode.png`). The menus, opened by temporary code for the screenshots (the harness can't click): the settings menu's Audio, Subtitles and General tabs, its track lists (the check on the playing track), the delay and size steppers and the speed row (`play17-settings.png`, `play20-settings-subtitles.png`, `play19-settings-general.png`), and the chapters menu with the dev setup's chapter images, names, starts and the current chapter highlighted (`play18-chapters.png`). While a menu is open egui keeps the keys (§3), so `]` and `x` did nothing until it closed.

- ⬜ **With a mouse**: a click on the video pauses (a double click toggles fullscreen and puts the pause back); the wheel changes the volume by one notch per 5 % (nothing over the bars; the notch conversion is unit-tested in round 2); the seek bar's hover time and trickplay thumbnail, dragging it (the controls stay up while it's held, round 2), its chapter marks and clicking it; clicking the clock (time left, remembered); the volume slider (it sends only changes and the release, round 2); the chapter, settings and version menus from their buttons; the skip, *Play now*, *Cancel* and *Start over* buttons; the previous/next tooltips.
- ⬜ A second **version** of an item (the dev library has one per item): the picker appears and switching restarts at the same position.
- ⬜ **Fullscreen** (`f`, its button, Esc leaving it first): the hidden display has no window manager.
- ⬜ A **reconnect** (Back over the last frame) and the fallback to software frames after a failed GPU import: neither was triggered here (the import only fails on real drivers).
- ⬜ A **10.11** server, and real GPU frames on Linux (V2's own check) and Windows (V3) with the controls over them.

## 31. Library grid and filters (U5)

✅ **egui (U5)**, against 12.2 (`TABBYFIN_ROUTE=library:f137a2dd21bbc1b99aa5c0f6bf02a805:movies`), screenshots in `U5-scratch/`: the grid with 2 items, the toolbar's Sort, order, Show, *Genres, tags…*, *Any year* and *Any resolution* and the count (`u5-library2.png`); Show stepped by Enter through Unwatched (empty: "Nothing matches these filters.", `Clear filters` appears, `u5-show-unwatched.png`), Watched and Favorites (1 item, `u5-favorites.png`), the config saving `filter` each time; Sort stepped to Date added with its descending order and the config saved (`u5-sort-added.png`); the order button flipping the grid and `descending = true` (`u5-order-desc.png`); Year stepped to *2020s* (both movies, `u5-year.png`); Resolution to 4K on 12.2 (Test Movie, which has a 4K version, `u5-resolution.png`); *Clear filters* resetting every choice and leaving the cursor on the last control (`u5-clear.png`); the old single-genre view (`genre = "Comedy"`) upgraded to `genres`, applied (1 item) and saved back as `genres` (`u5-upgraded.png`).

✅ The genres, tags and ratings dialog by keyboard: Enter on the toolbar's button opening it with the first genre highlighted (`u5-values.png`), Space ticking Comedy and Down then Space Documentary (the grid behind reloaded each time, the button reading "Comedy, Documentary", `Clear filters` appearing, `u5-values-two.png`), Enter closing it with the Comic filter applied (`u5-values-entered.png`), Esc closing it over 2 items (`u5-values-escaped.png`), and a config with a genre and year the library no longer has keeping both listed in the dialog and the year picker, checked (`u5-remembered-dialog.png`).

✅ The letter jump: `TABBYFIN_KEYS="… t"` moved the highlight to Test Movie on the dev library (`u5-letter-jump.png`), and on `dev/fake-library.py`'s 10,000 movies `z` scrolled to the start of the Z names and highlighted the first one (`u5-fake-jump.png`). The letter bar shows only while sorted by name, and the grid's toolbar and grid are the cursor's two sections (Up from the grid gives the toolbar, Up from the toolbar the top bar).

✅ **Virtualised rows and pages**: on the 10,000-item fake library, 150 Downs scrolled about 1,000 items with only the visible rows drawn and only their posters asked for (259 poster requests for the whole scroll; 56 for a letter jump), the cursor's row scrolled into view by the page itself (`u5-fake-scrolled2.png`), 10,000 items counted. Idle CPU was 0 ticks over 10 s and 8 s on that page.

✅ **Watch data from elsewhere**: unmarking Second Movie as played over REST while the grid was open dropped its badge (`u5-refresh-before.png`, `u5-refresh-after.png`) without a spinner, with the items and scroll kept; the played state was put back.

✅ **10.11** (`config-template-min`, `TABBYFIN_ROUTE=library:f137a2dd21bbc1b99aa5c0f6bf02a805:movies`): the grid and toolbar load; 4K gives 0 items, as `docs/PLAN.md` §4.3 says for 10.11's first-version width check, where 12.2 finds Test Movie's 4K version (`u5-1011-4k.png`).

✅ A collections folder (`collection_type` none, browsed folder by folder) lists *Test Collection* with only Sort, order and Show (`u5-folder.png`), and `TABBYFIN_ROUTE=person:<id>` opens a `Source::Person` page (toolbar without the value/year/resolution pickers, "Nothing here yet.", `u5-person.png`). Enter on a card opened the U6 stub and Esc came back with the cursor still on that card (`u5-open-item.png`, `u5-back-library.png`).

- ⬜ **With a mouse**: clicking a card, the toolbar's `ComboBox`es (choosing an entry), the letter bar's letters, and the dialog's checkboxes and *Done*. Right-click menus and *Add to playlist…* from a card (U8's dialog; its own checks are §27).
- ⬜ **Window resize** with the cursor in the grid (the column count changes), and zoom.
- ⬜ A page request that fails mid-library and is retried after its rows scroll out and back, and the error state's *Retry*.
- ⬜ Picking sort and filter on **10.11 with a bigger library**, and the resolution choices on 10.11 for HD and SD.

## 32. Settings (U9)

**egui (U9)**, against the 12.2 dev server, keyboard only (`TABBYFIN_KEYS`, screenshots in `U9-scratch/`).

✅ The page (`settings-bar-toggle.png`, and `TABBYFIN_ROUTE=settings`): the six groups — Appearance, Playback, Subtitles, Storage, Account, About — each with its icon, its intro where `main` had one, and its options with their description and the `key · unset: value` line under them, the pickers 220 px wide on the right. Opened from the top bar's Settings button (Up twice, Right twice, Enter) and Esc going back to Home (`settings-back.png`); the theme and text size applied at once (light in `settings-pick-1.png` at 90 %, dark at 150 % in `settings-pick-last.png`); the lower groups, Account (dev on tabbyfin dev, `http://localhost:8096/`, Jellyfin 12.2.0) and About (0.1.0) in `settings-lower.png` and `settings-about.png`, scrolled for the screenshot with a temporary offset that isn't in the tree.

✅ Every option writes `main`'s config key and value, checked against the TOML: `theme = "light" | "dark" | "system"`, `text_size = 90…150` (the key dropped for the system's), `animations`, `autoplay_next_episode`, `auto_skip_intro`, `prefer_hevc` and `check_for_updates` as `true`/`false`, `subtitle_color = "white" | "yellow"`, `subtitle_style = "outline" | "box"`, `subtitle_scale` (dropped at 1.0), and `automatic_bitrate` with `max_streaming_bitrate` in bits per second (dropped for *No limit*, kept when *Automatic* is picked). The flags were flipped with Tab and Space (`settings-flags.png`: animations, autoplay, skip intros, HEVC and the update check, each in the file).

✅ **Round 2, the pickers' keys** (`r2-*.png`): Tab to a picker, then Left and Right step its value, stopping at the ends, and each step is the same save a click makes. Theme `dark` → Left → `light` (applied at once, `r2-theme-left.png`) → Left → `system`, and Right at the end leaves `dark`; text size from the system's → Right → `90` → Right → `100`, and Left at the first choice writes nothing; max streaming bitrate *No limit* → Right → `max_streaming_bitrate = 120000000` with `automatic_bitrate = false` (`r2-bitrate-120.png`, the picker reading "120 Mbit/s") → Left → *No limit* again, and Left from there → `automatic_bitrate = true` with no cap; subtitle colour White → Right → `yellow`, style Outline → Right → `box`, size 100 % → Right → `1.25` → Left → `0.75` → Left → still `0.75` (the ends). With no picker focused, Right and Left write nothing, and Right on a focused switch does nothing either.

✅ **X6, the same check again on 12.2** (`X6-scratch/t4.png`), because X3's sweep reported the pickers as dead: the bar's five buttons come first, so `TABBYFIN_KEYS="wait tab*6 left"` puts the focus on the theme picker and writes `theme = "light"`, `tab*6 left left` → `"system"`, and `tab*7 right right` → `text_size = 100` (the page came back light at 90 %). The sweep had pressed Right on the default *Dark*, the last of the three, where a step has nowhere to go, and its other runs had two or three Tabs, still in the bar. `cargo test -p tabbyfin ui_tests::settings` covers the page's Tab order and both pickers headless, so a regression fails CI without a display.

✅ *Clear* deleted `cache/images` (a file planted in it was gone) and said "Image cache cleared." (`settings-cleared.png`). *Open folder* with no `xdg-open` on `PATH` said "Couldn't open the logs folder (…)" (`settings-openlogs-error.png`); here `xdg-open` exits 0 without opening anything (the display has no file manager), so that path reports nothing, as `main`'s `open_with_system` didn't.

- ⬜ **With a mouse**: opening each picker's list and choosing from it (the list takes no keys — Tab and Left/Right are the keyboard path), the switches, *Clear* and *Open folder*. Left and Right while a list is open are egui's, not the picker's.
- ⬜ **Following the system** for the text size (the option showing the system's percentage) and a system change while the app runs; the animations switch visibly, not just in the file.
- ⬜ **Windows and macOS**: the text-size sources, the log folder, `explorer` and `open`.
- ⬜ **Subtitle size from Settings while a session runs**: the picker calls `set_subtitle_scale`, which the running session follows (only the write was checked here).

*Harness note:* an occluded window on this window-manager-less display gets logic-only passes from eframe (no `ui` runs), and a key injected by `TABBYFIN_KEYS` in one of those passes reached no frame at all, which is what made the first round-2 runs miss a step now and then. X3 made `App` inject a press only in a pass that follows one that drew, so a script's presses wait for drawing to resume (one key per frame) instead of being swallowed; the earlier workaround (keeping the window at the display's top-left, 1600×1000, raised before each key) isn't needed any more. A real window with the keyboard focus can't lose keys this way.
