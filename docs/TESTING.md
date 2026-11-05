# Manual testing checklist

What works in development but hasn't been tried where it matters: on real hardware, real servers, release builds, with a real mouse and keyboard, or with a second person. Development happens on Windows, against the local dev servers in `dev/` (see [`dev/README.md`](../dev/README.md)), with keys pressed from inside the app (`TABBYFIN_KEYS`) and the headless UI tests in `crates/app/src/ui_tests/`.

When a check has been done, delete it, noting the hardware, OS and server version in the commit message, and file anything that fails. Roadmap numbers refer to [`ROADMAP.md`](ROADMAP.md).

## 1. Release builds and packages

- **Windows release build**: *Settings → Storage → Logs → Open folder* opens the folder; the autoplay switch and bitrate picker survive a restart.
- **The `.deb`** on a real Ubuntu desktop (only tried in a container).
- **The AppImage** on real Linux desktops (GNOME, KDE; Wayland and X11), from the file manager, on a distribution other than Ubuntu, and on a 2022-era one (Ubuntu 22.04, Debian 12) with a real GPU. In a 22.04 container the window was black (old software Vulkan).
- **The Flatpak** on a real desktop with a GPU: GPU video and hardware decoding through `--device=dri`, sign-in kept in the keychain across restarts (Secret Service from the sandbox), audio through PulseAudio/PipeWire, the menu entry and icon.
- **macOS** on Apple silicon and Intel: the app starts, signs in and plays with `brew install mpv` at a reasonable CPU use at 1080p (software video), and the Dock and Finder show the icon.
- **Playback from the packaged builds**: the container checks only opened the sign-in screen.
- **The CI jobs on GitHub Actions**, the `integration` job included (the same steps passed locally).

## 2. Video and GPUs (2.6, 2.7, 2.9)

