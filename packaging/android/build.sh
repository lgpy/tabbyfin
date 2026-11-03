#!/usr/bin/env bash
# Builds the Android APK: the Rust app as libtabbyfin.so for each ABI, then the Gradle project
# around it. Debug by default; `--release` builds the library in release, still packaged as a
# debug-signed APK so it installs anywhere (there's no release signing key yet).
#   bash packaging/android/build.sh [--release] [--abi arm64-v8a|x86_64]...
# Needs ANDROID_HOME (the SDK, with the NDK below), and the mise tools (Rust's Android targets,
# cargo-ndk, Gradle, Java).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
android="$root/packaging/android"
ndk_version=29.0.14206865

profile=debug
abis=()
while [ $# -gt 0 ]; do
    case "$1" in
        --release) profile=release ;;
        --abi) abis+=("$2"); shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done
[ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a x86_64)

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$ndk_version}"

cargo_args=(-P 26 -o "$android/src/main/jniLibs")
for abi in "${abis[@]}"; do cargo_args+=(-t "$abi"); done
release=()
[ "$profile" = release ] && release=(--release)
# Video spike: the dev aids (TABBYFIN_ROUTE and the rest) are debug-only; keep them in an optimised build.
[ -n "${SPIKE_DEBUG_ASSERTIONS:-}" ] && export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true
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

(cd "$android" && gradle --no-daemon -q assembleDebug)
find "$android/build/outputs/apk" -name '*.apk'
