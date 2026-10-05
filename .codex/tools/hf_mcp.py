"""Call current public Hugging Face MCP read tools and preserve research evidence."""

import argparse
import asyncio
from datetime import timedelta
import json
from pathlib import Path
import tomllib

import httpx
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client
from check_evidence import successful_mcp

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / ".codex"


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--discover", action="store_true")
    parser.add_argument("--tool", choices=("hub_repo_search", "hub_repo_details", "hf_fs"))
    parser.add_argument("--arguments", default="{}", help="JSON object; never include credentials")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.resolve().relative_to(BASE)
    if not args.discover and not args.tool:
        parser.error("--tool is required unless --discover is set")
    parameters = json.loads(args.arguments)
    if not isinstance(parameters, dict):
        parser.error("--arguments must be a JSON object")
    config = tomllib.loads((BASE / "config.toml").read_text())["mcp_servers"]["hugging_face_hub"]
    async with httpx.AsyncClient(timeout=config["tool_timeout_sec"], follow_redirects=True) as client:
        async with streamable_http_client(config["url"], http_client=client) as (read, write, _):
            async with ClientSession(read, write, read_timeout_seconds=timedelta(seconds=config["tool_timeout_sec"])) as session:
                initialized = await session.initialize()
                available = await session.list_tools()
                missing = set(config["enabled_tools"]) - {tool.name for tool in available.tools}
                if missing:
                    raise RuntimeError(f"Configured HF tools not found: {sorted(missing)}")
                if args.discover:
                    result = available.model_dump(mode="json")
                else:
                    result = await session.call_tool(args.tool, parameters)
                    if result.isError:
                        raise RuntimeError("HF MCP call failed: " + result.model_dump_json()[:2000])
                    result = result.model_dump(mode="json")
    payload = {"provenance": "observed_http_mcp", "server_info": initialized.serverInfo.model_dump(mode="json"), "protocol": initialized.protocolVersion, "tool": args.tool, "arguments": parameters, "result": "discovered" if args.discover else "pass", "exit_code": 0, "response": result}
    if not args.discover and not successful_mcp(payload):
        raise RuntimeError("HF MCP returned an incomplete or unsuccessful operation batch")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    print(f"Hugging Face MCP: {args.tool or 'tools/list'} -> {args.output}")


if __name__ == "__main__":
    asyncio.run(main())
