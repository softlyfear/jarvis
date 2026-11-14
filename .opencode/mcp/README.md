# Project MCP setup

Validated on Linux with OpenCode v2.3.0. These are development tools, not
dependencies of the Jarvis installer.

## Configuration

`opencode.jsonc` is the sole MCP configuration. OpenCode discovers root
`AGENTS.md`, `.agents/skills/`, and `.opencode/agents/reviewer.md` directly;
do not add duplicate instruction or skill entries.

| Server | Transport | Access |
|---|---|---|
| Serena 1.7.0 | Local stdio | Six read-only symbol/navigation tools |
| Playwright MCP 0.0.83 | Local stdio | Twelve browser tools; isolated headless Chromium |
| Context7 | `https://mcp.context7.com/mcp` | Public library documentation |
| Hugging Face | `https://huggingface.co/mcp` | Three public model/repository read tools |

OpenCode permission rules preserve the local and Hugging Face tool allowlists.
Serena also enforces a fixed read-only tool set server-side. Its OpenCode
context replaces the previous agent-specific context without enabling edits,
shell, memories, or onboarding. Rust analysis targets Windows GNU and disables
build scripts, proc macros, and check-on-save.

Launchers find this Git root even when started from a subdirectory. Runtime
state stays in ignored `.opencode/.cache/` and `.opencode/.tmp/`. Playwright
keeps `--headless --isolated --no-webmcp --no-sandbox` and the original
timeouts/viewport. The inherited `--no-sandbox` flag weakens Chromium isolation;
it does not connect to a personal browser or reuse personal cookies.

## Install and verify

Use existing Rust, Node, Bash, Git, uv, and `pyright-langserver` from PATH.
Dependencies are project-local and pinned by the Python requirements and npm
lockfile. TypeScript/Svelte language servers are maintained by Serena.

```sh
python3 .opencode/mcp/setup.py
opencode mcp list
.opencode/.cache/mcp-serena/venv/bin/python .agents/tools/check_mcp.py --output .opencode/.tmp/mcp-check.json
.opencode/.cache/mcp-serena/venv/bin/python .agents/tools/hf_mcp.py --tool hub_repo_details --arguments '{"repo_ids":["gpt2"],"repo_type":"model"}' --output .opencode/.tmp/hf-check.json
```

`setup.py --server serena|playwright` selects one runtime. Config changes reload
automatically; `/reload` forces a reload. A moved Python virtual environment
can contain obsolete absolute shebangs; recreate it locally with `setup.py`
if it is not usable, never add a compatibility link to the old directory.

The stdio check initializes both real servers, locates a Rust symbol and a
reference, clicks a local smoke-page button, checks its text, writes a screenshot,
and reads console messages. `--discover` only lists tools, not successful
behavior. These checks do not establish Windows/audio/GPU behavior.

## Authorization

No secrets or credential files were present in the project MCP configuration.
Local servers require no sign-in. Public Context7 and Hugging Face requests
worked anonymously during migration; external service limits may change.
No previous agent's connected Apps or account credentials are transferred.
GitHub remains available through the existing `gh` CLI, not an invented MCP.

If OpenCode reports `needs authentication`, use `/mcps`, select that server,
and complete sign-in. OAuth credentials stay outside project configuration.
Do not commit tokens. Use `{env:VARIABLE_NAME}` for a provider-required
header credential only when OAuth is unavailable or explicitly requested.
Anonymous success does not establish access to private or gated repositories.

## References

- https://opencode.ai/docs/config/
- https://opencode.ai/docs/mcp-servers/
- https://opencode.ai/docs/permissions/
- https://opencode.ai/docs/agents/
- https://opencode.ai/docs/skills/
- https://github.com/oraios/serena
- https://github.com/microsoft/playwright-mcp
