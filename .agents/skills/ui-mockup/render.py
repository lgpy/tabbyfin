#!/usr/bin/env python3
"""Render a disposable UI mockup file and open the results as a page.

    python3 -I .agents/skills/ui-mockup/render.py <mockup.rs> [--only <group>[,<group>…]] [--no-open]
    python3 -I .agents/skills/ui-mockup/render.py <mockup.rs> --zoom <name>[.light]:x,y,w,h

The file lives outside the repo (see `template.rs`); it is compiled into the app's UI tests under
the `mockup` feature and its `mockup_*` tests run. Output goes to `target/mockups/<file stem>/`:
a PNG and a JSON sidecar per mockup and theme (`ui_tests/mockups.rs` writes them), the previous
run's PNGs as `*.prev.png` where they changed, and `index.html`.

What it prints is meant for an agent and kept short: compile errors and the mockup file's own
warnings (one line each), failed tests' panics, which renders are new, changed or unchanged since
the last run, and `preview.png`: the new and changed ones on one sheet, sized to cost one image
read. `--only` renders just those groups (`welcome` for `welcome.now` and `welcome.cozy`) and
keeps the others' last renders on the page. `--zoom` re-renders nothing: it draws a region of an existing render, given in the
mockup's points, at full resolution into `zoom.png`.

The page is built from the mockup file itself:
  - its leading `//` comment block is the intro (a little markdown: paragraphs, `- ` lists,
    **bold**, `code`);
  - a `// ## Title` comment block names and introduces the section of the next mockup's group;
  - a test's `///` doc comment captions its mockup (first line the title, the rest below it);
Mockups are grouped by their name's part before the first dot (`welcome.now`, `welcome.cozy`);
a group of two gets side-by-side, swipe and flip comparison. The page follows the artifact
contract, so the folder can be published as it is.
"""

import html
import json
import os
import re
import shutil
import subprocess
import sys
import time
import webbrowser
from urllib.parse import quote
from urllib.request import urlopen
from pathlib import Path

root = Path(__file__).resolve().parents[3]
TODAY = {"now", "current", "before", "today"}


def main():
    argv = sys.argv[1:]
    zoom, only = option(argv, "--zoom"), option(argv, "--only")
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 1 or not Path(args[0]).is_file():
        sys.exit(__doc__)
    src = Path(args[0]).resolve()
    out = root / "target" / "mockups" / src.stem
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TABBYFIN_MOCKUP_DIR=str(out), TABBYFIN_MOCKUP_SRC=str(src))

    if zoom:
        return zoom_into(out, env, zoom)

    filters = ["ui_tests::scratch"]
    groups = None
    if only:
        groups = set(only.split(","))
        tests = [(fn, name) for fn, name in test_names(src.read_text()) if name and group_of(name) in groups]
        if not tests:
            known = sorted({group_of(name) for _, name in test_names(src.read_text()) if name})
            sys.exit(f"no mockups in {only}; the groups are: {', '.join(known)}")
        filters = ["--", "--exact", *(f"ui_tests::scratch::{fn}" for fn, _ in tests)]

    def rendering(stem):
        return groups is None or group_of(stem) in groups

    # Keep the last run's PNGs to flip against; the sidecars say what this run made.
    before = set()
    for png in out.glob("*.png"):
        stem = png.name[: -len(".png")]
        if not stem.endswith(".prev") and stem not in ("preview", "zoom") and rendering(stem):
            before.add(stem)
            png.replace(png.with_name(stem + ".prev.png"))
    for sidecar in out.glob("*.json"):
        if rendering(sidecar.name):
            sidecar.unlink()

    if not cargo(filters, env, src):
        sys.exit("mockup render failed")

    renders = [json.loads(p.read_text()) for p in sorted(out.glob("*.json"))]
    if not renders:
        sys.exit("no mockups rendered: the file needs #[test] fns that call mockup() or Mockup::show")
    status = {}
    for r in renders:
        stem = f"{r['name']}.{r['theme']}"
        prev = out / f"{stem}.prev.png"
        if not rendering(stem):
            r["prev"] = prev.exists()
            continue
        if prev.exists() and prev.read_bytes() == (out / f"{stem}.png").read_bytes():
            prev.unlink()
            status[stem] = "unchanged"
        else:
            status[stem] = "changed" if stem in before else "new"
        r["prev"] = prev.exists()
    for kind in ("new", "changed", "unchanged"):
        stems = [s for s, k in status.items() if k == kind]
        if stems:
            print(f"{kind}: {', '.join(stems)}")

    notes = parse(src.read_text())
    index = out / "index.html"
    index.write_text(build(src.stem, renders, notes))

    look = [s for s, k in status.items() if k != "unchanged"]
    sheet = out / "preview.png"
    sheet.unlink(missing_ok=True)
    if look:
        tiles = [{"path": str(out / f"{s}.png"), "label": f"{s} ({status[s]})", "crop": None} for s in look]
        if preview(out, env, src, tiles, sheet):
            print(f"look at: {sheet}")
    else:
        print("nothing changed since the last run")
    url = link(index)
    print(f"page: {url}")
    if "--no-open" not in sys.argv:
        # WSL: the Windows browser, since the Linux side has none.
        if shutil.which("wslview"):
            subprocess.run(["wslview", url])
        else:
            webbrowser.open(url)


