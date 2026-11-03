#!/usr/bin/env bash
# Finds a tar that can read what `download.sh` fetches: .7z (libmpv) and .tar.zst (ANGLE).
# Git Bash puts GNU tar first on PATH, which reads neither, so Windows' own `tar.exe` (bsdtar,
# in System32) has to be named explicitly. Sourced by fetch-libmpv.sh, fetch-angle.sh and
# dev/win-assets.sh.
#
#   bsdtar_bin   # prints the path, fails when there is none

bsdtar_bin() {
  local candidate
  for candidate in bsdtar bsdtar.exe /c/Windows/System32/tar.exe /c/WINDOWS/system32/tar.exe; do
    if command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  return 1
}
