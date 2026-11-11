# Grok Build integration

Use the root `AGENTS.md` for shared project rules and procedures.

- Grok configuration lives in `.grok/`; skill adapters point to the canonical
  `.codex/skills/<name>/SKILL.md` procedures.
- Pass `DOCS_RS=1` explicitly to Cargo commands: `.grok/config.toml` does not
  inject it into the shell environment.
- The Grok reviewer definition is `.grok/agents/reviewer.md`. If the delegation
  tool cannot select a named role, pass its instructions explicitly.
- Use the existing shared evidence, plans, and memory paths under `.codex/`.
