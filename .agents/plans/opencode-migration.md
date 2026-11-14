# OpenCode migration

## Scope

Current repository only, installed OpenCode v2.3.0. Global configuration,
application source, credentials, neighboring repositories, and Git history
are not migration targets.

## Outcomes and evidence

1. **Implemented:** one native `opencode.jsonc`; root `AGENTS.md` remains the
   project contract. Six `.agents/skills/` are discovered without wrappers.
   The native reviewer cannot edit, execute shell commands, or delegate.
2. **Implemented:** project tools/tests/checks, plans, scenarios, research,
   memory, reviews, and evidence moved to `.agents/`. MCP launchers and ignored
   caches/tmp moved to `.opencode/`. CI uses the new paths and no legacy hooks.
3. **Verified:** Serena and Playwright real stdio checks passed
   (`.opencode/.tmp/opencode-mcp-check.json`); Hugging Face public repository
   details succeeded (`.opencode/.tmp/opencode-hf-check.json`). Context7
   initialization/tool listing and `resolve-library-id` succeeded anonymously.
   `opencode mcp list` reports all four connected. No account sign-in was needed
   for these public operations; private/gated access is not verified.
4. **Verified:** migrated unit tests, migration regression tests, and diff
   whitespace checks passed: 63 tests, one optional numeric dependency test
   skipped. No Windows/GPU claims are made. An additional Python Playwright
   UI smoke attempt reached the migrated helper but could not launch because
   its Chromium revision 1255 is not installed. This does not affect the
   separately pinned and successfully tested Playwright MCP browser.
5. **Verified:** remaining old agent files were backed up outside Git, compared
   by SHA-256, and removed only after explicit confirmation of `.codex/` and
   `.grok/`. An original tracked-files archive supplements the residual-state
   backup. Local original caches/tmp are preserved, not deleted.
6. **Historical limitation:** original snapshots retain their bytes and
   provenance. `.agents/evidence/opencode-migration/snapshot-baseline.json`
   records byte comparison with the pre-migration Git blobs. Some older
   manifest hashes were already inconsistent; migration does not refresh
   historical claims. Live scripts and CI do not depend on old agent configs.
7. **Publication gate:** a master push triggers the installer release workflow.
   Do not publish a new release merely to migrate development configuration.
8. **Reviewed:** the independent read-only reviewer approved the full relevant
   6258-line diff after fixing shell approval, reviewer restrictions, historical
   snapshot references, and obsolete setup instructions. Backup and effective
   runtime permission enforcement were not independently tested by that reviewer.

## Documentation

Configuration was checked against the installed CLI help and official V2
guides for config, instructions, agents, skills, permissions, and MCP:
https://opencode.ai/docs/llms.txt

Setup and authorization: `.opencode/mcp/README.md`.
