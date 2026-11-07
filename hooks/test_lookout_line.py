"""Checks for lookout_line.py. Run: /usr/bin/python3 hooks/test_lookout_line.py"""

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

HOOK = Path(__file__).with_name("lookout_line.py")
INTERACTIVE = {"CLAUDE_CODE_ENTRYPOINT": "cli", "CLAUDE_CODE_SESSION_ATTENDED": "1"}
CLAUDE_P = {"CLAUDE_CODE_ENTRYPOINT": "sdk-cli", "CLAUDE_CODE_SESSION_ATTENDED": "0"}
# The three forms Lookout parses: sign, space, "Lookout:".
LINES = ("◌ Lookout: nothing needed", "? Lookout: request", "✓ Lookout: done")


def run(stdin, env):
    clean = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE_CODE_")}
    return subprocess.run(
        [sys.executable, str(HOOK)],
        input=stdin,
        capture_output=True,
        text=True,
        env={**clean, **env},
        timeout=10,
    )


class LookoutLineHook(unittest.TestCase):
    def test_interactive_sessions_get_the_instruction(self):
        for event in ("SessionStart", "UserPromptSubmit"):
            r = run(json.dumps({"hook_event_name": event}), INTERACTIVE)
            self.assertEqual((r.returncode, r.stderr), (0, ""))
            out = json.loads(r.stdout)["hookSpecificOutput"]
            self.assertEqual(out["hookEventName"], event)
            for line in LINES:
                self.assertIn(line, out["additionalContext"])

    def test_background_runs_get_nothing(self):
        sdk_py = {"CLAUDE_CODE_ENTRYPOINT": "sdk-py"}  # Python Agent SDK
        unattended = {**INTERACTIVE, "CLAUDE_CODE_SESSION_ATTENDED": "0"}
        for env in (CLAUDE_P, sdk_py, unattended):
            r = run(json.dumps({"hook_event_name": "UserPromptSubmit"}), env)
            self.assertEqual((r.returncode, r.stdout, r.stderr), (0, "", ""), env)

    def test_other_events_and_bad_input_get_nothing(self):
        for stdin in (json.dumps({"hook_event_name": "Stop"}), "", "not json", "[]"):
            r = run(stdin, INTERACTIVE)
            self.assertEqual((r.returncode, r.stdout, r.stderr), (0, "", ""), stdin)


if __name__ == "__main__":
    unittest.main()
