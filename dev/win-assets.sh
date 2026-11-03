#!/usr/bin/env bash
# Puts the Windows runtime libraries next to the built exe, so `cargo run` and `mise run run`
# play video: libmpv (packaging/mpv.lock) and ANGLE (packaging/angle.lock). Windows only.
#
#   bash dev/win-assets.sh        # into target/debug and target/release
#
# Build first: the folders that exist get the libraries. Delete libEGL.dll from one to try the
# WGL path instead of ANGLE.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

found=0
for profile in debug release; do
  dest="$root/target/$profile"
  [[ -d "$dest" ]] || continue
  found=1
  "$root/packaging/fetch-libmpv.sh" windows-x86_64 "$dest"
  [[ -f "$dest/libEGL.dll" ]] || "$root/packaging/fetch-angle.sh" "$dest"
done

if [[ $found == 0 ]]; then
  echo "no target/debug or target/release yet: build first (mise run build)" >&2
  exit 1
fi
