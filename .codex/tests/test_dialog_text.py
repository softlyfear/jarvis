from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DialogTextRanges(unittest.TestCase):
    def test_long_ranges_keep_terminal_tail_without_hidden_text(self):
        pwsh = shutil.which("pwsh")
        if not pwsh:
            local = ROOT / ".codex/.tmp/pwsh/pwsh"
            if local.is_file():
                pwsh = str(local)
        if not pwsh:
            self.skipTest("PowerShell is unavailable")
        result = subprocess.run(
            [pwsh, "-NoProfile", "-File", str(ROOT / ".codex/tools/check_dialog_text.ps1")],
            capture_output=True, text=True, timeout=30, cwd=ROOT,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("PASS: long range retains command", result.stdout)
