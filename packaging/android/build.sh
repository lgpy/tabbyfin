#!/usr/bin/env bash
# Builds the Android APK: the Rust app as libtabbyfin.so for each ABI, then the Gradle project
# around it.
#   bash packaging/android/build.sh [--release | --sign] [--abi arm64-v8a|x86_64]...
# - Debug by default: one debug-signed APK with every ABI built.
# - `--release` builds the library in release, still in a debug-signed APK, so it installs over
#   a debug build (CI's APK on every push).
# - `--sign` is the release itself: the library in release, R8, and a release-signed APK per ABI
#   (build/outputs/apk/release/). It needs the key in TABBYFIN_KEYSTORE (the file),
#   TABBYFIN_KEYSTORE_PASSWORD, TABBYFIN_KEY_ALIAS and TABBYFIN_KEY_PASSWORD (docs/android.md).
# Needs ANDROID_HOME (the SDK, with the NDK below), and the mise tools (Rust's Android targets,
# cargo-ndk, Gradle, Java).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
android="$root/packaging/android"
ndk_version=29.0.14206865

profile=debug
sign=
abis=()
while [ $# -gt 0 ]; do
    case "$1" in
        --release) profile=release ;;
        --sign) profile=release; sign=1 ;;
        --abi) abis+=("$2"); shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done
[ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a x86_64)

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
if [ -n "$sign" ]; then
    for var in TABBYFIN_KEYSTORE TABBYFIN_KEYSTORE_PASSWORD TABBYFIN_KEY_ALIAS TABBYFIN_KEY_PASSWORD; do
        [ -n "${!var:-}" ] || { echo "--sign needs $var" >&2; exit 2; }
    done
    [ -f "$TABBYFIN_KEYSTORE" ] || { echo "no keystore at $TABBYFIN_KEYSTORE" >&2; exit 2; }
fi
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$ndk_version}"

cargo_args=(-P 26 -o "$android/src/main/jniLibs")
for abi in "${abis[@]}"; do cargo_args+=(-t "$abi"); done
release=()
[ "$profile" = release ] && release=(--release)
# Video spike: the dev aids (TABBYFIN_ROUTE and the rest) are debug-only; keep them in an optimised build.
[ -n "${SPIKE_DEBUG_ASSERTIONS:-}" ] && export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true
# A release's library without its symbol table: 12 MB less per ABI. Debug and `--release` builds
# keep it, for stack traces in logcat.
[ -n "$sign" ] && export CARGO_PROFILE_RELEASE_STRIP=symbols
# The library only, as a shared library: the crate's own `crate-type` stays rlib, so desktop
# builds don't link it twice.
(cd "$root" && cargo ndk "${cargo_args[@]}" rustc -p tabbyfin --lib --crate-type cdylib "${release[@]}")

# libmpv and its ffmpeg, from mpv-android's release (mpv-android.lock), beside our library.
source "$android/mpv-android.lock"
cache="$root/target/mpv-android"
mkdir -p "$cache"
for abi in "${abis[@]}"; do
    for entry in "${MPV_ANDROID_APKS[@]}"; do
        read -r lock_abi apk sha <<<"$entry"
        [ "$lock_abi" = "$abi" ] || continue
        bash "$root/packaging/download.sh" "$MPV_ANDROID_BASE_URL/$apk" "$sha" "$cache/$apk"
        unzip -o -q -j "$cache/$apk" "lib/$abi/*.so" -x "lib/$abi/libplayer.so" -d "$android/src/main/jniLibs/$abi"
    done
done

# A library left in jniLibs by an earlier build for another ABI would go in the APKs too.
for dir in "$android/src/main/jniLibs"/*/; do
    [ -d "$dir" ] || continue
    keep=
    for abi in "${abis[@]}"; do [ "$(basename "$dir")" = "$abi" ] && keep=1; done
    [ -n "$keep" ] || rm -rf "$dir"
done

if [ -n "$sign" ]; then
    rm -rf "$android/build/outputs/apk/release"
    (cd "$android" && gradle --no-daemon -q assembleRelease)
    find "$android/build/outputs/apk/release" -name '*.apk'
else
    (cd "$android" && gradle --no-daemon -q assembleDebug)
    find "$android/build/outputs/apk/debug" -name '*.apk'
fi
