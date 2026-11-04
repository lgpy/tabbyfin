#!/usr/bin/env python3
"""A PreToolUse hook on AskUserQuestion: refuses a question about mockups until the page's link
has been in one of the agent's replies, not only in the question (`SKILL.md`, step 6).

It only acts in a session that has rendered a mockup page (`render.py` printed `tell the user:`
with the link); anywhere else a question that says "mockup" is about something else. There, a
question is about mockups when its text has a `http://localhost:<port>/…` URL or the word
"mockup", which includes shipit's dialog when its summary mentions them. It goes through when
every such URL in it (or, with none, the last rendered page's) is in the agent's text anywhere in
the session: a link given in an earlier turn is still in front of the user, and the page reloads
itself on each render. Each link is refused at most once a session.
"""

import json
import re
import sys
import tempfile
from pathlib import Path

URL = re.compile(r"https?://localhost:\d+/[^\s)\]>\"'`\\]*")
# render.py's line for the agent: "tell the user: [<stem> mockups](<url>), in your reply, …".
RENDERED = re.compile(r"tell the user: \[[^\]]*\]\((https?://localhost:\d+/[^)\s\\]*)\)")


def main():
    event = json.loads(sys.stdin.buffer.read().decode("utf-8"))
    entries = transcript(event.get("transcript_path"))
    if entries is None:
        return  # no transcript to check against: don't block on our own failure
    rendered = [url for raw, _ in entries for url in RENDERED.findall(raw)]
    if not rendered:
        return
    asked = json.dumps(event.get("tool_input", {}), ensure_ascii=False)
    # Without the sentence's punctuation after a link ("…/index.html: which one?").
    urls = {u.rstrip(".,:;!?") for u in URL.findall(asked)}
    if not urls and "mockup" not in asked.lower():
        return
    said = "\n".join(
        b.get("text", "")
        for _, entry in entries
        if entry.get("type") == "assistant"
        for b in content_of(entry)
        if isinstance(b, dict) and b.get("type") == "text"
    )
    missing = sorted(u for u in urls if u not in said) if urls else [u for u in rendered[-1:] if u not in said]
    # The transcript can lag (a reply in the same message as the question isn't written yet, and a
    # session that moved into a worktree can lose earlier ones), so each link is refused once a
    # session: the refusal is the reminder, and the retry goes through rather than looping.
    seen = Path(tempfile.gettempdir()) / "tabbyfin-link-guard" / re.sub(r"\W", "_", str(event.get("session_id")))
    try:
        warned = set(seen.read_text(encoding="utf-8").split())
    except OSError:
        warned = set()
    missing = [u for u in missing if u not in warned]
    if not missing:
        return
    try:
        seen.parent.mkdir(parents=True, exist_ok=True)
        seen.write_text("\n".join(sorted(warned | set(missing))), encoding="utf-8")
    except OSError:
        return  # can't remember the refusal: don't risk refusing forever
    # The link comes first: shipit shows only the start of a refusal.
    reason = (
        f"Not asked: write {', '.join(missing)} in a reply to the user first (as the markdown link "
        "render.py printed), then ask again with the link in the question too."
    )
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        },
        sys.stdout,
    )


def transcript(path):
    """Each line of the transcript with its parsed entry, or None if it can't be read."""
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except (OSError, TypeError, UnicodeDecodeError):
        return None
    entries = []
    for line in lines:
        try:
            entries.append((line, json.loads(line)))
        except ValueError:
            continue
    return entries


def content_of(entry):
    content = (entry.get("message") or {}).get("content")
    return content if isinstance(content, list) else []


if __name__ == "__main__":
    main()
