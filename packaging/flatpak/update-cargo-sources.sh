#!/usr/bin/env bash
# Regenerates cargo-sources.json from Cargo.lock (run after dependencies change): the crates
# the Flatpak build fetches, since it has no network. Uses flatpak-builder-tools' generator,
# pinned to a commit, in a throwaway virtualenv. Runs from Git Bash on Windows too.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
tools_commit=74697c75b630d7330e77250fc13cb5ea688d9479
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Git Bash paths (/tmp/…, /c/…) mean nothing to a native Windows Python: hand it mixed ones (C:/…).
native() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s\n' "$1"; fi
}

curl -fsSL -o "$work/flatpak-cargo-generator.py" \
  "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$tools_commit/cargo/flatpak-cargo-generator.py"
python -m venv "$work/venv"
# A Windows virtualenv keeps its interpreter in Scripts/, the others in bin/.
if [ -d "$work/venv/Scripts" ]; then venv_python="$work/venv/Scripts/python"; else venv_python="$work/venv/bin/python"; fi
"$venv_python" -m pip install -q 'aiohttp>=3.9.5,<4' 'PyYAML>=6.0.2,<7' 'tomlkit>=0.13.3,<1'
"$venv_python" -I "$(native "$work/flatpak-cargo-generator.py")" "$(native "$here/../../Cargo.lock")" \
  -o "$(native "$here/cargo-sources.json")"
# The generator writes text mode, so Windows gives it CRLF; the repo keeps LF (.gitattributes).
"$venv_python" -I -c 'import pathlib, sys; f = pathlib.Path(sys.argv[1]); f.write_bytes(f.read_bytes().replace(b"\r\n", b"\n"))' \
  "$(native "$here/cargo-sources.json")"
