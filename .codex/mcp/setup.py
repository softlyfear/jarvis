"""Install project MCP dependencies into .codex caches (Linux/WSL development)."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / ".codex"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", choices=("all", "serena", "playwright"), default="all")
    args = parser.parse_args()
    env = os.environ.copy()
    env.update(UV_CACHE_DIR=str(BASE / ".cache/uv"), PLAYWRIGHT_BROWSERS_PATH=str(BASE / ".cache/playwright"))
    if args.server in ("all", "serena"):
        venv = BASE / ".cache/mcp-serena/venv"
        if not (venv / "bin/python").is_file():
            subprocess.run(["uv", "venv", "--python", "3.13", str(venv)], cwd=ROOT, env=env, check=True)
        subprocess.run(["uv", "pip", "install", "--python", str(venv / "bin/python"), "-r", str(BASE / "mcp/serena-requirements.txt")], cwd=ROOT, env=env, check=True)
    if args.server in ("all", "playwright"):
        target = BASE / ".cache/mcp-playwright"
        target.mkdir(parents=True, exist_ok=True)
        for name in ("package.json", "package-lock.json"):
            shutil.copyfile(BASE / "mcp/playwright" / name, target / name)
        subprocess.run(["npm", "ci", "--prefix", str(target), "--cache", str(BASE / ".cache/npm"), "--no-audit", "--no-fund"], cwd=ROOT, env=env, check=True)
        subprocess.run(["node", str(target / "node_modules/playwright/cli.js"), "install", "chromium", "--only-shell"], cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
