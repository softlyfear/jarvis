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
    def test_reading_claude_and_copying_into_codex_is_allowed(self):
        for command in (
            "cat .claude/CLAUDE.md",
            "cat CLAUDE.md",
            "cp .claude/CLAUDE.md .codex/AGENTS.md",
            "rg rules .claude",
            "git diff -- .claude",
            "git status --short",
            "DOCS_RS=1 cargo check -p jarvis-core",
            "rm -rf .codex/.tmp/scratch",
        ):
            with self.subTest(command=command):
                self.assertIsNone(GUARD.check(command, ROOT))

    def test_claude_mutations_are_denied(self):
        for command in (
            "rm .claude/settings.json",
            "rm CLAUDE.md",
            "mv .claude other",
            "cp .codex/AGENTS.md CLAUDE.md",
            "cp -t .claude .codex/config.toml",
            "printf text > .claude/CLAUDE.md",
            'cat .codex/AGENTS.md >> "CLAUDE.md"',
            "tee .claude/settings.json",
            "sed -i s/foo/bar/ .claude/CLAUDE.md",
            "chmod -R 777 .claude",
            "cd .claude && rm settings.json",
            "git restore -- .claude",
        ):
            with self.subTest(command=command):
                self.assertEqual(GUARD.check(command, ROOT)[0], "deny")

    def test_system_project_and_agent_roots_are_denied(self):
        for command in (
            "sudo rm -rf /",
            "rm -rf ~",
            "rm -rf .",
            "rm -rf ../",
            "rm -rf .codex",
            "rm -rf .claude",
            "rm -rf /usr/lib",
            "dd if=/dev/zero of=/dev/sda",
            "mkfs.ext4 /dev/sda",
            ":(){ :|:& };:",
        ):
            with self.subTest(command=command):
                self.assertEqual(GUARD.check(command, ROOT)[0], "deny")

    def test_patch_paths_include_moves_and_absolute_paths(self):
        for header in (
            "*** Add File: CLAUDE.md",
            "*** Update File: .claude/settings.json",
            "*** Delete File: .claude/CLAUDE.md",
            "*** Move to: .claude/renamed.md",
            f"*** Update File: {ROOT}/.claude/settings.json",
        ):
            with self.subTest(header=header):
                self.assertEqual(GUARD.check_patch(header, ROOT)[0], "deny")
        self.assertIsNone(GUARD.check_patch("*** Update File: .codex/AGENTS.md", ROOT))

    def test_codex_wire_format_and_unsupported_ask(self):
        cases = (
            {"tool_name": "Bash", "tool_input": {"command": "git push --force origin master"}},
            {"tool_name": "exec_command", "tool_input": {"cmd": "docker compose down -v"}},
            {"tool_name": "apply_patch", "tool_input": {"command": "*** Update File: CLAUDE.md"}},
            {"tool_name": "exec_command", "tool_input": {"cmd": "rm settings.json", "workdir": ".claude"}},
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
