"""Check actual guard decisions without executing destructive commands."""

import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path


GUARD_PATH = Path(__file__).resolve().parents[1] / "hooks" / "guard-bash.py"
SPEC = importlib.util.spec_from_file_location("codex_guard", GUARD_PATH)
GUARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GUARD)
ROOT = str(GUARD_PATH.parents[2])


class GuardTests(unittest.TestCase):
    def test_everyday_commands_are_allowed(self):
        for command in (
            "git status --short",
            "DOCS_RS=1 cargo check -p jarvis-core",
            "rm -rf .codex/.tmp/scratch",
        ):
            with self.subTest(command=command):
                self.assertIsNone(GUARD.check(command, ROOT))

    def test_system_project_and_agent_roots_are_denied(self):
        for command in (
            "sudo rm -rf /",
            "rm -rf ~",
            "rm -rf .",
            "rm -rf ../",
            "rm -rf .codex",
            "rm -rf /usr/lib",
            "dd if=/dev/zero of=/dev/sda",
            "mkfs.ext4 /dev/sda",
            ":(){ :|:& };:",
        ):
            with self.subTest(command=command):
                self.assertEqual(GUARD.check(command, ROOT)[0], "deny")

    def test_codex_wire_format_and_unsupported_ask(self):
        cases = (
            {"tool_name": "Bash", "tool_input": {"command": "git push --force origin master"}},
            {"tool_name": "exec_command", "tool_input": {"cmd": "docker compose down -v"}},
            {"tool_name": "exec_command", "tool_input": {"cmd": "rm -rf .", "workdir": ".codex"}},
        )
        for payload in cases:
            with self.subTest(payload=payload):
                payload["cwd"] = ROOT
                result = subprocess.run(
                    [sys.executable, str(GUARD_PATH)],
                    input=json.dumps(payload), text=True, capture_output=True, check=True,
                )
                output = json.loads(result.stdout)["hookSpecificOutput"]
                self.assertEqual(output["hookEventName"], "PreToolUse")
                self.assertEqual(output["permissionDecision"], "deny")
                self.assertTrue(output["permissionDecisionReason"])


if __name__ == "__main__":
    unittest.main()
