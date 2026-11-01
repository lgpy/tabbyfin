#!/usr/bin/env python3
"""A PreToolUse hook on AskUserQuestion: refuses a question about mockups until the page's link
is in the agent's reply, not only in the question (`SKILL.md`, step 6).

A question counts as being about mockups when its text has a mockup page's URL
(`http://localhost:<port>/…`) or the word "mockup". It goes through only when every such URL in
it (or, with none, some mockup URL) is in the agent's text since the user last wrote.
"""

import json
import re
import sys

URL = re.compile(r"https?://localhost:\d+/[^\s)\]>\"'`]*")


def main():
    event = json.load(sys.stdin)
    asked = json.dumps(event.get("tool_input", {}), ensure_ascii=False)
    # Without the sentence's punctuation after a link ("…/index.html: which one?").
    urls = {u.rstrip(".,:;!?") for u in URL.findall(asked)}
    if not urls and "mockup" not in asked.lower():
        return
    said = replies(event.get("transcript_path"))
    if said is None:
        return  # no transcript to check against: don't block on our own failure
    if urls:
        missing = sorted(u for u in urls if u not in said)
    else:
        missing = [] if URL.search(said) else ["the mockup page's link"]
    if not missing:
        return
    reason = (
        f"Not asked: {', '.join(missing)} isn't in your reply yet. Write a reply first with the page "
        "as a markdown link (render.py prints it after `tell the user:`) and the decisions you need, "
        "then ask, with the same link in the question too."
    )
    if not urls:
        reason += " Put the link in the question as well."
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


def replies(path):
    """The agent's text since the user's last message, or None if the transcript can't be read."""
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except (OSError, TypeError):
        return None
    texts = []
    for line in lines:
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        content = (entry.get("message") or {}).get("content")
        if entry.get("type") == "user" and not entry.get("isMeta"):
            # Tool results come back as user entries too; only the user's own words start a turn.
            if isinstance(content, str) or any(
                isinstance(b, dict) and b.get("type") == "text" for b in content or []
            ):
                texts = []
        elif entry.get("type") == "assistant" and isinstance(content, list):
            texts += [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
    return "\n".join(texts)


if __name__ == "__main__":
    main()
