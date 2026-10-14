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

- ⬜ With a **real keyboard**: the ring appears on the first arrow key, a mouse click hides it.
- ⬜ A **large library** (hundreds of items): the grid scrolls to keep the selection visible, including rows that are still loading.
- ⬜ **Window resize** while the cursor is in the library grid or search results (the column count changes).
- ⬜ Clicking a picker or the search box and then pressing arrow keys doesn't move the cursor unexpectedly.

## 2b. Library sort and filter, favorites, cast (3.2–3.4)

✅ Saved sort and filter views (date added + favorites, name + all), the Favorite button by keyboard and the Home shelf, the cast shelf with initials and roles, a person page.

- ⬜ **Picking a sort or filter with the mouse** reloads the grid from the top and is remembered when the library is opened again; the order button flips it (and reshuffles for Random).
- ⬜ A library with **real metadata** (TMDb): sorting by rating, release date and runtime gives sensible orders; cast **photos** show.
- ⬜ A person page for someone in **many** movies and shows.
- ⬜ Favoriting from another device shows up in the Home shelf (via the live connection).
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

## 6. SyncPlay (2.4.3–2.4.6)

✅ Against the scripted member (`cargo run -p jellyfin --example syncplay`): following the group (load, Ready, Unpause, Pause, Unpause, Seek, Stop) and leading it (Play on an item page, Space, →, Esc); the popover in and out of a group; drift correction catching up 0.8 s.

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

## 8. Minimum server version (4.6)

✅ The API smoke test on 10.11.11 with legacy authorization off.

- ⬜ **Clicking through the app on 10.11**: sign-in (password and Quick Connect), Home, libraries, search, item pages, mark watched, playback with resume and reporting.
