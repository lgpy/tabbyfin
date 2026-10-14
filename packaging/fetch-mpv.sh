#!/usr/bin/env bash
# Downloads the pinned mpv build for a platform, verifies its checksum, and extracts what the
# package needs into <dest>.
#
#   packaging/fetch-mpv.sh windows-x86_64 <dest>   -> <dest>/mpv.exe, <dest>/vulkan-1.dll
#   packaging/fetch-mpv.sh macos-aarch64 <dest>    -> <dest>/mpv.app
#   packaging/fetch-mpv.sh macos-x86_64 <dest>     -> <dest>/mpv.app
set -euo pipefail

platform="$1"
dest="$2"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=mpv.lock
source "$here/mpv.lock"

case "$platform" in
  windows-x86_64) asset="mpv-$MPV_VERSION-x86_64-pc-windows-msvc.zip"; sha="$MPV_WINDOWS_X86_64_SHA256" ;;
  macos-aarch64)  asset="mpv-$MPV_VERSION-macos-15-arm.zip";          sha="$MPV_MACOS_AARCH64_SHA256" ;;
  macos-x86_64)   asset="mpv-$MPV_VERSION-macos-15-intel.zip";        sha="$MPV_MACOS_X86_64_SHA256" ;;
  *) echo "unknown platform: $platform" >&2; exit 1 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/$asset" "https://github.com/mpv-player/mpv/releases/download/$MPV_VERSION/$asset"

if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "$work/$asset" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$work/$asset" | cut -d' ' -f1)"
fi
if [[ "$actual" != "$sha" ]]; then
  echo "checksum mismatch for $asset: expected $sha, got $actual" >&2
  exit 1
fi

mkdir -p "$dest"
case "$platform" in
  windows-x86_64)
    # Only the player and its Vulkan loader; skip the debug symbols and shell scripts.
    if command -v unzip >/dev/null; then
      unzip -q -o "$work/$asset" mpv.exe vulkan-1.dll -d "$dest"
    else
      7z x -y -o"$dest" "$work/$asset" mpv.exe vulkan-1.dll >/dev/null
    fi
    ;;
  macos-*)
    unzip -q -o "$work/$asset" -d "$work"
    tar -xzf "$work/mpv.tar.gz" -C "$dest"
    ;;
esac
echo "mpv $MPV_VERSION ($platform) -> $dest"
