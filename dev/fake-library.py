#!/usr/bin/env python3
"""A big library for performance checks: a reverse proxy in front of the dev server that adds
a synthetic movie library of N items with generated posters, and logs the poster requests.

    dev/fake-library.py [port] [upstream] [items] [poster delay ms]
    dev/fake-library.py 8098 http://localhost:8096 10000 60

Point a copy of the config at http://localhost:8098 and open the library with
TABBYFIN_ROUTE=library:ffffffffffffffffffffffffffffffff:movies. Everything else (sign-in,
Home, playback) goes through to the real server. Each poster request is logged to
dev/.data/fake-library/images.log with how many were in flight; the delay stands in for the
server resizing the image. Posters are made with ffmpeg on the first run.
"""

import hashlib
import json
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8098
UPSTREAM = sys.argv[2] if len(sys.argv) > 2 else "http://localhost:8096"
COUNT = int(sys.argv[3]) if len(sys.argv) > 3 else 10000
DELAY = (int(sys.argv[4]) if len(sys.argv) > 4 else 60) / 1000
FAKE = "f" * 32
DATA = Path(__file__).resolve().parent / ".data" / "fake-library"


def make_posters():
    """64 plain posters in different colours, numbered."""
    (DATA / "posters").mkdir(parents=True, exist_ok=True)
    for i in range(64):
        path = DATA / "posters" / f"{i}.jpg"
        if path.exists():
            continue
        color = f"0x{i * 37 % 256:02x}{i * 91 % 256:02x}{i * 53 % 256:02x}"
        subprocess.run(
            ["ffmpeg", "-loglevel", "error", "-y", "-f", "lavfi", "-i", f"color=c={color}:size=400x600",
             "-frames:v", "1", "-vf", f"drawtext=text='{i}':fontcolor=white:fontsize=120:x=(w-tw)/2:y=(h-th)/2",
             "-q:v", "3", str(path)],
            check=True,
        )


make_posters()
POSTERS = [(DATA / "posters" / f"{i}.jpg").read_bytes() for i in range(64)]
LOG = open(DATA / "images.log", "w", buffering=1)

WORDS = ["Amber", "Blue", "Crimson", "Dawn", "Echo", "Frost", "Gold", "Harbor", "Iron", "Jade",
         "Kite", "Lunar", "Moss", "North", "Ocean", "Pine", "Quiet", "River", "Stone", "Tide",
         "Umber", "Vale", "Willow", "Xenon", "Yarrow", "Zephyr"]
NAMES = sorted((f"{WORDS[i % 26]} {WORDS[(i * 7) % 26]} {i:05d}" for i in range(COUNT)), key=str.lower)
IDS = [hashlib.md5(name.encode()).hexdigest() for name in NAMES]

lock = threading.Lock()
active = 0
peak = 0


def item(i):
    return {
        "Name": NAMES[i], "Id": IDS[i], "Type": "Movie", "IsFolder": False, "ServerId": "x",
        "ProductionYear": 1980 + i % 45, "ImageTags": {"Primary": "t" + IDS[i][:8]},
        "PrimaryImageAspectRatio": 0.6667,
        "UserData": {"Played": False, "IsFavorite": False, "PlaybackPositionTicks": 0, "PlayCount": 0},
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def send_json(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        q = {k.lower(): v[0] for k, v in urllib.parse.parse_qs(url.query).items()}
        parts = url.path.split("/")
        if url.path == "/Items" and q.get("parentid") == FAKE:
            names = range(COUNT)
            if "namelessthan" in q:
                bound = q["namelessthan"].lower()
                names = [i for i in names if NAMES[i].lower() < bound]
            names = list(names)
            if q.get("sortorder") == "Descending":
                names.reverse()
            start, limit = int(q.get("startindex", 0)), int(q.get("limit", 100))
            return self.send_json({"Items": [item(i) for i in names[start:start + limit]],
                                   "TotalRecordCount": len(names), "StartIndex": start})
        if url.path == "/Items/Filters" and q.get("parentid") == FAKE:
            return self.send_json({"Genres": [], "Tags": [], "OfficialRatings": [], "Years": []})
        if len(parts) >= 5 and parts[1] == "Items" and parts[3] == "Images" and parts[2] in IDS_SET:
            return self.poster(parts[2])
        self.forward()

    def poster(self, item_id):
        global active, peak
        with lock:
            active += 1
            peak = max(peak, active)
            now_active = active
        LOG.write(f"{time.time():.3f} start {item_id} active={now_active}\n")
        time.sleep(DELAY)
        body = POSTERS[int(item_id[:4], 16) % 64]
        try:
            self.send_response(200)
            self.send_header("Content-Type", "image/jpeg")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        finally:
            with lock:
                active -= 1
            LOG.write(f"{time.time():.3f} end {item_id}\n")

    def do_POST(self):
        self.forward()

    def do_DELETE(self):
        self.forward()

    def forward(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else None
        headers = {k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection")}
        req = urllib.request.Request(UPSTREAM + self.path, data=body, headers=headers, method=self.command)
        try:
            resp = urllib.request.urlopen(req)
            status, data, rheaders = resp.status, resp.read(), resp.headers
        except urllib.error.HTTPError as err:
            status, data, rheaders = err.code, err.read(), err.headers
        self.send_response(status)
        for k, v in rheaders.items():
            if k.lower() not in ("transfer-encoding", "connection", "content-length"):
                self.send_header(k, v)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


IDS_SET = set(IDS)
print(f"proxy on :{PORT} -> {UPSTREAM}, {COUNT} items, {DELAY * 1000:.0f} ms per poster", flush=True)
ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
