"""Render the settings window in headless Chromium with Tauri IPC mocked.

Starts `vite` from frontend/ (unless --url is given), opens each route,
saves a screenshot per route and prints console errors and the Tauri
commands the page called. Exit code 1 if any page logged an error.

    uv run --no-project --with playwright python .codex/skills/gui-check/scripts/shoot.py
    ... shoot.py --routes / /settings --width 1100 --height 760 --out .codex/.tmp/gui/shots
    ... shoot.py --routes / --state speaking   # orb with a fake jarvis-app speaking
    ... shoot.py --fixtures .codex/.tmp/gui/fixtures.json   # override command results: {"cmd": value}
"""
import argparse
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
CODEX = ROOT / ".codex"
os.environ.setdefault("PLAYWRIGHT_BROWSERS_PATH", str(CODEX / ".cache/playwright"))

from playwright.sync_api import sync_playwright
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
    "get_jarvis_app_stats": {"running": True, "ram_mb": 312, "cpu_usage": 3.5},
    "get_log_file_path": "C:\\Users\\user\\AppData\\Roaming\\com.priler.jarvis",
    "voice_server_status": {"installed": True, "running": True, "gpu": "cuda", "stt_engine": "faster-whisper", "tts_device": "F5-TTS, CUDA", "tts_error": None},
    "update_status": {"phase": "idle", "version": "", "done": 0, "total": 0, "error": ""},
    "assistant_settings_read": {"kilo_key": "", "polza_key": "sk-test", "gateway": "polza", "free_only": False,
                                "stt_engine": "whisper", "tts_backend": "http", "address": "сэр"},
    "check_update": None,
}

# Runs before any page script: same shape as mockIPC() from @tauri-apps/api/mocks.
MOCK_JS = """
(() => {
  const fixtures = __FIXTURES__;
  const db = { language: "ru", selected_microphone: "-1",
               selected_wake_word_engine: "Vosk", selected_vosk_model: "",
               noise_suppression: "None", gain_normalizer: "false" };
  const calls = [];
  const callbacks = new Map();
  const listeners = new Map();
  window.__TAURI_CALLS__ = calls;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, id) {
      listeners.set(event, (listeners.get(event) || []).filter(h => h !== id));
      callbacks.delete(id);
    },
  };
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
      if (cmd === "db_read") return db[args.key] ?? "";
      if (cmd === "db_write") { db[args.key] = args.val; return null; }
      if (cmd === "db_write_many") {
        for (const [key, value] of args.values || []) db[key] = value;
        return null;
      }
      if (cmd in fixtures) return structuredClone(fixtures[cmd]);
      return null;
    },
  };

  // jarvis-app IPC (ws://127.0.0.1:9712): "off" = no app, otherwise a fake app in that state
  const state = __STATE__;
  const RealSocket = window.WebSocket;
  class FakeSocket {
    static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
    constructor(url, protocols) {
      const target = new URL(url, window.location.href);
      if (target.hostname !== "127.0.0.1" || target.port !== "9712") return new RealSocket(url, protocols);  // vite HMR
      this.url = url; this.readyState = 0;
      setTimeout(() => {
        if (this.readyState !== 0) return;
        if (state === "off") {
          this.readyState = 3;
          this.onerror?.({ type: "error" });
          this.onclose?.({ code: 1006, reason: "Mock jarvis-app is off" });
          return;
        }
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
    ap.add_argument("--out", default=str(CODEX / ".tmp/gui"))
    ap.add_argument("--width", type=int, default=550)  # window size from tauri.conf.json
    ap.add_argument("--height", type=int, default=800)
    ap.add_argument("--fixtures", help="JSON file merged over the built-in command results")
    ap.add_argument("--state", choices=["off", "idle", "listening", "speaking"], default="idle",
                    help="fake jarvis-app behind the IPC WebSocket (off = app not running)")
    ap.add_argument("--wait-ms", type=int, default=1500, help="extra time for canvas animation and async loads")
    args = ap.parse_args()

    out = Path(args.out).resolve()
    fixture_path = Path(args.fixtures).resolve() if args.fixtures else None
    for path in [out, fixture_path]:
        if path is not None and not path.is_relative_to(CODEX.resolve()):
            ap.error("--out and --fixtures must be inside the repository .codex/")
    fixtures = dict(FIXTURES, get_translations=load_ftl("ru"))
    if args.state == "off":
        fixtures.update(is_jarvis_app_running=False,
                        get_jarvis_app_stats={"running": False, "ram_mb": 0, "cpu_usage": 0})
    if fixture_path:
        fixtures.update(json.loads(fixture_path.read_text(encoding="utf-8")))
    out.mkdir(parents=True, exist_ok=True)
    runtime_tmp = CODEX / ".tmp"
    runtime_tmp.mkdir(parents=True, exist_ok=True)
    os.environ["TMPDIR"] = str(runtime_tmp)

    vite = None
    base = args.url
    if not base:
        vite_bin = FRONTEND / "node_modules/vite/bin/vite.js"
        if not vite_bin.is_file():
            ap.error("frontend/node_modules is missing: run npm ci --cache ../.codex/.cache/npm in frontend/")
        vite = subprocess.Popen(["node", str(vite_bin), "--host", "127.0.0.1", "--port", "1420", "--strictPort"],
                                cwd=FRONTEND, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
        base = "http://127.0.0.1:1420"
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
            page.on("pageerror", lambda e: errors.append(f"pageerror: {e.stack or e}"))
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
