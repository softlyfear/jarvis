#!/usr/bin/env bash
# Codex SessionStart: check Linux tools; install only on explicit --install.
set -u

log() { echo "[codex-session-start] $*" >&2; }
printf '%s\n' 'Read .codex/AGENTS.md in full before repository work. Claude configuration is read-only; all GPT instructions and service files belong inside .codex/.'

if [ "$(uname -s)" != "Linux" ]; then
  exit 0
fi

install_tools=false
case "${1:-}" in
  "") ;;
  --install) install_tools=true ;;
  *) log "usage: bash .codex/hooks/session-start.sh [--install]"; exit 1 ;;
esac

if command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then
  packages=()
  for package in libasound2-dev libssl-dev pkg-config gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64; do
    dpkg -s "$package" >/dev/null 2>&1 || packages+=("$package")
  done
  if [ "${#packages[@]}" -gt 0 ]; then
    log "missing packages: ${packages[*]}"
    if [ "$install_tools" = true ]; then
      if [ "$(id -u)" = 0 ]; then
        apt-get install -y -q "${packages[@]}" \
          || (apt-get update -q && apt-get install -y -q "${packages[@]}") \
          || log "apt install failed"
      else
        log "system packages need an authorized administrator installation"
      fi
    fi
  fi
fi

if command -v rustup >/dev/null 2>&1; then
  if ! rustup target list --installed 2>/dev/null | rg -q '^x86_64-pc-windows-gnu$'; then
    log "missing Rust target: x86_64-pc-windows-gnu"
    if [ "$install_tools" = true ]; then
      rustup target add x86_64-pc-windows-gnu || log "rustup target add failed"
    fi
  fi
  if ! rustup component list --installed 2>/dev/null | rg -q '^rust-analyzer'; then
    log "missing rust-analyzer"
    if [ "$install_tools" = true ]; then
      rustup component add rust-analyzer || log "rust-analyzer install failed"
    fi
  fi
else
  log "rustup is unavailable"
fi

# DOCS_RS comes from config.toml or the explicit check/test command.
# Never write Claude environment files or assume exports survive this process.
exit 0
