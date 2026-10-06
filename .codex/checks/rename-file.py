"""Exercise the exact embedded .NET rename script with portable PowerShell."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pwsh", required=True)
    args = parser.parse_args()
    source = ROOT / "crates/jarvis-core/src/actions/files.rs"
    text = source.read_text()
    script = text.split('let script = r#"$ErrorActionPreference', 1)[1].split('"#;', 1)[0]
    script = "$ErrorActionPreference" + script
    with tempfile.TemporaryDirectory(dir=ROOT / ".codex/.tmp", prefix="rename-check-") as temp:
        root = Path(temp)

        def move(old, new, ok):
            env = dict(os.environ, JARVIS_SOURCE=str(old), JARVIS_DESTINATION=str(new))
            result = subprocess.run([args.pwsh, "-NoProfile", "-NonInteractive", "-Command", script], env=env, capture_output=True, timeout=20)
            assert (result.returncode == 0) == ok, result.stderr.decode(errors="replace")

        folder = root / "Новая папка"
        folder.mkdir()
        (folder / "keep.txt").write_text("keep this content")
        renamed = root / "Лучший проект"
        move(folder, renamed, True)
        assert not folder.exists() and (renamed / "keep.txt").read_text() == "keep this content"
        occupied = root / "occupied"
        occupied.mkdir()
        (occupied / "other.txt").write_text("other")
        move(renamed, occupied, False)
        assert (renamed / "keep.txt").read_text() == "keep this content"
        assert (occupied / "other.txt").read_text() == "other"
        file = root / "a & b.txt"
        file.write_text("file content")
        destination = root / "new ' file.txt"
        move(file, destination, True)
        assert not file.exists() and destination.read_text() == "file content"
        occupied_file = root / "existing.txt"
        occupied_file.write_text("existing")
        move(destination, occupied_file, False)
        assert destination.read_text() == "file content" and occupied_file.read_text() == "existing"
    print(json.dumps({"result": "pass", "exit_code": 0, "environment": {"os": os.name, "runtime": "portable PowerShell/.NET; not Windows acceptance"}, "cases": 4, "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}, ensure_ascii=False))


if __name__ == "__main__":
    main()
