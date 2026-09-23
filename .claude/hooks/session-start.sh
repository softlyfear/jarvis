#!/usr/bin/env bash
# SessionStart hook: prepares a Linux container (Claude Code on the web) to type-check
# the project natively and cross-check it for the Windows target. No-op locally on Windows.
set -u

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

log() { echo "[session-start] $*" >&2; }

if command -v apt-get >/dev/null 2>&1; then
  need=""
  dpkg -s libasound2-dev >/dev/null 2>&1 || need="$need libasound2-dev pkg-config"
  dpkg -s gcc-mingw-w64-x86-64 >/dev/null 2>&1 || need="$need gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64"
  if [ -n "$need" ]; then
    log "installing:$need"
    (apt-get install -y -q $need >/dev/null 2>&1 || (apt-get update -q >/dev/null 2>&1 && apt-get install -y -q $need >/dev/null 2>&1)) \
      || log "apt install failed, continuing"
  fi
fi

if command -v rustup >/dev/null 2>&1; then
  rustup target list --installed 2>/dev/null | grep -q x86_64-pc-windows-gnu \
    || rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || log "rustup target add failed"
  rustup component list --installed 2>/dev/null | grep -q rust-analyzer \
    || rustup component add rust-analyzer >/dev/null 2>&1 || log "rust-analyzer install failed"
fi

# ort-sys downloads ONNX Runtime binaries from a CDN that the web sandbox proxy may block;
# DOCS_RS=1 skips linking so `cargo check` still works.
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export DOCS_RS=1" >> "$CLAUDE_ENV_FILE"
fi

exit 0
