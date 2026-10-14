#!/usr/bin/env bash
# Downloads the pinned ANGLE build and its runtime DLLs (packaging/angle.lock), verifies the
# checksums, and puts the DLLs into <dest> and their licenses into <dest>/licenses.
#
#   packaging/fetch-angle.sh <dest>
#
# The packages are .tar.zst: extracted with zstd, a libarchive tar (bsdtar, Windows' tar.exe
# from WSL when <dest> is on a Windows drive), or Python's zstandard module.
set -euo pipefail

dest="$1"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=angle.lock
source "$here/angle.lock"

mkdir -p "$dest/licenses"
dest="$(cd "$dest" && pwd)"
# Next to <dest>, so Windows' tar.exe can reach it through a relative path.
work="$(mktemp -d "$dest/.angle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cd "$work"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

extract() { # <archive> <member>...
  local archive="$1"
  shift
  if command -v zstd >/dev/null; then
    zstd -dcq "$archive" | tar -x "$@"
  elif command -v bsdtar >/dev/null; then
    bsdtar -xf "$archive" "$@"
  elif command -v tar.exe >/dev/null; then
    tar.exe -xf "$archive" "$@"
  else
    python3 -I -c 'import sys, tarfile, zstandard
with zstandard.open(sys.argv[1], "rb") as f, tarfile.open(fileobj=f, mode="r|") as t:
    t.extractall(members=(m for m in t if m.name in sys.argv[2:]), filter="data")' "$archive" "$@"
  fi
}

for entry in "${ANGLE_PACKAGES[@]}"; do
  read -r package sha files <<<"$entry"
  # `+` in names (libstdc++) must be escaped in the URL.
  curl -fsSL -o "$package" "$ANGLE_BASE_URL/${package//+/%2B}"
  actual="$(sha256 "$package")"
  if [[ "$actual" != "$sha" ]]; then
    echo "checksum mismatch for $package: expected $sha, got $actual" >&2
    exit 1
  fi
  members=()
  for file in $files; do members+=("ucrt64/$file"); done
  extract "$package" "${members[@]}"
  for file in $files; do
    case "$file" in
      bin/*) cp "ucrt64/$file" "$dest/" ;;
      share/licenses/*)
        name="${file#share/licenses/}"
        mkdir -p "$dest/licenses/${name%%/*}"
        cp "ucrt64/$file" "$dest/licenses/$name"
        ;;
    esac
  done
done
echo "ANGLE -> $dest"
