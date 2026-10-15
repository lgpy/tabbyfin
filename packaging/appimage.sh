#!/usr/bin/env bash
# Builds the Linux AppImage: the release binary, plus the system's libmpv and the libraries it
# needs (FFmpeg, libass…), so it runs where mpv isn't installed. linuxdeploy leaves out the
# libraries every desktop has and that must match the system (glibc, GL, X11), and Vulkan's
# loader is left out too: it has to find the system's drivers, and GPUI can't run without it.
#
#   packaging/appimage.sh <binary> <version> <out-dir>     (needs libmpv installed)
#
# The AppImage needs the glibc of the system it's built on, so releases build it in Ubuntu
# 22.04 (packaging/appimage-docker.sh).
set -euo pipefail

binary="$(realpath "$1")"
version="$2"
mkdir -p "$3"
out="$(realpath "$3")"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=linuxdeploy.lock
source "$here/linuxdeploy.lock"

system_lib() { # <soname>: its path, from the linker cache
  ldconfig -p | sed -n "s/.*$1 (libc6,x86-64) => //p" | head -n1
}
# libmpv.so.2 (mpv 0.35+), or libmpv.so.1 on older systems like Ubuntu 22.04.
libmpv="$(system_lib 'libmpv\.so\.2')"
[[ -n "$libmpv" ]] || libmpv="$(system_lib 'libmpv\.so\.1')"
if [[ -z "$libmpv" ]]; then
  echo "libmpv isn't installed (apt install libmpv2, or libmpv1 on older systems)" >&2
  exit 1
fi
# libmpv needs these, and linuxdeploy's exclude list leaves them out as if every desktop had
# them; plenty don't, and without them libmpv doesn't load at all. Bundled, JACK's client
# library just finds no JACK server, and mpv uses another audio output.
extra=()
for soname in 'libjack\.so\.0' 'libusb-1\.0\.so\.0'; do
  path="$(system_lib "$soname")"
  [[ -n "$path" ]] && extra+=(--library "$path")
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tool="$work/linuxdeploy-x86_64.AppImage"
# appimage-docker.sh fetches it outside the container (where `gh` can reach our copy).
[[ -n "${LINUXDEPLOY:-}" ]] && cp "$LINUXDEPLOY" "$tool"
"$here/download.sh" "$LINUXDEPLOY_X86_64_URL" "$LINUXDEPLOY_X86_64_SHA256" "$tool"
chmod +x "$tool"

appdir="$work/AppDir"
mkdir -p "$appdir/usr/share/doc/jelly-gpui"
cp "$here/../LICENSE" "$here/THIRD_PARTY.md" "$appdir/usr/share/doc/jelly-gpui/"

# The app finds libmpv in ../lib next to its executable (usr/bin -> usr/lib). Without FUSE
# (containers, CI, WSL) linuxdeploy runs from its own extracted copy.
(
  cd "$work"
  APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="jelly-gpui-$version-linux-x86_64.AppImage" LDAI_VERSION="$version" \
    "$tool" --appdir "$appdir" \
    --executable "$binary" \
    --library "$libmpv" "${extra[@]}" \
    --exclude-library 'libvulkan.so*' \
    --desktop-file "$here/jelly-gpui.desktop" \
    --icon-file "$here/jelly-gpui.svg" \
    --output appimage
)
mv "$work/jelly-gpui-$version-linux-x86_64.AppImage" "$out/"
