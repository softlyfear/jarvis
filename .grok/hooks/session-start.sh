#!/usr/bin/env bash
# Grok SessionStart: run the shared toolchain check, then point at Grok rules.
set -euo pipefail
IFS=$'\n\t'

# The shared check filters rustup output with rg. A missing rg makes the
# pipeline fail, so an installed target is reported as missing. grep -E covers
# the two fixed patterns without changing the shared script.
ensure_rg() {
  if command -v rg >/dev/null 2>&1; then
    return
  fi
  local shim
  shim="$(mktemp -d)"
  printf '%s\n' '#!/bin/sh' 'exec grep -E "$@"' >"${shim}/rg"
  chmod +x "${shim}/rg"
  PATH="${shim}:${PATH}"
  export PATH
  trap 'rm -rf "'"${shim}"'"' EXIT
}

main() {
  local root
  root="${GROK_WORKSPACE_ROOT:-${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}}"
  ensure_rg
  bash "${root}/.codex/hooks/session-start.sh" "$@" >/dev/null
  printf '%s\n' '{"systemMessage":"Grok: прочитай корневой AGENTS.md и .grok/rules/ перед работой. Настройки Grok лежат в .grok/. Общие процедуры остаются в .codex/."}'
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
