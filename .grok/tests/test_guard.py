"""Guard decisions for the Grok wrapper, without running destructive commands."""

import json
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / ".grok" / "hooks" / "guard-bash.py"
SESSION = ROOT / ".grok" / "hooks" / "session-start.sh"


def run_guard(payload: dict) -> subprocess.CompletedProcess[str]:
    payload = {"cwd": str(ROOT), **payload}
    return subprocess.run(
        [sys.executable, str(GUARD)],
        input=json.dumps(payload),
        text=True,
        capture_output=True,
        check=True,
    )


class GuardTests(unittest.TestCase):
    def test_everyday_command_has_no_decision(self):
        result = run_guard({"tool_input": {"command": "DOCS_RS=1 cargo check -p jarvis-core"}})
        self.assertEqual(result.stdout, "")

    def test_grok_directory_and_shared_roots_are_denied(self):
        commands = (
            "rm -rf .grok",
            "rm -rf .grok/hooks",
            "rm -rf .codex",
            "rm -rf /",
            "sudo rm -rf ~",
        )
        for command in commands:
            with self.subTest(command=command):
                result = run_guard({"tool_input": {"command": command}})
                output = json.loads(result.stdout)
                self.assertEqual(output["decision"], "deny")
                self.assertEqual(output["hookSpecificOutput"]["permissionDecision"], "deny")

    def test_force_push_asks_instead_of_denying(self):
        result = run_guard({
            "toolInput": {"command": "git push --force origin master"},
        })
        output = json.loads(result.stdout)
        self.assertEqual(output["decision"], "ask")
        self.assertIn("explicit user authorization", output["reason"])

    def test_relative_workdir_protects_grok(self):
        result = run_guard({
            "tool_input": {"command": "rm -rf .", "workdir": ".grok"},
        })
        output = json.loads(result.stdout)
        self.assertEqual(output["decision"], "deny")

    def test_session_start_script_parses(self):
        subprocess.run(["bash", "-n", str(SESSION)], check=True)

    def test_session_start_emits_grok_message(self):
        result = subprocess.run(
            ["bash", str(SESSION)], text=True, capture_output=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"systemMessage"', result.stdout)
        listed = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            text=True,
            capture_output=True,
        )
        if listed.returncode == 0 and "x86_64-pc-windows-gnu" in listed.stdout.splitlines():
            self.assertNotIn("missing Rust target", result.stderr)

    def test_hooks_json_inlines_commands(self):
        # Grok 1.0.70 drops a separate args array and runs command alone.
        hooks = json.loads((ROOT / ".grok" / "hooks" / "hooks.json").read_text())
        handlers = [
            handler
            for groups in hooks["hooks"].values()
            for group in groups
            for handler in group["hooks"]
        ]
        commands = [handler["command"] for handler in handlers]
        self.assertTrue(any("session-start.sh" in command for command in commands))
        self.assertTrue(any("guard-bash.py" in command for command in commands))
        self.assertTrue(all("args" not in handler for handler in handlers))
        matcher = hooks["hooks"]["SessionStart"][0]["matcher"]
        for source in ("new", "load", "startup", "resume"):
            self.assertIn(source, matcher.split("|"))


if __name__ == "__main__":
    unittest.main()
