---
name: llm-providers
description: "Configure or debug Jarvis LLM fallback, OpenRouter/Polza routing, provider cooldowns, tool calling, prompts, and history."
---

# Maintain LLM fallback

Read root `AGENTS.md` and current `crates/jarvis-core/src/llm.rs`,
`llm/tools.rs`, and `crates/jarvis-core/assets/assistant.default.toml`.
Use current code for model constants, gateway selection, cooldown durations,
and migrations instead of a historical provider catalog.

`app.rs::ask_llm` runs when no command matches or an action returns
`ActionError::NotFound`. `complete_with` tries providers/models and rotates keys.
Preserve distinct key/model, key, and provider cooldowns, `Retry-After`, and
HTTP status handling. HTTP failures discovered before a truncated body must
retain their cooldown. Network failures skip a provider for three seconds and
refresh the remote HTTP pool without shortening existing cooldowns.

Gateways are OpenRouter and Polza. Keep `gateway_of_key` and `configure.ps1`
consistent, retain the selected gateway first, migrate legacy Kilo/paid blocks
to OpenRouter, reject retired Kilo JWTs, and remove/ignore obsolete Gemini
blocks as current code requires. Do not revive the retired Kilo endpoint.
Free OpenRouter fallback is a separate provider so a paid-model cooldown does
not disable it; it still needs a key. Preserve `free_fallback` and `free_only`
semantics. Verify changing model IDs, tool support, quotas, and regional access
against dated official provider evidence rather than assuming old limits.

LLM tool calling has at most five rounds; a dangerous action stops for its
confirmation. History follows `memory_minutes` and starts with a user message.
Read provider/model/key-index failures from sanitized logs; reproduce using
Linux `mock_server` / `queue_server` tests, without real credentials.

For another compatible provider use the existing `[[llm.providers]]` fields
(`name`, `base_url` without `/chat/completions`, `models`, `keys`, `keyless`).
Verify `/chat/completions` tool-call support and current catalog metadata.
Compare candidate models on Jarvis requests with the complete
`tools::definitions()`, including actions, timers, typing, facts, and ASR errors.
Measure median/p90 latency sequentially: parallel trials previously triggered
incidental 403/429 failures. Do not treat a provider catalog as measured behavior.
