# Manual testing checklist

Things that work in development but haven't been tested where it matters: on real hardware, real servers, release builds, or with a real second person. Development happens in WSL2 without a GPU, against the local dev servers in `dev/` (see [`dev/README.md`](../dev/README.md)), with keys pressed from inside the app (`JELLY_GPUI_KEYS`) rather than a real mouse and keyboard.

Tick an item when it's been tried; note the hardware, OS and server version, and file anything that fails. Roadmap numbers refer to [`ROADMAP.md`](ROADMAP.md).

**Legend:** ✅ tested in development · ⬜ still to test

---

## 1. Release build on Windows

Everything below was built and run only as Linux debug builds. Use a CI package (`mise run ci:build`), unzipped over the old copy.

- ⬜ **Player controls don't pause the video when clicked** (the `.occlude()` fix). Click the volume slider, the pickers, -10s/+10s and Fullscreen; playback must keep going.
- ⬜ **Log file:** *Settings → Logs → Open log folder* opens the folder, today's `jelly-gpui.*.log` exists and has no colour codes in it.
- ⬜ **Settings → Playback:** the autoplay switch and the bitrate picker are remembered after a restart.
- ⬜ **Everything in §2–§6** at least once on Windows, since the GPU and ANGLE paths only exist there.

## 2. Keyboard navigation (3.1)

✅ Home shelves, library grid, search (Enter into results, Up back to the box), item page buttons, seasons and episodes, Back keeping the cursor; Enter on Resume plays and Esc returns focus to the item page.

✅ Item page pickers by keyboard (Test Movie: version to 720p rebuilt the track pickers, then audio and subtitles stepped), W on a season's first episode (watched off and on again on the server), and Settings (theme to Light, autoplay off, subtitle colour to Yellow, saved to the config; the page scrolls with the cursor).

- ⬜ With a **real keyboard**: the ring appears on the first arrow key, a mouse click hides it.
✅ Top bar and library toolbar by keyboard: Up from Home's first row into the top bar and back down (the cursor returns to its card), the account menu's highlight, a SyncPlay group created and the menu reopened with Leave highlighted, Esc closing the menu then the top bar; the library's Show stepped to Unwatched and *Clear filters*, the cursor staying in the toolbar.

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

✅ The item page's *More* menu by keyboard: Enter on *More* opens it (a `PopupMenu` in a controlled popover, since `DropdownMenu` only opens on a click); on a playlist it holds *Edit…* and, past a separator, *Delete playlist…*; on a movie *Add to playlist…*, whose dialog opens with the keyboard in its list. *Watched* and *Favorite* are `Toggle`s, pressed while on (a favorite movie shows *Favorite* filled).

✅ The context menu's contents and commands, built in place of the *More* menu by temporary code (the harness can't right-click): on a resumable movie *Resume from 0:29*, *Play from beginning*, *Watched*, *Favorite* (checked) and *Add to playlist…*; Enter on *Watched* marked it on the server and the page's toggle turned on. Home, the library grid and search lay out as before with the menus attached.

✅ State shown as pressed, not as `primary` or as a changing label: the SyncPlay button is selected while in a group; the group panel's *Shuffle* and *Repeat* are `Toggle`s (Repeat's tooltip says which mode), with the keyboard's ring on them; the player's speeds are a segmented `ToggleGroup` with the current one checked; an episode row's watched button is a `Toggle`, pressed for watched episodes. Seen with a scripted SyncPlay member's queue, a paused player with its settings open by temporary code, and Season 1.
- ⬜ With the mouse: clicking a speed (and the checked one, which stays), *Shuffle* and *Repeat* cycling off, all, one.
- ⬜ **Context menus with a real mouse**: right-click on a Home card, a library poster, a search result, a season and an episode row; *Add to playlist…* opens its dialog with the keyboard in it; a library or person card has no menu.

- ⬜ **Playlist editing with the mouse**: the entry buttons, clicking a playlist in the *Add to playlist* dialog, and quick repeated moves landing in the server's order. As a second user (dev2), adding to dev's playlist or deleting it gives the "belongs to someone else" message.
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

## 5. Live connection (2.4.1)

✅ The websocket on 12.2 and 10.11 (`cargo run -p jellyfin --example socket`), a 150 s keep-alive hold; Home refreshing when an item is marked watched from elsewhere.

- ⬜ **Another real device**: mark something watched in the web client or a phone app; jelly-gpui's open page refreshes within a second or so.
- ⬜ **Reconnecting**: restart the server, or drop the network for a minute, while the app is open; the log shows "session websocket connected" again and refreshes still work.
- ⬜ Behind a **reverse proxy with HTTPS** (`wss://`), e.g. nginx or Caddy, including a base path like `/jellyfin`.
- ⬜ The app shows up in *Dashboard → Activity / Sessions* as a session that can play video.

