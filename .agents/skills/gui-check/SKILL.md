---
name: gui-check
description: "Inspect mocked Jarvis settings in Chromium with screenshots and console checks after frontend or UI-text changes."
---

# Inspect the settings UI

Read root `AGENTS.md`. The Tauri 2/Svelte frontend can run in Linux Chromium
with mocked Tauri IPC and a fake jarvis-app WebSocket. Helper scripts live
beside this skill in `scripts/`.

From the repository root:

```sh
mkdir -p "$PWD/.opencode/.tmp"
export TMPDIR="$PWD/.opencode/.tmp"
export UV_CACHE_DIR="$PWD/.opencode/.cache/uv"
export PLAYWRIGHT_BROWSERS_PATH="$PWD/.opencode/.cache/playwright"
export PYTHONDONTWRITEBYTECODE=1
uv run --no-project --with playwright python .agents/skills/gui-check/scripts/shoot.py
```

Use `--routes / --state speaking` for the speaking orb, or
`--fixtures .opencode/.tmp/gui/fixtures.json` for custom command responses.
States are `off|idle|listening|speaking`, with fake audio levels/16 bands near
30 Hz. Default size comes from `tauri.conf.json` (550x800). Console page errors
cause exit 1. Open the screenshots using an available image viewer: type checks
cannot verify layout or a visible orb.

If necessary, install project-scoped Chromium with the same cache environment
using `uv run --no-project --with playwright python -m playwright install chromium`.
For missing frontend dependencies run `npm ci --cache ../.opencode/.cache/npm`
inside `frontend/`. Screenshot defaults, `--out`, and JSON fixtures stay inside
`.opencode/` or `.agents/`; the helper rejects paths outside those directories.

Run `uv run --no-project --with playwright python
.agents/skills/gui-check/scripts/settings_check.py` for the five settings
sections, keyboard navigation, 550x800/400x800 wrapping, persisted values after
section changes, gateway keys, save errors, memory, and model states.
It uses fictional keys/mocked IPC. It does not prove real configuration writes
or Windows hardware behavior.

Unknown Tauri commands return `null`. If a page fails on that value, extend
the helper's `FIXTURES` using current Rust serde fields under
`crates/jarvis-gui/src/tauri_commands/`. Translations come from
`crates/jarvis-core/src/i18n/locales/ru.ftl`.
Native windows, tray/file dialogs, real Rust invokes, and audio need target
system evidence.

Continue the existing dark style, reactor orb, and green settings accents.
Keep the orb the main expressive element; animation should reflect state or
interaction and respect reduced motion. Check long Russian labels, wrapping,
button bounds, visible keyboard focus, and contrast in the narrow window.
Russian UI copy names user outcomes, uses consistent action verbs, explains
concrete failures and recovery, and gives empty states a useful next step.
