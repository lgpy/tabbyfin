#!/usr/bin/env python3
"""Saves real API responses from a dev server as test fixtures for the jellyfin crate.

    dev/capture-fixtures.py [server url] [version label]
    dev/capture-fixtures.py http://localhost:8096 12.2
    dev/capture-fixtures.py http://localhost:8097 10.11

Writes crates/jellyfin/tests/fixtures/<label>/*.json, which `cargo test -p jellyfin` parses
into the client's models. Access tokens and API keys are replaced, so the files are safe to
commit. Needs the dev media (dev/make-media.sh) and setup (dev/setup-server.py).
"""

import json
import re
import sys
import urllib.request
from pathlib import Path

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8096"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "12.2"
OUT = Path(__file__).resolve().parent.parent / "crates/jellyfin/tests/fixtures" / LABEL
CLIENT = 'Client="tabbyfin-fixtures", Device="fixtures", DeviceId="tabbyfin-fixtures", Version="0.1.0"'
SECRET = re.compile(r"(?i)((?:api_?key|token)[\"=:]+\s*\"?)[0-9a-f]{32}")


def request(path, body=None, token=None, method=None):
    auth = "MediaBrowser " + CLIENT + (f', Token="{token}"' if token else "")
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        BASE + path,
        data=data,
        method=method or ("POST" if body is not None else "GET"),
        headers={"Content-Type": "application/json", "Authorization": auth},
    )
    with urllib.request.urlopen(req) as resp:
        text = resp.read().decode()
    return json.loads(text) if text else None


def save(name, value):
    text = json.dumps(value, indent=2, sort_keys=True)
    text = SECRET.sub(lambda m: m.group(1) + "0" * 32, text)
    (OUT / f"{name}.json").write_text(text + "\n")
    print(f"  {name}.json")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    print(f"capturing {BASE} into {OUT}")
    save("public_info", request("/System/Info/Public"))
    auth = request("/Users/AuthenticateByName", {"Username": "dev", "Pw": "dev"})
    token = auth["AccessToken"]
    save("authenticate", auth)
    get = lambda path: request(path, token=token)

    save("user_views", get("/UserViews"))
    fields = "PrimaryImageAspectRatio,Overview,Genres,MediaSources,People,Chapters"
    movies = get(f"/Items?Recursive=true&IncludeItemTypes=Movie&SortBy=SortName&Fields={fields}")
    save("items_movies", movies)
    movie = next(m for m in movies["Items"] if m["Name"].startswith("Test Movie"))
    save("item_movie", get(f"/Items/{movie['Id']}?Fields={fields}"))
    series = get("/Items?Recursive=true&IncludeItemTypes=Series")["Items"][0]
    save("item_series", get(f"/Items/{series['Id']}?Fields=Overview,Genres,People"))
    seasons = get(f"/Shows/{series['Id']}/Seasons")
    save("seasons", seasons)
    episodes = get(f"/Shows/{series['Id']}/Episodes?SeasonId={seasons['Items'][0]['Id']}&Fields=Overview,Chapters")
    save("episodes", episodes)
    save("resume", get("/UserItems/Resume?MediaTypes=Video&Limit=24"))
    save("next_up", get("/Shows/NextUp?Limit=24"))
    views = get("/UserViews")["Items"]
    save("latest", get(f"/Items/Latest?ParentId={views[0]['Id']}&Limit=24"))
    library = next(v for v in views if v.get("CollectionType") == "movies")
    save("item_filters", get(f"/Items/Filters?ParentId={library['Id']}&IncludeItemTypes=Movie"))
    save("media_segments", get(f"/MediaSegments/{episodes['Items'][0]['Id']}"))
    save("syncplay_list", get("/SyncPlay/List"))
    save("utc_time", get("/GetUtcTime"))
    save("cultures", get("/Localization/Cultures"))

    profile = {
        "MaxStreamingBitrate": 2_000_000,
        "DirectPlayProfiles": [{"Type": "Video"}],
        "TranscodingProfiles": [
            {"Type": "Video", "Context": "Streaming", "Protocol": "hls", "Container": "ts",
             "VideoCodec": "h264", "AudioCodec": "aac"}
        ],
        "SubtitleProfiles": [{"Format": "subrip", "Method": "External"}],
    }
    body = {"MaxStreamingBitrate": 2_000_000, "EnableDirectPlay": True, "EnableTranscoding": True,
            "DeviceProfile": profile}
    save("playback_info_transcode", request(f"/Items/{movie['Id']}/PlaybackInfo", body, token))
    # The app's own profile (crates/jellyfin/src/playback.rs): no bitrate limit, direct play
    # of everything.
    unlimited = 1_000_000_000
    # 12.2 refuses direct play without subtitle profiles (10.11 doesn't), so they're needed.
    profile = {"MaxStreamingBitrate": unlimited, "MaxStaticBitrate": unlimited,
               "DirectPlayProfiles": [{"Type": "Video"}, {"Type": "Audio"}],
               "TranscodingProfiles": profile["TranscodingProfiles"],
               "SubtitleProfiles": [{"Format": "subrip", "Method": "External"},
                                    {"Format": "subrip", "Method": "Embed"}]}
    body = {"MaxStreamingBitrate": unlimited, "EnableDirectPlay": True, "EnableDirectStream": True,
            "EnableTranscoding": True, "DeviceProfile": profile}
    save("playback_info_direct", request(f"/Items/{movie['Id']}/PlaybackInfo", body, token))


if __name__ == "__main__":
    main()