## 5b. Network drops (2.2.2)

✅ In development: a server restart mid-play (ffmpeg reconnects, no interruption), and a 30 s outage with a tiny cache (EOF at 0:30, reconnect after 2 s, resumed at 0:30 to the end).

- ⬜ **Wi-Fi off for 10 s and for a minute** on a laptop, with the normal cache: playback continues, or reconnects with the "Reconnecting…" notice.
- ⬜ A **transcoded** stream (HLS) through an outage.
- ⬜ Esc while "Reconnecting…" closes the player and doesn't start again.

- ⬜ **Automatic bitrate** over a real slow link (Wi-Fi, a phone hotspot, or a remote server): the measurement in the log is plausible and playback transcodes to fit. Only a mocked 4 Mbit/s and the local 1.1 Gbit/s were tried.
- ⬜ **HEVC transcodes** on a server with hardware HEVC encoding (only software x265 on the dev server).

- ⬜ The **AppImage** on a real Linux desktop (GNOME, KDE; Wayland and X11), from the file manager, on a distribution other than Ubuntu, and on a 2022-era one (Ubuntu 22.04, Debian 12) with a real GPU (in a 22.04 container the window was black: old software Vulkan).
- ⬜ The **Flatpak** on a real desktop with a GPU: GPU video and hardware decoding through `--device=dri`, sign-in kept in the keychain across restarts (Secret Service from the sandbox), audio through PulseAudio/PipeWire, and the menu entry and icon.
- ⬜ The **Windows installer from CI** (a release build inside): install, the Start-menu entry, the app running from it, upgrade from an older version while it's running, uninstall.

- ⬜ **Text size following the system** on Windows (Settings → Accessibility → Text size), GNOME (Large Text) and KDE (System Settings → Fonts), both at launch and when changed while the app runs (it applies on switching back to the app). Tried in WSL: fixed sizes (130%, 150%), and KDE's font size from a stand-in `kdeglobals`, at launch and changed after launch.

## 6. SyncPlay (2.4.3–2.4.6)

✅ Against the scripted member (`cargo run -p jellyfin --example syncplay`): following the group (load, Ready, Unpause, Pause, Unpause, Seek, Stop) and leading it (Play on an item page, Space, →, Esc); the popover in and out of a group; drift correction catching up 0.8 s.

✅ Also against it: an audio change during a transcode (remote command) reported `Buffering`, the member paused, and both resumed at 18.6 s after the reload; shuffle, repeat and moving an entry from the SyncPlay menu by keyboard reached it as queue updates.

- ⬜ A **version switch** in a group, from the player's version menu (the same reload path as the audio change, not clicked yet), and removing a queue entry.

- ⬜ **With the Jellyfin web client** as the other member, in a browser against the same server:
  - ⬜ Create the group in jelly-gpui, join from the web, and the other way round.
  - ⬜ Start an item from each side; both load and start together.
  - ⬜ Pause, unpause and seek from each side.
  - ⬜ Stop from each side (Esc in jelly-gpui stops the group).
  - ⬜ Join a group that's **already playing**; jelly-gpui loads the item and joins at the right position.
- ⬜ **Two jelly-gpui instances on two machines** (or two users on one server). Check by ear that the audio stays in sync for 10+ minutes.
- ⬜ **Next episode in a group**: let an episode end; the group moves on to the next episode together.
- ⬜ **Notifications** for members joining and leaving, and Leave from the popover.
- ⬜ A member **with a slow connection** or a transcode (bitrate limit on one side): the group waits for it, and it stays in sync.
- ⬜ **Drift on real hardware**: `RUST_LOG=jelly_gpui::syncplay=debug` logs the drift each second; check it stays under ~0.1 s, and tune the thresholds in `syncplay.rs` if needed.
- ⬜ Known gaps, check how badly they behave: changing audio during a transcode, switching version, and the next-episode button while in a group.

## 7. Video output and hardware (2.3.4, 2.3.5, 1.5)

✅ Windows with an AMD RX 6600: ANGLE zero-copy (`d3d11va`, 4K HEVC 10-bit at ~11% CPU) and the WGL fallback. Linux: only Mesa llvmpipe in WSL (software rendering).

