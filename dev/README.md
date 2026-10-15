# Dev Jellyfin server

A local Jellyfin 12.2 with a small synthetic library, for developing against, plus an optional 10.11 server for the minimum supported version.

```sh
mise run dev:setup     # generate media, start the server, run the startup wizard, scan
mise run dev:down      # stop it
mise run dev:reset     # wipe the server state (keeps the media) and set it up again
```

Without mise, the same steps by hand:

```sh
./dev/make-media.sh                          # generate test media into dev/.data/media (needs ffmpeg)
docker compose -f dev/compose.yaml up -d     # start the server on http://localhost:8096
./dev/setup-server.py                        # run the startup wizard, create libraries, scan
```

Login: `dev` / `dev`. Legacy authorization is disabled, which is the 12.x default.

**Library contents**
- `Test Movie (2024)`: three versions (1080p and 720p H.264, and 4K HEVC 10-bit for hardware decoding).
- `Second Movie (2025)`: has an external `.en.srt` subtitle.
- `Test Show`: Season 1 with three episodes, Season 2 with two.

Every video is 60 s long and has:
- English and Japanese audio tracks.
- An embedded English subtitle track.
- Chapters: Intro (0–10 s), Episode, and Credits (the last 15 s), for skip intro and the up-next card. `make-media.sh` adds them to videos made before it did; rescan the library afterwards.
- Trickplay tiles (a 320 px thumbnail every 10 s) for the seek-bar preview. The setup script turns them on and starts the server's generation task without waiting; they appear a minute or two later.

Remote metadata and image fetchers are turned off, so the fake titles never match real TMDb or TVDb entries.

**Minimum version (10.11):** `mise run dev:min` starts Jellyfin 10.11 on http://localhost:8097, with its own state in `dev/.data/min` and the same media, sets it up the same way (also turning legacy authorization off, which 10.11 leaves on), and runs `cargo run -p jellyfin --example smoke` against it. `mise run dev:down` stops both servers.

**Websocket check:** `cargo run -p jellyfin --example socket` (with `JELLYFIN_URL=localhost:8097` for 10.11) connects the session websocket, samples the server clock and waits for a `UserDataChanged` push after toggling a movie's watched state (which it toggles back). `JELLYFIN_SOCKET_HOLD=150` also checks that the keep-alive holds the connection.

**Network drops:** debug builds take extra mpv options from `JELLY_GPUI_MPV_OPTIONS` (`name=value;…`). With a small cache, `"ao=null;cache=no;demuxer-max-bytes=4MiB;demuxer-readahead-secs=0.5"`, playback actually depends on the network, so `docker stop` / `docker start` of the server during playback exercises the reconnect. `ao=null` also avoids WSL's audio, which sometimes takes 30 s to open. Play an item with no resume point (mark it unwatched first), or it may finish before the outage matters.

**Test fixtures:** `dev/capture-fixtures.py http://localhost:8096 12.2` and `dev/capture-fixtures.py http://localhost:8097 10.11` save real responses (items, playback info, segments…) to `crates/jellyfin/tests/fixtures/<version>/`, with tokens and API keys zeroed. `cargo test -p jellyfin` parses them all into the client's models. Recapture after changing the dev media or adding a model.

**Player process:** playback runs in `jelly-gpui --player-host` (see `docs/PLAN.md` §5.1b). `JELLY_GPUI_PLAYER=embedded` runs libmpv in the app instead, to compare. To test crash recovery, kill the host process during playback; on Linux, match it by process name (`pgrep -x jelly-gpui`, then check `/proc/<pid>/cmdline` for `--player-host`), since `pkill -f -- --player-host` also matches the shell running it.

**SyncPlay:** `cargo run -p jellyfin --example syncplay` is a scripted group member. It joins (or creates) `SYNCPLAY_GROUP` (default `dev group`), plays along with `Buffering`/`Ready`, prints every update and command, and runs `SYNCPLAY_SCRIPT`, e.g. `"wait:12 queue:<item id> wait:6 pause wait:3 unpause wait:5 seek:30 wait:5 stop leave"`. Debug builds of the app join a group at startup with `JELLY_GPUI_SYNCPLAY=<group name>`, so the app and the example can be run together. `JELLY_GPUI_SYNCPLAY_SKEW_MS=800` makes the app start playback that late, to watch drift correction catch up (`RUST_LOG=jelly_gpui::syncplay=debug` logs the drift every second).

**Reset everything:** `docker compose -f dev/compose.yaml down && rm -rf dev/.data/config dev/.data/cache`, then run the start and setup steps again. The container runs as your user, so no `sudo` is needed.
