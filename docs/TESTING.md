# Manual testing checklist

Things that work in development but haven't been tested where it matters: on real hardware, real servers, release builds, or with a real second person. Development happens in WSL2 without a GPU, against the local dev servers in `dev/` (see [`dev/README.md`](../dev/README.md)), with keys pressed from inside the app (`TABBYFIN_KEYS`) rather than a real mouse and keyboard.

Tick an item when it's been tried; note the hardware, OS and server version, and file anything that fails. Roadmap numbers refer to [`ROADMAP.md`](ROADMAP.md).

**The app runs on egui** (ROADMAP §7). The checks marked ✅ were run on the egui build, or don't depend on the UI (the API examples); the old GPUI build's results have been removed.

**Legend:** ✅ tested in development · ⬜ still to test

---

## 1. Release build on Windows

Everything below was built and run only as Linux debug builds. Use a CI package (`mise run ci:build`), unzipped over the old copy.

- ⬜ **Log file:** *Settings → Storage → Logs → Open folder* opens the folder, today's `tabbyfin.*.log` exists and has no colour codes in it.
- ⬜ **Settings → Playback:** the autoplay switch and the bitrate picker are remembered after a restart.
- ⬜ **Everything in §2–§6** at least once on Windows, since the GPU and ANGLE paths only exist there.

## 2. Keyboard navigation (3.1)

- ⬜ With a **real keyboard**: the ring appears on the first arrow key, a mouse click hides it.

✅ **egui (U3)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U3-scratch/`): Down walks Home's shelves and the page scrolls the whole shelf into view, downwards and back up (`home-keys4.png`, `home-keys-up.png`); Enter on the first card opened Collections and Esc came back to Home with the ring on the same card (`home-enter-stub.png`, `home-back.png`).

✅ **egui (U4)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U4-scratch/`): `TABBYFIN_ROUTE=search:test` shows the three groups — Movies and Shows as posters, Episodes as stills — two rows of four episodes in a 1280 px window, sorted by name (`u4-search-test2.png`); Enter in the box moved the cursor onto the first result and two Downs walked it Movies → Shows → Episodes and scrolled the Episodes group into view (`u4-search-episodes.png`); a fourth Down moved inside the episode grid to its second row, kept on screen (`u4-search-grid-row2.png`); Up and Shift+Tab from the first row went back into the box with typing continuing there (`test` → `testx`, `test` → `testy`: `u4-search-typed-x.png`, `u4-search-shifttab.png`), the debounced search then showing *No results.*; Enter on a result opened its item page and Esc came back to the results with the box focused and typing going in again (`u4-search-open-item.png`, `u4-search-back.png`); Enter with no results kept the keyboard in the box (`zzz` → `zzzq`, `u4-search-noresults-enter.png`); Ctrl+F then typing `test` from Home opened the results (`u4-search-typed-from-home.png`) and backspacing the box empty closed them again (`u4-search-cleared.png`). Idle CPU on the results page was 0 ticks over 10 s.

- ⬜ With a real keyboard: the top bar after mouse use (a click elsewhere hides its ring), and the account menu switching to another saved account.
- ⬜ Changing a picker by keyboard and then by mouse (or the other way) keeps them in step, and playing after a keyboard change uses the chosen version and tracks.
- ⬜ A **large library** (hundreds of items): the grid scrolls to keep the selection visible, including rows that are still loading.
- ⬜ Clicking a picker or the search box and then pressing arrow keys doesn't move the cursor unexpectedly.

## 2b. Library sort and filter, favorites, cast (3.2–3.4)

- ⬜ A library with **real metadata** (TMDb): sorting by rating, release date and runtime gives sensible orders; cast **photos** show.
- ⬜ A person page for someone in **many** movies and shows.
- ⬜ Favoriting from another device shows up in the Home shelf (via the live connection).

✅ **egui (U7, round 2)**, against 12.2 through a local proxy that holds playlist requests for 2.5 s: two Deletes on Test Playlist (S02E01, then S01E01) followed by Esc 1.5 s before the first request left — the page was gone, and both edits still reached the server in order (`/Playlists/…/Items` ended as [Second Movie]); the same with a `dev2`-owned playlist: the refused Delete answered after Back, Home showed "This playlist belongs to someone else; it can't be changed." and the server's order was unchanged (`r2-refused-after-back.png`). A refused move on an open page still reloads it to the server's order with the toast (`r2-refused.png`). Five Alt+Ups 30 ms apart still end in the order shown after the page's reload (`r2-playlist-order.png`). Down into a playlist's first entry or a collection's first card keeps the page's header in view (`r2-playlist-order.png`, `r2-collection.png`), and Tab into an open season's band scrolls just enough to show the band with the seasons above still there (`r2-band.png`).

✅ **egui (U7)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U7-scratch/`): fast repeated moves on a playlist end in the order shown, checked with a reload — five Alt+Up presses 30 ms apart on the third of three entries moved it to the top, the page reloaded to the server's order (`playlist-alt-up5.png`; `/Playlists/…/Items` gave S01E01, Second Movie, S02E01), and two Alt+Down presses on the first entry did the same downwards earlier (`playlist-after-move.png`); Delete removed the highlighted movie and left the cursor on the row that took its place (`playlist-delete.png`); a `dev2`-owned playlist shared with `dev` refused Alt+Down with "This playlist belongs to someone else; it can't be changed." and the page reloaded showing the server's order unchanged (`playlist-refused-toast.png`, the toast logged too). The rows carry the image, "S1:E2 · name" (or the movie's name with the year on the runtime line), three lines of overview and the Watched toggle; the entry buttons and tooltips are there but pointer-only.

- ⬜ With the mouse: clicking a speed (and the checked one, which stays), *Shuffle* and *Repeat* cycling off, all, one.
- ⬜ **Context menus with a real mouse**: right-click on a Home card, a library poster, a search result, a season and an episode row; *Add to playlist…* opens its dialog with the keyboard in it; a library or person card has no menu.

- ⬜ **Playlist editing with the mouse**: the entry buttons (move up, move down, remove) and clicking a playlist in the *Add to playlist* dialog. Quick repeated moves landing in the server's order were checked by keyboard (U7, below); as a second user (dev2), adding to dev's playlist or deleting it gives the "belongs to someone else" message (the U7 entry below checks the edit side).
- ⬜ **Collections and playlists** from a real library (TMDb collections, a long playlist): *Play all* and *Shuffle* go through the whole list, the up-next card names the next entry, and playing something else ends the queue.
- ⬜ *Play all* while in a SyncPlay group plays the list for the whole group.

## 3. Transcoding (2.2.1)

- ⬜ **Playing through the app on 10.11** (`mise run dev:min`, sign in to port 8097).
- ⬜ **Changing audio during a transcode** (restarts at the same position) and **switching to or from an image subtitle** (PGS, burned in, also restarts).
- ⬜ A user whose **policy forbids direct play** or remuxing (Dashboard → Users → Playback), with no bitrate limit set.
- ⬜ **Remote access** over a real slow connection, e.g. a phone hotspot with the limit at 4 or 8 Mbit/s.
- ⬜ A server using **hardware transcoding** (NVENC, QSV, VAAPI).
- ⬜ Seeking far ahead in a long (feature-length) transcode.

## 4. Skip intro, up-next card, chapters (2.1.1–2.1.3)

- ⬜ **Real media segments** from a plugin: install [Intro Skipper](https://github.com/intro-skipper/intro-skipper) on a server, let it analyse a series, and check "Skip intro" and the up-next card at the credits.
- ⬜ **Real chapter names**: anime releases (OP/ED chapters), Blu-ray rips with plain "Chapter 1…" names (no skip button should appear).
- ⬜ Autoplay off in Settings: the episode ends and the player closes.

✅ The egui build (U10), on 12.2: the skip button during the intro, Enter skipping it (0:01 → 0:12), `auto_skip_intro = true` skipping the 10 s intro by itself, the up-next card from the Outro with its countdown, Esc dismissing it (no autoplay) and Esc again stopping, autoplay into the next episode, and PgUp to the previous chapter. Screenshots and the gaps in §30.

## 5. Live connection (2.4.1)

✅ The websocket on 12.2 and 10.11 (`cargo run -p jellyfin --example socket`), a 150 s keep-alive hold.

✅ The egui build (P3): `GET /Sessions` shows the session with `SupportsMediaControl: true` and the general commands reported on connect (so the dashboard offers a remote); unmarking an item as played from REST (as another device would) made Home drop its badge and progress bar about a second later; five episodes marked at once (a series) gave one refresh (`watch data changed elsewhere: pages refresh` once, ~0.5 s after the burst's first `UserDataChanged`); a change while something was playing gave none, and the page caught up when playback ended. Idle CPU stayed at 0 ticks over 10 s with the timer armed.

- ⬜ **Another real device**: mark something watched in the web client or a phone app; Tabbyfin's open page refreshes within a second or so.
- ⬜ **Reconnecting**: restart the server, or drop the network for a minute, while the app is open; the log shows "session websocket connected" again and refreshes still work.
- ⬜ Behind a **reverse proxy with HTTPS** (`wss://`), e.g. nginx or Caddy, including a base path like `/jellyfin`.
- ⬜ The app shows up in *Dashboard → Activity / Sessions* as a session that can play video.

