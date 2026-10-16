<p align="center"><img src="packaging/tabbyfin.png" alt="" width="160"></p>

<h1 align="center">Tabbyfin</h1>

A native desktop client for [Jellyfin](https://jellyfin.org), built with [GPUI](https://www.gpui.rs/) through [GPUI Kit](https://github.com/longbridge/gpui-kit). Video plays inside the app with [mpv](https://mpv.io) (libmpv), drawn straight from the GPU.

Runs on **Linux**, **Windows** and **macOS**. Needs Jellyfin **10.11 or newer**; developed against 12.x.

> Early days: version 0.1. See [`docs/ROADMAP.md`](docs/ROADMAP.md) for what's missing.

## Features

- **Sign in** with a username and password, or with Quick Connect. The session is kept between launches, and the token is stored in the system keychain.
- **Several servers and users**: accounts stay signed in side by side; switch from the menu under your name, or pick one on the sign-in screen.
- **Browse:**
  - Home: Continue Watching, Next Up, Latest.
  - Library grids, and movie, series, season and episode pages.
  - Sort and filter libraries (remembered per library), and browse by cast and crew.
  - Collections and playlists, with Play all and Shuffle.
  - Search.
  - Mark items as watched or as favorites; favorites get their own shelf on Home.
- **Play inside the window:**
  - Direct play of nearly anything mpv can decode. Above a bitrate you set, or when your server requires it, the server transcodes instead.
  - Hardware decoding. On Windows, frames go from the decoder to the screen without a copy.
  - Audio, subtitle and version pickers, including external subtitles from Jellyfin.
  - Volume, chapters (marked on the seek bar), next episode, and an up-next card that plays the next episode at the end (or not: it's a setting).
  - Playback speed, subtitle and audio delay and subtitle size from the player's settings menu; the time under the mouse on the seek bar, and a clock that can show the time left.
  - Skip intro, recap and similar, from the server's media segments (with a segment plugin such as Intro Skipper) or from chapter names like "Intro", "Opening" or "OP".
  - Resume points and watched state stay in sync with the server, including changes made on other devices.
- **Remote control** from the Jellyfin web dashboard or a phone app: play items here, pause, seek, skip to the next item, change the volume and tracks, or send a message.
- **Watch together** with Jellyfin's SyncPlay: create or join a group from the top bar, and play, pause and seek in sync with the other members.
- **Isolation:** your own mpv configuration, scripts and key bindings are never loaded, so they can't change how the embedded player looks.

**Keys:** in the browsing pages, the arrow keys move a highlight over the cards, Tab and Shift+Tab jump between rows, and Enter opens. Esc or Backspace goes back, and Ctrl+F (⌘F on macOS) searches. In a library sorted by name, typing a letter jumps to it (or use the letter bar). In search, Enter in the box jumps to the results.

**Player keys:**

| Key | Action |
|---|---|
| Space | Play/pause |
| ← / → | Seek 10 s |
| ↑ / ↓ | Volume |
| M | Mute |
| Shift+N / Shift+P | Next / previous episode (or movie of the collection) |
| PgUp / PgDn | Previous / next chapter |
| Enter or S | Skip the intro, play the next episode from the up-next card, or start over after an automatic resume |
| [ / ] | Slower / faster |
| Z / X | Subtitle delay −0.1 s / +0.1 s |
| F | Fullscreen |
| Esc | Leave fullscreen, then dismiss the up-next card, then leave the player |

## Install

Download the package for your system from the [releases](https://github.com/lgpy/tabbyfin/releases).

| System | Package | Notes |
|---|---|---|
| Windows 10/11 (x86_64) | `tabbyfin-<version>-windows-x86_64-setup.exe` | Installs for your user (no admin rights needed), with a Start-menu entry and an uninstaller in Settings → Apps. libmpv and ANGLE are included. The build isn't code-signed, so SmartScreen may ask before it runs. |
| Windows, portable | `tabbyfin-<version>-windows-x86_64.zip` | The same files: unzip anywhere and run `tabbyfin.exe`. |
| Debian, Ubuntu (x86_64) | `tabbyfin_<version>-1_amd64.deb` | `sudo apt install ./tabbyfin_<version>-1_amd64.deb` pulls in `libmpv2` and Vulkan. |
| Other Linux (x86_64) | `tabbyfin-<version>-linux-x86_64.AppImage` | `chmod +x` it and run it. libmpv is included; needs a Vulkan driver and a 2022 or newer distribution (glibc 2.35: Ubuntu 22.04, Debian 12, Fedora 36, Linux Mint 21…). Without FUSE, run it with `--appimage-extract-and-run`. |
| Linux, Flatpak (x86_64) | `tabbyfin-<version>.flatpak` | `flatpak install --user tabbyfin-<version>.flatpak` (it fetches the freedesktop runtime from Flathub), then start it from the menu or with `flatpak run io.github.lgpy.tabbyfin`. mpv is included. Settings live in `~/.var/app/io.github.lgpy.tabbyfin/`, apart from the other packages'. |
| Linux, tarball | `tabbyfin-<version>-linux-x86_64.tar.gz` | Needs libmpv (`libmpv.so.2`) and a Vulkan driver installed. For a menu entry, copy `tabbyfin` to `~/.local/bin/`, `tabbyfin.desktop` to `~/.local/share/applications/` and `tabbyfin.png` to `~/.local/share/icons/hicolor/512x512/apps/`. |
| macOS 12+ (Apple silicon, Intel) | `tabbyfin-<version>-macos-<arch>.zip` | Needs `brew install mpv` for playback. The app is ad-hoc signed, not notarised: right-click it and choose Open the first time. |

**Logs** are written to a daily file. *Settings → Logs → Open log folder* opens it; attach the log to bug reports.

## Build from source

The easiest way is [mise](https://mise.jdx.dev), which installs the pinned Rust toolchain and tools:

```sh
mise install
mise run run      # debug build of the app
mise run ci       # what CI checks: formatting, clippy, tests
mise tasks        # everything else
```

Plain `cargo run` works too, with the Rust version pinned in `mise.toml` (1.99).

**System libraries:**
- **Linux:** a C toolchain, plus Vulkan, Wayland/X11, fontconfig and OpenSSL headers. The full `apt` line is in [`docs/PLAN.md`](docs/PLAN.md) §8. Install `libmpv2` to play video. libmpv is loaded at runtime, so building doesn't need it.
- **Windows:** the MSVC toolchain. For playback, put `libmpv-2.dll` next to the exe (`packaging/fetch-libmpv.sh`), and optionally ANGLE (`packaging/fetch-angle.sh`).
- **macOS:** Xcode command line tools, and `brew install mpv`.

**Test server:** `mise run dev:setup` starts a local Jellyfin 12.2 in Docker with a small generated library (login `dev` / `dev`). `mise run dev:min` starts Jellyfin 10.11, the minimum supported version, on port 8097 and runs the API smoke test against it. See [`dev/README.md`](dev/README.md).

## Documentation

- [`docs/PLAN.md`](docs/PLAN.md): architecture, Jellyfin API notes, and the dev environment.
- [`docs/in-app-video.md`](docs/in-app-video.md): how video gets from mpv into the window on each OS.
- [`docs/ROADMAP.md`](docs/ROADMAP.md): missing features and open work.
- [`docs/TESTING.md`](docs/TESTING.md): what still needs testing on real hardware, servers and with other people.
- [`third_party/gpui/PATCHES.md`](third_party/gpui/PATCHES.md): our changes to GPUI.

## License

Tabbyfin is licensed under the [GNU General Public License v3.0 or later](LICENSE).

Packages include third-party software under its own licenses: libmpv (GPL-2.0-or-later) and, on Windows, ANGLE and the MinGW runtime. See [`packaging/THIRD_PARTY.md`](packaging/THIRD_PARTY.md). GPUI and GPUI Kit are Apache-2.0; the patched GPUI crates in `third_party/gpui/` keep that license.