- **Windows on NVIDIA and Intel** (only AMD's RX 6600 is done): `video decoding:` says `d3d11va` and the 4K HEVC version of Test Movie plays smoothly; the log says `video output: GPU (ANGLE/D3D11)` and `video: GPU frames`; the picture is right (not black, colours not swapped), resizing and pausing work, no `couldn't import a GPU frame` or `skipped a GPU frame` lines beyond a resize, no tearing in fast motion. Then the same with ANGLE hidden (delete `libEGL.dll`: `OpenGL/D3D11 interop`).
- **Linux on AMD, Intel and NVIDIA, each on X11 and Wayland** (six runs): the log says `video output: GPU (OpenGL/Vulkan interop)` and `video: GPU frames`; the picture is right (not black, garbled, swapped or upside down) through the host and with `TABBYFIN_PLAYER=embedded`; resizing and pausing; no `couldn't import a GPU frame` or `switching to software frames`; no tearing or flicker.
- **Hybrid laptops**: on Windows, with the window on either GPU, the log's `OpenGL ES (ANGLE) context for mpv renderer=` names the window's adapter, and with ANGLE hidden WGL shows video or falls back to software frames rather than black. On Linux (Intel + AMD or NVIDIA), `OpenGL context for mpv renderer=` names the window's GPU, found through EGL's device list when the default display is the other GPU.
- **The fallback to software frames** after a failed GPU import (only real drivers fail it).
- **CPU in a Windows release build** for 4K HEVC 10-bit with ANGLE, app and host together (debug: 14–21 %, `docs/in-app-video.md`).
- **High refresh rates** (120/144 Hz): no judder at 24 fps.

## 3. A real mouse and keyboard

The harness presses keys from inside the app and the UI tests click headless, so nothing has had a real pointer, trackpad or keyboard.

- **Keyboard**: the cursor ring appears on the first arrow key and a click hides it; the top bar after mouse use; clicking a picker or the search box then pressing arrows doesn't move the cursor unexpectedly; changing a picker by keyboard then by mouse (or the reverse) keeps them in step, and playing uses the chosen version and tracks.
- **Cards and shelves**: the hover dim and Home's play button (resumes Continue Watching, plays a series' next episode; a click elsewhere opens the page); card text not stealing the card's click, right-click or drag; the shelf arrows; resting on a card then clicking opens its page with no spinner, and sweeping across a shelf sends the server no item requests.
- **Right-click menus** at the pointer, flipping near the window's edges, on a Home card, a library poster, a search result, a season, a *More like this* card and an episode row: *Play* (on a series or season too), *Watched*, *Favorite*, *Add to playlist…*. A library or person card has no menu. The hover highlight gives way to the arrows while the pointer rests.
- **Selects and popups**: the library toolbar's, the Settings pickers' and the item page's version list opened and chosen by click; the lists drawn with `popup::row` (*Add to playlist*, genres and tags, ratings, the player's tracks) taking the highlight from the pointer.
- **Library**: the toolbar's selects (a choice reloads from the top and is remembered), the order button (reshuffling for Random), the letter bar, the genre/tag pills (include, exclude, off), clicking outside and Esc closing the popover, its search box, *Clear filters*.
- **Item page**: Watched and Favorite popping from a click; the version list and *More* menu; the season rows; a playlist's ↑ ↓ ✕ buttons.
- **Dialogs**: *Add to playlist…* from a card, clicking a playlist row, *Save*, *Cancel*, *Delete*, the sharing switch, a click on the backdrop closing it; a list longer than its 320 px cap scrolling with the highlight kept on screen.
- **Top bar and search**: the search box's click, typing and clear button; the bar's buttons; Back leaving no keyboard in the bar; the account menu opening on a click and closing on a click outside; a result's click and right-click.
- **SyncPlay panel**: the button opening and closing it; a row's name playing the entry; ↑ ↓ ✕, *Shuffle* and *Repeat* (the panel stays up, *Repeat* cycling off, all, one); *Join*, *New group* and its name, *Leave group*; such a click not moving the page's cursor; a queue longer than the 320 pt cap scrolling, and the highlight brought into view by Up/Down.
- **Settings**: the menu's entries and hover, the switches (the knob sliding, jumping with animations off), *Clear*, *Open folder*, the search box's clear button, the *Languages ›* links, the language pickers' long lists; the wheel scrolling the page with the menu's mark following, also within a second of a menu entry's scroll.
- **Sign-in**: *Show password* opening the mascot's eyes and shutting them again; the Quick Connect code's copy button pasting into a browser; a saved account row's hover and focus ring, Tab from a row to its ✕, Enter continuing as that account; *Use another server*; the brand button's hover and press.
- **Player**: a click pauses and a double click toggles fullscreen (the pause put back); the wheel steps the volume 5 % a notch; the seek bar's hover time and trickplay thumbnail following the mouse and staying on screen at the bar's ends, dragging it, its chapter marks, clicking it; clicking the clock; the volume slider; the chapter, settings and version menus from their buttons, and the Audio and Subtitles buttons opening the menu on their tab (the lit one closes it, another switches tab without a flicker); the settings menu's steppers repeating cleanly, ↺ appearing once a value moves, the speeds by click; hover on the control buttons, tabs and track rows; the skip, *Play now*, *Cancel* and *Start over* buttons; the controls reappearing after their 3 s hide.
- **Trackpad** (macOS, or a Windows precision touchpad): small deltas add up, and one flick doesn't slam the volume to 0 or 100.
- **Fullscreen** (`f`, its button, Esc leaving it first, and from the dashboard): the hidden test display has no window manager.

## 4. Playback

