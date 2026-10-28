#!/usr/bin/env bash
# Windows test loop from WSL: cross-builds a debug tabbyfin.exe with mingw, copies it to
# C:\Temp\jelly-dev, and runs it on the Windows host with logs on this terminal.
#
#   dev/win-run.sh [seconds]      # default: run until the window is closed
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest=/mnt/c/Temp/jelly-dev
target=x86_64-pc-windows-gnu

cargo build -p tabbyfin --target "$target"
mkdir -p "$dest"
x86_64-w64-mingw32-strip -o "$dest/tabbyfin.exe" "$root/target/$target/debug/tabbyfin.exe"

# libmpv, pinned in packaging/mpv.lock. Extracted with Windows' own tar.exe (libarchive reads
# .7z), since 7z usually isn't installed in WSL.
if [[ ! -f "$dest/libmpv-2.dll" ]]; then
  # shellcheck source=../packaging/mpv.lock
  source "$root/packaging/mpv.lock"
  "$root/packaging/download.sh" "$LIBMPV_WINDOWS_X86_64_URL" "$LIBMPV_WINDOWS_X86_64_SHA256" "$dest/mpv-dev.7z"
  (cd "$dest" && tar.exe -xf mpv-dev.7z libmpv-2.dll && rm mpv-dev.7z)
fi

# ANGLE, for the zero-copy video path (packaging/angle.lock).
[[ -f "$dest/libEGL.dll" ]] || "$root/packaging/fetch-angle.sh" "$dest"

cd "$dest"
# A separate config folder, so test runs don't touch your real sign-in. Set it to an empty
# string to use the normal one.
export TABBYFIN_CONFIG_DIR="${TABBYFIN_CONFIG_DIR-C:\\Temp\\jelly-dev\\config}"
# Pass these through WSL interop to the Windows process.
export WSLENV="${WSLENV:+$WSLENV:}RUST_LOG:TABBYFIN_ROUTE:TABBYFIN_CONFIG_DIR:TABBYFIN_VIDEO:TABBYFIN_MPV_LOG:TABBYFIN_PLAYER"
if [[ $# -gt 0 ]]; then
  RUST_LOG="${RUST_LOG:-info}" timeout "$1" ./tabbyfin.exe || true
  taskkill.exe /IM tabbyfin.exe /F >/dev/null 2>&1 || true
else
  RUST_LOG="${RUST_LOG:-info}" ./tabbyfin.exe
fi
