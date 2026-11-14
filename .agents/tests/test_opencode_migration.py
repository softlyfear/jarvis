"""Check project migration contracts without invoking destructive commands."""

import fnmatch
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


def project_config():
    # This project uses full-line JSONC comments and no trailing commas.
    text = (ROOT / "opencode.jsonc").read_text(encoding="utf-8")
    return json.loads("\n".join(line for line in text.splitlines()
                                if not line.lstrip().startswith("//")))


class OpenCodeMigration(unittest.TestCase):
    def test_old_agent_configuration_is_absent(self):
        for name in (".codex", ".grok"):
            self.assertFalse((ROOT / name).exists())
        for name in ("AGENTS.md", "opencode.jsonc", ".github/workflows/windows.yml"):
            text = (ROOT / name).read_text(encoding="utf-8")
            self.assertNotIn(".codex/", text)
            self.assertNotIn(".grok/", text)

    def test_dangerous_shell_commands_are_never_auto_allowed(self):
        rules = project_config()["permissions"]
        for command in ("rm -rf /etc", "rm -rf .", "git reset --hard",
                        "git clean -fd", "docker compose down -v",
                        "python3 -c 'import shutil'", "sudo apt-get install package"):
            with self.subTest(command=command):
                matches = [rule["effect"] for rule in rules
                           if fnmatch.fnmatchcase("shell", rule["action"])
                           and fnmatch.fnmatchcase(command, rule["resource"])]
                self.assertTrue(matches)
                self.assertIn(matches[-1], ("ask", "deny"))

    def test_reviewer_cannot_write_through_shell_or_delegate(self):
        text = (ROOT / ".opencode/agents/reviewer.md").read_text(encoding="utf-8")
        self.assertIn('  - action: "*"\n    resource: "*"\n    effect: deny', text)
        for action in ("shell", "edit", "subagent"):
            self.assertIn(f"  - action: {action}\n    resource: \"*\"\n    effect: deny", text)
        self.assertNotIn("resource: \"git ", text)

    def test_mcp_servers_keep_transports_timeouts_and_allowlists(self):
        config = project_config()
        servers = config["mcp"]["servers"]
        self.assertEqual(set(servers), {"context7", "hugging_face_hub", "serena", "playwright"})
        for name, startup, execution in (("serena", 90000, 120000),
                                          ("playwright", 30000, 60000)):
            spec = servers[name]
            self.assertEqual(spec["type"], "local")
            self.assertEqual(spec["command"][-1], name)
            self.assertIn("/.opencode/mcp/launch.py", spec["command"][2])
            self.assertEqual(spec["timeout"], {"startup": startup, "execution": execution})
        for name, count in (("serena", 6), ("playwright", 12), ("hugging_face_hub", 3)):
            allowed = [rule for rule in config["permissions"]
                       if rule["action"].startswith(name + "_") and rule["effect"] == "allow"]
            self.assertEqual(len(allowed), count)
        self.assertNotIn("instructions", config)
        self.assertNotIn("skills", config)

    def test_historical_snapshot_targets_exist_with_preserved_bytes(self):
        import hashlib

        path = ROOT / ".agents/plans/development-2026-10-05/requirements.json"
        document = json.loads(path.read_text(encoding="utf-8"))
        for task in document["tasks"]:
            for requirement in task["requirements"]:
                for evidence in requirement.get("evidence", []):
                    for source in evidence.get("inputs", []):
                        if "/source-snapshots/" in source["path"]:
                            snapshot = ROOT / source["path"]
                            self.assertTrue(snapshot.is_file(), source["path"])
        baseline = json.loads((ROOT / ".agents/evidence/opencode-migration/snapshot-baseline.json")
                              .read_text(encoding="utf-8"))["snapshots"]
        snapshots = ROOT / ".agents/evidence/development-2026-10-05/source-snapshots/ad8324971527066261f3d388dbd8bd6bff1a6d1f"
        for name, digest in baseline.items():
            self.assertEqual(hashlib.sha256((snapshots / name).read_bytes()).hexdigest(), digest)


if __name__ == "__main__":
    unittest.main()
