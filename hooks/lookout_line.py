#!/usr/bin/python3
"""Lookout hook for Claude Code. It gives every interactive session the Lookout
line instruction as additionalContext, at session start (also after /clear,
/compact and on resume) and with every prompt. It skips background runs
(claude -p and Agent SDK runs), because scripts read their output and an
extra line can break them.
Install: see the README section "The session's own status line".
"""

import json
import os
import sys

LOOKOUT_LINE = """Lookout line: end every reply that ends your turn with exactly one final line for Lookout (it sorts the user's windows), nothing after it:
◌ Lookout: nothing needed (what you wait for) = the user has nothing to do in THIS window AND something else wakes you on its own (a background command, CI, another session, a timer).
? Lookout: request (what the user should do) = the user should do anything in THIS window: answer, decide, approve, merge, test, click, type. Also when the rest says 'nothing needed'. If something waits for the user in ANOTHER window (e.g. 'approve the plan in the front desk'), that belongs there, not here.
✓ Lookout: done (result in a few words) = otherwise: work finished or answer given, it is the user's turn or the user can close the window.
If in doubt, not ◌. Not in subagent replies and not when an output format (e.g. JSON) is prescribed."""


def unattended():
    # Measured with Claude Code 2.1.319: claude -p runs hooks with CLAUDE_CODE_SESSION_ATTENDED=0 and
    # CLAUDE_CODE_ENTRYPOINT=sdk-cli; the Python Agent SDK sets sdk-py, an interactive session cli and 1.
    return os.environ.get("CLAUDE_CODE_SESSION_ATTENDED") == "0" or os.environ.get(
        "CLAUDE_CODE_ENTRYPOINT", ""
    ).startswith("sdk")


def main():
    try:
        event = json.load(sys.stdin).get("hook_event_name")
    except Exception:
        return 0  # bad input: stay silent, never block a prompt
    if event in ("SessionStart", "UserPromptSubmit") and not unattended():
        out = {"hookEventName": event, "additionalContext": LOOKOUT_LINE}
        print(json.dumps({"hookSpecificOutput": out}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
