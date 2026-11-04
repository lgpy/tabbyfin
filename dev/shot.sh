#!/usr/bin/env bash
# Screenshot the app at a size an agent can afford to look at: capture, then shrink to a JPEG
# (default 540 px wide for the phone, 960 for the desktop window). A full-size 1080x2400 PNG
# costs several times the tokens of the shrunk copy and shows an agent nothing more.
#
#   bash dev/shot.sh android OUT.jpg [WIDTH]   # the emulator or phone adb sees
#   bash dev/shot.sh window  OUT.jpg [WIDTH]   # the running tabbyfin window (Windows)
#
# Needs ImageMagick (`magick`) on PATH.
set -euo pipefail

usage="usage: bash dev/shot.sh android|window OUT.jpg [WIDTH]"
target=${1:?$usage}
out=${2:?$usage}
tmp=$(mktemp --suffix=.png)
trap 'rm -f "$tmp"' EXIT

case $target in
  android)
    width=${3:-540}
    adb=$(command -v adb || true)
    if [ -z "$adb" ]; then
      sdk=${ANDROID_HOME:-$(cygpath -u "${LOCALAPPDATA:-}")/Android/Sdk}
      adb=$sdk/platform-tools/adb
    fi
    "$adb" exec-out screencap -p > "$tmp"
    ;;
  window)
    width=${3:-960}
    here=$(dirname -- "$0")
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w "$here/shot-window.ps1")" "$(cygpath -w "$tmp")"
    ;;
  *)
    echo "$usage" >&2
    exit 2
    ;;
esac

magick "$tmp" -resize "${width}x>" -quality 80 "$out"
echo "$out"
