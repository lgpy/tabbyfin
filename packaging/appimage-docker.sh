#!/usr/bin/env bash
# Builds the Linux AppImage in an Ubuntu 22.04 container: the binary and the libraries it
# bundles get that system's glibc (2.35) as their minimum, so the AppImage runs on
# distributions from 2022 on (Ubuntu 22.04, Debian 12, Fedora 36…). The Rust toolchain and the
# cargo cache stay in target/appimage between runs.
#
#   packaging/appimage-docker.sh [out-dir]      (needs Docker; CI runs the same)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-dist}"
mkdir -p "$root/$out" "$root/target/appimage"
rust="$(sed -n 's/^rust = { version = "\([^"]*\)".*/\1/p' "$root/mise.toml")"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)"

docker run --rm -v "$root":/src -w /src \
  -e RUST="$rust" -e VERSION="$version" -e OUT="$out" -e OWNER="$(id -u):$(id -g)" \
  ubuntu:22.04 bash -c '
set -euo pipefail
# Files written here belong to root; hand them back whatever happens.
trap "chown -R $OWNER /src/target/appimage /src/$OUT" EXIT
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
# Build dependencies (docs/PLAN.md §8), libmpv1 (mpv 0.34; the app also takes libmpv.so.1)
# to bundle, and file, which linuxdeploy uses.
apt-get install -y -qq --no-install-recommends curl ca-certificates build-essential pkg-config \
  libmpv1 libvulkan-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev \
  libfontconfig-dev libfreetype-dev libssl-dev file >/dev/null
export CARGO_HOME=/src/target/appimage/cargo RUSTUP_HOME=/src/target/appimage/rustup
[[ -x "$CARGO_HOME/bin/cargo" ]] || curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal \
  --default-toolchain "$RUST" --no-modify-path
"$CARGO_HOME/bin/rustup" toolchain install --profile minimal "$RUST" >/dev/null
CARGO_TARGET_DIR=/src/target/appimage/build "$CARGO_HOME/bin/cargo" "+$RUST" build --release --locked -p jelly-gpui
packaging/appimage.sh target/appimage/build/release/jelly-gpui "$VERSION" "$OUT"
'
ls -l "$root/$out"/*.AppImage
