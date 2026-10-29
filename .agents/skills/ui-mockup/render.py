#!/usr/bin/env python3
"""Render the UI mockups (`crates/app/src/ui_tests/mockups.rs`) and open them as an HTML gallery.

    python3 -I .agents/skills/ui-mockup/render.py [filter] [--no-open]

`filter` picks mockup tests by name (default: all `mockup_*`). Output goes to
`target/mockups/`: one PNG per mockup plus `index.html`, newest first. The gallery lists every
PNG there, so earlier renders stay for comparison; delete the folder to start clean.
"""

import html
import os
import shutil
import subprocess
import sys
import time
import webbrowser
from pathlib import Path

root = Path(__file__).resolve().parents[3]
out = root / "target" / "mockups"


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TABBYFIN_MOCKUP_DIR=str(out))
    cmd = ["cargo", "test", "-p", "tabbyfin", args[0] if args else "mockup_", "--", "--ignored"]
    started = time.time()
    if subprocess.run(cmd, cwd=root, env=env).returncode != 0:
        sys.exit("mockup render failed")

    pngs = sorted(out.glob("*.png"), key=lambda p: p.stat().st_mtime, reverse=True)
    cards = "\n".join(
        f'<figure><figcaption>{html.escape(p.stem)}{"" if p.stat().st_mtime >= started else " (older)"}</figcaption>'
        f'<a href="{html.escape(p.name)}"><img src="{html.escape(p.name)}?{int(p.stat().st_mtime)}"></a></figure>'
        for p in pngs
    )
    (out / "index.html").write_text(
        "<!doctype html><meta charset=utf-8><title>tabbyfin mockups</title>"
        "<style>body{font:14px system-ui;margin:24px;background:#1b1b1b;color:#ddd}"
        "main{display:flex;flex-wrap:wrap;gap:24px}figure{margin:0}"
        "figcaption{margin-bottom:6px}img{max-width:100%;border:1px solid #444;background:#fff}</style>"
        f"<main>{cards}</main>"
    )
    index = out / "index.html"
    print(f"{len(pngs)} mockup(s): {index}")
    if "--no-open" not in sys.argv:
        # WSL: the Windows browser, since the Linux side has none.
        if shutil.which("wslview"):
            subprocess.run(["wslview", str(index)])
        else:
            webbrowser.open(index.as_uri())


main()
