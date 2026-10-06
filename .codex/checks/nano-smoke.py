"""Observed CPU inference with pinned Nano weights and bundled Jarvis New reference."""

import hashlib
import io
import json
from pathlib import Path
import sys
import threading
import time
import urllib.request
import wave

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "voice-server"))
import nano
import server

model_root = ROOT / ".codex" / ".cache" / "nano-models"
download = nano.download_models
nano.download_models = lambda: download(model_root)
server.VOICES_DIR = ROOT / "resources" / "sound" / "voices"
voice = nano.NanoVoice()
assert "torch" not in sys.modules and "torchaudio" not in sys.modules
http = server.Server(("127.0.0.1", 0), server.make_handler(voice=voice, profile={"profile": "cpu"}))
threading.Thread(target=http.serve_forever, daemon=True).start()
base = f"http://127.0.0.1:{http.server_address[1]}"
with urllib.request.urlopen(base + "/health") as response:
    health = json.load(response)
assert health["tts"] and health["tts_engine"] == voice.engine and health["tts_revision"] == voice.revision
assert health["tts_reference"] == voice.reference_hash and health["tts_streaming"] is False
results = []
out = ROOT / ".codex" / ".tmp" / "nano-smoke"
out.mkdir(parents=True, exist_ok=True)
for index, text in enumerate(["Проверка голоса. Не сохранять файл, сэр.", "Открой папку загрузок и проверь обновления. " * 5]):
    start = time.perf_counter()
    request = urllib.request.Request(base + "/tts", data=json.dumps({"text": text, "language": "ru", "voice": "jarvis-remaster"}).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=120) as response:
        assert response.headers["Content-Type"] == "audio/wav"
        data = response.read()
    elapsed = time.perf_counter() - start
    with wave.open(io.BytesIO(data)) as wav:
        seconds = wav.getnframes() / wav.getframerate()
        assert wav.getframerate() == 24000 and wav.getnchannels() == 1 and wav.getsampwidth() == 2
        assert seconds > 0.2
    (out / f"sample-{index}.wav").write_bytes(data)
    results.append({"chars": len(text), "http_streaming": False, "seconds": seconds, "http_complete_ms": round(elapsed * 1000), "rtf": round(elapsed / seconds, 3), "wav_sha256": hashlib.sha256(data).hexdigest()})
proof = {"result": "pass", "exit_code": 0, "provenance": "observed_cpu_inference", "platform": sys.platform, "engine": voice.engine, "revision": voice.revision, "reference": voice.reference_hash, "torch_imported": "torch" in sys.modules, "measurements": results, "user_windows_quality_verified": False}
(out / "result.json").write_text(json.dumps(proof, indent=2) + "\n")
print(json.dumps(proof, indent=2))
http.shutdown()
http.server_close()
