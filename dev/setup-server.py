#!/usr/bin/env python3
"""Runs the Jellyfin startup wizard over the API and creates the dev libraries.

Safe to re-run: steps that are already done are skipped. Python stdlib only.
Env: JELLYFIN_URL (default http://localhost:8096), JELLYFIN_USER / JELLYFIN_PASS (default dev/dev).
"""
import json
import os
import time
import urllib.error
import urllib.request

BASE = os.environ.get("JELLYFIN_URL", "http://localhost:8096")
USER = os.environ.get("JELLYFIN_USER", "dev")
PASS = os.environ.get("JELLYFIN_PASS", "dev")
CLIENT = 'Client="jelly-gpui-dev", Device="setup-script", DeviceId="jelly-gpui-dev-setup", Version="0.1.0"'

LIBRARIES = [
    ("Movies", "movies", "/media/Movies", ["Movie"]),
    ("Shows", "tvshows", "/media/Shows", ["Series", "Season", "Episode"]),
]


def request(path, method="GET", body=None, token=None):
    auth = f"MediaBrowser {CLIENT}" + (f', Token="{token}"' if token else "")
    req = urllib.request.Request(
        BASE + path,
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json", "Authorization": auth},
    )
    with urllib.request.urlopen(req) as resp:
        data = resp.read()
    return json.loads(data) if data else None


def wait_for_server():
    # While starting up, the server answers with plain text ("Jellyfin Server is loading")
    # or with a partial object, so wait until the field we need is present.
    print(f"waiting for {BASE} ...")
    while True:
        try:
            info = request("/System/Info/Public")
            if isinstance(info, dict) and "StartupWizardCompleted" in info:
                return info
        except (urllib.error.URLError, json.JSONDecodeError, ConnectionError):
            pass
        time.sleep(2)


def type_options(item_type):
    # No remote metadata or image fetchers, so the synthetic titles are never matched to real
    # TMDb/TVDb entries. Local artwork still applies; episodes get frame grabs.
    return {
        "Type": item_type,
        "MetadataFetchers": [],
        "MetadataFetcherOrder": [],
        "ImageFetchers": ["Screen Grabber"] if item_type == "Episode" else [],
        "ImageFetcherOrder": [],
        "ImageOptions": [],
    }


def item_count(token):
    q = "/Items?Recursive=true&IncludeItemTypes=Movie,Series,Episode&Limit=0"
    return request(q, token=token)["TotalRecordCount"]


def main():
    info = wait_for_server()

    if not info["StartupWizardCompleted"]:
        print("running startup wizard")
        # The initial user is only created by this GET (obsolete in the spec, but POST 404s without it).
        request("/Startup/User")
        request("/Startup/User", "POST", {"Name": USER, "Password": PASS})
        request("/Startup/Complete", "POST")

    token = request("/Users/AuthenticateByName", "POST", {"Username": USER, "Pw": PASS})["AccessToken"]

    existing = {f["Name"] for f in request("/Library/VirtualFolders", token=token)}
    for name, collection_type, path, _ in LIBRARIES:
        if name not in existing:
            print(f"adding library {name}")
            # Options sent on create are ignored, so they are applied below via the update endpoint.
            request(
                f"/Library/VirtualFolders?name={name}&collectionType={collection_type}"
                f"&paths={path}&refreshLibrary=false",
                "POST",
                {"LibraryOptions": {}},
                token,
            )

    folders = {f["Name"]: f for f in request("/Library/VirtualFolders", token=token)}
    for name, _, _, types in LIBRARIES:
        options = folders[name]["LibraryOptions"]
        options["TypeOptions"] = [type_options(t) for t in types]
        request("/Library/VirtualFolders/LibraryOptions", "POST", {"Id": folders[name]["ItemId"], "LibraryOptions": options}, token)

    # The scan started by adding a library races the folder registration, so refresh explicitly
    # and wait until the item count settles.
    print("scanning libraries")
    request("/Library/Refresh", "POST", token=token)
    last, stable = -1, 0
    while stable < 3:
        time.sleep(2)
        count = item_count(token)
        stable = stable + 1 if count == last and count > 0 else 0
        last = count

    legacy = request("/System/Configuration", token=token)["EnableLegacyAuthorization"]
    print(f"server ready: {BASE}  user={USER} pass={PASS}  items={last}  EnableLegacyAuthorization={legacy}")


if __name__ == "__main__":
    main()
