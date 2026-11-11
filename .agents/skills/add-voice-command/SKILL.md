---
name: add-voice-command
description: "Add or change Jarvis command phrases, action packs, Lua commands, native actions, and their LLM tools."
---

# Add a voice command

Read root `AGENTS.md`. Choose the smallest existing mechanism:

| Need | Mechanism |
|---|---|
| Open a named app, game, or folder | Alias in `[apps]`, `[games]`, or `[folders]` of `crates/jarvis-core/assets/assistant.default.toml` |
| New phrase for an existing action | `resources/commands/<pack>/command.toml` |
| New OS operation | `Action`, `from_voice_command`, and an LLM tool when useful |
| Network/state logic without Windows APIs | Lua, following `resources/commands/weather` |
| Recorded response only | `type = "voice"` and `sounds.ru` names from the voice resources |

Command entries use unique IDs, `type`, an existing `action` ID, optional
`args`, `sounds.ru`, and language-keyed `phrases.ru` / `phrases.en`.
`phrases` must be a language map: an array prevents the pack from loading.
Object slots such as `{app}` are handled by `text::extract_object`.
Provide varied conversational formulations for the intent classifier and avoid
overlap with other packs. A changed phrase hash retrains it on the next launch.

For a new action, update `enum Action`, `Action::execute`, and, when required,
`is_dangerous` / `confirmation_question`. Implement it in `actions/*.rs` behind
Windows guards with appropriate fallbacks. Route user text through process
arguments or `platform::powershell(script, env)`. Update `from_voice_command`
and LLM `definitions` / `to_action`. Add parsing/tool tests; the bundled-pack
test verifies that packs parse and reference known actions.

Run the applicable root checks, including:

```sh
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
```
