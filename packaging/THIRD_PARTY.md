# Third-party software

The Windows and macOS packages of jelly-gpui include a copy of **mpv**, which jelly-gpui runs
as a separate program for video playback.

- mpv, version pinned in `packaging/mpv.lock`, from https://github.com/mpv-player/mpv/releases
- License: GPL-2.0-or-later (these builds), see https://github.com/mpv-player/mpv/blob/master/Copyright
- Source code: https://github.com/mpv-player/mpv (the tag matching the bundled version)

Linux packages use the mpv installed on the system and don't include it.
