#!/usr/bin/env bash
# Packages a release build for one target into <out-dir> (default: dist/).
# Run `cargo build --release --target <target>` first.
#
#   linux   (x86_64-unknown-linux-gnu): .tar.gz and .deb (needs cargo-deb); uses the system mpv
#   windows (x86_64-pc-windows-msvc):   .zip with mpv.exe next to jelly-gpui.exe
#   macos   (aarch64/x86_64-apple-darwin): .zip of jelly-gpui.app with mpv.app inside, ad-hoc signed
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
    cp "$bin_dir/jelly-gpui" "$packaging/jelly-gpui.desktop" "$packaging/jelly-gpui.svg" "$stage/$name/"
    tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
    (cd "$root" && cargo deb -p jelly-gpui --target "$target" --no-build --output "$out/")
    ;;

  x86_64-pc-windows-msvc)
    name="jelly-gpui-$version-windows-x86_64"
    mkdir -p "$stage/$name"
    cp "$bin_dir/jelly-gpui.exe" "$packaging/THIRD_PARTY.md" "$stage/$name/"
    "$packaging/fetch-mpv.sh" windows-x86_64 "$stage/$name"
    if command -v 7z >/dev/null; then
      (cd "$stage" && 7z a -tzip "$out/$name.zip" "$name" >/dev/null)
    else
      (cd "$stage" && zip -qr "$out/$name.zip" "$name")
    fi
    ;;

  aarch64-apple-darwin | x86_64-apple-darwin)
    arch="${target%%-*}"
    app="$stage/jelly-gpui.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin_dir/jelly-gpui" "$app/Contents/MacOS/"
    sed "s/@VERSION@/$version/g" "$packaging/Info.plist" > "$app/Contents/Info.plist"
    cp "$packaging/THIRD_PARTY.md" "$app/Contents/Resources/"
    "$packaging/fetch-mpv.sh" "macos-$arch" "$app/Contents/Resources"
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
