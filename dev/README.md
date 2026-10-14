# Dev Jellyfin server

A local Jellyfin 12.2 with a small synthetic library, for developing against.

```sh
./dev/make-media.sh                          # generate test media into dev/.data/media (needs ffmpeg)
docker compose -f dev/compose.yaml up -d     # start the server on http://localhost:8096
./dev/setup-server.py                        # run the startup wizard, create libraries, scan
```

Login: `dev` / `dev`. Legacy authorization is disabled, which is the 12.x default.

**Library contents**
- `Test Movie (2024)`: two versions (1080p and 720p).
- `Second Movie (2025)`: has an external `.en.srt` subtitle.
- `Test Show`: Season 1 with three episodes, Season 2 with two.

Every video is 60 s long and has:
- English and Japanese audio tracks.
- An embedded English subtitle track.

Remote metadata and image fetchers are turned off, so the fake titles never match real TMDb or TVDb entries.

**Reset everything:** `docker compose -f dev/compose.yaml down && rm -rf dev/.data/config dev/.data/cache`, then run the start and setup steps again. The container runs as your user, so no `sudo` is needed.