# The local server for `target/mockups` (see `link`), and the file that tells it's ours.
PORT = 8737
MARKER = ".tabbyfin-mockups"
SERVER = """
import functools, http.server, sys
class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        # A re-render keeps its file names; never show the last one's images.
        self.send_header("Cache-Control", "no-store")
        super().end_headers()
    def log_message(self, *args):
        pass
handler = functools.partial(Handler, directory=sys.argv[1])
http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), handler).serve_forever()
"""


def link(path):
    """A URL for `path` that opens from a click in Claude Code: `http://localhost`, served from
    `target/mockups` by a small server started on the first run and reused after. Terminals
    (Windows Terminal among them) don't open `file://` links on a click, and on WSL a Linux path
    means nothing to the Windows browser, while WSL forwards `localhost` to it. Falls back to a
    `file://` URL if the port is taken by something else."""
    mockups = root / "target" / "mockups"
    (mockups / MARKER).write_text(MARKER)
    relative = quote(path.relative_to(mockups).as_posix())
    base = f"http://localhost:{PORT}"
    if not serving(base):
        subprocess.Popen(
            [sys.executable, "-I", "-c", SERVER, str(mockups), str(PORT)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        for _ in range(50):
            if serving(base):
                break
            time.sleep(0.05)
        else:
            return path.as_uri()
    return f"{base}/{relative}"


def serving(base):
    """Whether our server answers at `base` (and not something else on the port)."""
    try:
        with urlopen(f"{base}/{MARKER}", timeout=0.5) as answer:
            return answer.read().decode() == MARKER
    except OSError:
        return False


def zoom_into(out, env, spec):
    """Draws a region of one render, `<name>[.dark|.light]:x,y,w,h` in points, into `zoom.png`."""
    m = re.fullmatch(r"(.+?):(\d+(?:\.\d+)?),(\d+(?:\.\d+)?),(\d+(?:\.\d+)?),(\d+(?:\.\d+)?)", spec)
    if not m:
        sys.exit("--zoom takes <name>[.dark|.light]:x,y,w,h, the region in the mockup's points")
    stem = m.group(1)
    if not stem.endswith((".dark", ".light")):
        stem += ".dark"
    png = out / f"{stem}.png"
    if not png.exists():
        known = sorted(p.name[: -len(".png")] for p in out.glob("*.png") if p.name.count(".") >= 2 and ".prev." not in p.name)
        sys.exit(f"no render {stem}; there are: {', '.join(known)}")
    crop = [float(m.group(i)) for i in range(2, 6)]
    target = out / "zoom.png"
    tile = {"path": str(png), "label": f"{stem} at {','.join(f'{c:g}' for c in crop)}", "crop": crop}
    if preview(out, env, Path(env["TABBYFIN_MOCKUP_SRC"]), [tile], target):
        print(f"look at: {target}")


def preview(out, env, src, tiles, target):
    """Draws `tiles` onto one sheet at `target` (`ui_tests::mockups::preview`)."""
    listing = out / "preview.json.in"
    listing.write_text(json.dumps({"out": str(target), "tiles": tiles}))
    ok = cargo(["ui_tests::mockups::preview"], dict(env, TABBYFIN_MOCKUP_PREVIEW=str(listing)), src)
    listing.unlink()
    return ok


def option(argv, flag):
    """Removes `flag` and its value from `argv`, returning the value."""
    if flag not in argv:
        return None
    at = argv.index(flag)
    if at + 1 >= len(argv):
        sys.exit(__doc__)
    argv.pop(at)
    return argv.pop(at)


def group_of(name):
    return name.split(".", 1)[0]


def test_names(source):
    """Each test's fn name and the mockup it renders: the first string literal in its body
    (`mockup("…"`, `Mockup::new("…")` or a helper's argument)."""
    tests = []
    for body in re.split(r"(?m)^\s*#\[test\]", source)[1:]:
        fn = re.search(r"fn\s+(\w+)", body)
        name = re.search(r'"([^"\\]+)"', body)
        if fn:
            tests.append((fn.group(1), name.group(1) if name else None))
    return tests


def cargo(filters, env, src):
    """Runs the matching tests, printing only what needs acting on. True if they passed."""
    cmd = ["cargo", "test", "-p", "tabbyfin", "--features", "mockup", "--message-format", "short", *filters]
    run = subprocess.run(cmd, cwd=root, env=env, capture_output=True, text=True)
    lines = (run.stderr + "\n" + run.stdout).splitlines()
    shown = []
    for line in lines:
        diagnostic = re.match(r"^\S+:\d+:\d+: (error|warning)", line)
        # The mockup file's own warnings, and every error; warnings elsewhere are the app's.
        if diagnostic and (diagnostic.group(1) == "error" or line.startswith(str(src))):
            shown.append(line)
        elif re.match(r"^error(\[E\d+\])?: ", line) and not re.search(r"could not compile|test failed", line):
            shown.append(line)
    # A failed test's output: from `---- name stdout ----` to the next one or the failures list.
    block = None
    for line in run.stdout.splitlines() + ["failures:"]:
        if (line.startswith("---- ") and line.endswith(" ----")) or line == "failures:":
            if block:
                shown.extend(block[:20])
            block = [line] if line.startswith("---- ") else None
        elif block is not None and line.strip() and not line.startswith("note: run with"):
            block.append(line)
    if run.returncode != 0 and not shown:
        shown = lines[-30:]
    if shown:
        print("\n".join(shown))
    return run.returncode == 0


def parse(source):
    """The intro, section notes and captions written as comments in the mockup file."""
    lines = source.splitlines()
    intro, i = [], 0
    while i < len(lines) and lines[i].lstrip().startswith("//") and not lines[i].lstrip().startswith("///"):
        intro.append(re.sub(r"^\s*// ?", "", lines[i]))
        i += 1
    if any("python3 -I .agents/skills/ui-mockup/render.py" in l for l in intro):
        intro = []  # the template's own header, left in

    sections, captions = {}, {}
    pending_section, doc = None, []
    names = [name for _, name in test_names(source)]

    test_index = 0
    block = []
    for line in lines[i:]:
        s = line.strip()
        if s.startswith("///"):
            doc.append(re.sub(r"^/// ?", "", s))
        elif s.startswith("//"):
            block.append(re.sub(r"^// ?", "", s))
            continue
        elif s.startswith("#[test]"):
            name = names[test_index] if test_index < len(names) else None
            test_index += 1
            if name:
                group = name.split(".", 1)[0]
                if doc:
                    captions[name] = doc
                if pending_section and group not in sections:
                    sections[group] = pending_section
            pending_section, doc = None, []
        if block and block[0].startswith("## "):
            pending_section = block
        block = []
    return {"intro": intro, "sections": sections, "captions": captions}


def markdown(lines):
    """Paragraphs and `- ` lists, with **bold** and `code`."""
    out, para, items = [], [], []

    def inline(text):
        text = html.escape(text)
        text = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", text)
        return re.sub(r"`(.+?)`", r"<code>\1</code>", text)

    def flush():
        if para:
            out.append(f"<p>{inline(' '.join(para))}</p>")
            para.clear()
        if items:
            out.append("<ul>" + "".join(f"<li>{inline(i)}</li>" for i in items) + "</ul>")
            items.clear()

    for line in lines:
        if not line.strip():
            flush()
        elif line.lstrip().startswith("- "):
            if para:
                flush()
            items.append(line.lstrip()[2:])
        elif items and line.startswith("  "):
            items[-1] += " " + line.strip()
        else:
            if items:
                flush()
            para.append(line.strip())
    flush()
    return "\n".join(out)


def build(title, renders, notes):
    groups = {}
    for r in renders:
        name = r["name"]
        group, _, variant = name.partition(".")
        variants = groups.setdefault(group, {})
        variants.setdefault(variant or name, {"name": name, "themes": {}})["themes"][r["theme"]] = r

    sections, all_themes, pairs = [], set(), False
    for group, variants in groups.items():
        # Today's app first, so a pair reads before → after.
        ordered = sorted(variants.items(), key=lambda kv: kv[0] not in TODAY)
        sizes = {tuple(r["size"]) for _, v in ordered for r in v["themes"].values()}
        if len(sizes) > 1:
            print(f"warning: group '{group}' mixes sizes {sorted(sizes)}; compare like with like", file=sys.stderr)
        themes = sorted({t for _, v in ordered for t in v["themes"]}, key=["dark", "light"].index)
        section_notes = notes["sections"].get(group)
        heading = section_notes[0][3:] if section_notes else group.replace("_", " ").capitalize()
        body = markdown(section_notes[1:]) if section_notes else ""

        figures = "\n".join(figure(variant, v, notes["captions"].get(v["name"])) for variant, v in ordered)
        all_themes.update(themes)
        compare = ""
        if len(ordered) == 2 and len(sizes) == 1:
            (a_label, a), (b_label, b) = ordered
            w, h = next(iter(sizes))
            pairs = True
            compare = (
                f'<div class="stage" hidden data-a="{html.escape(a["name"])}" data-b="{html.escape(b["name"])}" '
                f'data-a-label="{html.escape(tag_text(a_label))}" data-b-label="{html.escape(tag_text(b_label))}" '
                f'style="max-width:{w:g}px;aspect-ratio:{w:g}/{h:g}">'
                f'<img class="a" alt=""><img class="b" alt=""><span class="which"></span>'
                f'<input class="swipe" type="range" min="0" max="100" value="50" aria-label="Swipe position"></div>'
            )
        sections.append(
            f'<section data-group="{html.escape(group)}" data-themes="{" ".join(themes)}" '
            f'data-theme="{themes[0]}" data-mode="side">'
            f"<h2>{html.escape(heading)}</h2>{body}"
            f'<div class="figures n{len(ordered)}">{figures}</div>{compare}</section>'
        )

    intro = markdown(notes["intro"]) if notes["intro"] else ""
    pretty = title.replace("_", " ").replace("-", " ").capitalize()
    return PAGE.format(
        title=html.escape(pretty),
        intro=intro,
        sections="\n".join(sections),
        fab=fab(pairs, sorted(all_themes, key=["dark", "light"].index)),
    )


def fab(pairs, themes):
    """The view options, in a button at the bottom right that opens on hover (or a tap): how
    pairs are compared and which theme every mockup shows. Nothing when there's nothing to pick."""
    rows = []
    if pairs:
        modes = [("side", "Side by side", "S"), ("swipe", "Swipe", "W"), ("flip", "Flip", "F")]
        rows.append(segmented("mode", modes, "Compare"))
    if len(themes) > 1:
        rows.append(segmented("theme", [(t, t.capitalize(), "T") for t in themes], "Theme"))
    if not rows:
        return ""
    icon = (
        '<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path fill="currentColor" '
        'd="M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z"/></svg>'
    )
    return (
        f'<aside class="fab"><div class="fab-panel" role="group" aria-label="View options">{"".join(rows)}'
        f'<p class="keys">Keys: <kbd>S</kbd> <kbd>W</kbd> <kbd>F</kbd> compare, <kbd>T</kbd> theme</p></div>'
        f'<button type="button" class="fab-button" aria-label="View options" aria-expanded="false">'
        f'{icon}<span class="fab-state"></span></button></aside>'
    )


def tag_text(variant):
    return "Today" if variant in TODAY else variant.replace("_", " ").capitalize()


def figure(variant, v, caption):
    any_render = next(iter(v["themes"].values()))
    w, h = any_render["size"]
    srcs = {t: f"{v['name']}.{t}.png" for t in v["themes"]}
    prevs = {t: f"{v['name']}.{t}.prev.png" for t, r in v["themes"].items() if r["prev"]}
    data = " ".join(f'data-{t}="{html.escape(s)}"' for t, s in srcs.items())
    data += " " + " ".join(f'data-prev-{t}="{html.escape(s)}"' for t, s in prevs.items())
    first = srcs.get("dark") or next(iter(srcs.values()))
    tag_class = "tag was" if variant in TODAY else "tag"
    title, rest = "", ""
    if caption:
        title = f"<b>{html.escape(caption[0])}</b>"
        rest = markdown(caption[1:])
    prev_button = '<button type="button" class="prev" aria-pressed="false">Previous render</button>' if prevs else ""
    return (
        f'<figure data-name="{html.escape(v["name"])}" {data}>'
        f'<img src="{html.escape(first)}" width="{w:g}" height="{h:g}" alt="{html.escape(v["name"])}">'
        f'<figcaption><div class="row"><span class="{tag_class}">{html.escape(tag_text(variant))}</span>{prev_button}</div>'
        f"{title}{rest}</figcaption></figure>"
    )


def segmented(kind, options, label):
    buttons = "".join(
        f'<button type="button" data-{kind}="{value}" aria-pressed="{"true" if i == 0 else "false"}" '
        f'aria-keyshortcuts="{key}">{text}</button>'
        for i, (value, text, key) in enumerate(options)
    )
    return f'<div class="segmented" role="group" aria-label="{label}"><span>{label}</span>{buttons}</div>'


PAGE = """<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{
  --bg: #121117; --surface: #1c1a24; --edge: #2e2b3a; --text: #ece9f3; --muted: #a29db0;
  --accent: #c396e7; --on-accent: #1d1230;
  color-scheme: dark;
}}
@media (prefers-color-scheme: light) {{
  :root:not([data-theme="dark"]) {{
    --bg: #f7f5fb; --surface: #ffffff; --edge: #e2ddec; --text: #1f1b29; --muted: #625c70;
    --accent: #7b4fb0; --on-accent: #ffffff;
    color-scheme: light;
  }}
}}
:root[data-theme="light"] {{
  --bg: #f7f5fb; --surface: #ffffff; --edge: #e2ddec; --text: #1f1b29; --muted: #625c70;
  --accent: #7b4fb0; --on-accent: #ffffff;
  color-scheme: light;
}}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: var(--bg); color: var(--text); font: 15px/1.55 system-ui, sans-serif; }}
main {{ max-width: 2100px; margin: 0 auto; padding: 40px 16px 80px; display: grid; gap: 56px; }}
.intro {{ max-width: 70ch; display: grid; gap: 10px; }}
h1 {{ font-size: 1.9rem; font-weight: 500; margin: 0; }}
h2 {{ font-size: 1.3rem; font-weight: 500; margin: 0; }}
p, ul, ol {{ margin: 0; }}
section > p, section > ul {{ max-width: 70ch; color: var(--muted); }}
section {{ display: grid; gap: 14px; }}
code {{ font-size: .9em; background: var(--surface); border: 1px solid var(--edge); border-radius: 4px; padding: 0 4px; }}
.fab {{ position: fixed; right: 20px; bottom: 20px; z-index: 10; display: flex; flex-direction: column;
  align-items: flex-end; gap: 10px; }}
.fab-button {{ display: flex; align-items: center; gap: 8px; padding: 0 16px 0 14px; height: 48px; border-radius: 24px;
  background: var(--accent); color: var(--on-accent); box-shadow: 0 6px 20px rgb(0 0 0 / .35); font-weight: 600; }}
.fab-panel {{ display: grid; gap: 8px; padding: 12px; border-radius: 16px; background: var(--surface);
  border: 1px solid var(--edge); box-shadow: 0 10px 32px rgb(0 0 0 / .3);
  opacity: 0; transform: translateY(8px) scale(.97); transform-origin: bottom right; pointer-events: none;
  transition: opacity .15s, transform .15s; }}
.fab:hover .fab-panel, .fab:focus-within .fab-panel, .fab.open .fab-panel {{
  opacity: 1; transform: none; pointer-events: auto; }}
@media (prefers-reduced-motion: reduce) {{ .fab-panel {{ transition: none; }} }}
.fab .keys {{ color: var(--muted); font-size: .75rem; padding: 2px 4px 0; }}
@media (hover: none) {{ .fab .keys {{ display: none; }} }}
kbd {{ font: inherit; font-size: .7rem; border: 1px solid var(--edge); border-radius: 4px; padding: 0 4px; }}
main {{ padding-bottom: 120px; }}
.segmented {{ display: flex; align-items: center; gap: 2px; padding: 3px; border-radius: 10px;
  background: var(--bg); border: 1px solid var(--edge); }}
.segmented span {{ color: var(--muted); font-size: .8rem; padding: 0 8px; min-width: 66px; }}
button {{ font: inherit; font-size: .85rem; color: var(--text); background: transparent; border: 0;
  border-radius: 7px; padding: 4px 12px; cursor: pointer; }}
button[aria-pressed="true"] {{ background: var(--accent); color: var(--on-accent); }}
.figures {{ display: grid; gap: 20px; align-items: start; }}
.figures.n2 {{ grid-template-columns: repeat(2, minmax(0, 1fr)); }}
.figures.n3 {{ grid-template-columns: repeat(3, minmax(0, 1fr)); }}
@media (max-width: 900px) {{ .figures.n2, .figures.n3 {{ grid-template-columns: 1fr; }} }}
section:not([data-mode="side"]) .figures {{ display: none; }}
figure {{ margin: 0; display: grid; gap: 8px; min-width: 0; }}
img {{ display: block; max-width: 100%; height: auto; }}
figure img {{ border-radius: 10px; border: 1px solid var(--edge); }}
figcaption {{ display: grid; gap: 4px; justify-items: start; }}
figcaption p, figcaption ul {{ color: var(--muted); }}
figcaption .row {{ display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }}
.tag {{ font-size: .75rem; font-weight: 600; padding: 1px 9px; border-radius: 999px;
  background: var(--accent); color: var(--on-accent); }}
.tag.was {{ background: transparent; color: var(--muted); border: 1px solid var(--edge); }}
button.prev {{ border: 1px solid var(--edge); padding: 1px 9px; font-size: .75rem; }}
.stage {{ position: relative; width: 100%; border-radius: 10px; overflow: hidden; border: 1px solid var(--edge);
  user-select: none; }}
.stage img {{ position: absolute; inset: 0; width: 100%; height: 100%; max-width: none; }}
.stage .which {{ position: absolute; top: 10px; left: 10px; font-size: .8rem; font-weight: 600; padding: 2px 10px;
  border-radius: 999px; background: rgb(0 0 0 / .65); color: #fff; pointer-events: none; }}
.stage .swipe {{ position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: ew-resize; }}
.stage::after {{ content: ""; position: absolute; top: 0; bottom: 0; left: var(--at, 50%); width: 2px;
  background: var(--accent); pointer-events: none; }}
section[data-mode="flip"] .stage::after, section[data-mode="flip"] .swipe {{ display: none; }}
section[data-mode="flip"] .stage {{ cursor: pointer; }}
</style>
<main>
<header class="intro"><h1>{title}</h1>{intro}</header>
{sections}
</main>
{fab}
<script>
// One view for the whole page, set from the floating options: how pairs are compared, and
// which theme every mockup shows (a section without it keeps its own).
const view = {{ mode: "side", theme: null }};
const sections = [...document.querySelectorAll("section[data-group]")].map((section) => {{
  const stage = section.querySelector(".stage");
  const src = (name) => {{
    const fig = section.querySelector(`figure[data-name="${{CSS.escape(name)}}"]`);
    return fig.dataset[section.dataset.theme];
  }};
  const draw = () => {{
    const themes = section.dataset.themes.split(" ");
    if (view.theme && themes.includes(view.theme)) section.dataset.theme = view.theme;
    for (const fig of section.querySelectorAll("figure")) {{
      const prev = fig.querySelector("button.prev")?.getAttribute("aria-pressed") === "true";
      const t = section.dataset.theme;
      const key = prev ? "prev" + t[0].toUpperCase() + t.slice(1) : t;
      if (fig.dataset[key]) fig.querySelector("img").src = fig.dataset[key];
    }}
    if (!stage) return;
    section.dataset.mode = view.mode;
    stage.hidden = view.mode === "side";
    stage.querySelector(".a").src = src(stage.dataset.a);
    stage.querySelector(".b").src = src(stage.dataset.b);
    const swipe = view.mode === "swipe";
    const showB = stage.dataset.flipped === "1";
    const at = stage.querySelector(".swipe").value;
    stage.style.setProperty("--at", at + "%");
    stage.querySelector(".b").style.clipPath = swipe ? `inset(0 0 0 ${{at}}%)` : "none";
    stage.querySelector(".b").style.visibility = swipe || showB ? "visible" : "hidden";
    stage.querySelector(".which").textContent = swipe
      ? `${{stage.dataset.aLabel}} ◂ ▸ ${{stage.dataset.bLabel}}`
      : `${{showB ? stage.dataset.bLabel : stage.dataset.aLabel}} (click to flip)`;
  }};
  for (const button of section.querySelectorAll("button.prev")) {{
    button.addEventListener("click", () => {{
      button.setAttribute("aria-pressed", button.getAttribute("aria-pressed") !== "true");
      button.textContent = button.getAttribute("aria-pressed") === "true" ? "Showing previous render" : "Previous render";
      draw();
    }});
  }}
  if (stage) {{
    stage.querySelector(".swipe").addEventListener("input", draw);
    stage.addEventListener("click", () => {{
      if (view.mode !== "flip") return;
      stage.dataset.flipped = stage.dataset.flipped === "1" ? "0" : "1";
      draw();
    }});
  }}
  return {{ stage, draw }};
}});

const fab = document.querySelector(".fab");
const set = (kind, value) => {{
  view[kind] = value;
  for (const b of document.querySelectorAll(`.fab button[data-${{kind}}]`)) {{
    b.setAttribute("aria-pressed", b.dataset[kind] === value);
  }}
  const state = document.querySelector(".fab-state");
  if (state) {{
    const label = (kind) => document.querySelector(`.fab button[data-${{kind}}][aria-pressed="true"]`)?.textContent;
    state.textContent = [label("mode"), label("theme")].filter(Boolean).join(" · ");
  }}
  for (const s of sections) s.draw();
}};
if (fab) {{
  fab.addEventListener("click", (e) => {{
    const button = e.target.closest("button");
    if (!button) return;
    if (button.classList.contains("fab-button")) {{
      // Hover opens it; a tap (no hover on touch screens) toggles it.
      fab.classList.toggle("open");
      button.setAttribute("aria-expanded", fab.classList.contains("open"));
    }} else if (button.dataset.mode) set("mode", button.dataset.mode);
    else if (button.dataset.theme) set("theme", button.dataset.theme);
  }});
  document.addEventListener("click", (e) => {{
    if (!fab.contains(e.target)) fab.classList.remove("open");
  }});
}}
document.addEventListener("keydown", (e) => {{
  if (e.ctrlKey || e.metaKey || e.altKey || e.target.closest("input, textarea")) return;
  const modes = {{ s: "side", w: "swipe", f: "flip" }};
  const key = e.key.toLowerCase();
  if (modes[key] && document.querySelector(".fab button[data-mode]")) set("mode", modes[key]);
  else if (key === "t") {{
    const themes = [...document.querySelectorAll(".fab button[data-theme]")].map((b) => b.dataset.theme);
    if (themes.length) set("theme", themes[(themes.indexOf(view.theme) + 1) % themes.length]);
  }} else if (e.key === " " && view.mode === "flip") {{
    const hovered = sections.find((s) => s.stage?.matches(":hover"));
    if (hovered) {{
      e.preventDefault();
      hovered.stage.click();
    }}
  }}
}});
const first = document.querySelector(".fab button[data-theme]");
set("theme", first ? first.dataset.theme : null);
set("mode", "side");
</script>
"""

main()
