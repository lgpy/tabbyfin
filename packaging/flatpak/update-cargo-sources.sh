#!/usr/bin/env bash
# Regenerates cargo-sources.json from Cargo.lock (run after dependencies change): the crates
# the Flatpak build fetches, since it has no network. Uses flatpak-builder-tools' generator,
# pinned to a commit, in a throwaway virtualenv.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
tools_commit=74697c75b630d7330e77250fc13cb5ea688d9479
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/flatpak-cargo-generator.py" \
  "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$tools_commit/cargo/flatpak-cargo-generator.py"
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install -q 'aiohttp>=3.9.5,<4' 'PyYAML>=6.0.2,<7' 'tomlkit>=0.13.3,<1'
"$work/venv/bin/python" -I "$work/flatpak-cargo-generator.py" "$here/../../Cargo.lock" -o "$here/cargo-sources.json"