- ⬜ **Windows, NVIDIA** and **Windows, Intel**: the log line `video decoding:` says `d3d11va` and playback is smooth with the 4K HEVC version of Test Movie.
- ⬜ A **hybrid laptop** (Intel + NVIDIA): video shows (ANGLE and GPUI must end up on the same GPU).
- ⬜ **Linux, AMD or Intel** (Mesa) and **Linux, NVIDIA** (proprietary driver), on **X11 and Wayland**: GPU video output, not "using software rendering".
- ⬜ The `.deb` on a **real Ubuntu desktop** (only tested in a container).
- ⬜ **macOS** on Apple silicon and Intel: the app starts, signs in, and plays with `brew install mpv` (software video).
- ⬜ High refresh-rate displays (120/144 Hz): no judder at 24 fps (2.3.7).

## 7b. Player host process (2.2.3)

✅ Software video (Linux) and ANGLE/D3D11 + d3d11va (Windows, AMD) through the host; killing the host mid-play resumes in a new one on both; no leftover processes after quitting.

- ⬜ **Linux with a real GPU:** GPU frames through the host (Vulkan memory fds over the socket). Watch for "a frame's memory never arrived" in the log.
- ⬜ **macOS:** playback works through the host (software frames over the pipe), and CPU use is reasonable at 1080p.
- ⬜ **Windows on NVIDIA and Intel**, and a hybrid laptop: the host's ANGLE device and GPUI must be on the same adapter for shared handles.
- ⬜ **A release build on Windows** (no console): the host starts without a window flashing up, and its log reaches the app's log file.
- ⬜ Quitting the app during playback leaves no `jelly-gpui --player-host` process behind (Task Manager / `ps`).

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

## 10. Player mouse niceties (2.1.8)

Code-level and startup checks only so far: the development harness cannot send mouse input. The cursor hiding uses `App::hide_cursor_until_mouse_moves` (the gpui-pre patch), which the platforms restore on the next mouse move.

