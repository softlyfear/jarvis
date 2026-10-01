"""Render the settings window in headless Chromium with Tauri IPC mocked.

Starts `vite` from frontend/ (unless --url is given), opens each route,
saves a screenshot per route and prints console errors and the Tauri
commands the page called. Exit code 1 if any page logged an error.

    uv run --no-project --with playwright python .claude/skills/gui-check/scripts/shoot.py
    ... shoot.py --routes / /settings --width 1100 --height 760 --out /tmp/shots
    ... shoot.py --routes / --state speaking   # orb with a fake jarvis-app speaking
    ... shoot.py --fixtures my.json   # override command results: {"cmd": value}
"""
import argparse
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[4]
FRONTEND = ROOT / "frontend"
LOCALES = ROOT / "crates/jarvis-core/src/i18n/locales"


def load_ftl(lang: str) -> dict[str, str]:
    """Flat key = value pairs of a Fluent file; indented lines continue the value."""
    result: dict[str, str] = {}
    key = None
    for line in (LOCALES / f"{lang}.ftl").read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if line[0].isspace() and key:
            result[key] = (result[key] + "\n" + line.strip()).strip()
        elif "=" in line:
            key, value = (part.strip() for part in line.split("=", 1))
            result[key] = value
    return result


# Results of Tauri commands the window calls on load. Unknown commands get null.
FIXTURES = {
    "get_app_version": "0.2.0-dev",
    "get_current_language": "ru",
    "get_supported_languages": ["ru", "en", "ua"],
    "is_jarvis_app_running": True,
    "get_commands_count": 42,
    "get_command_packs": [],
    "pv_get_audio_devices": ["Микрофон (Realtek Audio)", "Гарнитура (USB)"],
    "list_vosk_models": [],
    "list_gliner_models": [],
    "list_voices": [],
    "get_jarvis_app_stats": {"running": True, "ram_mb": 312, "cpu_usage": 3.5},
    "get_log_file_path": "C:\\Users\\user\\AppData\\Roaming\\com.priler.jarvis",
    "voice_server_status": {"installed": True, "running": True, "gpu": "cuda", "stt_engine": "faster-whisper", "tts_device": "cuda"},
    "update_status": {"phase": "idle", "version": "", "done": 0, "total": 0, "error": ""},
    "assistant_settings_read": {"kilo_key": "", "polza_key": "sk-test", "gateway": "polza", "free_only": False,
                                "stt_engine": "whisper", "tts_backend": "http", "address": "сэр"},
    "check_update": None,
}

# Runs before any page script: same shape as mockIPC() from @tauri-apps/api/mocks.
MOCK_JS = """
(() => {
  const fixtures = __FIXTURES__;
  const db = { language: "ru" };
  const calls = [];
  const callbacks = new Map();
  const listeners = new Map();
  window.__TAURI_CALLS__ = calls;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(cb, once) {
      const id = crypto.getRandomValues(new Uint32Array(1))[0];
      callbacks.set(id, d => { if (once) callbacks.delete(id); return cb && cb(d); });
      return id;
    },
    unregisterCallback(id) { callbacks.delete(id); },
    runCallback(id, d) { callbacks.get(id)?.(d); },
    callbacks,
    convertFileSrc: p => p,
    async invoke(cmd, args) {
      calls.push(cmd);
      if (cmd === "plugin:event|listen") {
        if (!listeners.has(args.event)) listeners.set(args.event, []);
        listeners.get(args.event).push(args.handler);
        return args.handler;
      }
      if (cmd === "plugin:event|emit") {
        for (const h of listeners.get(args.event) || []) callbacks.get(h)?.({ event: args.event, payload: args.payload });
        return null;
      }
      if (cmd === "db_read") return db[args.key] ?? null;
      if (cmd === "db_write_many") { Object.assign(db, args.values || args.data || {}); return null; }
      if (cmd in fixtures) return structuredClone(fixtures[cmd]);
      return null;
    },
  };

  // jarvis-app IPC (ws://127.0.0.1:9712): "off" = no app, otherwise a fake app in that state
  const state = __STATE__;
  if (state === "off") return;
  const RealSocket = window.WebSocket;
  class FakeSocket {
    static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
    constructor(url, protocols) {
      if (!String(url).startsWith("ws://127.0.0.1:")) return new RealSocket(url, protocols);  // vite HMR
      this.url = url; this.readyState = 0;
      setTimeout(() => {
        this.readyState = 1; this.onopen?.();
        const send = d => this.readyState === 1 && this.onmessage?.({ data: JSON.stringify(d) });
        if (state === "listening") send({ event: "listening" });
        if (state === "speaking") send({ event: "speaking", active: true });
        if (state === "listening" || state === "speaking") {
          let t = 0;
          this.timer = setInterval(() => {
            t += 1;
            const bands = Array.from({ length: 16 }, (_, i) => 0.25 + 0.5 * Math.abs(Math.sin(t / 5 + i / 2)));
            send({ event: "audio_level", level: 0.55 + 0.3 * Math.sin(t / 4), bands });
          }, 33);
        }
      }, 50);
    }
    send() {}
    close() { clearInterval(this.timer); this.readyState = 3; this.onclose?.(); }
  }
  window.WebSocket = FakeSocket;
})();
"""


