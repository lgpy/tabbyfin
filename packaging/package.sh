#!/usr/bin/env bash
# Packages a release build for one target into <out-dir> (default: dist/).
# Run `cargo build --release --target <target>` first.
#
#   linux   (x86_64-unknown-linux-gnu): .tar.gz and .deb (needs cargo-deb), which use the system
#           libmpv. The AppImage is built separately, in Ubuntu 22.04 (appimage-docker.sh).
#   windows (x86_64-pc-windows-msvc):   .zip with libmpv-2.dll and ANGLE next to jelly-gpui.exe, and
#           an installer from the same files when Inno Setup's ISCC is found (on the PATH, in
#           its default folder, or $ISCC)
#   macos   (aarch64/x86_64-apple-darwin): .zip of jelly-gpui.app, ad-hoc signed; needs Homebrew's
#           libmpv (brew install mpv) until libmpv is bundled
set -euo pipefail

target="$1"
out="${2:-dist}"
root="$(cd "$(dirname "$0")/.." && pwd)"
packaging="$root/packaging"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)"
bin_dir="$root/target/$target/release"

mkdir -p "$out"
out="$(cd "$out" && pwd)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

case "$target" in
  x86_64-unknown-linux-gnu)
    name="jelly-gpui-$version-linux-x86_64"
    mkdir -p "$stage/$name"
    cp "$bin_dir/jelly-gpui" "$packaging/jelly-gpui.desktop" "$packaging/jelly-gpui.svg" "$root/LICENSE" \
      "$stage/$name/"
    tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
    (cd "$root" && cargo deb -p jelly-gpui --target "$target" --no-build --output "$out/")
    ;;

  x86_64-pc-windows-msvc)
    name="jelly-gpui-$version-windows-x86_64"
    mkdir -p "$stage/$name"
    cp "$bin_dir/jelly-gpui.exe" "$packaging/THIRD_PARTY.md" "$root/LICENSE" "$stage/$name/"
    "$packaging/fetch-libmpv.sh" windows-x86_64 "$stage/$name"
    "$packaging/fetch-angle.sh" "$stage/$name"
    if command -v 7z >/dev/null; then
      (cd "$stage" && 7z a -tzip "$out/$name.zip" "$name" >/dev/null)
    else
      (cd "$stage" && zip -qr "$out/$name.zip" "$name")
    fi
    "$packaging/installer.sh" "$version" "$stage/$name" "$out"
    ;;

  aarch64-apple-darwin | x86_64-apple-darwin)
    arch="${target%%-*}"
    app="$stage/jelly-gpui.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin_dir/jelly-gpui" "$app/Contents/MacOS/"
    sed "s/@VERSION@/$version/g" "$packaging/Info.plist" > "$app/Contents/Info.plist"
    cp "$packaging/THIRD_PARTY.md" "$root/LICENSE" "$app/Contents/Resources/"
    # Ad-hoc signature only: not notarized, so Gatekeeper asks before the first launch.
    codesign --force --deep --sign - "$app"
    ditto -c -k --keepParent "$app" "$out/jelly-gpui-$version-macos-$arch.zip"
    ;;

  *)
    echo "unsupported target: $target" >&2
    exit 1
    ;;
esac
ls -l "$out"
