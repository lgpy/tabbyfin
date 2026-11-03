#!/usr/bin/env bash
# Builds the debug APK for one ABI and installs and starts it on the device adb sees (the
# emulator, or a phone over Wi-Fi). The ABI is the device's own unless given.
#   bash packaging/android/install.sh [--abi arm64-v8a|x86_64]
set -euo pipefail

abi="${2:-}"
[ "${1:-}" = --abi ] || abi="$(adb shell getprop ro.product.cpu.abi | tr -d '\r')"
bash "$(dirname "$0")/build.sh" --abi "$abi"
adb install -r "$(dirname "$0")/build/outputs/apk/debug/tabbyfin-debug.apk"
adb shell am start -n io.github.lgpy.tabbyfin/.MainActivity