def wait_port(port: int, timeout: float) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        with socket.socket() as s:
            if s.connect_ex(("127.0.0.1", port)) == 0:
                return
        time.sleep(0.3)
    raise SystemExit(f"vite did not open port {port} in {timeout:.0f} s")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--routes", nargs="+", default=["/", "/settings", "/commands"])
    ap.add_argument("--url", help="use an already running dev server instead of starting vite")
    ap.add_argument("--out", default=os.environ.get("TMPDIR", "/tmp") + "/jarvis-gui")
    ap.add_argument("--width", type=int, default=550)  # window size from tauri.conf.json
    ap.add_argument("--height", type=int, default=800)
    ap.add_argument("--fixtures", help="JSON file merged over the built-in command results")
    ap.add_argument("--state", choices=["off", "idle", "listening", "speaking"], default="idle",
                    help="fake jarvis-app behind the IPC WebSocket (off = app not running)")
    ap.add_argument("--wait-ms", type=int, default=1500, help="extra time for canvas animation and async loads")
    args = ap.parse_args()

    fixtures = dict(FIXTURES, get_translations=load_ftl("ru"))
    if args.fixtures:
        fixtures.update(json.loads(Path(args.fixtures).read_text(encoding="utf-8")))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    vite = None
    base = args.url
    if not base:
        vite = subprocess.Popen(["npx", "vite", "--port", "1420", "--strictPort"], cwd=FRONTEND,
                                stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
        base = "http://localhost:1420"
    failed = False
    try:
        if vite:
            wait_port(1420, 60)
        with sync_playwright() as p:
            browser = p.chromium.launch(headless=True)
            page = browser.new_page(viewport={"width": args.width, "height": args.height})
            page.add_init_script(MOCK_JS.replace("__FIXTURES__", json.dumps(fixtures)).replace("__STATE__", json.dumps(args.state)))
            errors: list[str] = []
            page.on("console", lambda m: m.type == "error" and errors.append(m.text))
            page.on("pageerror", lambda e: errors.append(f"pageerror: {e}"))
            for route in args.routes:
                errors.clear()
                page.goto(base + route)
                page.wait_for_load_state("networkidle")
                page.wait_for_timeout(args.wait_ms)
                name = route.strip("/").replace("/", "_") or "index"
                shot = out / f"{name}.png"
                page.screenshot(path=str(shot), full_page=True)
                calls = sorted(set(page.evaluate("window.__TAURI_CALLS__")))
                print(f"{route}: {shot}")
                print(f"  invoke: {', '.join(calls) or '-'}")
                # with --state off the WebSocket to jarvis-app fails, as on a PC without the app
                real = [e for e in errors if not (args.state == "off" and ("[IPC]" in e or "ws://127.0.0.1:" in e))]
                for e in real:
                    print(f"  ERROR {e}")
                failed |= bool(real)
            browser.close()
    finally:
        if vite:
            vite.terminate()
            vite.wait(10)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
