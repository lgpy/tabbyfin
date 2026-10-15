#!/usr/bin/env bash
# Uploads every pinned download (mpv.lock, angle.lock, linuxdeploy.lock) to this repo's `deps`
# release, named by its SHA256, so packaging/download.sh can fetch our copy if the original
# disappears. Run it after changing a lock file; files already there are skipped. Needs `gh`
# signed in with write access. Creates the release the first time.
#
#   packaging/mirror-deps.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo=lgpy/jelly-gpui
# shellcheck source=mpv.lock
source "$here/mpv.lock"
# shellcheck source=angle.lock
source "$here/angle.lock"
# shellcheck source=linuxdeploy.lock
source "$here/linuxdeploy.lock"

pins=("$LIBMPV_WINDOWS_X86_64_URL $LIBMPV_WINDOWS_X86_64_SHA256" "$LINUXDEPLOY_X86_64_URL $LINUXDEPLOY_X86_64_SHA256")
for entry in "${ANGLE_PACKAGES[@]}"; do
  read -r package sha _ <<<"$entry"
  pins+=("$ANGLE_BASE_URL/${package//+/%2B} $sha")
done

if ! gh release view deps -R "$repo" >/dev/null 2>&1; then
  gh release create deps -R "$repo" --prerelease --latest=false --title "Pinned build dependencies" \
    --notes "Copies of the third-party files the packaging scripts download (packaging/*.lock), named by SHA256. Uploaded by packaging/mirror-deps.sh; not a release of the app."
fi
have="$(gh release view deps -R "$repo" --json assets -q '.assets[].name')"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for pin in "${pins[@]}"; do
  read -r url sha <<<"$pin"
  if grep -qx "$sha" <<<"$have"; then
    echo "have ${url##*/}"
    continue
  fi
  # From the original, never from our copy: this is what checks the pin against its source.
  curl -fsSL -o "$work/$sha" "$url"
  actual="$(sha256sum "$work/$sha" | cut -d' ' -f1)"
  [[ "$actual" == "$sha" ]] || { echo "checksum mismatch for ${url##*/}: got $actual" >&2; exit 1; }
  gh release upload deps -R "$repo" "$work/$sha"
  echo "uploaded ${url##*/}"
done
