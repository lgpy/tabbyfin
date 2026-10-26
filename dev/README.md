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

Login: `dev` / `dev` (an administrator), and `dev2` / `dev2` for testing several users. Legacy authorization is disabled, which is the 12.x default.

**Library contents**
- `Test Movie (2024)`: three versions (1080p and 720p H.264, and 4K HEVC 10-bit for hardware decoding), a local trailer and a featurette (20 s each, without chapters). Science Fiction and Documentary, tags `test pattern` and `multi-version`, rated PG.
- `Second Movie (2025)`: has an external `.en.srt` subtitle. Comedy, tag `test pattern`, rated PG-13.
- The setup script sets the movies' genres, studio, tags and ratings (locked, re-applied when they differ), for the item page, "More like this" and the library filters.
- `Test Show`: Season 1 with three episodes, Season 2 with two.

Every video is 60 s long and has:
- English and Japanese audio tracks.
- An embedded English subtitle track.
- Chapters: Intro (0–10 s), Episode, and Credits (the last 15 s), for skip intro and the up-next card. `make-media.sh` adds them to videos made before it did; rescan the library afterwards.
- Trickplay tiles (a 320 px thumbnail every 10 s) for the seek-bar preview, and chapter images. The setup script turns both on and starts the server's tasks without waiting; they appear a minute or two later.

Remote metadata and image fetchers are turned off, so the fake titles never match real TMDb or TVDb entries.

**Minimum version (10.11):** `mise run dev:min` starts Jellyfin 10.11 on http://localhost:8097, with its own state in `dev/.data/min` and the same media, sets it up the same way (also turning legacy authorization off, which 10.11 leaves on), and runs `cargo run -p jellyfin --example smoke` against it. `mise run dev:down` stops both servers.

**Websocket check:** `cargo run -p jellyfin --example socket` (with `JELLYFIN_URL=localhost:8097` for 10.11) connects the session websocket, samples the server clock and waits for a `UserDataChanged` push after toggling a movie's watched state (which it toggles back). `JELLYFIN_SOCKET_HOLD=150` also checks that the keep-alive holds the connection.

**Network drops:** debug builds take extra mpv options from `TABBYFIN_MPV_OPTIONS` (`name=value;…`). With a small cache, `"ao=null;cache=no;demuxer-max-bytes=4MiB;demuxer-readahead-secs=0.5"`, playback actually depends on the network, so `docker stop` / `docker start` of the server during playback exercises the reconnect. `ao=null` also avoids WSL's audio, which sometimes takes 30 s to open. Play an item with no resume point (mark it unwatched first), or it may finish before the outage matters.

**Test fixtures:** `dev/capture-fixtures.py http://localhost:8096 12.2` and `dev/capture-fixtures.py http://localhost:8097 10.11` save real responses (items, playback info, segments…) to `crates/jellyfin/tests/fixtures/<version>/`, with tokens and API keys zeroed. `cargo test -p jellyfin` parses them all into the client's models. Recapture after changing the dev media or adding a model.

**Player process:** playback runs in `tabbyfin --player-host` (see `docs/PLAN.md` §5.1b). `TABBYFIN_PLAYER=embedded` runs libmpv in the app instead, to compare. To test crash recovery, kill the host process during playback; on Linux, match it by process name (`pgrep -x tabbyfin`, then check `/proc/<pid>/cmdline` for `--player-host`), since `pkill -f -- --player-host` also matches the shell running it.

**SyncPlay:** `cargo run -p jellyfin --example syncplay` is a scripted group member. It joins (or creates) `SYNCPLAY_GROUP` (default `dev group`), plays along with `Buffering`/`Ready`, prints every update and command, and runs `SYNCPLAY_SCRIPT`, e.g. `"wait:12 queue:<item id> wait:6 pause wait:3 unpause wait:5 seek:30 wait:5 stop leave"`. Debug builds of the app join a group at startup with `TABBYFIN_SYNCPLAY=<group name>`, so the app and the example can be run together. `TABBYFIN_SYNCPLAY_SKEW_MS=800` makes the app start playback that late, to watch drift correction catch up (`RUST_LOG=tabbyfin::syncplay=debug` logs the drift every second).

**Reset everything:** `docker compose -f dev/compose.yaml down && rm -rf dev/.data/config dev/.data/cache`, then run the start and setup steps again. The container runs as your user, so no `sudo` is needed.

**Big library:** `dev/fake-library.py` is a proxy in front of the dev server that adds a 10,000-movie library with generated posters (see its header). With a copy of the config pointing at it and `TABBYFIN_KEYS="wait wait wait down*150 wait"` (a key with `*N` repeats it N times, 30 ms apart), it shows how many posters a fast scroll loads, how many at once, and how long the last ones take; sample the app's `/proc/<pid>/stat` and `VmRSS` for CPU and memory. Until U5's library page lands, `TABBYFIN_ROUTE=posters:ffffffffffffffffffffffffffffffff` opens I1's debug page of the fake library's posters (docs/egui-migration.md §4). With `TABBYFIN_CONFIG_DIR` set, the image cache goes under it too (`<dir>/cache`), so these runs don't fill the real one.

**Update notice:** debug builds check `TABBYFIN_UPDATE_URL` instead of GitHub's latest release, on every launch rather than once a day. Serve a JSON like `{"tag_name": "v9.9.9", "html_url": "https://…"}` (e.g. `python3 -m http.server`) to see the notification.
