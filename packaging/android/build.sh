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
# The library only, as a shared library: the crate's own `crate-type` stays rlib, so desktop
# builds don't link it twice.
(cd "$root" && cargo ndk "${cargo_args[@]}" rustc -p tabbyfin --lib --crate-type cdylib "${release[@]}")

(cd "$android" && gradle --no-daemon -q assembleDebug)
find "$android/build/outputs/apk" -name '*.apk'
