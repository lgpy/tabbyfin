#!/usr/bin/env bash
# Downloads the pinned libmpv for a platform, verifies its checksum, and extracts it into <dest>.
#
#   packaging/fetch-libmpv.sh windows-x86_64 <dest>   -> <dest>/libmpv-2.dll
set -euo pipefail

platform="$1"
dest="$2"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=mpv.lock
source "$here/mpv.lock"

case "$platform" in
  windows-x86_64) url="$LIBMPV_WINDOWS_X86_64_URL"; sha="$LIBMPV_WINDOWS_X86_64_SHA256" ;;
  *) echo "unknown platform: $platform" >&2; exit 1 ;;
esac
asset="${url##*/}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
"$here/download.sh" "$url" "$sha" "$work/$asset"

mkdir -p "$dest"
# A .7z: needs 7-Zip (on Windows CI runners) or a libarchive tar (bsdtar, Windows' tar.exe).
if command -v 7z >/dev/null; then
  7z e -y -o"$dest" "$work/$asset" libmpv-2.dll >/dev/null
elif command -v bsdtar >/dev/null; then
  bsdtar -xf "$work/$asset" -C "$dest" libmpv-2.dll
else
  echo "extracting $asset needs 7z or bsdtar" >&2
  exit 1
fi
echo "libmpv ($platform) -> $dest"
