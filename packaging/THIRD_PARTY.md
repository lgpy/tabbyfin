# Third-party software

jelly-gpui plays video with **libmpv**, the library form of mpv, running inside the app.

- **Windows:** the package includes `libmpv-2.dll`, a build from
  https://github.com/shinchiro/mpv-winbuild-cmake. The version is pinned in `packaging/mpv.lock`.
- **Linux:** uses the system libmpv (`libmpv2`).
- **macOS:** uses Homebrew's libmpv (`brew install mpv`) for now.

**License:** GPL-2.0-or-later for these builds, see https://github.com/mpv-player/mpv/blob/master/Copyright.

**Source code:** https://github.com/mpv-player/mpv. The shinchiro build records the exact mpv commit in its file name.
