#!/usr/bin/env bash
# Builds the Flatpak and bundles it into <out-dir>/tabbyfin-<version>.flatpak, installable
# with `flatpak install --user <file>`. Needs flatpak and flatpak-builder; the freedesktop
# 26.08 SDK and its rust-stable extension come from Flathub (added for the user if missing).
# Build state stays in target/flatpak.
#
#   packaging/flatpak/build.sh [out-dir]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-dist}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)"
work="$root/target/flatpak"
cd "$root"
mkdir -p "$work" "$out"
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --force-clean --disable-rofiles-fuse --install-deps-from=flathub \
  --state-dir="$work/state" --repo="$work/repo" "$work/build" packaging/flatpak/io.github.lgpy.tabbyfin.yml
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$work/repo" "$out/tabbyfin-$version.flatpak" io.github.lgpy.tabbyfin
ls -l "$out/tabbyfin-$version.flatpak"
