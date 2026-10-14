#!/usr/bin/env bash
# Windows test loop from WSL: cross-builds a debug jelly-gpui.exe with mingw, copies it to
# C:\Temp\jelly-dev, and runs it on the Windows host with logs on this terminal.
#
#   dev/win-run.sh [seconds]      # default: run until the window is closed
#
# Debug builds of GPUI compile their HLSL shaders at runtime from the build-time source path.
# Windows resolves that Linux path on C:, so the shaders are mirrored there. (Icons are
# embedded via rust-embed's debug-embed feature; see crates/app/Cargo.toml.)
# Release builds embed everything; use CI for those.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest=/mnt/c/Temp/jelly-dev
target=x86_64-pc-windows-gnu

cargo build -p jelly-gpui --target "$target"
mkdir -p "$dest"
x86_64-w64-mingw32-strip -o "$dest/jelly-gpui.exe" "$root/target/$target/debug/jelly-gpui.exe"

mirror() { # <dir under a Linux path> -> same path on C:
  [[ -d "$1" ]] || return 0
  mkdir -p "/mnt/c$1"
  cp -ru "$1/." "/mnt/c$1/"
}
registry="$(ls -d "$HOME"/.cargo/registry/src/*/ | head -n1)"
for crate in "$registry"gpui-pre-windows-* "$root"/third_party/gpui/gpui-pre-windows; do
  mirror "${crate%/}/src"
done

# libmpv, pinned in packaging/mpv.lock. Extracted with Windows' own tar.exe (libarchive reads
# .7z), since 7z usually isn't installed in WSL.
if [[ ! -f "$dest/libmpv-2.dll" ]]; then
  # shellcheck source=../packaging/mpv.lock
  source "$root/packaging/mpv.lock"
  curl -fsSL -o "$dest/mpv-dev.7z" "$LIBMPV_WINDOWS_X86_64_URL"
  actual="$(sha256sum "$dest/mpv-dev.7z" | cut -d' ' -f1)"
  [[ "$actual" == "$LIBMPV_WINDOWS_X86_64_SHA256" ]] || { echo "libmpv checksum mismatch: $actual" >&2; exit 1; }
  (cd "$dest" && tar.exe -xf mpv-dev.7z libmpv-2.dll && rm mpv-dev.7z)
fi

# ANGLE, for the zero-copy video path (packaging/angle.lock).
[[ -f "$dest/libEGL.dll" ]] || "$root/packaging/fetch-angle.sh" "$dest"

cd "$dest"
# A separate config folder, so test runs don't touch your real sign-in. Set it to an empty
# string to use the normal one.
export JELLY_GPUI_CONFIG_DIR="${JELLY_GPUI_CONFIG_DIR-C:\\Temp\\jelly-dev\\config}"
# Pass these through WSL interop to the Windows process.
export WSLENV="${WSLENV:+$WSLENV:}RUST_LOG:JELLY_GPUI_ROUTE:JELLY_GPUI_CONFIG_DIR:JELLY_GPUI_VIDEO:JELLY_GPUI_MPV_LOG"
if [[ $# -gt 0 ]]; then
  RUST_LOG="${RUST_LOG:-info}" timeout "$1" ./jelly-gpui.exe || true
  taskkill.exe /IM jelly-gpui.exe /F >/dev/null 2>&1 || true
else
  RUST_LOG="${RUST_LOG:-info}" ./jelly-gpui.exe
fi
