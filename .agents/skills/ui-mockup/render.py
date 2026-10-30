#!/usr/bin/env python3
"""Render a disposable UI mockup file and open the results as a page.

    python3 -I .agents/skills/ui-mockup/render.py <mockup.rs> [--only <group>[,<group>…]] [--open | --no-open]
    python3 -I .agents/skills/ui-mockup/render.py <mockup.rs> --zoom <name>[.light]:x,y,w,h

The file lives outside the repo (see `template.rs`); it is compiled into the app's UI tests under
the `mockup` feature and its `mockup_*` tests run. Output goes to `target/mockups/<file stem>/`:
a PNG and a JSON sidecar per mockup and theme (`ui_tests/mockups.rs` writes them), the previous
run's PNGs as `*.prev.png` where they changed, `index.html`, and `source.rs`, a copy of the mockup
file, since the file itself lives in a scratch dir a later session won't have.

The browser opens on a file's first run only (`--open` forces it, `--no-open` never): an open
page notices a re-render and reloads itself.

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
a group of two gets side-by-side, swipe and flip comparison. On the page the reader can pick a
variant per group and comment on each mockup, then copy it all as one Markdown reply (starting
`# Re:`) to paste back to the agent. The page follows the artifact contract, so the folder can be
published as it is.
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
# The page's layout, in CSS pixels, kept in step with STYLE: the gap between mockups in a row and
# the "or" between them, the gap beside a lone mockup's words, the side padding, the narrowest and widest that column
# of words gets, and the narrowest the page's column gets (for the intro's sake).
GAP, OR, GAP_WIDE, PAD, SIDE_MIN, SIDE_MAX, PAGE_MIN = 24, 40, 56, 24, 320, "34rem", 760


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
    first = not (out / "index.html").exists()
    shutil.copyfile(src, out / "source.rs")

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
    # In the file's order, so its sections read as written.
    order = [name for _, name in test_names(src.read_text()) if name]
    renders.sort(key=lambda r: order.index(r["name"]) if r["name"] in order else len(order))
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
    if "--open" in sys.argv or (first and "--no-open" not in sys.argv):
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
        elif not s.startswith("#["):
            doc = []  # a helper's doc comment, not a caption
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
    widths, solo_rules = [], []
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

        figures = '<div class="or" aria-hidden="true"><span>or</span></div>'.join(
            figure(variant, v, notes["captions"].get(v["name"])) for variant, v in ordered
        )
        # The row of mockups as drawn (up to three side by side), which sets the page's column.
        w = max(s[0] for s in sizes)
        n = len(ordered) if len(ordered) <= 3 else 1
        widths.append(n * w + (n - 1) * (2 * GAP + OR))
        solo = len(ordered) == 1
        if solo:
            # A lone mockup takes its words beside it once there's room for a column of text.
            at = w + GAP_WIDE + SIDE_MIN + 2 * PAD
            sel = f'section[data-group="{html.escape(group)}"]'
            solo_rules.append(
                f"@media (min-width: {at:g}px) {{ {sel} {{ grid-template-columns: minmax(0, {w:g}px) "
                f"minmax(0, {SIDE_MAX}); grid-template-rows: auto auto 1fr; }} "
                f"{sel} .shot {{ grid-column: 1; grid-row: 1 / span 3; }} "
                f"{sel} .head, {sel} figcaption {{ grid-column: 2; }} "
                f"{sel} figcaption {{ margin-top: 8px; padding-top: 16px; border-top: 1px solid var(--edge); }} }}"
            )
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
            f'<section data-group="{html.escape(group)}" data-heading="{html.escape(heading)}" '
            f'data-themes="{" ".join(themes)}" '
            f'data-theme="{themes[0]}" data-mode="side"{" class=\"solo\"" if solo else ""}>'
            f'<header class="head"><h2>{html.escape(heading)}</h2>{body}</header>'
            f'<div class="figures n{len(ordered)}">{figures}</div>{compare}</section>'
        )

    intro = markdown(notes["intro"]) if notes["intro"] else ""
    pretty = title.replace("_", " ").replace("-", " ").capitalize()
    page = max([*widths, PAGE_MIN])
    return PAGE.format(
        style=STYLE + "\n".join(solo_rules),
        page=f"{page:g}",
        script=SCRIPT,
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
    # The variant's name, unless the caption's title already says it ("Stacked" over "Stacked").
    same = caption and caption[0].strip().lower() == tag_text(variant).lower()
    tag = "" if same and variant not in TODAY else f'<span class="{tag_class}">{html.escape(tag_text(variant))}</span>'
    prev_button = '<button type="button" class="prev" aria-pressed="false">Previous render</button>' if prevs else ""
    # On the title's line, so a pair's captions line up whether or not each has a tag.
    row = f'<div class="row">{title}{tag}{prev_button}</div>' if title or tag or prev_button else ""
    return (
        f'<figure data-name="{html.escape(v["name"])}" style="max-width:{w:g}px" {data}>'
        f'<div class="shot"><img src="{html.escape(first)}" width="{w:g}" height="{h:g}" alt="{html.escape(v["name"])}"></div>'
        f"<figcaption>{row}{rest}</figcaption></figure>"
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
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Ubuntu:wght@400;500&display=swap">
<style>{style}</style>
<main style="--page: {page}px">
<header class="intro"><h1>{title}</h1>{intro}</header>
{sections}
</main>
{fab}
<aside class="reply" hidden aria-live="polite"><span class="count"></span>
<button type="button" class="reload" hidden>Reload</button>
<button type="button" class="copy">Copy reply</button><button type="button" class="clear">Clear</button>
<textarea class="fallback" readonly hidden aria-label="Reply to paste back"></textarea></aside>
<div class="note-pop" hidden role="dialog" aria-label="Comment">
<b class="note-title"></b>
<textarea rows="4" placeholder="What to change, keep or ask about" aria-label="Comment"></textarea>
<div class="note-foot"><span class="hint">Ctrl+Enter to close</span>
<button type="button" class="delete">Delete</button><button type="button" class="done">Done</button></div>
</div>
<script>{script}</script>
"""

