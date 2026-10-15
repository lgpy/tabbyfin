#!/usr/bin/env bash
# Builds the Windows installer (packaging/jelly-gpui.iss) with Inno Setup's ISCC.exe, from a
# staged app folder. Works from Git Bash on Windows (CI) and from WSL. Without ISCC it says so
# and does nothing, so the .zip alone still gets built.
#
#   packaging/installer.sh <version> <staged app folder> <out-dir>
set -euo pipefail

version="$1"
source_dir="$2"
out="$3"
here="$(cd "$(dirname "$0")" && pwd)"

iscc="${ISCC:-}"
if [[ -z "$iscc" ]]; then
  for candidate in ISCC.exe iscc "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
    "/mnt/c/Program Files (x86)/Inno Setup 6/ISCC.exe"; do
    if command -v "$candidate" >/dev/null 2>&1 || [[ -x "$candidate" ]]; then
      iscc="$candidate"
      break
    fi
  done
fi
if [[ -z "$iscc" ]]; then
  echo "Inno Setup's ISCC.exe not found (set ISCC); skipping the installer" >&2
  exit 0
fi

# ISCC is a Windows program: it wants Windows paths.
winpath() {
  if command -v cygpath >/dev/null; then cygpath -w "$1"; else wslpath -w "$1"; fi
}
"$iscc" /Q "/DAppVersion=$version" "/DSourceDir=$(winpath "$source_dir")" "/DOutputDir=$(winpath "$out")" \
  "$(winpath "$here/jelly-gpui.iss")"
