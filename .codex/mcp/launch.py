"""Launch pinned project MCP servers with all service state inside .codex."""

import argparse
import os
import json
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / ".codex"


def copy_atomic(source, target):
    # Concurrent MCP launches must never expose a partially copied YAML file.
    with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as handle:
        temporary = Path(handle.name)
        handle.write(source.read_bytes())
    try:
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)


def prepare(server):
    env = os.environ.copy()
    env["DOCS_RS"] = "1"
    env["UV_CACHE_DIR"] = str(BASE / ".cache/uv")
    if server == "serena":
        executable = BASE / ".cache/mcp-serena/venv/bin/serena"
        home = BASE / ".cache/serena-home"
        project = BASE / ".cache/serena-project"
        if not executable.is_file():
            raise RuntimeError("Serena не установлена: python3 .codex/mcp/setup.py")
        home.mkdir(parents=True, exist_ok=True)
        project.mkdir(parents=True, exist_ok=True)
        # Canonical configuration is versioned; generated logs/caches are ignored.
        copy_atomic(BASE / "mcp/serena.yml", home / "serena_config.yml")
        copy_atomic(BASE / "mcp/serena-project.yml", project / "project.yml")
        env["SERENA_HOME"] = str(home)
        return [str(executable), "start-mcp-server", "--project", str(ROOT), "--context", "codex", "--mode", "planning", "--mode", "no-memories", "--mode", "no-onboarding", "--enable-web-dashboard", "false", "--open-web-dashboard", "false", "--enable-gui-log-window", "false"], env
    cli = BASE / ".cache/mcp-playwright/node_modules/@playwright/mcp/cli.js"
    if not cli.is_file():
        raise RuntimeError("Playwright MCP не установлен: python3 .codex/mcp/setup.py")
    output = BASE / ".tmp/mcp-playwright"
    output.mkdir(parents=True, exist_ok=True)
    env["PLAYWRIGHT_BROWSERS_PATH"] = str(BASE / ".cache/playwright")
    registry = json.loads((BASE / ".cache/mcp-playwright/node_modules/playwright-core/browsers.json").read_text())
    revision = next(item["revision"] for item in registry["browsers"] if item["name"] == "chromium-headless-shell")
    browser = BASE / f".cache/playwright/chromium_headless_shell-{revision}/chrome-headless-shell-linux64/chrome-headless-shell"
    if not browser.is_file():
        raise RuntimeError("Chromium headless shell не установлен: python3 .codex/mcp/setup.py --server playwright")
    return ["node", str(cli), "--executable-path", str(browser), "--headless", "--isolated", "--no-sandbox", "--output-dir", str(output), "--viewport-size", "1100x850", "--timeout-action", "10000", "--timeout-navigation", "30000", "--no-webmcp"], env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", choices=("serena", "playwright"))
    args = parser.parse_args()
    command, env = prepare(args.server)
    os.chdir(ROOT)
    os.execvpe(command[0], command, env)


if __name__ == "__main__":
    main()