STYLE = r"""
/* The app's cozy tokens (theme.rs): its plum backdrop, surfaces and lavender brand. */
:root {
  --bg: #0e0b16; --surface: #1a1526; --edge: #2c2440; --text: #f4f0fa; --muted: #a89fbb;
  --accent: #c396e7; --on-accent: #1d1230; --glow: rgb(195 150 231 / .3);
  color-scheme: dark;
}
@media (prefers-color-scheme: light) {
  :root:not([data-theme="dark"]) {
    --bg: #f8f5fc; --surface: #ffffff; --edge: #e8e0f4; --text: #1f182b; --muted: #605770;
    --accent: #7b4fb0; --on-accent: #ffffff; --glow: rgb(123 79 176 / .25);
    color-scheme: light;
  }
}
:root[data-theme="light"] {
  --bg: #f8f5fc; --surface: #ffffff; --edge: #e8e0f4; --text: #1f182b; --muted: #605770;
  --accent: #7b4fb0; --on-accent: #ffffff; --glow: rgb(123 79 176 / .25);
  color-scheme: light;
}
* { box-sizing: border-box; }
/* Ubuntu is what egui draws the mockups in, so the page's words read like the app's. */
body { margin: 0; background: var(--bg); color: var(--text);
  font: 16px/1.55 Ubuntu, system-ui, sans-serif; -webkit-font-smoothing: antialiased; }
/* One column, as wide as the widest row of mockups, that everything starts at. */
main { max-width: calc(var(--page) + 48px); margin: 0 auto; padding: 56px 24px 140px;
  display: grid; gap: 2.5rem; }
@media (max-width: 700px) { main { padding: 32px 16px 140px; gap: 48px; } }
/* Everything starts at the figures' left edge; prose keeps a readable measure. */
.intro { display: grid; gap: 12px; max-width: 68ch; }
h1 { font-size: 2.4rem; line-height: 1.1; font-weight: 500; letter-spacing: -.02em; margin: 0 0 4px; }
.intro p, .intro ul { font-size: 1.06rem; }
h2 { font-size: 1.45rem; line-height: 1.2; font-weight: 500; letter-spacing: -.01em; margin: 0; }
p, ul, ol { margin: 0; }
ul { padding-left: 1.2em; }
li + li { margin-top: 2px; }
b { font-weight: 500; }
/* A hairline over each section, so where one group ends and the next begins reads at a glance. */
section { display: grid; gap: 16px; padding-top: 2.5rem; border-top: 1px solid var(--edge); }
.head { display: grid; gap: 6px; max-width: 68ch; }
.head p, .head ul { color: var(--muted); }
code { font: .88em ui-monospace, monospace; background: var(--surface); border: 1px solid var(--edge);
  border-radius: 5px; padding: 1px 5px; }
:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.fab { position: fixed; right: 20px; bottom: 20px; z-index: 10; display: flex; flex-direction: column;
  align-items: flex-end; gap: 10px; }
.fab-button { display: flex; align-items: center; gap: 8px; padding: 0 16px 0 14px; height: 48px; border-radius: 24px;
  background: var(--accent); color: var(--on-accent); box-shadow: 0 6px 20px rgb(0 0 0 / .35); font-weight: 600; }
.fab-panel { display: grid; gap: 8px; padding: 12px; border-radius: 16px; background: var(--surface);
  border: 1px solid var(--edge); box-shadow: 0 10px 32px rgb(0 0 0 / .3);
  opacity: 0; transform: translateY(8px) scale(.97); transform-origin: bottom right; pointer-events: none;
  transition: opacity .15s, transform .15s; }
.fab:hover .fab-panel, .fab:focus-within .fab-panel, .fab.open .fab-panel {
  opacity: 1; transform: none; pointer-events: auto; }
@media (prefers-reduced-motion: reduce) { .fab-panel { transition: none; } }
.fab .keys { color: var(--muted); font-size: .75rem; padding: 2px 4px 0; }
@media (hover: none) { .fab .keys { display: none; } }
kbd { font: inherit; font-size: .7rem; border: 1px solid var(--edge); border-radius: 4px; padding: 0 4px; }
.segmented { display: flex; align-items: center; gap: 2px; padding: 3px; border-radius: 10px;
  background: var(--bg); border: 1px solid var(--edge); }
.segmented span { color: var(--muted); font-size: .8rem; padding: 0 8px; min-width: 66px; }
button { font: inherit; font-size: .85rem; color: var(--text); background: transparent; border: 0;
  border-radius: 7px; padding: 4px 12px; cursor: pointer; }
button[aria-pressed="true"] { background: var(--accent); color: var(--on-accent); }
.figures { display: grid; gap: 24px; align-items: start; justify-items: start; }
.figures.n2 { grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); }
.figures.n3 { grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto minmax(0, 1fr); }
/* Between variants to pick from: a rule with "or" on it, upright beside them, level across
   when they stack. */
.or { align-self: stretch; display: grid; grid-template-rows: 1fr auto 1fr; justify-items: center; gap: 8px;
  width: 40px; color: var(--muted); }
.or::before, .or::after { content: ""; width: 1px; background: var(--edge); }
.or span { font-size: .9rem; font-weight: 500; line-height: 1; padding: 6px 10px 7px; border-radius: 999px;
  border: 1px solid var(--edge); background: var(--surface); }
@media (max-width: 900px) {
  .figures.n2, .figures.n3 { grid-template-columns: 1fr; }
  .or { width: 100%; grid-template-rows: none; grid-template-columns: 1fr auto 1fr; align-items: center;
    justify-items: stretch; }
  .or::before, .or::after { width: auto; height: 1px; }
}
section:not([data-mode="side"]) .figures { display: none; }
figure { margin: 0; display: grid; gap: 10px; min-width: 0; width: 100%; }
img { display: block; max-width: 100%; height: auto; }
.shot { position: relative; }
.shot img { border-radius: 12px; border: 1px solid var(--edge); transition: opacity .2s, filter .2s; }
/* What's been decided shows while scrolling: the pick glows like the app's focus, and the
   variants it beat step back until they're pointed at. */
figure.picked .shot img { border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent), 0 16px 48px -8px var(--glow); }
section.decided figure:not(.picked) .shot img { opacity: .45; filter: saturate(.5); }
section.decided figure:not(.picked) .shot:hover img,
section.decided figure:not(.picked):focus-within .shot img { opacity: 1; filter: none; }
@media (prefers-reduced-motion: reduce) { .shot img { transition: none; } }
/* A lone mockup: heading, mockup, caption stacked; where there's room its section's own rule
   (from render.py) puts the heading, notes and caption in a column beside it. */
section.solo { column-gap: 56px; align-content: start; }
section.solo .figures, section.solo figure { display: contents; }
figcaption { display: grid; gap: 4px; justify-items: start; max-width: 68ch; }
figcaption .row > b { font-size: 1.05rem; }
figcaption p, figcaption ul { color: var(--muted); }
figcaption .row { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; }
.tag { font-size: .78rem; font-weight: 500; padding: 1px 10px; border-radius: 999px;
  background: var(--accent); color: var(--on-accent); }
.tag.was { background: transparent; color: var(--muted); border: 1px solid var(--edge); }
button.prev { border: 1px solid var(--edge); padding: 1px 9px; font-size: .75rem; }
.stage { position: relative; width: 100%; border-radius: 12px; overflow: hidden; border: 1px solid var(--edge);
  user-select: none; }
.stage > img { position: absolute; inset: 0; width: 100%; height: 100%; max-width: none; }
.stage .which { position: absolute; bottom: 10px; left: 50%; transform: translateX(-50%); font-size: .8rem;
  font-weight: 600; padding: 2px 10px; border-radius: 999px; background: rgb(0 0 0 / .65); color: #fff;
  pointer-events: none; white-space: nowrap; }
.stage .swipe { position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: ew-resize; }
.stage::after { content: ""; position: absolute; top: 0; bottom: 0; left: var(--at, 50%); width: 2px;
  margin-left: -1px; background: var(--accent); box-shadow: 0 0 12px var(--glow); pointer-events: none; }
section[data-mode="flip"] .stage::after, section[data-mode="flip"] .swipe { display: none; }
section[data-mode="flip"] .stage { cursor: pointer; }
/* Pick and comment, over a mockup: shown on hover, and kept once they're on. */
.marks { position: absolute; top: 10px; z-index: 2; display: flex; gap: 6px; }
.marks[hidden] { display: none; }
.marks.right { right: 10px; }
.marks.left { left: 10px; flex-direction: row-reverse; }
/* Out of the layout while hidden, so what's shown sits in the corner. */
.marks button { display: none; align-items: center; gap: 6px; font-size: .8rem; font-weight: 600;
  padding: 4px 12px; border-radius: 999px; background: rgb(20 18 26 / .8); color: #fff;
  box-shadow: 0 2px 8px rgb(0 0 0 / .3); }
.shot:hover .marks button, .stage:hover .marks button, .marks:has(:focus-visible) button,
.marks button.on, .marks button[aria-expanded="true"] { display: inline-flex; }
.marks button.on { background: var(--accent); color: var(--on-accent); }
/* Away from the pointer, what's on shrinks to its icon, to cover as little of the mockup as it can. */
.marks .text { display: none; }
.marks button.on { padding: 6px 8px; }
.shot:hover .marks .text, .stage:hover .marks .text, .marks:has(:focus-visible) .text,
.marks button[aria-expanded="true"] .text { display: inline; }
.shot:hover .marks button.on, .stage:hover .marks button.on, .marks:has(:focus-visible) button.on,
.marks button[aria-expanded="true"] { padding: 4px 12px; }
@media (hover: none) { .marks .text { display: inline; } .marks button.on { padding: 4px 12px; } }
@media (hover: none) { .marks button { display: inline-flex; } }
.note-pop { position: absolute; top: 48px; z-index: 3; width: min(340px, calc(100% - 20px)); display: grid; gap: 8px;
  padding: 12px; border-radius: 12px; background: var(--surface); color: var(--text); border: 1px solid var(--edge);
  box-shadow: 0 10px 32px rgb(0 0 0 / .35); cursor: auto; user-select: text; }
.note-pop[hidden] { display: none; }
.note-pop.right { right: 10px; }
.note-pop.left { left: 10px; }
.note-pop b { font-size: .85rem; }
.note-pop textarea { width: 100%; font: inherit; color: var(--text); background: var(--bg);
  border: 1px solid var(--edge); border-radius: 8px; padding: 6px 10px; resize: vertical; }
.note-pop textarea:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
.note-foot { display: flex; align-items: center; gap: 6px; }
.note-foot .hint { margin-right: auto; color: var(--muted); font-size: .75rem; }
@media (hover: none) { .note-foot .hint { visibility: hidden; } }
.note-foot button { border: 1px solid var(--edge); font-size: .8rem; padding: 3px 10px; }
.note-foot .done { background: var(--accent); color: var(--on-accent); border-color: transparent; font-weight: 600; }
.reply { position: fixed; left: 20px; bottom: 20px; z-index: 10; display: flex; flex-wrap: wrap; align-items: center;
  gap: 8px; max-width: calc(100% - 40px); padding: 8px 8px 8px 16px; border-radius: 16px; background: var(--surface);
  border: 1px solid var(--edge); box-shadow: 0 10px 32px rgb(0 0 0 / .3); }
.reply[hidden] { display: none; }
.reply .count { color: var(--muted); font-size: .85rem; }
.reply button { border: 1px solid var(--edge); }
.reply button.copy { background: var(--accent); color: var(--on-accent); border-color: transparent; font-weight: 600; }
.reply textarea { flex-basis: 100%; height: 8em; font: .8rem/1.4 ui-monospace, monospace; color: var(--text);
  background: var(--bg); border: 1px solid var(--edge); border-radius: 8px; padding: 6px 8px; }
.reply.updated { border-color: var(--accent); }
@media (max-width: 700px) { .reply { bottom: 84px; left: 16px; max-width: calc(100% - 32px); } }
"""

