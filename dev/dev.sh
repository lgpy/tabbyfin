#!/usr/bin/env bash
# The dev-server steps that take more than one command, so mise's tasks (`dev:min`, `dev:reset`)
# are one command each: mise runs tasks through cmd.exe on Windows, which has no `mkdir -p`,
# `rm -rf` or `VAR=x cmd`.
#
#   bash dev/dev.sh min      # 10.11 server on :8097, set up, smoke test against it
#   bash dev/dev.sh reset     # wipe the 12.2 server's state (keeps the media) and set it up again
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

case "${1-}" in
  min)
    # Created as the user first; Docker would create missing bind mounts as root.
    mkdir -p dev/.data/min/config dev/.data/min/cache
    docker compose -f dev/compose.yaml --profile min up -d jellyfin-min
    JELLYFIN_URL=http://localhost:8097 python -I dev/setup-server.py
    JELLYFIN_URL=localhost:8097 cargo run -q -p jellyfin --example smoke
    ;;

  reset)
    docker compose -f dev/compose.yaml down
    rm -rf dev/.data/config dev/.data/cache
    mkdir -p dev/.data/config dev/.data/cache
    bash dev/make-media.sh
    docker compose -f dev/compose.yaml up -d
    python -I dev/setup-server.py
    ;;

  *)
    echo "usage: $0 {min|reset}" >&2
    exit 2
    ;;
esac
