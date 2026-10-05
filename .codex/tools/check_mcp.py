"""Exercise the configured MCP transports and real code/browser tools.

Run using .codex/.cache/mcp-serena/venv/bin/python after mcp/setup.py.
"""

import argparse
import asyncio
from datetime import timedelta
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import threading
import tomllib

from mcp import ClientSession, StdioServerParameters, types
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / ".codex"


async def roots(context):
    return types.ListRootsResult(roots=[types.Root(uri=ROOT.as_uri(), name="jarvis")])


def texts(result):
    value = "\n".join(item.text for item in result.content if item.type == "text")
    if result.isError:
        raise RuntimeError(f"MCP tool returned isError: {value[:2000]}")
    for link in re.findall(r"\[Snapshot\]\(([^)]+\.yml)\)", value):
        snapshot = (ROOT / link).resolve()
        snapshot.relative_to(BASE / ".tmp/mcp-playwright")
        value += "\n" + snapshot.read_text(encoding="utf-8")
    return value


async def exercise(name, config, page_url, discover):
    params = StdioServerParameters(command=config["command"], args=config["args"], cwd=str(ROOT / "crates"))
    # Deliberately start from a subdirectory to validate Git-root discovery.
    log = BASE / ".tmp" / f"check-mcp-{name}.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w", encoding="utf-8") as errors:
        async with stdio_client(params, errlog=errors) as (read, write):
            async with ClientSession(read, write, read_timeout_seconds=timedelta(seconds=120), list_roots_callback=roots) as session:
                initialized = await session.initialize()
                available = (await session.list_tools()).tools
                tool_names = {tool.name for tool in available}
                missing = set(config["enabled_tools"]) - tool_names
                if missing:
                    raise RuntimeError(f"Missing configured tools: {sorted(missing)}")
                if name == "serena" and tool_names != set(config["enabled_tools"]):
                    raise RuntimeError("Serena must expose only the configured read tools")
                report = {"server": name, "server_info": initialized.serverInfo.model_dump(), "protocol": initialized.protocolVersion, "tools_available": sorted(tool_names), "calls": []}
                if discover:
                    report["schemas"] = {tool.name: tool.inputSchema for tool in available if tool.name in config["enabled_tools"]}
                    return report

                async def call(tool, arguments, expected):
                    result = texts(await session.call_tool(tool, arguments))
                    if not all(value in result for value in expected):
                        raise AssertionError(f"{name}.{tool}: expected {expected!r}, got {result[:2000]}")
                    report["calls"].append({"tool": tool, "arguments": arguments, "assert_contains": expected, "result": "pass", "response_sha256": hashlib.sha256(result.encode()).hexdigest()})
                    return result

                if name == "serena":
                    await call("get_current_config", {}, ["jarvis", "planning"])
                    await call("get_symbols_overview", {"relative_path": "crates/jarvis-core/src/actions/apps.rs", "depth": 0}, ["open"])
                    await call("find_symbol", {"name_path_pattern": "open", "relative_path": "crates/jarvis-core/src/actions/apps.rs", "include_body": False}, ["start_line"])
                    await call("find_referencing_symbols", {"name_path": "open", "relative_path": "crates/jarvis-core/src/actions/apps.rs"}, ["actions.rs"])
                else:
                    await call("browser_navigate", {"url": page_url}, ["Проверить"])
                    await call("browser_click", {"target": "button", "element": "Проверить"}, ["MCP проверен"])
                    await call("browser_snapshot", {}, ["MCP проверен"])
                    await call("browser_take_screenshot", {"type": "png", "scale": "css", "filename": str(BASE / ".tmp/mcp-playwright/mcp-smoke.png"), "fullPage": True}, ["mcp-smoke.png"])
                    await call("browser_console_messages", {"level": "error"}, ["Errors: 0"])
                    await session.call_tool("browser_close", {})
                report["result"] = "pass"
                return report


class Page(BaseHTTPRequestHandler):
    def do_GET(self):
        payload = """<!doctype html><html lang="ru"><meta charset="utf-8"><title>Jarvis MCP smoke</title><body><h1>Проверка Playwright MCP</h1><button onclick="document.querySelector('[role=status]').textContent='MCP проверен'">Проверить</button><p role="status">Ожидание</p></body></html>""".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", choices=("all", "serena", "playwright"), default="all")
    parser.add_argument("--discover", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.resolve().relative_to(BASE)
    config = tomllib.loads((BASE / "config.toml").read_text())
    names = ("serena", "playwright") if args.server == "all" else (args.server,)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Page)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        reports = []
        for name in names:
            reports.append(await exercise(name, config["mcp_servers"][name], f"http://127.0.0.1:{server.server_port}/", args.discover))
    finally:
        server.shutdown()
        server.server_close()
    payload = {"provenance": "observed_stdio", "result": "discovered" if args.discover else "pass", "exit_code": 0, "servers": reports, "limits": ["Linux development MCP only; no Windows/GPU validation", "Playwright uses a local smoke page, not a Jarvis hardware session"]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    print(f"MCP checked: {', '.join(names)} -> {args.output}")


if __name__ == "__main__":
    asyncio.run(main())
