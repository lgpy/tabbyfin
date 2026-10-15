#!/usr/bin/env bash
# Downloads a pinned file and verifies its SHA256. Tries our own copy first: the `deps` release
# of this repo, where packaging/mirror-deps.sh uploads every pinned file under its checksum, so
# builds keep working if a third party moves or prunes it. Then the original URL.
#
#   packaging/download.sh <url> <sha256> <out-file>
#
# The copy is fetched with `gh` when it's signed in (CI: GH_TOKEN), which also works while the
# repo is private, or anonymously from the release's public URL.
set -euo pipefail

url="$1"
sha="$2"
out="$3"
repo=lgpy/jelly-gpui

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

verified() {
  [[ -f "$out" && "$(sha256 "$out")" == "$sha" ]]
}

# Already there (handed in by the caller) and intact: nothing to do.
verified && exit 0
rm -f "$out"
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  dir="$(mktemp -d)"
  if gh release download deps -R "$repo" -p "$sha" -D "$dir" >/dev/null 2>&1; then
    mv "$dir/$sha" "$out"
  fi
  rm -rf "$dir"
fi
verified || curl -fsSL -o "$out" "https://github.com/$repo/releases/download/deps/$sha" 2>/dev/null || true
verified || curl -fsSL -o "$out" "$url"
if ! verified; then
  echo "checksum mismatch for ${url##*/}: expected $sha, got $(sha256 "$out")" >&2
  exit 1
fi
