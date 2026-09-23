"""Local voice-clone TTS server for Jarvis (XTTS-v2 via the coqui-tts package).

POST /tts  {"text": "...", "language": "ru"}  ->  audio/wav
GET  /health                                ->  {"ok": true}

The reference voice is built once at startup from WAV samples (by default the original
Jarvis phrases in resources/sound/voices/jarvis-og/ru). Standard library HTTP server,
no extra web framework.
"""

import argparse
import io
import json
import os
import re
import sys
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# XTTS-v2 weights are distributed under the Coqui Public Model License (non-commercial)
os.environ.setdefault("COQUI_TOS_AGREED", "1")

HERE = Path(__file__).resolve().parent
DEFAULT_REFS = [
    HERE.parent.parent / "resources" / "sound" / "voices" / "jarvis-og" / "ru",
    HERE / "voice",
]
MAX_CHARS = 180  # XTTS limit per chunk for Russian is ~182 characters
SAMPLE_RATE = 24000


def find_reference_wavs(paths):
    wavs = []
    for p in paths:
        p = Path(p)
        if p.is_file() and p.suffix.lower() == ".wav":
            wavs.append(str(p))
        elif p.is_dir():
            wavs.extend(str(f) for f in sorted(p.glob("*.wav")))
    return wavs


def split_text(text, limit=MAX_CHARS):
    """Split into sentence chunks no longer than `limit` characters."""
    sentences = re.split(r"(?<=[.!?…])\s+", text.strip())
    chunks, current = [], ""
    for s in sentences:
        while len(s) > limit:
            cut = s.rfind(" ", 0, limit)
            cut = cut if cut > 0 else limit
            if current:
                chunks.append(current)
                current = ""
            chunks.append(s[:cut].strip())
            s = s[cut:].strip()
        if len(current) + len(s) + 1 <= limit:
            current = f"{current} {s}".strip()
        else:
            if current:
                chunks.append(current)
            current = s
    if current:
        chunks.append(current)
    return [c for c in chunks if c]


def to_wav_bytes(samples, sample_rate=SAMPLE_RATE):
    import numpy as np

    pcm = np.clip(np.asarray(samples, dtype=np.float32), -1.0, 1.0)
    pcm = (pcm * 32767).astype(np.int16)
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


class Voice:
    def __init__(self, refs, device):
        import torch
        from TTS.api import TTS

        if device == "auto":
            device = "cuda" if torch.cuda.is_available() else "cpu"
        print(f"[tts] loading XTTS-v2 on {device} ...", flush=True)
        api = TTS("tts_models/multilingual/multi-dataset/xtts_v2").to(device)
        self.model = api.synthesizer.tts_model
        print(f"[tts] reference samples: {len(refs)}", flush=True)
        self.latent, self.embedding = self.model.get_conditioning_latents(audio_path=refs)
        self.lock = threading.Lock()

    def synthesize(self, text, language="ru"):
        import numpy as np

        parts = []
        silence = np.zeros(int(SAMPLE_RATE * 0.15), dtype=np.float32)
        with self.lock:  # one GPU, one request at a time
            for chunk in split_text(text):
                out = self.model.inference(chunk, language, self.latent, self.embedding, temperature=0.7)
                parts.append(np.asarray(out["wav"], dtype=np.float32))
                parts.append(silence)
        return to_wav_bytes(np.concatenate(parts) if parts else silence)


def make_handler(voice):
    class Handler(BaseHTTPRequestHandler):
        def _send(self, code, body, content_type):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/health":
                self._send(200, b'{"ok": true}', "application/json")
            else:
                self._send(404, b"not found", "text/plain")

        def do_POST(self):
            if self.path != "/tts":
                self._send(404, b"not found", "text/plain")
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                data = json.loads(self.rfile.read(length) or b"{}")
                text = str(data.get("text", "")).strip()
                if not text:
                    self._send(400, b"empty text", "text/plain")
                    return
                audio = voice.synthesize(text[:2000], str(data.get("language", "ru")))
                self._send(200, audio, "audio/wav")
            except Exception as e:  # report instead of dropping the connection
                self._send(500, f"error: {e}".encode("utf-8"), "text/plain; charset=utf-8")

        def log_message(self, fmt, *args):
            sys.stderr.write("[tts] " + (fmt % args) + "\n")

    return Handler


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=5055)
    ap.add_argument("--device", default="auto", help="auto | cuda | cpu")
    ap.add_argument("--voice", action="append", help="WAV file or folder with reference samples (repeatable)")
    args = ap.parse_args()

    refs = find_reference_wavs(args.voice or DEFAULT_REFS)
    if not refs:
        sys.exit("no reference WAV files found; pass --voice <folder with .wav>")

    voice = Voice(refs, args.device)
    server = ThreadingHTTPServer((args.host, args.port), make_handler(voice))
    print(f"[tts] ready on http://{args.host}:{args.port}/tts", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
