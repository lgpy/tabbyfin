# Third-party software

The Windows and macOS packages of jelly-gpui include **mpv**. On Windows that's libmpv
(`libmpv-2.dll`, from https://github.com/shinchiro/mpv-winbuild-cmake, version in
`packaging/mpv.lock`), which plays video inside the app, plus the `mpv` program used as a
fallback in its own window. macOS currently bundles the `mpv` program only.

- mpv, version pinned in `packaging/mpv.lock`, from https://github.com/mpv-player/mpv/releases
- License: GPL-2.0-or-later (these builds), see https://github.com/mpv-player/mpv/blob/master/Copyright
- Source code: https://github.com/mpv-player/mpv (the tag matching the bundled version)

Linux packages use the mpv installed on the system and don't include it.