## 5b. Network drops (2.2.2)

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

## 7. Video output and hardware (2.3.4, 2.3.5, 1.5)

- ⬜ (egui branch, V3) **Windows GPU frames on NVIDIA and Intel**: `video decoding:` says `d3d11va` and the 4K HEVC version of Test Movie plays smoothly; the log says `video output: GPU (ANGLE/D3D11)` and `video: GPU frames`, the picture is right (colours not swapped, not black), resizing and pausing work, no `couldn't import a GPU frame` or `skipped a GPU frame` lines beyond a resize, no tearing in fast motion (sync is `glFinish` plus the ring, no fences); then the same with ANGLE hidden (`OpenGL/D3D11 interop`).
- ⬜ (egui branch, V3) A **hybrid laptop** on Windows: the window on either GPU (Windows' per-app graphics preference): `OpenGL ES (ANGLE) context for mpv renderer=` names the window's GPU (`graphics backend=… adapter=`), and with ANGLE hidden the WGL path either shows video or falls back to software frames rather than black.
- ⬜ (egui branch, V3) A **release build** on Windows: CPU for 4K HEVC 10-bit with ANGLE, app and host together (debug: 14–21%, `docs/in-app-video.md`).
- ⬜ (egui branch, V2) **Linux GPU frames on real hardware**, each of **AMD, Intel and NVIDIA**, each on **X11 and Wayland** (six runs): the log says `video output: GPU (OpenGL/Vulkan interop)` and `video: GPU frames`, the picture is right (not black, not garbled, colours not swapped, not upside down) through the host and with `TABBYFIN_PLAYER=embedded`; resizing during play and pausing; no `couldn't import a GPU frame` / `switching to software frames` in the log (if there is, the fallback kept it playing, but the GPU path doesn't work there); no tearing or flicker in fast motion (sync is `glFinish` plus the 3-image ring, no fences); CPU well under the software path's; `video decoding:` names a hardware decoder (`vaapi-copy`, `nvdec`…) where the GPU has one; no "a frame's memory never arrived" through the host.
- ⬜ (egui branch, V2) A **hybrid laptop** on Linux (Intel + AMD or NVIDIA): the log's `OpenGL context for mpv renderer=` names the GPU the window runs on (`graphics backend=… adapter=`), found through EGL's device list when the default display is the other GPU (on WSL that path was only reached by skipping the default display in a temporary build).
- ⬜ The `.deb` on a **real Ubuntu desktop** (only tested in a container).
- ⬜ **macOS** on Apple silicon and Intel: the app starts, signs in, and plays with `brew install mpv` (software video), and the Dock and Finder show the app icon.
- ⬜ High refresh-rate displays (120/144 Hz): no judder at 24 fps (2.3.7).

## 7b. Player host process (2.2.3)

- ⬜ **A release build on Windows** (no console): the host starts without a window flashing up, and its log reaches the app's log file.
- ⬜ Quitting the app during playback leaves no `tabbyfin --player-host` process behind (Task Manager / `ps`).
- ✅ (egui branch, V1) Software video in the egui window through the host and embedded: Linux (WSL, llvmpipe) and Windows (RX 6600, `TABBYFIN_VIDEO=software`); resizing re-renders at the new size on both; killing the host mid-play brings the video back in a new one (Linux). GPU frames came with V2 (Linux) and V3 (Windows); see below.
- ✅ (egui branch, V2) Linux GPU frames on WSL (wgpu on lavapipe, mpv's OpenGL on llvmpipe, `TABBYFIN_VIDEO=gpu`): the picture through the host and embedded, the right way up and in the right colours; resizing (1280×800 → 800×600 → 1600×900, and five more) brings a new ring each time with the app's fd count steady; killing the host mid-play comes back on GPU frames; paused and after leaving the player the app sits at ~0% CPU. Without `TABBYFIN_VIDEO=gpu` the llvmpipe rule keeps software frames, and with `WGPU_BACKEND=gl` the app asks for pixels by itself. A failed import (injected in a temporary build) switched the session to software frames within half a second. Real GPUs: §7.
- ✅ (egui branch, V3) Windows GPU frames on the RX 6600 (debug builds): ANGLE with `d3d11va` through the host and embedded (1080p H.264 and 4K HEVC 10-bit), and the WGL path with ANGLE hidden (`d3d11va-copy`), in the right colours; resizing (1296×759 → 900×600 → 1100×700 → 900×600 → back) brings a new ring each time with no skipped frames; killing the host mid-play comes back on ANGLE with GPU frames. CPU for 4K HEVC 10-bit with ANGLE: 14–21% of a core for app and host together, 18–19% embedded (GPUI: ~11%); `docs/in-app-video.md` has the table. Other GPUs: §7.
- ⬜ (egui branch) **macOS:** software video in the egui window (Metal), through the host, at a reasonable CPU use at 1080p.
- ✅ (egui branch, Linux) Quitting during playback stops it first: the server gets the stop report and the config's `unfinished_playback` is cleared, rather than the next launch reporting it. Closing the window waits up to 3 s for that.

## 8. Minimum server version (4.6)

✅ The API smoke test on 10.11.11 with legacy authorization off.

- ⬜ **Clicking through the app on 10.11**: sign-in (password and Quick Connect), Home, libraries, search, item pages, mark watched, playback with resume and reporting.

## 9. Window placement (4.4)

- ⬜ **Fullscreen playback**, then quit: the next launch opens windowed at the last normal bounds.

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

The development harness cannot send mouse input. The player page hides the cursor with `CursorIcon::None` while the controls are hidden, and the platform brings it back on the next mouse move.

- ⬜ **Trackpad** (macOS, or a Windows precision touchpad): small deltas add up instead of being ignored, and one flick does not slam the volume to 0 or 100.

✅ The egui build (U10): the controls hide after 3 s while playing, stay while paused, and come back with any key that acts on the player; the *hidden display*'s pointer went nowhere, so the click, wheel, trackpad and cursor rows above are unchecked there (§30).

## 11. Volume and mute in reports (2.2.4)

- ⬜ The **web dashboard's session view** shows the app's volume and mute, and they follow a change made in the app (about a second later; the report is batched).

✅ The egui build (P3), with the dashboard's requests over REST against 12.2: `SetVolume 42`, `Mute`, `Unmute` and `VolumeUp` gave `VolumeLevel = 42`, `IsMuted = true`, `IsMuted = false` and `VolumeLevel = 47` in `/Sessions` (about 1–3 s later, the batch).

## 12. Remote control (2.2.5)

✅ The egui build (P3), same way: play now from 10 s, pause, seek, unpause, volume, mute, unmute, `SetSubtitleStreamIndex 3` and `SetAudioStreamIndex 2` (both in `PlayState`), `PreviousTrack` (back to the start, then to S01E01), a message (a toast, screenshot in the report), play next (series) plus play last (movie), next twice (S01E01, then S01E02), and stop. In a SyncPlay group with the scripted member as dev2, remote pause, unpause and seek reached the member as group commands, remote play next added the episode to the group's queue (`QueueNext` at the member), and remote stop stopped both. Round 2: two play requests sent 25 ms apart, both still fetching (play next Second Movie, then play last Test Movie), played Second Movie and then Test Movie on `NextTrack`, i.e. they applied in the order they were made.

- ⬜ From the **web dashboard** (and a phone app): the session shows a remote control, its buttons work, and the volume and tracks it shows follow the app.
- ⬜ **Fullscreen** from the dashboard toggles the window (the harness can't check fullscreen; P3 verified the event reaches the app, and winit asks the window manager, which the hidden display doesn't have).
- ✅ In a **SyncPlay group**: remote pause and seek act on the whole group, and play next/last add to the group's queue (egui build, P3, with the scripted member).
- ⬜ Changing the **audio track** remotely during a transcode restarts the stream at the same position.

## 13. Speed, delays, subtitle size and the clock (2.1.5–2.1.7)

- ⬜ **Subtitle delay** and **size** on a real subtitle track (the dev media's text subtitles), and **audio delay** with sound.
- ⬜ The next episode keeps the speed and size and drops the delays; a **SyncPlay** group shows "Normal in SyncPlay" and stays at 1×.

✅ The egui build (U10): `]` twice set 1.5× (the badge next to the clock), `x` twice the subtitle delay (+0.2 s, shown by the settings menu opened for the screenshot only), and `show_remaining_time = true` in the config showed "-0:31 / 1:00" after the chapter's name. The menu's buttons and the clock's click are unchecked (no pointer; §30).

## 14. Theme (4.3)

- ⬜ **Switching in Settings** applies at once, without a restart, on every page.
- ⬜ Every page in light: login, libraries (sort and filter menus), search, seasons and episodes, the SyncPlay popover, notifications.

**egui (A1):**

✅ `theme = "light"` in the config renders light and `"dark"` dark (Home); `text_size = 130` makes text bigger without growing the layout's fixed sizes; `text_size` unset picked up GNOME's 1.5 text scaling factor (text 1.47× measured, read and applied after the window opened rather than before it).

✅ `theme = "system"` on Linux shows light when the desktop exports no `org.freedesktop.portal.Settings` (this one; the log says so), which is what `main` showed without a signal.

✅ Following the desktop, against a fake `org.freedesktop.portal.Desktop` on a private session bus (`dbus-run-session`): it said dark at launch and the app opened dark; switching it to light and back to dark switched the app within a second each time. With KDE's font size raised from 10 to 15 pt (`XDG_CURRENT_DESKTOP=KDE`, a scratch `kdeglobals`) and the focus moved away from the window and back (`XSetInputFocus`), the app read the system text size again ("system text size changed scale=1.5"), drew text 1.5× larger, and stayed dark: applying the appearance again keeps the portal's scheme. The portal reads themselves (`ReadOne`, `Read` on portals before 1.15, and following a portal that answers neither) are covered by tests against fake portals on the session bus (`cargo test -p tabbyfin theme::tests::portal`), which skip where there's no session bus.

✅ Zoom: Ctrl+= twice from 100 % saved `ui_zoom = 1.25`, the UI drew 1.25× larger, a restart opened at 1.25, and Ctrl+0 reset it and removed the key from the config.

✅ CJK (Japanese), Arabic and Greek text render through the system font provider.

Accepted on Linux: a launch with `theme = "system"` draws its first frames light until the portal answers (well under a second here), and one with the system's text size draws at 100 % until the background read lands (that read starts a process, `gsettings`, which took several seconds under load here).

- ⬜ `theme = "system"` follows the system on Windows and macOS (egui/winit), switching while the app runs included; on a real GNOME and KDE desktop through the real portal, including Ubuntu 22.04's portal 1.14 (`Read` only).
- ✅ Animations off (`animations = false`) draws every page entry, fade, pop and slide at its end state at once (R2, §34), and the shelf's arrow jumps and the wheel steps at once (R5). The setting being switched while the app runs is still untested (§32's list).

## 15. Previous and next (2.1.9)

- ⬜ The up-next card offering a collection's next movie.
- ⬜ A movie ending doesn't start the next one in its collection (only Shift+N does).
- ⬜ A remote **previous** from the dashboard: restarts the item, or within 5 s goes to the previous one.
- ⬜ A movie in **several collections** uses the first the server lists.

✅ The egui build (U10): Shift+P on S02E02 played S02E01 and Shift+N came back (the buttons appear for an item that has them, with their tooltips); clicking them is unchecked (§30).

## 16. Image memory (6.6)

✅ I1 (the egui build's image cache, checked on its temporary posters page before U5 landed; X3 deleted that page): with the budget lowered to 32 MB (a local build only) and `dev/fake-library.py` on :8098 serving 10,000 posters 300 ms apart, `TABBYFIN_ROUTE=posters:<fake library id>` with `TABBYFIN_KEYS="wait down*200"` logged two trims: 39 images dropped but 281 MB left (the 10 s keep-recent rule held the rest), then 259 dropped and 32.6 MB left; the process's RSS went from 563 MB to 432 MB. Only the rows on screen asked for posters: at most 6 loads were in flight, and in a 12 s scroll past about 400 cards with a 300 ms delay only 165 were requested, the rest having scrolled out of the queue before a slot freed. After the scroll stopped, with no input at all, the trim ran on its own: 34 images dropped at 219.8 MB (the keep-recent window), 10 s later 195 more and 32.6 MB left, and the RSS fell from 501 MB to 369 MB.

- ⬜ A **10k-item library**: scroll the whole grid and check the process memory levels off instead of growing, and that scrolling back up doesn't flash placeholders for long.
- ⬜ **Long session** (an hour of browsing and playback): memory stays flat.

## 17. Seek-bar previews (2.1.4)

- ⬜ **Real hovering** along a long film: thumbnails follow the mouse, appear quickly after the first hover over a sheet's range, and stay on screen at the bar's ends.
- ⬜ An item **without tiles** shows only the time; a version switch shows the new version's tiles.
- ⬜ **10.11** (the same endpoints) with tiles generated.

## 18. Resume prompt (2.1.10)

- ⬜ **Autoplay** into an episode with a resume point shows the card, and it goes away by itself after 10 s.
- ⬜ *Start over* in a **SyncPlay** group is a group seek.

✅ The egui build (U10): Shift+N to Second Movie resumed at 0:25 with "Resumed from 0:25 · Start over", and Enter started over from 0:00 (§30).

## 19. Subtitle appearance (4.2)

✅ **egui (U9, round 2)**: the settings page shows the three subtitle pickers, and each steps with Left and Right once Tab has focused it (the size went 100 % → 125 % and back to 75 %, the colour White → Yellow and the style Outline → Dark box, each in the config), the list itself staying pointer-only.

- ⬜ **Windows** (the bundled 0.38+ libmpv, which takes `sub-border-style`): box and outline both look right.
- ⬜ Changing the **size** in Settings, then in the player's menu: both agree afterwards.
- ⬜ An **ASS** subtitle keeps its own colours and style.

## 20. Alphabet jump (3.9)

- ⬜ A **large library**: clicking letters lands on the first item with that letter, ascending and descending, and with a filter (Unwatched…).
- ⬜ Names with **articles** ("The …"), accents and digits land where the grid sorts them (the server's sort name).

## 21. Home settings (3.8)

- ⬜ Changing **Settings → Home** in the web client, then reloading Home here (a restart, or anything that refreshes Home).

✅ **egui (U3)**, against 12.2 (`U3-scratch/home-default.png`) and 10.11 (`U3-scratch/home-1011.png`, `home-1011-favorites.png`): the default order drew My Media, Continue Watching (Next Up was empty, so it had no shelf) and then Favorites; with `homesection0`–`2` set through the display preferences API (and put back afterwards) to Next Up, Continue Watching and My Media, Home drew Continue Watching, My Media and Favorites last, without Latest (`home-prefs.png`); Home's cards are 1.2× the other pages' with 24 px gaps between them; on 10.11, with Test Movie favorited, Favorites came just before Latest Shows, and unfavoriting it through the API while the app ran made the shelf go away without a restart (the watch-data refresh, `U3-scratch/home-1011-reloaded.png`); idle CPU on Home was 0 ticks over 10 s.

- ⬜ The **Latest** shelves against 12.2: this dev server's `/Items/Latest` answers `[]` for every library (so does the captured fixture), so they were only seen on 10.11 (Latest Shows; Latest Movies was empty there and had no shelf). `LatestItemsExcludes` is untested for the same reason.
- ⬜ The empty state's line with an account that has **no libraries** (the same `state::empty` widget W1 checked).

## 22. Item page extras (3.7)

✅ **egui (U6)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U6-scratch/`): Test Movie's page with the backdrop fading into the background, the poster, its genres, studio and tags, and the actions (Play, Trailer, Watched filled green and pressed, Favorite filled red and pressed, More) beside the title (`item-movie.png`); the Version picker under the header stepping 1080p → 720p with Right (`item-version-720.png`) and cycling with Enter (`item-version-final.png`); the Chapters shelf (`item-scrolled-chapters.png`) and the cursor moving on to Extras (`item-scrolled-extras.png`); More opening with the keyboard's highlight and *Add to playlist…* opening U8's dialog (`item-more-menu.png`, `item-add-dialog.png`); a playlist's More menu with *Edit…* and the separated *Delete playlist…*, and *Edit…* opening its dialog (`item-playlist-more2.png`, `item-playlist-edit2.png`); More like this (Second Movie) as a 1.2× poster card on `dev2`'s page, where everything was unplayed and the toggles were unlit (`item-dev2-similar2.png`); Watched marked through the page and shown filled at once, then unmarked again (`item-dev2-top.png`, `item-dev2-unmarked.png`); and the refused mark: behind a local proxy that answers 500 for `/UserPlayedItems` and `/UserFavoriteItems` (not stopping the shared server), Watched showed the change at once and a frame later was back with "The server returned an error (500)." (`item-refused-mark2.png`). Choosing 4K HEVC on the page and pressing Play opened the player on that version (`item-play-4k.png`), and Enter on the "Episode" chapter started the session at 0:10 (checked with `GET /Sessions`: position 20 s ten seconds in); the page reloaded on watch data changed elsewhere, the heart filling in when the API favorited the movie while it was open (`item-userdata-before.png`, `item-userdata-after.png`); in an 820×600 window the actions moved under the title (`item-narrow2.png`). Idle CPU on the page was 0 ticks over 10 s.

✅ **egui (U6, round 2)**: the backdrop is asked for at a fixed `fillWidth=1280` (a proxy's request log; twice the 1280 px it was drawn at would have been 2560), so resizing doesn't refetch it; a Watched mark refused while another item's page was on top was taken back on Back — pressing Watched again sent a `POST` (`/UserPlayedItems`), not the `DELETE` a still-played page would have sent (the refusing proxy sat on the mark for 1.2 s so the refusal landed while the first page was off screen); Ctrl+F with the More menu open closed the menu and opened the top bar's search box (`r2-ctrl-f-debug.png`); *Shuffle* on Test Show started playing S02E01 after Esc had already gone Back, checked with `GET /Sessions`. Tab while the version list is open is covered by a unit test (the list needs a click to open).

✅ **Popup menus** (`widgets/popup.rs`, 2026-10-29), against 12.2 with `TABBYFIN_KEYS` on Xvfb: Enter on Test Movie's *More* opened the menu at its full 220 px, right-aligned under the button, with *Add to playlist…* filled as the keyboard's highlight; Test Playlist's *More* listed *Edit…* and, past a separator, *Delete playlist…* in the danger colour (a faint red fill when highlighted), Down moving the highlight there and wrapping back; the account menu from the bar listed `dev` checked with *tabbyfin dev* in muted text, then *Add account…* and *Sign out*, Down walking it. The headless tests (`ui_tests::menus`, `widgets::popup`) cover the rest: a right-click on a season card opens its menu (*Play*, *Watched* and *Favorite* as unchecked toggles, *Add to playlist…*), picking *Favorite* closes it and sends `POST /UserFavoriteItems/…`; Tab, Left/Right stay in an open menu, Backspace and Alt+Left close it, Ctrl/Cmd+F closes it and asks for search, Esc closes it, and Enter waits for a highlight in a menu a click opened.

✅ **SyncPlay panel restyle** (2026-10-29), against 12.2 with `TABBYFIN_KEYS` on Xvfb: with no groups, the panel opened from the bar shows *Watch together* and its line, "No one is watching together right now." and *New group* as a menu row, highlighted from the keyboard; in *dev group* with a one-item queue (the scripted member queued Test Movie and stopped), the header showed the name with a green *Playing* dot and "With dev", the queue Test Movie marked ▶, Down ringed the repeat toggle and then filled the queue row with its buttons (up and down dimmed at the queue's ends, remove live), and *Leave group* in red.

- ⬜ The SyncPlay panel with a mouse and with several groups and a longer queue: hover showing a row's buttons, clicking them, a group row joining, and the panel in the light theme.

- ⬜ The **Cast & crew** shelf: this dev server's items have no people (the captured 12.2 fixture does), so the shelf didn't draw. Person cards were checked in W1.
- ⬜ Right-clicking a season, *More like this* card or episode row, and clicking the version list or the More menu (the pointer paths). With a real mouse, the popup menus: the hover highlight following the pointer and giving way to the arrows (the arrows win while the pointer rests), the menu opening at the pointer and flipping to fit near the window's edges, its fade-in (none with animations off), and both themes.
- ⬜ The shared selects (`widgets/select.rs`) on screen, in both themes: the library toolbar's, the Settings pickers (Tab to one, Enter opens its list, the focus ring on the box), the item page's version picker with the cursor's ring and the player's small one over video; and the lists drawn with `popup::row` (*Add to playlist*, genres, tags and ratings, the player's tracks) with the pointer taking the highlight.
- ⬜ The **remote trailer** and a series' extras (unchanged from above).
- ⬜ A **refusal with the server down** rather than a 500: the same `Services::refused_marks` path (the delayed proxy covered the off-screen case), but a real outage also fails the requests the page might otherwise make.

## 23. Update notice (5.5)

- ⬜ With `TABBYFIN_UPDATE_URL` pointing at a local fake `v9.9.9` release: the notification shows "Version 9.9.9 is out (this is 0.1.0)" with Download, and the check time is saved.

- ⬜ After the **first published release**: an older build shows the notice once a day; the current one doesn't; *Download* opens the release page; the setting turns it off.

## 24. Several accounts (4.5)

✅ **egui (S1)**, against 12.2 with `dev` and `dev2` on one server: restoring `dev`; the form's server address (HTTPS tried before HTTP), username and password with Enter moving between them and signing in; Quick Connect, approved through `POST /QuickConnect/Authorize` (the endpoint the web client uses); a saved account's *continue as*; *Add account* keeping the signed-in session, *Back to your account* returning to it, and signing in as `dev2` instead (the waiting session is dropped); *Sign out* revoking the token and dropping the account from the config while `dev2` stays listed; switching accounts; *Remove* on the sign-in screen; removing the *parked* account from the Add-account form, which ends its session and takes *Back to your account* away; an expired session, which keeps the account, deletes the token and prefills the form with the error; a server stopped during restore, which waits and signs back in when it answers. The account menu's choices and the expiry event were driven with temporary test code: egui 0.37 menus don't take the keyboard yet, so the button and menu are pointer-only until U1.

✅ **egui (X7)**, on the hidden `:99` display with an empty config dir and `TABBYFIN_KEYS` typing the dev server's address at launch (no click, no Tab): the address reached the field (`X7-scratch/X7-1-address-typed-at-launch.png`), Enter connected and the username field was focused for `dev` (`X7-2-credentials-username-typed.png`), Enter moved the keyboard to the password, and Enter there signed in (`X7-3-signed-in.png`). In kittest, a failed connect puts the keyboard back in the address field, and the field keeps it when a row above it goes.

- ⬜ **Clicking** through all of the above.
- ⬜ Signing in to a **new server** while another account waits.
- ⬜ **Sign out with no account left** (the form prefilled with the server and username).
- ⬜ **Windows and macOS keychains** with two accounts.
- ⬜ Switching **during playback** and **in a SyncPlay group** (the group is left first).

## 25. Playing a series or season (3.12)

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

- ⬜ **Right-click** opening the menu at the pointer, and *Play*, *Play* on a series or season, *Watched*, *Favorite* and *Add to playlist…* from it.
- ⬜ Card text not stealing the card's click, right-click and drag (architecturally checked: with `style.interaction.selectable_labels = false` a label senses hover only, and egui's hit-test gives the click to the card under it).
✅ **egui (R5)**, against 12.2, keyboard only (`R5-scratch/`): Down walked Home's shelves with the page easing between them (`home-mid-scroll.png` mid-animation, `home-rest.png` at rest), Test Movie's page scrolled from its chapters on to Extras the same way (`item-mid-scroll.png`, `item-rest.png`), and the 10,000-item fake library's grid walked a dozen rows (`gridf/g10.png` partway, `grid-rest.png` at rest) — the keyboard's reveal-into-view still lands each card on screen. The 12.2 server's own libraries have 2–6 items, so only the fake library's grid scrolls. Idle CPU after each scroll stopped was 0 ticks over 10 s. The wheel, the glide and the drag need a pointer and are covered headless below.

- ✅ **R5, headless** (`ui_tests/shelf.rs`): a press and a move drag a shelf 1:1, a release leaves it where it stopped (no coasting), dragging back past the start clamps there (no rubber band), the wheel over the row doesn't move it, and a click hides the cursor. Untested with a real pointer: the closed hand, a drag ending on another card waking that card (a drag doesn't click), and Shift+wheel (which now scrolls the page, as on `main`, rather than a row).
- ✅ **R5, headless**: an arrow glides a shelf a page over (the first frame is a step, then it eases to its end, then nothing repaints), a second click while it moves isn't needed (the maths' test covers aiming further), and with animations off the arrow jumps in one frame. The same test steps the harness at 60 fps because `egui_kittest`'s default step is a quarter second.
- ⬜ Person cards; episode cards in a season's row were drawn with U7 (§25) but not clicked, and cards' hover dimming still needs a real pointer. Chapter cards and their cursor were checked in U6 (`TESTING.md` §22).

## 27. Playlist dialogs (U8)

✅ **egui (U8)**, against 12.2, keyboard only (`TABBYFIN_KEYS`, screenshots in `U8-scratch/`): *Add to playlist* on "Second Movie (2025)" with the dev server's three playlists, the field focused, each row with its entry count and the first one highlighted; typing "test" narrowing the list to "smoke test", "Test Playlist" and *New playlist “test”*; Down, Down, Enter on "Test Playlist" answering "Second Movie (2025) is already in Test Playlist." and closing (12.2 adds an item twice, so the dialog looks first); typing "u8 test" and Enter making that playlist with the movie in it (the server had it, private, with one entry); *Edit playlist* on it: the name filled in, the sharing read as off from the server, a longer name typed, Down to the switch (with the keyboard ring on it), Space to share, Enter saving both (the server then said `OpenAccess` true and the new name); reopening it showed the switch on; Down, Left, Enter unsharing it with the name untouched, and Down, Right, Enter sharing a second playlist; Down, Up, Space, x typing " x" into the name, which shows Up hands the keyboard back to the field; *Delete playlist…* showing the note, Esc closing it and leaving the playlist, then Enter deleting it ("Deleted u8 test renamed.", the item 404 after); adding to an existing playlist that didn't have the movie ("Added to u8 add.", the server having both entries); a second user's shared playlist answering "You can't change u8 dev2 list; it belongs to someone else." from the add dialog (which stays open) and "This playlist belongs to someone else; it can't be changed." from *Edit*. Home sat at 0 CPU ticks over 10 s once the dialogs were closed.

- ⬜ **A real pointer**: right-clicking a card, *Add to playlist…* opening the dialog, clicking a playlist row, the Save, Cancel and Delete buttons, the sharing switch, and a click on the backdrop closing the dialog (the same `ModalResponse::should_close` path as Esc).
- ⬜ A list longer than the 320 px cap scrolling, with the highlight kept on screen.
- ⬜ The dialogs with the **server down** (the playlist list fails to load) and for an account with **no playlists** ("No playlists yet. Type a name to make one.").

## 28. Top bar and search (U1)

✅ **egui (U1)**, screenshots in `U1-scratch/`. The first rounds ran against a local stand-in for the dev server (Docker was down after a restart; `U1-scratch/fake-jellyfin.py`), and the same checks were then redone against 12.2 once Docker came back: the bar with the app icon, the page title and the search, SyncPlay, Settings and account buttons (`u1-home.png`); Up past Home's first row into the bar (the second Up: the first shows the cursor, as F1's cursor does), Search ringed, then Left/Right to Settings and the account initial (`u1-bar-search2.png`, `u1-bar-settings.png`, `u1-bar-account.png`, and `u1-bar-122.png` on 12.2); Down and Esc back to the page with its cursor on the card it was on; Enter on Settings opening it, the Back button appearing and Esc coming back (`u1-settings-page.png`); the account menu from the bar, the current account checked and the first entry highlighted, Down walking it to *Add account…* and *Sign out* (`u1-account-menu.png`, `u1-account-menu-down2.png`, `u1-account-signout-hl.png`), Esc closing it with the ring back on the user button and Down then returning to the page, and Enter on the other saved account switching to it (`u1-account-switch-122.png`: the log says `switching account account=dev2 on tabbyfin dev` then `restored session user=dev2`, and Home shows dev2's unplayed cards); Ctrl+F, typing opening the results page and clearing the box closing them (`u1-search-typed2.png`, `u1-search-typed-122.png`, `u1-search-cleared.png`), Esc closing the results and the box (`u1-search-esc4.png`), Enter on the bar's Search button opening and focusing it (`u1-bar-search-enter.png`), `TABBYFIN_ROUTE=search:test` (`u1-search-route.png`, `u1-search-route-122.png`), the box narrowing at 175 % zoom (`u1-zoom-search.png`) and in an 800×500 window (`u1-narrow-search.png`); the SyncPlay button answering with a toast until U2 (`u1-syncplay-toast2.png`); playback on 12.2 (`TABBYFIN_ROUTE=play:<id>`, `ao=null`): the bar gone and the player's line with Stop in its place (`u1-playing.png`), and Esc stopping playback, the line going and the bar coming back (`u1-after-player.png`). Idle CPU on Home was 0 ticks over 10 s.

✅ **egui (U1, round 2)**, against 12.2 (`U1-scratch/r2-*.png`): Backspace and Alt+Left in the account menu close it like Esc (the ring back on the user button, the page below unchanged), Ctrl/Cmd+F closes it and opens the box (typing then searches); Tab in the menu keeps the keyboard there (on the posters page, Enter chose the current account instead of pressing the Back button it would otherwise have focused); Tab and Shift+Tab in the box stay in it (typing continued: "test" became "testx"); Alt+↓ in the menu doesn't move its highlight; Ctrl+F then Esc leaves the page's cursor on the card it was on. With a temporary `Browse` stand-in for the results page (not in the tree): Down from the box moved into the results; Up and Shift+Tab from the first row came back to the box (typing continued); Enter opened an item page from the results, and Esc back to them put the keyboard in the box again (typing continued). Round-1 checks re-run too: the bar walk, `search:test`, clearing the box, the menu (Esc then Down; Down/Enter switching to dev2), the SyncPlay toast, and playback with V1's video and player bar.

✅ **egui (R4)**, against 12.2, keyboard and screen grabs only (`R4-scratch/`): the box closed to its button (`r4-closed-dark.png`, `r4-closed-light.png`) and open with the search icon inside the field at the left, where `main`'s prefix was (`r4-open-dark.png`, `r4-open-light.png`), next to `main`'s own bar (`main-closed-dark.png`, `main-closed-light.png`, `main-open.png`, striped together in `compare-dark.png` and `compare-light.png`). The open/close motion was recorded with a 30 fps screen grab (`r4-motion-dark.mp4`) rather than a screenshot, since `import` can't land inside 260 ms: the box grows from the button's left edge into the middle of the bar while the button fades out (`r4-mid-open-dark-bar.png`), and shrinks back into it with the button fading in (`r4-mid-close-dark-bar.png`). A `TABBYFIN_KEYS` run (Up into the bar, Enter on the bar's Search, typing `abc`, Esc, Esc) opened the box from the button, searched and closed it back into the button (`r4-after-esc.png`, the log in `barkeys.log`), and Down then Right afterwards moved the page's cursor, so closing the box gives the keyboard back (`r4-keys-after-close.png`). With `animations = false` in the config, a lossless 60 fps capture (`r4-animoff.mkv`) shows the box appear and go in a single frame, with the button switching with it and no fade: one frame has the closed bar and the next the open box, and likewise when it closes. Idle CPU with the box closed was 0 ticks over 10 s.

- ⬜ **Search box with a mouse**: the always-visible box takes a click, typing and its clear button; nothing here could click it.
- ⬜ **R4 at real speed**: how the 260 ms morph reads at 60 Hz on a real GPU, and a window resized while the box is opening or closing (the closing box aims at the button's current place).
- ⬜ **With a mouse**: the bar's buttons, a click on the bar's Back leaving no keyboard in the bar, opening the account menu by clicking the user button and a click outside closing it, and a click on the page hiding the bar's ring.
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
- ⬜ **Playback** from those packaged builds; the checks above only opened the sign-in screen.

## 30. Player page (U10)

**egui (U10)**, against the 12.2 dev server with `TABBYFIN_ROUTE=play:<id>` and `TABBYFIN_KEYS`; screenshots in `U10-scratch/`. Video was software frames (llvmpipe), as before.

✅ The page draws V1's video with `main`'s bars over it: the top bar (Back, "Series · S1:E1 · name", the SyncPlay and Transcoding badges) and the bottom bar (seek bar with chapter marks, play/pause, ±10 s, mute, volume slider, the clock with the chapter's name, a speed badge, previous/next, chapters, settings, fullscreen) (`play11-autoskip.png`, `play3-keys.png`, `play23-timeleft.png`). The controls go after 3 s without mouse movement while playing (`play1.png`, `play4-upnext.png`), stay while paused (`play3-keys.png`) and with a menu open (`play17-settings.png`), and the page keeps the last frame with a spinner while the next item's session loads (`play13-resume.png`, taken before R3; the loader is the mascot since). While a session waits to reconnect, a bar with Back says "Starting playback…" (not triggered here).

✅ Keys (`play3-keys.png`, `play10-prev.png`, `play10-next.png`): Space, `]`/`[`, `x`/`z`, PgUp/PgDn, Shift+P/Shift+N, Enter (skip / *Play now* / *Start over*), Esc (fullscreen → up-next card → stop). The up-next card (`play4-upnext.png`, `play7-card.png`), Esc cancelling it (`play7-cancelled.png`) and the player closing (`play7-closed.png`); the resume card (`play14-resume.png`, `play16-resume-paused.png`) and Enter starting over (`play16-startedover.png`); the skip button (`play8-skip.png`) and Enter after it (`play8-skipped.png`); auto-skip from the config (`play11-autoskip.png`); autoplay keeping the page into the next episode's resume point (`play25-autoplay.png`).

✅ Round 2, on Test Movie (three versions, chapters "Intro"/"Episode"/"Credits" from the dev media): the clock's chapter name is the *chapter's*, not the item's kind — "· Intro" at 0:01 and "· Credits" at 0:52 on the movie (`r2-movie-intro.png`, `r2-movie-episode.png`); `main` did the same (`render_controls` appends `current_chapter`'s name with no kind check), so a movie mid-chapter reads "· Episode" because that is what the dev media calls its middle chapter. The version picker appears because the movie has three versions, now as a 140×24 bordered select with a caret and a "Version" tooltip (`r2-movie-intro.png`, R6's size) instead of the 40 px filled box it was. A remote `Message` (a toast) over the player kept its look, and Tab with the toast up didn't take the keyboard: Space still reached the group as a Pause (`r2-toast-tab.png`).

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

- ⬜ **With a mouse**: clicking a card, the toolbar's selects (choosing an entry from the popup menu reloads the grid from the top and is remembered; the order button flips it and reshuffles for Random), the letter bar's letters, and the dialog's checkboxes and *Done*. Right-click menus and *Add to playlist…* from a card (U8's dialog; its own checks are §27).
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

✅ **The menu, the search and the server's settings (4.7)**, against the 12.2 dev server on the hidden display (`TABBYFIN_ROUTE=settings`, a scratch config signed in as dev): the menu with *This device* (Appearance, Playback, Subtitles, Storage, About) and *Jellyfin server* (Account, Languages, Home) beside the open section, under its banner ("Kept on this computer…", "Kept in dev's account on tabbyfin dev…"); Tab through the top bar's five buttons, the search box and the menu, and Enter on an entry opening it (Playback with its *Languages ›* link, Languages with the user's configuration loaded: *Any language* twice, *Default* mode, the three flags on); typing "subtitle" in the box showing Playback's note, the device's Subtitles group and the server's Languages rows, with 1, 5 and 3 beside them in the menu. `POST /Users/Configuration` round-trips a changed subtitle mode and audio language on 12.2 and 10.11.11 (and the old configuration was posted back). `cargo test -p tabbyfin ui_tests::settings` covers the menu opening a server section, the search across both stores, Esc clearing it, and a stepped subtitle mode posted with the rest of the configuration as the server had it; `cargo test -p jellyfin --test http` the update keeping fields the client doesn't model.

✅ **Every section on one page**, against the 12.2 dev server on the hidden display: the column opens on the device's banner with Appearance marked in the menu, Playback under it; Tab to the Languages entry and Enter scrolled Languages' heading to the top of the column, marked Languages, and left Home's heading just below the box. `cargo test -p tabbyfin ui_tests::settings` scrolls to Languages, to Home (the last and shortest section, which reaches the top through the room left under it) and back to Appearance, checking each heading's place and the menu's mark.

- ⬜ **Scrolling with the wheel**: the menu's mark follows the section at the top, and a wheel turn during a menu entry's scroll lets the mark follow again within a second.
- ⬜ **A server setting seen elsewhere**: change the subtitle mode or audio language here, then check the web client's *Settings → Playback* shows it, and that the next item starts with the track it implies.
- ⬜ *Leave watched items out of Latest*: Home's Latest rows lose their watched items once it's saved (the page bumps `user_data`).
- ⬜ **With a mouse**: the menu's entries and hover, the search box's clear button, the *Languages ›* links, and the language pickers' long lists (scrolling, choosing).
- ⬜ The page in **light**, at **150 % text** and **175 % zoom** in an 800×500 window (the menu is 190–250 px wide, the section takes the rest), and on **10.11** in the app (only the endpoint was checked there).
- ⬜ A **failed load or save**: the server's sections say so with *Retry*, and a failed save's toast and the configuration reloaded.

*Harness note:* an occluded window on this window-manager-less display gets logic-only passes from eframe (no `ui` runs), and a key injected by `TABBYFIN_KEYS` in one of those passes reached no frame at all, which is what made the first round-2 runs miss a step now and then. X3 made `App` inject a press only in a pass that follows one that drew, so a script's presses wait for drawing to resume (one key per frame) instead of being swallowed; the earlier workaround (keeping the window at the display's top-left, 1600×1000, raised before each key) isn't needed any more. A real window with the keyboard focus can't lose keys this way.

## 33. Theme tokens (R1)

**egui (R1)**, against the 12.2 dev server, keyboard only (`TABBYFIN_KEYS`, `TABBYFIN_ROUTE`, screenshots in `R1-scratch/`).

✅ The tokens behind every screen, in dark and light: Home (`r1-home-dark.png`, `r1-home-light.png`), a library's grid (`r1-library-dark.png`, `r1-library-light.png`), a movie's and a series' item page (`r1-movie-*.png`, `r1-series-*.png`), Settings (`r1-settings-*.png`) and the sign-in form (`r1-login-*.png`), next to the same screens of `main` (`main-*.png`). The backgrounds, the text and muted greys, the card titles and subtitles (14/12), the shelf headings (18), the top bar's 72 points with 24-point icons, the played badges, progress bars, the watched tick and favourite heart, the buttons' and pickers' fills and radii, and the settings groups' outlines match `main`'s.

✅ Text size 150 % scales the whole table and nothing else: the same screens at `r1-*-150-*.png` against `main-home-150.png`, `main-movie-150.png` and `main-settings-150.png`.

✅ The open search box (`r1-search-dark.png`), a season's open episodes band (`r1-series-band.png`), the player with its up-next card (`r1-player-dark.png`), and a project's menu rows' and SyncPlay panel's highlights (`accent`).

- ⬜ **The card's hover dim**: `main` faded a card to 80 % under the pointer; here it's the page's background at 20 % over the picture. Needs a mouse (the dim only shows while a card is hovered), in both themes.
- ⬜ **Home's play button**: hovering a card shows a play button in the middle of its image (larger with the pointing hand over it); clicking it resumes a Continue Watching item, plays a series' next episode, and a click elsewhere on the card still opens its page. Not shown on library cards (My Media). Needs a mouse.
- ⬜ **Toasts** in both themes: their info, success and error icons take the theme's colours, and the popup frame the tokens' fill, border and radius.
- ⬜ **A person card's initials** at the table's `text_2xl` (the dev library's items have no cast to show one).
- ⬜ **The card title's baseline** and the action buttons' centred icon and label at 90 % and 150 % zoom.
- ⬜ **With a mouse**: a primary button's hover and press (the sign-in form's *Connect*, the item page's *Resume*), and a picker's open list highlighting its current choice with the theme's `accent` (the list is pointer-only, §32).
- ⬜ **A checked settings switch** in the light theme on a white page (the box is `secondary` with a foreground tick, not `main`'s black box with a white tick — egui draws both the checked and unchecked box from one colour).

## 34. Animations (R2)

**egui (R2)**, against the 12.2 dev server (`TABBYFIN_ROUTE`, `TABBYFIN_KEYS`, screenshots in `R2-scratch/`).

✅ Home's entry at real speed, recorded at 30 fps and picked apart frame by frame (`home-entry.mp4`, `r2-home-entry-mid.png`): the page fades and rises in, the shelves come in one after another ("My Media" readable while "Continue Watching" is still faint), and the library posters fade in over their blurhash previews as they load. The stills `r2-home-stagger.png` and `r2-home-rest.png` show the stagger and the end state.

✅ With `animations = false` in the config (`r2-home-off.png`, `home-off2.mp4`): the same frame has the page, both shelves and their images at full brightness at once, with no fade or stagger, and Home is idle at 0 CPU ticks over 10 s.

✅ Animations on: Home is idle at 0 CPU ticks over 10 s once the entry is over (nothing repaints for a finished animation).

✅ The player (`player-flow2.mp4`, `r2-player-controls-fade.png`, `r2-player-prompt-slide.png`): the top and bottom bars fade in as the player opens, and the Skip intro prompt slides in from the right while it fades. `r2-player-upnext-slide.png` and `r2-player-upnext-rest.png` show the up-next card part way through its fade/slide and at rest (reached by seeking into the credits), and `r2-player-resume.png` the "Resumed from 0:20" card after Shift+N (`play_from_resume`), which is where the resume prompt appears. The mid-animation stills were taken with the durations scaled ×10 (not committed): `import` and a 30 fps screen grab can't reliably land inside 220–420 ms on this box.

✅ The item page's Watched toggle pops its icon as it turns on (`r2-item-toggle.png`, Tab then Right/Right to the toggle, Enter), and a season's episodes band rises in when it opens (Enter on a season: `r2-item-band-fade.png` part way, `r2-item-band.png` at rest, the page scrolled to it).

- ⬜ **The motion at 60 Hz on a real GPU and display**, with a real mouse showing the player's controls again after the 3 s hide (the fade replays on hidden → shown; the keys reach the same path).
- ⬜ **The pop from a click** on Watched and Favorite (the keys' Enter goes through the page's `activate`; a click sets the same `asked.action`).
- ⬜ **The prompt while it slides over real video**: every mid-slide still was caught over the loader or before a frame arrived, since the dev media is a test pattern.

## 35. Player overlay (R6)

**egui (R6)**, against the 12.2 dev server with `TABBYFIN_ROUTE=play:<id>` and `TABBYFIN_KEYS` (slow scripts: the first frame of a debug build takes ~6 s, so a key sent before it goes to the page under the player). Screenshots in `R6-scratch/`, next to `main`'s own build (`X3-scratch/main`, rebuilt for these shots) on the same hidden display.

✅ The bars, paused on Test Movie next to `main`'s (`egui-player-paused3.png` against `main-paused3.png`, side by side in `compare-bars.png`): the top and bottom gradients, 16-point padding around a 72-point top bar, 12-point gaps between the controls, the 40/48-point control buttons with their 20.8/25-point icons, the 24-point seek row with its 6-point track, the thumb (16 points, a white-50 % ring around a dark middle) at the play position, the chapter mark, the 20-point gap before the clock, the same slider for the volume, the 8-point badge radius, the 24-point version select, and the Skip intro button (32 points high, a 24-point icon, a 16-point label, radius 10). Sampled pixels of the unfilled track came out equal to `main`'s over the same video (white at 20 %).

✅ The up-next card (`egui-upnext4.png` against `main-upnext7.png`, side by side in `compare-upnext.png`): 320 points wide with its 12-point padding, the 16-point title, the muted "Up next" and "Starts in N s" lines at 14, the white *Play now* primary button with its 16-point icon and the ghost *Cancel*, 24 points from the window's edge and 120 from the bottom while the controls are up. The resume card's *Start over* uses the same primary button (`egui-upnext.png`).

✅ The menus, opened with temporary code for the shots and reverted (the harness can't click): the settings menu (`egui-menu-settings.png` against `main-menu-settings.png`, side by side in `compare-menu.png`, cropped in `egui-crop-menu.png`/`main-crop-menu.png`) — an opaque popover with radius 10, 12-point inset and soft shadow; the segmented tab bar (a `#171717` track with an 18-point pill in the popover colour on the selected tab); "Track" and the hints at 12 muted; the check-only track rows at 28, no fill on the current one; the 24-point ghost stepper buttons. The General tab (`egui-menu-general.png` against `main-menu-general.png`, side by side in `compare-menu-general.png`): the joined outline speed group with its selected segment filled with the accent. The chapters menu (`egui-menu-chapters.png` against `main-menu-chapters.png`, side by side in `compare-chapters.png`): 364 points wide, 80-point rows with their 4-point padding and radius 8, the current chapter on the accent, thumbnails, names at 14 and starts at 12.

✅ A Transcoding badge in the top bar with `max_streaming_bitrate = 400000`, 12 points after the title (`egui-transcode4.png`), and the 1.5× speed badge after the clock (`egui-upnext4.png`).

✅ The controls' fade (R2) and the prompt's slide-in are unchanged, the keys and the bars' click-occlusion are untouched (§30), and the UI tests still pass.

✅ **AccessKit:** the painted widgets name themselves, with a role and their state — the bars' control buttons (their tooltip without its key hint: "Back", "Play", "Playback settings"), the prompt's buttons, the settings tabs (`Tab`, selected), the track and version rows (`RadioButton`, checked), the speed segments, the steppers, the version picker (`ComboBox`, its value the version's name) and the two sliders (`Slider`, in seconds or percent). `views::player::tests::an_open_menu_exposes_its_tabs_and_rows` opens the real settings popup headless, checks the labels and the selected state by query, and picks another tab and row by AccessKit click. None of them is keyboard-focusable (U8's rule, as for its dialogs), so Tab still belongs to the player and no focus ring is drawn; the speed group's own rect only senses hover, so it can't take a segment's click.

- ⬜ **With a mouse**: the hover fills of the control buttons, the ghost buttons and the track rows; the slider hover (gpui-kit grows a 3-point ring around the thumb under the pointer — not ported), dragging either slider; the seek bar's hover time and trickplay thumbnail at their new place over the bar; the menus from their buttons; the dots and dividers of the speed group.

## 36. Layout details (R7)

**egui (R7)**, against the 12.2 dev server (`TABBYFIN_ROUTE`, `TABBYFIN_KEYS`, screenshots in `R7-scratch/`), next to `main`'s own build (`X3-scratch/main`) on the same hidden display. The sign-in form was shot from a copy of `config-template` with no accounts or saved server and a fresh `device_id`.

✅ The sign-in form is centred in the window, as `main`'s was, in dark (`signin-fixed-dark.png` against `main-signin-dark.png`) and light (`signin-fixed-light.png` against `main-signin-light.png`): the 380-point column's middle is the window's middle, and its steps grow around the centre.

✅ A form taller than the window scrolls instead of being cut off: 150 % text, 800×500, eight saved accounts. The address field, which has the keyboard at launch, is scrolled into view; two Tabs reach the saved accounts and scroll back to the top (`signin-tall-150-tab-top.png`), and tabbing round to the address field again reveals it at the bottom with *Connect* (`signin-tall-150-tab-end.png`). `ui_tests/login.rs` checks the same tall form headless: the focused field, then a Tabbed-to saved account, are on screen.

✅ Card titles and subtitles are left-aligned one-line labels under the image, cut short when they don't fit: Home's landscape cards (`home-dark.png`, `home-light.png` against `main-home-*.png`), a library's poster grid (`library-dark.png`, `library-light.png` against `main-library-*.png`), a season's episode rows (`season-dark.png`, `season-light.png` against `main-season-*.png`) and a series' open season's episode cards (`item-episodes-dark.png`). The episode card's "Episode N" and its muted name are the same line, as on `main`.

✅ Home's shelves carry `main`'s `px_6` on the title row and the scrolling row, its `pb_2` under the row and its 24-point title row, with the page keeping only its `py_6` (`home-*.png` against `main-home-*.png`); the item page's shelves keep their own 24-point frame inset and gain the 8 points under each row.

✅ The search box shows a clear (×) button at its right end while it has text (`search-dark.png`, `search-light.png`, against `main-search-dark.png`, `main-search-light.png`), and only then; the field leaves it the room `main`'s prefix icon takes on the left. The button names itself to AccessKit ("Clear search") and stays non-focusable, so Tab still belongs to the field.

✅ The shelf UI test (`cargo test -p tabbyfin ui_tests::shelf`) now checks that an arrow's page is the viewport less both side paddings, as `main`'s `glide_by` distance was, and the rest of `mise run ci` is green.

- ⬜ **With a mouse**: the clear button's click (it clears the text, closes the results and keeps the keyboard in the box), its hover colour, and the pointer on Home's arrows after the padding moved into the row.
- ⬜ The grid, shelves and card lines at **other text sizes and zoom** (the room under a card and the shelf's title row are fixed, the lines scale).

## Library toolbar, rating and genre/tag popover

✅ Against 12.2 (`TABBYFIN_ROUTE=library:<id>:movies`, `TABBYFIN_KEYS` into the popover): the toolbar draws *Any progress*, *Any rating*, *Genres and tags*, *Any year*, *Any resolution* on the left and *Sort*, its picker, the order arrow and the count on the right; the popover opens under its button with the search box focused, genres and tags as pills; Enter on the highlighted pill includes it (✓, white) and again excludes it (−, red; the button reads "Comedy, Not Science Fiction" and the grid drops that movie), the chosen pills sit under the search box. `mise run ci` is green (the UI tests follow the new toolbar order).

- ⬜ **With a mouse**: clicking pills (include, exclude, off), clicking outside and Esc closing the popover, the search box's typing narrowing the pills, and the Clear filters button.
- ⬜ A favorites-filtered view saved by an older build comes back as *Any progress*, and a saved multi-rating view as its first rating.
- ⬜ Home's *Latest* title opens the library sorted by *Date added* (descending) for movies, shows and any other library, and opening the library from *My Media* keeps the remembered sort.
- ⬜ Excluding on a big library (10,000 items, `dev/fake-library.py`): the id listing's time, the letter bar and scrolling with exclusions on, and on 10.11.

## 37. The sign-in mascot

**egui**, against the 12.2 dev server on the hidden display, from an empty config dir (a fresh `device_id`, dark and light), with `TABBYFIN_KEYS` typing the address, `dev` and a wrong password; a screenshot every half second.

✅ The form is one bordered panel under a 160-point mascot whose shadow falls on its top edge, in dark and light. Each step's title says what to do: *Connect to your server* (with the minimum Jellyfin version under it), *Choose an account* with saved accounts, *Sign in to {server}* (with "Jellyfin {version} at {host}" and *Change*), *Approve this sign-in* for Quick Connect. *Back to your account* sits under the panel.

✅ The mascot looks at the caret while the address and the username are typed, down and to the left at an empty field, moving right as the text grows; it shuts its eyes as soon as the password field has the keyboard; the wrong password brings the confused face and question mark, eyes still shut while the password keeps the keyboard. `ui_tests/login.rs` checks the gaze and the shut eyes headless, and `widgets/mascot.rs` the gaze's limits and the lids.

✅ The form follows the focused widget's id, not `Response::has_focus`: egui's `has_focus` also asks for the window to have the OS's focus, which the hidden display never gives, so the mascot didn't react there at first.

- ⬜ **With a mouse and a real keyboard**: *Show password* opening its eyes to read along, then shutting them again; the Quick Connect step looking at the code; the saved-accounts step; the tall form (150 % text, 800×500) with the bigger mascot above it.
- ⬜ With the **animations setting off**: the eyes jump to their target and shut at once, and nothing repaints while idle.