- **Transcoding**: changing audio during a transcode restarts at the same position, as does switching to or from an image subtitle (PGS, burned in); a user whose policy forbids direct play or remuxing, with no bitrate limit; seeking far ahead in a feature-length transcode; a server with hardware transcoding (NVENC, QSV, VAAPI), HEVC transcodes included.
- **Slow and dropped connections**: *Automatic* bitrate over a real slow link (Wi-Fi, a phone hotspot at 4–8 Mbit/s, a remote server), with a plausible measurement in the log; Wi-Fi off for 10 s and for a minute, direct and transcoded, continuing or reconnecting with the "Reconnecting…" notice.
- **Media segments** from a plugin: install [Intro Skipper](https://github.com/intro-skipper/intro-skipper), let it analyse a series, and check *Skip intro* and the up-next card at the credits.
- **Real chapter names**: anime releases (OP/ED chapters), and Blu-ray rips with plain "Chapter 1…" names (no skip button should appear).
- **Up next and autoplay**: autoplay off ends the episode and closes the player; autoplay into an episode with a resume point shows the resume card, which goes after 10 s; the next episode keeps the speed and subtitle size and drops the delays.
- **Collections and playlists** from a real library (TMDb collections, a long playlist): *Play all* and *Shuffle* run through the whole list, the up-next card names the next entry, and playing something else ends the queue; the up-next card offering a collection's next movie, a movie ending without starting the next (only Shift+N does), and a movie in several collections using the first the server lists.
- **Series**: *Shuffle* on a big series; a series with specials placed between seasons; a season page's *Play* and *Shuffle*.
- **Versions**: switching to an item's second version restarts at the same position; the seek bar then shows that version's trickplay tiles, and an item without tiles shows only the time.
- **Subtitles and delays**: subtitle delay and size on a real subtitle track and audio delay with sound; an ASS subtitle keeps its own style; the size set in Settings, during playback too, and in the player's menu agree afterwards.
- **A reconnect** over the last frame (Back while it shows).

## 5. SyncPlay (2.10)

- **With the Jellyfin web client** as the other member: create the group on either side and join from the other; start an item from each side and both start together; pause, unpause, seek and stop from each side (Esc in Tabbyfin stops the group); join a group that's already playing and land at the right position.
- **Two Tabbyfin instances on two machines** (or two users): the audio stays in sync by ear for 10+ minutes, and `RUST_LOG=tabbyfin::syncplay=debug` logs drift under about 0.1 s (tune `syncplay.rs` otherwise).
- The group moving on to the next episode together; join and leave notifications.
- A member with a slow connection or a transcode: the group waits for it and it stays in sync.
- A version switch in a group, from the player's version menu; removing a queue entry.
- *Play all*, *Shuffle* on a series, *Start over* (a group seek) and the speed fixed at 1× in a group.
- Switching accounts during playback and in a group (the group is left first).

## 6. Live connection and remote control

- **Another device**: marking something favorite in the web client or a phone app shows it in Home's Favorites shelf within a second or so.
- **Reconnecting** after the network drops for a minute: the log shows "session websocket connected" again and refreshes still work.
- Behind a **reverse proxy with HTTPS** (`wss://`), with a base path like `/jellyfin`.
- **The web dashboard and a phone app**: the session shows under *Dashboard → Activity / Sessions* with a remote control; its buttons work; the volume, mute and tracks it shows follow the app (about a second later); a remote audio change during a transcode restarts at the same position; a remote *previous* restarts the item, or within 5 s goes to the previous one.

## 7. Real libraries

- **Real metadata** (TMDb): sorting by rating, release date and runtime gives sensible orders; cast photos and the Cast & crew shelf show (the dev items have no people); a person page for someone in many movies and shows, and a person card without a picture.
- **A large library** (hundreds to 10,000 items, `python dev/fake-library.py`): the grid keeps the selection visible, rows still loading included; letters land on the first item with that letter, ascending and descending, with a filter, and names with articles, accents and digits land where the server sorts them; excluding genres or tags (the id listing's time, the letter bar, scrolling); a page request failing mid-library and retried after its rows scroll out and back; window resize and zoom with the cursor in the grid.
- **Home settings** changed in the web client, then Home reloaded here; the Latest shelves and `LatestItemsExcludes` against 12.2 (the dev server's `/Items/Latest` answers `[]`); a Latest title opening its library sorted by date added, newest first, while *My Media* keeps the remembered sort.
- An account with **no libraries**, and one with **no playlists** ("No playlists yet. Type a name to make one.").
- **Remote trailers**, and a series' extras.
- **Older saved views**: a favorites filter saved by an older build comes back as *Any progress*, and a multi-rating view as its first rating.

## 8. Accounts, Settings and the window

- **Several accounts**: signing in to a new server while another account waits; signing out with no account left (the form prefilled); the Windows and macOS keychains with two accounts.
- **Server settings seen elsewhere**: change the subtitle mode or audio language here and check the web client shows it and the next item starts with the track it implies; *Leave watched items out of Latest* drops watched items from Home once saved.
- **Failures**: Settings' failed load or save (*Retry*, the toast, the configuration reloaded); the dialogs with the server down; marking watched while the server is down (the toast, the item put back); an expired token while browsing (6.5).
- **Theme**: switching in Settings applies at once on every page; `theme = "system"` follows the system on macOS, and on real GNOME and KDE through the portal, Ubuntu 22.04's portal 1.14 (`Read` only) included, changes while running included.
- **Text size** following the system on GNOME (Large Text) and KDE (*Fonts*), at launch and changed while running (applied on switching back to the app); the log folder through `open`.
- **Sizes**: the grid, shelves, card lines, Settings and the sign-in form at 90 % and 150 % text and 175 % zoom in an 800×500 window.
- **Window placement on Windows at 150 % and in mixed DPI** (the machine's monitors are both at 100 %): one monitor at 150 % (the config's `scale` is 1.5, `x`/`y` are physical pixels, no visible move); a window saved on the other-scaled monitor reopens there after one jump, maximized too, and un-maximizes to its saved spot; a spot between monitors of different heights opens centred; un-maximizing after a maximized launch at 150 % restores the saved size, not 1.5× it; a maximized window at 150 % reopening maximized with no flicker.
- **Window placement on Linux** (a real window manager, X11 and Wayland, where only the size is kept) and **macOS**: a saved spot reopens as saved, a maximized window reopens maximized, and one saved on a missing monitor still opens on screen.

## 9. Jellyfin 10.11 (4.1)

Against `mise run dev:min` (port 8097): signing in with a password and with Quick Connect, libraries (sort and filter on a bigger library, the HD and SD resolution choices), marking watched, Settings, and the seek bar's trickplay tiles.

## 10. Memory and long sessions (6.4)

- A 10,000-item library scrolled end to end: memory levels off rather than growing, and scrolling back up doesn't flash placeholders for long.
- An hour of browsing and playback: memory stays flat.

## 11. Update notice

*Download* on the update notice opens the release page in the browser.

## 12. Android

Built with `mise run android:install`, and tried on the emulator and one phone (a Honor with Android 16; `docs/android.md`).

- Playback on Media3: external subtitles (`sub-add`), image subtitles (PGS, VobSub), subtitle and audio delay, an HLS transcode, and audio focus (a call or another app's audio pauses it).
- Playback on mpv: ASS subtitles with their styling and the system's fonts.
- Playback on a second phone, and whether HDR reaches an HDR screen as HDR.
- The player's gestures with real fingers (they were tried with `adb shell input`): a double tap's timing, a drag that starts diagonally, and a second finger during a drag.
- Typing with Gboard, Samsung Keyboard, SwiftKey and FUTO (on the phone; the emulator only had FUTO typing an address): suggestions and autocorrect, swipe typing, Backspace held down, and a password field getting the password keyboard.
- The status bar, gesture bar and a display cutout keep clear of the app, in portrait and landscape.
- The player under a real display cutout, landscape either way up: the video reaches under it while the controls stay clear of it and centred, on Android 15 or newer and on 9 to 14 (where `SHORT_EDGES` is asked for), and the pages after the player closes keep clear of it again.
- The phone player's controls with real fingers: the middle's buttons against the double tap on a side third (a tap near a skip button), the subtitles toggle and a long press on it on Media3 and on mpv, and the side sheet closing on Back with gesture navigation and on a tap beside it.
- The phone item page with a real library: long titles and synopses, a series with many seasons and Specials on the shelf, an episode without a still of its own, and a person's photo in the round faces.
- The keyboard per field: the server address's has the URL layout, the user name's suggests and learns nothing, a search box's (the top bar's, and the library's genres and tags) action key says Search and still submits, and the playlist name's suggests nothing.
- The sign-in form taller than the room the keyboard leaves, in portrait: the field the keyboard opens over scrolls into view above it.
- The status and navigation bar icons are dark on the light theme and light on the dark one, and still so after the player closes and its bars come back.
- Back with gesture navigation (and three-button navigation): closes a menu, goes back a page, then puts the app in the background; opening it again shows the same page.
- The phone layout on a tablet (medium and expanded with touch: the desktop's top bar, the grid filling its rows) and on the phone in landscape.
- A quick vertical flick that starts on a Home shelf coasts as one on the page between shelves does (a page long enough to tell; the dev server's Home is short).
- One axis per gesture: a diagonal drag on a shelf moves the row or the page, never both; a touch on a coasting page or row stops it; a flung row coasts and stops at its ends.
- The bottom bar slides away while scrolling down Home, a library and an item page and comes back on a scroll up, without flickering at a page's end or on a page barely taller than the screen.
- Scrubbing the library's letter bar with a finger: the bubble follows, and the grid lands on each letter (on a big library, `dev/fake-library.py`).
- The library's filters dialog on the phone: each select and the genres and tags popover open over it, and the grid behind updates as they're picked.
- Touch targets with real fingers: the 30-point SyncPlay and playlist handles, the field's clear and show-password buttons, and a long press on a card that's also the start of a scroll.
- The release-signed APK (`build.sh --sign`, with R8) on a phone: playback on Media3 and on mpv, DTS and TrueHD audio through the ffmpeg extension, and the keyboard and Back (all Java the Rust side calls into). The emulator only showed it signs in and plays Media3 with its subtitles.
- The app restores its session after Android ends it in the background (Developer options, *Don't keep activities*, or `adb shell am kill`).