- ⬜ **Single click** on the video pauses; a **double click** toggles fullscreen and leaves the video playing (the first click's pause is undone), also in a SyncPlay group, where the group must not end up paused.
- ⬜ **Wheel over the video**: one notch is 5% volume, scrolling down takes it away, and a fast spin follows the wheel rather than the event count.
- ⬜ **Trackpad** (macOS, or a Windows precision touchpad): small deltas add up instead of being ignored, and one flick does not slam the volume to 0 or 100.
- ⬜ **Cursor and controls**: after 3 s without movement both disappear while playing; any movement brings both back; neither happens while paused.
- ⬜ The wheel over the control bars does not change the volume.

## 11. Volume and mute in reports (2.2.4)

✅ Against the 12.2 dev server: three volume-down presses and `m` inside the app left `VolumeLevel = 85` and `IsMuted = true` in the session's `PlayState` (`GET /Sessions`). The 10.11 spec carries the same fields on `PlaybackProgressInfo` (checked), not run at runtime.

- ⬜ The **web dashboard's session view** shows the app's volume and mute, and they follow a change made in the app (about a second later; the report is batched).
✅ Re-run after batching the reports: the same check gave `VolumeLevel = 85` and `IsMuted = true` about 3 s after the keys.

## 12. Remote control (2.2.5)

✅ Against the 12.2 dev server, driving the app's session over REST the way the dashboard does (`POST /Sessions/{id}/Playing…`, `/Command`, `/Message`): play now with a start position, pause, seek, unpause, set volume, mute, unmute, subtitles off, a message (shown as a notification), next and stop all show up in `/Sessions`. Play last plus play next with a series put the series' episodes (expanded by the server) ahead of the movie, and next went to S01E01. The 10.11.11 server sends the same `Play`, `Playstate` and `GeneralCommand` payloads (captured, not run against the app).

- ⬜ From the **web dashboard** (and a phone app): the session shows a remote control, its buttons work, and the volume and tracks it shows follow the app.
- ⬜ **Fullscreen** from the dashboard toggles the window (the harness can't check fullscreen).
- ⬜ In a **SyncPlay group**: remote pause and seek act on the whole group, and play next/last add to the group's queue.
- ⬜ Changing the **audio track** remotely during a transcode restarts the stream at the same position.

## 13. Speed, delays, subtitle size and the clock (2.1.5–2.1.7)

✅ With `JELLY_GPUI_KEYS`: `]` twice set 1.5× (the test pattern's timecode advanced about 1.5 s per second, and the badge showed "1.5×"), and `x` three times set the subtitle delay to +0.3 s, as the menu showed (opened by default for that screenshot only).

- ⬜ **Clicking** through the menu: each speed, the delay and size buttons, Reset; the controls stay up while it's open.
- ⬜ **Subtitle delay** and **size** on a real subtitle track (the dev media's text subtitles), and **audio delay** with sound.
- ⬜ **Hovering the seek bar** shows the time under the mouse, close to where a click would seek.
- ⬜ **Clicking the clock** switches to the time left, and the choice survives a restart.
- ⬜ The next episode keeps the speed and size and drops the delays; a **SyncPlay** group shows "Normal in SyncPlay" and stays at 1×.

## 14. Theme (4.3)

✅ In WSLg with a config set to light: Home, an item page (the backdrop fades into the light background) and Settings render with the light theme. Set to follow the system, the app came up light, as WSLg reports.

- ⬜ **Switching in Settings** applies at once, without a restart, on every page.
- ⬜ **Follow the system** on Windows, macOS and a Linux desktop, including switching the system between light and dark while the app runs.
- ⬜ Every page in light: login, libraries (sort and filter menus), search, seasons and episodes, the SyncPlay popover, notifications.

## 15. Previous and next (2.1.9)

✅ With `JELLY_GPUI_KEYS` against 12.2: Shift+P on S01E02 played S01E01, Shift+N on Test Movie played Second Movie (the next in Test Collection), and Test Movie ending did not start Second Movie. The smoke test reads a movie's collections on 12.2 and gets none (no route) on 10.11.

- ⬜ The **buttons** and their tooltips, and the up-next card offering a collection's next movie.
- ⬜ A remote **previous** from the dashboard: restarts the item, or within 5 s goes to the previous one.
- ⬜ A movie in **several collections** uses the first the server lists.

## 16. Image memory (6.6)

✅ With the budget lowered to 1 MB and the keep-recent window to 1 s (a local build only): leaving Home for an item page and a library, and coming back, dropped 10, 10 and 9 images, and Home's images reloaded from disk and showed correctly.

- ⬜ A **10k-item library**: scroll the whole grid and check the process memory levels off instead of growing, and that scrolling back up doesn't flash placeholders for long.
- ⬜ **Long session** (an hour of browsing and playback): memory stays flat.

## 17. Seek-bar previews (2.1.4)

✅ Against 12.2 with tiles generated by the dev setup, a hover forced at 0:25 in a local build showed the thumbnail from 0:20 above the time.

- ⬜ **Real hovering** along a long film: thumbnails follow the mouse, appear quickly after the first hover over a sheet's range, and stay on screen at the bar's ends.
- ⬜ An item **without tiles** shows only the time; a version switch shows the new version's tiles.
- ⬜ **10.11** (the same endpoints) with tiles generated.

## 18. Resume prompt (2.1.10)

✅ With `JELLY_GPUI_KEYS` against 12.2: Shift+P from S01E02 resumed S01E01 at 0:25 with "Resumed from 0:25 · Start over", and Enter restarted it from 0:00.

- ⬜ **Autoplay** into an episode with a resume point shows the card, and it goes away by itself after 10 s.
- ⬜ Clicking **Start over**; in a **SyncPlay** group it's a group seek.

## 19. Subtitle appearance (4.2)

✅ Linux, libmpv 0.37, with a config set to yellow and a dark box: the embedded SRT showed yellow on a translucent box. The settings page shows the three pickers.

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

## 22. Item page extras (3.7)

✅ Against 12.2: Test Movie's page shows its genres, studio and tag, a Trailer button, Extras (Trailer, Making of Test Movie) and, with Second Movie unwatched, More like this (Second Movie). Selecting the featurette with the keyboard played it. The setup's metadata step works on 10.11 too.

- ⬜ A **remote trailer** (a library with real metadata): the button opens it in the browser.
- ⬜ A series' extras and similar shows; extras with their own images.

## 23. Update notice (5.5)

✅ With `JELLY_GPUI_UPDATE_URL` pointing at a local fake `v9.9.9` release: the notification showed "Version 9.9.9 is out (this is 0.1.0)" with Download, and the check time was saved.

- ⬜ After the **first published release**: an older build shows the notice once a day; the current one doesn't; *Download* opens the release page; the setting turns it off.

## 24. Several accounts (4.5)

✅ With `dev` on 12.2 and `dev2` on 10.11 saved (tokens from a scratch config): the sign-in screen lists both; launching with `dev2` current restores it on 10.11; the account menu lists the saved accounts with the current one checked, then *Add account…* and *Sign out*; switching restores the other account with a new websocket; *Add account* shows sign-in with *Back to your account*, which returns to the same session without reconnecting; signing out deletes and revokes the token and leaves `dev2` listed; an unfinished playback recorded for `dev` isn't reported by `dev2`. The menu, switch, add/cancel and sign-out were triggered through temporary test code, since the harness can't click and gpui-kit buttons don't take Enter or Space.

- ⬜ **Clicking** through it all, including *Remove* on the sign-in screen and signing in to a new server while another account waits.
- ⬜ **Windows and macOS keychains** with two accounts.
- ⬜ Switching **during playback** and **in a SyncPlay group** (the group is left first).
