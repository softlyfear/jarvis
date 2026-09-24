"""Local voice server for Jarvis: speech recognition (Whisper) and voice-clone TTS (XTTS-v2).

POST /stt  body: audio/wav (16 kHz mono PCM16)       ->  {"text": "..."}
POST /tts  {"text": "...", "language": "ru"}          ->  audio/wav
GET  /health                                          ->  {"ok": true, "stt": bool, "tts": bool}

Both models run on the GPU when CUDA is available. Either part can be switched off
(--no-stt / --no-tts); if a part fails to load, the server keeps running with the other.
Standard library HTTP server, no web framework.
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
XTTS_MODEL = "tts_models/multilingual/multi-dataset/xtts_v2"

# keep downloaded models next to the server: an ASCII install path avoids native
# loaders failing on non-Latin user profile paths, and uninstall removes them
os.environ.setdefault("HF_HOME", str(HERE / "models" / "hf"))
os.environ.setdefault("TTS_HOME", str(HERE / "models" / "tts"))
DEFAULT_REFS = [
    HERE.parent.parent / "resources" / "sound" / "voices" / "jarvis-og" / "ru",
    HERE / "voice",
]
MAX_CHARS = 180  # XTTS limit per chunk for Russian is ~182 characters
TTS_SAMPLE_RATE = 24000
STT_SAMPLE_RATE = 16000
MAX_STT_SECONDS = 30

# vocabulary hint for Whisper: the wake word and typical command words
STT_PROMPT = "Джарвис, открой Телеграм. Запусти Steam, Discord, Dota 2. Громкость пятьдесят."

# well-known Whisper hallucinations on silence and noise (Russian subtitles in training data)
HALLUCINATIONS = (
    "субтитр",
    "продолжение следует",
    "спасибо за просмотр",
    "подписывайтесь",
    "редактор субтитров",
    "dimatorzok",
    "amara.org",
)


def prepare_cuda_dlls():
    """On Windows, torch ships cuBLAS/cuDNN DLLs; importing it registers their folder
    so CTranslate2 (faster-whisper) finds them without a separate CUDA install."""
    try:
        import torch  # noqa: F401
    except Exception:
        pass


# ---------------------------------------------------------------- helpers


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


def to_wav_bytes(samples, sample_rate=TTS_SAMPLE_RATE):
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


def wav_to_float32(data):
    """Decode a 16 kHz PCM16 WAV into mono float32 samples in [-1, 1]."""
    import numpy as np

    with wave.open(io.BytesIO(data)) as w:
        if w.getsampwidth() != 2:
            raise ValueError("expected 16-bit PCM")
        if w.getframerate() != STT_SAMPLE_RATE:
            raise ValueError(f"expected {STT_SAMPLE_RATE} Hz, got {w.getframerate()}")
        channels = w.getnchannels()
        frames = w.readframes(min(w.getnframes(), STT_SAMPLE_RATE * MAX_STT_SECONDS))
    samples = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
    if channels > 1:
        samples = samples.reshape(-1, channels).mean(axis=1)
    return samples


def clean_transcript(text):
    """Drop known hallucinations, collapse whitespace."""
    text = " ".join(text.split())
    lower = text.lower()
    if any(h in lower for h in HALLUCINATIONS):
        return ""
    return text


# ---------------------------------------------------------------- models


def _silence(seconds):
    import numpy as np

    return np.zeros(int(STT_SAMPLE_RATE * seconds), dtype=np.float32)


class Recognizer:
    """Whisper via faster-whisper (CTranslate2)."""

    def __init__(self, model_name, device, compute_type):
        prepare_cuda_dlls()
        from faster_whisper import WhisperModel

        attempts = []
        if device in ("auto", "cuda"):
            attempts.append((model_name, "cuda", compute_type))
        if device in ("auto", "cpu"):
            # the full model is too slow on a weak CPU; "small" keeps latency acceptable
            cpu_model = model_name if device == "cpu" else "small"
            attempts.append((cpu_model, "cpu", "int8"))

        self.lock = threading.Lock()
        last_error = None
        for name, dev, compute in attempts:
            try:
                print(f"[stt] loading Whisper {name} on {dev} ({compute}) ...", flush=True)
                self.model = WhisperModel(name, device=dev, compute_type=compute)
                # CUDA libraries load lazily: warm up so a missing DLL fails here, not on the first command
                self.transcribe_samples(_silence(0.5))
                self.description = f"{name}/{dev}"
                print(f"[stt] ready: {self.description}", flush=True)
                return
            except Exception as e:  # try the next device
                last_error = e
                print(f"[stt] {name} on {dev} failed: {e}", flush=True)
        raise RuntimeError(f"Whisper could not be loaded: {last_error}")

    def transcribe_samples(self, samples, language="ru"):
        with self.lock:  # one GPU, one request at a time
            segments, _info = self.model.transcribe(
                samples,
                language=language,
                beam_size=5,
                vad_filter=True,
                condition_on_previous_text=False,
                initial_prompt=STT_PROMPT,
                no_speech_threshold=0.6,
            )
            text = " ".join(s.text.strip() for s in segments)
        return clean_transcript(text)

    def transcribe_wav(self, data, language="ru"):
        return self.transcribe_samples(wav_to_float32(data), language)


class Voice:
    """XTTS-v2 voice clone via coqui-tts."""

    def __init__(self, refs, device):
        import torch
        from TTS.api import TTS

        if device == "auto":
            device = "cuda" if torch.cuda.is_available() else "cpu"
        print(f"[tts] loading XTTS-v2 on {device} ...", flush=True)
        api = TTS(XTTS_MODEL).to(device)
        self.model = api.synthesizer.tts_model
        print(f"[tts] reference samples: {len(refs)}", flush=True)
        self.latent, self.embedding = self.model.get_conditioning_latents(audio_path=refs)
        self.lock = threading.Lock()

    def synthesize(self, text, language="ru"):
        import numpy as np

        parts = []
        silence = np.zeros(int(TTS_SAMPLE_RATE * 0.15), dtype=np.float32)
        with self.lock:
            for chunk in split_text(text):
                out = self.model.inference(chunk, language, self.latent, self.embedding, temperature=0.7)
                parts.append(np.asarray(out["wav"], dtype=np.float32))
                parts.append(silence)
        return to_wav_bytes(np.concatenate(parts) if parts else silence)


# ---------------------------------------------------------------- HTTP


def make_handler(recognizer=None, voice=None):
    class Handler(BaseHTTPRequestHandler):
        def _send(self, code, body, content_type):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _json(self, code, obj):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self._send(code, body, "application/json; charset=utf-8")

        def _body(self):
            length = int(self.headers.get("Content-Length", "0"))
            return self.rfile.read(length) if length > 0 else b""

        def do_GET(self):
            if self.path == "/health":
                self._json(200, {"ok": True, "stt": recognizer is not None, "tts": voice is not None})
            else:
                self._send(404, b"not found", "text/plain")

        def do_POST(self):
            path, _, query = self.path.partition("?")
            try:
                if path == "/stt":
                    if recognizer is None:
                        self._json(503, {"error": "speech recognition is disabled"})
                        return
                    language = "ru"
                    for pair in query.split("&"):
                        k, _, v = pair.partition("=")
                        if k == "language" and v:
                            language = v
                    text = recognizer.transcribe_wav(self._body(), language)
                    self._json(200, {"text": text})
                elif path == "/tts":
                    if voice is None:
                        self._json(503, {"error": "voice synthesis is disabled"})
                        return
                    data = json.loads(self._body() or b"{}")
                    text = str(data.get("text", "")).strip()
                    if not text:
                        self._json(400, {"error": "empty text"})
                        return
                    audio = voice.synthesize(text[:2000], str(data.get("language", "ru")))
                    self._send(200, audio, "audio/wav")
                else:
                    self._send(404, b"not found", "text/plain")
            except (ValueError, wave.Error, EOFError) as e:
                self._json(400, {"error": str(e)})
            except Exception as e:  # report instead of dropping the connection
                self._json(500, {"error": str(e)})

        def log_message(self, format, *args):  # noqa: A002 (base class signature)
            sys.stderr.write("[http] " + (format % args) + "\n")

    return Handler


def download(args):
    """Fetch model files ahead of time so the first voice command is not a multi-GB wait."""
    failed = False
    if not args.no_stt:
        try:
            from faster_whisper import download_model

            print(f"[stt] downloading Whisper {args.whisper_model} ...", flush=True)
            download_model(args.whisper_model)
        except Exception as e:
            failed = True
            print(f"[stt] download failed: {e}", flush=True)
    if not args.no_tts:
        try:
            from TTS.utils.manage import ModelManager

            print("[tts] downloading XTTS-v2 ...", flush=True)
            ModelManager().download_model(XTTS_MODEL)
        except Exception as e:
            failed = True
            print(f"[tts] download failed: {e}", flush=True)
    print("[server] download finished" + (" with errors" if failed else ""), flush=True)
    if failed:
        sys.exit(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=5055)
    ap.add_argument("--no-stt", action="store_true", help="do not load Whisper")
    ap.add_argument("--no-tts", action="store_true", help="do not load the voice clone")
    ap.add_argument("--whisper-model", default="large-v3-turbo", help="large-v3-turbo | medium | small | path")
    ap.add_argument("--whisper-device", default="auto", help="auto | cuda | cpu")
    ap.add_argument("--whisper-compute", default="int8_float16", help="GPU precision: int8_float16 | float16")
    ap.add_argument("--device", default="auto", help="TTS device: auto | cuda | cpu")
    ap.add_argument("--voice", action="append", help="WAV file or folder with reference samples (repeatable)")
    ap.add_argument("--download-only", action="store_true", help="download the models and exit (used by the installer)")
    args = ap.parse_args()

    if args.download_only:
        download(args)
        return

    recognizer = voice = None

    if not args.no_stt:
        try:
            recognizer = Recognizer(args.whisper_model, args.whisper_device, args.whisper_compute)
        except Exception as e:
            print(f"[stt] disabled: {e}", flush=True)

    if not args.no_tts:
        refs = find_reference_wavs(args.voice or DEFAULT_REFS)
        if not refs:
            print("[tts] disabled: no reference WAV files; pass --voice <folder with .wav>", flush=True)
        else:
            try:
                voice = Voice(refs, args.device)
            except Exception as e:
                print(f"[tts] disabled: {e}", flush=True)

    if recognizer is None and voice is None:
        sys.exit("nothing to serve: speech recognition and voice synthesis both failed or are disabled")

    server = ThreadingHTTPServer((args.host, args.port), make_handler(recognizer, voice))
    print(
        f"[server] ready on http://{args.host}:{args.port} "
        f"(stt={recognizer is not None}, tts={voice is not None})",
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