SCRIPT = r"""
// Picks (one variant per group) and comments (one per mockup), kept in this browser so a reload
// keeps them, and copied as one Markdown reply for the reader to paste back to the agent.
const REPLY_KEY = "mockup-reply:" + location.pathname;
const reply = { picks: {}, comments: {} };
try { Object.assign(reply, JSON.parse(localStorage.getItem(REPLY_KEY)) || {}); } catch {}
const groupOf = (name) => name.split(".")[0];

// The pick and comment buttons over a mockup. `data-for` names the mockup they act on; the
// comparison stage's retarget as it flips.
const allMarks = [];
const icon = (d) => `<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="${d}"/></svg>`;
const bubble = icon("M4 4h16v12H8l-4 4z");
const check = icon("M9 16.2 4.8 12l-1.4 1.4L9 19 21 7l-1.4-1.4z");
const text = (t) => `<span class="text">${t}</span>`;
const makeMarks = (host, side, pickable) => {
  const marks = document.createElement("div");
  marks.className = "marks " + side;
  marks.innerHTML = (pickable ? '<button type="button" class="pick"></button>' : "")
    + '<button type="button" class="comment" aria-expanded="false"></button>';
  host.append(marks);
  marks.addEventListener("click", (e) => {
    // The stage flips on a click; these clicks are the buttons'.
    e.stopPropagation();
    const button = e.target.closest("button");
    if (!button) return;
    const name = marks.dataset.for;
    if (button.classList.contains("pick")) {
      const group = groupOf(name);
      if (reply.picks[group] === name) delete reply.picks[group];
      else reply.picks[group] = name;
      save();
    } else if (pop.dataset.for === name && !pop.hidden && pop.parentElement === host) closeNote();
    else openNote(marks);
  });
  allMarks.push(marks);
  return marks;
};
const paintMarks = () => {
  for (const section of document.querySelectorAll("section[data-group]")) {
    const pick = reply.picks[section.dataset.group];
    let decided = false;
    for (const fig of section.querySelectorAll("figure")) {
      const on = fig.dataset.name === pick;
      fig.classList.toggle("picked", on);
      decided ||= on;
    }
    section.classList.toggle("decided", decided);
  }
  for (const marks of allMarks) {
    const name = marks.dataset.for;
    const pick = marks.querySelector(".pick");
    if (pick) {
      const on = reply.picks[groupOf(name)] === name;
      pick.classList.toggle("on", on);
      pick.setAttribute("aria-pressed", on);
      pick.innerHTML = check + text(on ? "Picked" : "Pick");
      pick.title = on ? "Picked" : "";
    }
    const comment = marks.querySelector(".comment");
    const has = !!reply.comments[name];
    comment.classList.toggle("on", has);
    comment.innerHTML = bubble + text(has ? "Commented" : "Comment");
    comment.title = has ? reply.comments[name] : "";
  }
};

// One comment popover, moved to the mockup it's opened on.
const pop = document.querySelector(".note-pop");
const note = pop.querySelector("textarea");
let opener = null;
const openNote = (marks) => {
  closeNote();
  opener = marks;
  pop.dataset.for = marks.dataset.for;
  pop.className = "note-pop " + (marks.classList.contains("left") ? "left" : "right");
  pop.querySelector(".note-title").textContent = "Comment on " + marks.dataset.for;
  note.value = reply.comments[marks.dataset.for] || "";
  marks.parentElement.append(pop);
  pop.hidden = false;
  marks.querySelector(".comment").setAttribute("aria-expanded", true);
  note.focus();
};
const closeNote = () => {
  if (pop.hidden) return;
  pop.hidden = true;
  opener?.querySelector(".comment").setAttribute("aria-expanded", false);
  opener = null;
};
note.addEventListener("input", () => {
  if (note.value.trim()) reply.comments[pop.dataset.for] = note.value;
  else delete reply.comments[pop.dataset.for];
  save();
});
note.addEventListener("keydown", (e) => {
  if (e.key === "Escape" || (e.key === "Enter" && (e.ctrlKey || e.metaKey))) {
    e.preventDefault();
    const back = opener?.querySelector(".comment");
    closeNote();
    back?.focus();
  }
});
pop.addEventListener("click", (e) => {
  e.stopPropagation();
  if (e.target.closest(".done")) closeNote();
  else if (e.target.closest(".delete")) {
    delete reply.comments[pop.dataset.for];
    closeNote();
    save();
  }
});
document.addEventListener("click", (e) => {
  if (!pop.hidden && !pop.contains(e.target)) closeNote();
});

// One view for the whole page, set from the floating options: how pairs are compared, and
// which theme every mockup shows (a section without it keeps its own).
const view = { mode: "side", theme: null };
const sections = [...document.querySelectorAll("section[data-group]")].map((section) => {
  const stage = section.querySelector(".stage");
  const pickable = section.querySelectorAll("figure").length > 1;
  for (const fig of section.querySelectorAll("figure")) {
    makeMarks(fig.querySelector(".shot"), "right", pickable).dataset.for = fig.dataset.name;
  }
  // In a swipe each side has its own; in a flip, the one on the right follows what's shown.
  const marksA = stage && makeMarks(stage, "left", true);
  const marksB = stage && makeMarks(stage, "right", true);
  const src = (name) => {
    const fig = section.querySelector(`figure[data-name="${CSS.escape(name)}"]`);
    return fig.dataset[section.dataset.theme];
  };
  const draw = () => {
    const themes = section.dataset.themes.split(" ");
    if (view.theme && themes.includes(view.theme)) section.dataset.theme = view.theme;
    for (const fig of section.querySelectorAll("figure")) {
      const prev = fig.querySelector("button.prev")?.getAttribute("aria-pressed") === "true";
      const t = section.dataset.theme;
      const key = prev ? "prev" + t[0].toUpperCase() + t.slice(1) : t;
      if (fig.dataset[key]) fig.querySelector(".shot img").src = fig.dataset[key];
    }
    if (!stage) return;
    section.dataset.mode = view.mode;
    stage.hidden = view.mode === "side";
    stage.querySelector(".a").src = src(stage.dataset.a);
    stage.querySelector(".b").src = src(stage.dataset.b);
    const swipe = view.mode === "swipe";
    const showB = stage.dataset.flipped === "1";
    const at = stage.querySelector(".swipe").value;
    stage.style.setProperty("--at", at + "%");
    stage.querySelector(".b").style.clipPath = swipe ? `inset(0 0 0 ${at}%)` : "none";
    stage.querySelector(".b").style.visibility = swipe || showB ? "visible" : "hidden";
    stage.querySelector(".which").textContent = swipe
      ? `${stage.dataset.aLabel} ◂ ▸ ${stage.dataset.bLabel}`
      : `${showB ? stage.dataset.bLabel : stage.dataset.aLabel} (click to flip)`;
    marksA.hidden = !swipe;
    marksA.dataset.for = stage.dataset.a;
    marksB.dataset.for = swipe || showB ? stage.dataset.b : stage.dataset.a;
    if (pop.parentElement === stage && pop.dataset.for !== marksB.dataset.for && pop.dataset.for !== marksA.dataset.for) {
      closeNote();
    }
    paintMarks();
  };
  for (const button of section.querySelectorAll("button.prev")) {
    button.addEventListener("click", () => {
      button.setAttribute("aria-pressed", button.getAttribute("aria-pressed") !== "true");
      button.textContent = button.getAttribute("aria-pressed") === "true" ? "Showing previous render" : "Previous render";
      draw();
    });
  }
  if (stage) {
    stage.querySelector(".swipe").addEventListener("input", draw);
    stage.addEventListener("click", () => {
      if (view.mode !== "flip") return;
      stage.dataset.flipped = stage.dataset.flipped === "1" ? "0" : "1";
      draw();
    });
  }
  return { stage, draw };
});

const fab = document.querySelector(".fab");
const set = (kind, value) => {
  view[kind] = value;
  // The page around the mockups wears their theme, so a light render isn't judged on a dark page.
  if (kind === "theme" && value) document.documentElement.dataset.theme = value;
  for (const b of document.querySelectorAll(`.fab button[data-${kind}]`)) {
    b.setAttribute("aria-pressed", b.dataset[kind] === value);
  }
  const state = document.querySelector(".fab-state");
  if (state) {
    const label = (kind) => document.querySelector(`.fab button[data-${kind}][aria-pressed="true"]`)?.textContent;
    state.textContent = label("mode") || label("theme") || "";
  }
  // The mockup a comment was open on may not be drawn in the new mode.
  if (kind === "mode") closeNote();
  for (const s of sections) s.draw();
};
if (fab) {
  fab.addEventListener("click", (e) => {
    const button = e.target.closest("button");
    if (!button) return;
    if (button.classList.contains("fab-button")) {
      // Hover opens it; a tap (no hover on touch screens) toggles it.
      fab.classList.toggle("open");
      button.setAttribute("aria-expanded", fab.classList.contains("open"));
    } else if (button.dataset.mode) set("mode", button.dataset.mode);
    else if (button.dataset.theme) set("theme", button.dataset.theme);
  });
  document.addEventListener("click", (e) => {
    if (!fab.contains(e.target)) fab.classList.remove("open");
  });
}
document.addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.metaKey || e.altKey || e.target.closest("input, textarea")) return;
  const modes = { s: "side", w: "swipe", f: "flip" };
  const key = e.key.toLowerCase();
  if (modes[key] && document.querySelector(".fab button[data-mode]")) set("mode", modes[key]);
  else if (key === "t") {
    const themes = [...document.querySelectorAll(".fab button[data-theme]")].map((b) => b.dataset.theme);
    if (themes.length) set("theme", themes[(themes.indexOf(view.theme) + 1) % themes.length]);
  } else if (e.key === " " && view.mode === "flip" && !e.target.closest("button")) {
    const hovered = sections.find((s) => s.stage?.matches(":hover"));
    if (hovered) {
      e.preventDefault();
      hovered.stage.click();
    }
  }
});

// The reply bar: what's been picked and commented, and copying it as one Markdown message.
const bar = document.querySelector(".reply");
const groups = [...document.querySelectorAll("section[data-group]")].map((section) => ({
  group: section.dataset.group,
  heading: section.dataset.heading,
  names: [...section.querySelectorAll("figure")].map((f) => f.dataset.name),
}));
// Only what's still on the page: a re-render may have renamed or dropped a mockup.
const picked = (g) => (g.names.includes(reply.picks[g.group]) ? reply.picks[g.group] : null);
const commented = (g) => g.names.filter((n) => reply.comments[n]);
const replyText = () => {
  const out = ["# Re: " + document.title];
  for (const g of groups) {
    const pick = picked(g);
    const notes = commented(g);
    if (!pick && !notes.length) continue;
    out.push("", "## " + g.heading + " (`" + g.group + "`)");
    if (pick) out.push("Picked: `" + pick + "`");
    for (const n of notes) {
      out.push("`" + n + "`:", ...reply.comments[n].trim().split("\n").map((l) => "> " + l));
    }
  }
  return out.join("\n");
};
let updated = false;
const drawReply = () => {
  let picks = 0, notes = 0;
  for (const g of groups) {
    picks += picked(g) ? 1 : 0;
    notes += commented(g).length;
  }
  paintMarks();
  const parts = [];
  if (updated) parts.push("Page updated");
  if (picks) parts.push(picks + (picks === 1 ? " pick" : " picks"));
  if (notes) parts.push(notes + (notes === 1 ? " comment" : " comments"));
  bar.querySelector(".count").textContent = parts.join(" · ");
  bar.querySelector(".reload").hidden = !updated;
  bar.querySelector(".copy").hidden = bar.querySelector(".clear").hidden = !(picks || notes);
  bar.classList.toggle("updated", updated);
  bar.hidden = !(updated || picks || notes);
  const fallback = bar.querySelector(".fallback");
  if (!fallback.hidden) fallback.value = replyText();
};
const save = () => {
  try { localStorage.setItem(REPLY_KEY, JSON.stringify(reply)); } catch {}
  drawReply();
};
bar.querySelector(".copy").addEventListener("click", async (e) => {
  const button = e.currentTarget;
  try {
    await navigator.clipboard.writeText(replyText());
    button.textContent = "Copied: paste it in the chat";
    setTimeout(() => (button.textContent = "Copy reply"), 2500);
  } catch {
    // No clipboard (a sandboxed frame, say): show the text to copy by hand.
    const fallback = bar.querySelector(".fallback");
    fallback.hidden = false;
    fallback.value = replyText();
    fallback.select();
  }
});
bar.querySelector(".clear").addEventListener("click", (e) => {
  const button = e.currentTarget;
  if (button.dataset.armed !== "1") {
    // Two clicks, since dialogs may be blocked where the page is embedded.
    button.dataset.armed = "1";
    button.textContent = "Clear all?";
    setTimeout(() => {
      button.dataset.armed = "";
      button.textContent = "Clear";
    }, 3000);
    return;
  }
  reply.picks = {};
  reply.comments = {};
  closeNote();
  bar.querySelector(".fallback").hidden = true;
  save();
});
bar.querySelector(".reload").addEventListener("click", () => location.reload());

// The view outlives a reload (a re-render reloads the page), in this browser only.
const VIEW_KEY = "mockup-view:" + location.pathname;
let saved = {};
try { saved = JSON.parse(sessionStorage.getItem(VIEW_KEY)) || {}; } catch {}
const has = (kind, value) => value && document.querySelector(`.fab button[data-${kind}="${CSS.escape(value)}"]`);
const first = document.querySelector(".fab button[data-theme]");
const scheme = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
const start = has("theme", saved.theme) ? saved.theme : has("theme", scheme) ? scheme : first?.dataset.theme;
set("theme", start || null);
set("mode", has("mode", saved.mode) ? saved.mode : "side");
addEventListener("pagehide", () => {
  try { sessionStorage.setItem(VIEW_KEY, JSON.stringify(view)); } catch {}
});
drawReply();

// From render.py's local server, notice a re-render and reload. Picks and comments survive a
// reload, but a comment being typed would lose its focus mid-word, so then offer it instead.
if (["localhost", "127.0.0.1"].includes(location.hostname)) {
  const stamp = async () => {
    const r = await fetch(location.href, { method: "HEAD", cache: "no-store" });
    return r.ok ? r.headers.get("last-modified") + " " + r.headers.get("content-length") : null;
  };
  let seen = null;
  const check = async () => {
    if (document.hidden || updated) return;
    let now = null;
    try { now = await stamp(); } catch {}
    if (!now) return;
    if (seen === null) seen = now;
    else if (now !== seen) {
      if (!pop.hidden) {
        updated = true;
        drawReply();
      } else location.reload();
    }
  };
  check();
  setInterval(check, 1500);
  document.addEventListener("visibilitychange", check);
}
"""

main()
