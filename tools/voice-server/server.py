"""Local voice server for Jarvis: speech recognition (Whisper) and voice-clone TTS (XTTS-v2).

POST /stt  body: audio/wav (16 kHz mono PCM16)       ->  {"text": "..."}
POST /tts  {"text": "...", "language": "ru"}          ->  audio/wav
GET  /health                                          ->  {"ok": true, "stt": bool, "tts": bool, ...}

How the models run depends on the graphics card (gpu.py, saved by the installer in
gpu-profile.json): NVIDIA uses faster-whisper and XTTS on CUDA; AMD, Intel and machines
without a GPU use whisper.cpp (Vulkan or CPU) for speech and XTTS on ROCm or the CPU.
Either part can be switched off (--no-stt / --no-tts); if a part fails to load, the
server keeps running with the other. Standard library HTTP server, no web framework.
"""

import argparse
import atexit
import ctypes
import io
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import gpu

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

# whisper.cpp (AMD, Intel, CPU): same Whisper weights as faster-whisper, 8-bit like its int8_float16
WHISPERCPP_DIR = HERE / "whispercpp"
WHISPERCPP_MODELS_DIR = HERE / "models" / "whispercpp"
WHISPERCPP_URL = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"
WHISPERCPP_MODELS = {
    "large-v3-turbo": ("ggml-large-v3-turbo-q8_0.bin", 874188075),
    "medium": ("ggml-medium-q8_0.bin", 823369779),
    "small": ("ggml-small-q8_0.bin", 264464607),
}

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
    """Whisper via faster-whisper (CTranslate2): NVIDIA GPUs, or the CPU."""

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
                self.description = f"faster-whisper {name}/{dev}"
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


def whispercpp_model(name):
    """(path, expected size or None) for a model name or a path to a ggml .bin file."""
    if name in WHISPERCPP_MODELS:
        file, size = WHISPERCPP_MODELS[name]
        return WHISPERCPP_MODELS_DIR / file, size
    return Path(name), None


def whispercpp_exe():
    return WHISPERCPP_DIR / ("whisper-server.exe" if os.name == "nt" else "whisper-server")


def download_file(url, dest, size=None):
    """Download with resume into dest (via dest.part); prints progress for the installer window."""
    dest = Path(dest)
    if dest.exists() and (size is None or dest.stat().st_size == size):
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    have = part.stat().st_size if part.exists() else 0
    request = urllib.request.Request(url, headers={"Range": f"bytes={have}-"} if have else {})
    with urllib.request.urlopen(request, timeout=60) as response:
        if have and response.status != 206:  # server ignored the range: start over
            have = 0
        total = size or (have + int(response.headers.get("Content-Length", "0")))
        shown = -1
        with open(part, "ab" if have else "wb") as out:
            while True:
                block = response.read(1 << 20)
                if not block:
                    break
                out.write(block)
                have += len(block)
                if total:
                    percent = have * 100 // total
                    if percent // 10 != shown:
                        shown = percent // 10
                        print(f"[download] {dest.name}: {percent}%", flush=True)
    if size is not None and have != size:
        raise IOError(f"{dest.name}: got {have} bytes, expected {size}")
    os.replace(part, dest)
    return dest


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def multipart(fields):
    """Encode {name: str | (filename, bytes)} as multipart/form-data."""
    boundary = uuid.uuid4().hex
    parts = []
    for name, value in fields.items():
        if isinstance(value, tuple):
            filename, data = value
            head = f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\nContent-Type: audio/wav'
        else:
            head, data = f'Content-Disposition: form-data; name="{name}"', str(value).encode("utf-8")
        parts.append(f"--{boundary}\r\n{head}\r\n\r\n".encode("utf-8") + data + b"\r\n")
    body = b"".join(parts) + f"--{boundary}--\r\n".encode("utf-8")
    return body, f"multipart/form-data; boundary={boundary}"


_JOBS = []  # job handles must stay open for the lifetime of the server


def kill_with_parent(process):
    """Windows: put the child into a job object that is closed (and the child killed) when
    this process exits for any reason, so whisper-server never outlives the voice server."""
    if os.name != "nt":
        return
    try:
        from ctypes import wintypes

        class BasicLimits(ctypes.Structure):
            _fields_ = [
                ("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD), ("SchedulingClass", wintypes.DWORD),
            ]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [
                ("BasicLimitInformation", BasicLimits), ("IoInfo", ctypes.c_uint64 * 6),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.CreateJobObjectW.restype = wintypes.HANDLE
        kernel32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        job = kernel32.CreateJobObjectW(None, None)
        info = ExtendedLimits()
        info.BasicLimitInformation.LimitFlags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        ok = job and kernel32.SetInformationJobObject(job, 9, ctypes.byref(info), ctypes.sizeof(info))
        ok = ok and kernel32.AssignProcessToJobObject(job, int(process._handle))
        if ok:
            _JOBS.append(job)
        else:
            print(f"[stt] job object not set (error {ctypes.get_last_error()})", flush=True)
    except Exception as e:  # never fatal: the process still works, it just may outlive a crash
        print(f"[stt] job object not set: {e}", flush=True)


class WhisperCppRecognizer:
    """Whisper via a whisper.cpp server process (Vulkan on any GPU vendor, or the CPU)."""

    def __init__(self, model_name, use_gpu=True, exe=None, ready_timeout=300):
        self.exe = Path(exe) if exe else whispercpp_exe()
        if not self.exe.exists():
            raise RuntimeError(f"{self.exe} not found")
        self.model, size = whispercpp_model(model_name)
        if not self.model.exists():
            print(f"[stt] downloading {self.model.name} ...", flush=True)
            download_file(WHISPERCPP_URL + self.model.name, self.model, size)
        self.model_name = model_name
        self.ready_timeout = ready_timeout
        self.lock = threading.Lock()
        self.process = None
        atexit.register(self.stop)

        attempts = [True, False] if use_gpu else [False]
        last_error = None
        for gpu_on in attempts:
            try:
                self._start(gpu_on)
                self.transcribe_samples(_silence(0.5))  # Vulkan compiles its shaders on the first run
                print(f"[stt] ready: {self.description}", flush=True)
                return
            except Exception as e:
                last_error = e
                print(f"[stt] whisper.cpp ({'GPU' if gpu_on else 'CPU'}) failed: {e}", flush=True)
                self.stop()
        raise RuntimeError(f"whisper.cpp could not be started: {last_error}")

    def _start(self, gpu_on):
        self.port = free_port()
        env = dict(os.environ)
        device = "CPU"
        if gpu_on:
            devices = gpu.vulkan_devices()
            index = gpu.pick_vulkan_device(devices)
            if index is None:
                raise RuntimeError("no Vulkan GPU")
            env["GGML_VK_VISIBLE_DEVICES"] = str(index)
            device = f"Vulkan: {dict((i, n) for i, n, _t in devices)[index]}"
        args = [
            str(self.exe), "-m", str(self.model), "--host", "127.0.0.1", "--port", str(self.port),
            "-l", "ru", "-bs", "5", "-nth", "0.6",
        ]
        if not gpu_on:
            args.append("-ng")
        print(f"[stt] starting whisper.cpp {self.model.name} on {device}", flush=True)
        self.process = subprocess.Popen(
            args,
            cwd=str(self.exe.parent),  # ggml backends (ggml-vulkan.dll, ggml-cpu-*.dll) load from here
            env=env,
            stdin=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        kill_with_parent(self.process)
        self.description = f"whisper.cpp {self.model_name}/{'vulkan' if gpu_on else 'cpu'}"
        self.device = device
        deadline = time.monotonic() + self.ready_timeout
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"whisper-server exited with code {self.process.returncode}")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/", timeout=2):
                    return
            except (urllib.error.URLError, OSError):
                time.sleep(0.5)
        raise RuntimeError("whisper-server did not start in time")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.kill()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass

    def _post(self, wav_bytes, language):
        body, content_type = multipart(
            {"file": ("audio.wav", wav_bytes), "language": language, "response_format": "json", "prompt": STT_PROMPT}
        )
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/inference", data=body, headers={"Content-Type": content_type}
        )
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read().decode("utf-8", "replace"))

    def transcribe_samples(self, samples, language="ru"):
        wav_bytes = to_wav_bytes(samples, STT_SAMPLE_RATE)
        with self.lock:
            try:
                result = self._post(wav_bytes, language)
            except (urllib.error.URLError, ConnectionError) as e:
                if self.process is None or self.process.poll() is None:
                    raise
                # the process died (driver reset, killed by an update): start it again once
                print(f"[stt] whisper-server stopped ({e}), restarting", flush=True)
                self._start("vulkan" in self.description)
                result = self._post(wav_bytes, language)
        if "error" in result:
            raise RuntimeError(result["error"])
        return clean_transcript(str(result.get("text", "")))

    def transcribe_wav(self, data, language="ru"):
        return self.transcribe_samples(wav_to_float32(data), language)


def read_wav(path):
    """Mono float32 samples and the sample rate of a PCM WAV file (standard library + numpy)."""
    import numpy as np

    with wave.open(str(path)) as w:
        width, channels, rate = w.getsampwidth(), w.getnchannels(), w.getframerate()
        frames = w.readframes(w.getnframes())
    if width == 2:
        samples = np.frombuffer(frames, dtype="<i2").astype(np.float32) / 32768.0
    elif width == 4:
        samples = np.frombuffer(frames, dtype="<i4").astype(np.float32) / 2147483648.0
    elif width == 3:
        raw = np.frombuffer(frames, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        ints = (raw[:, 0] | (raw[:, 1] << 8) | (raw[:, 2] << 16)) << 8 >> 8
        samples = ints.astype(np.float32) / 8388608.0
    elif width == 1:
        samples = (np.frombuffer(frames, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    else:
        raise ValueError(f"unsupported sample width {width}")
    if channels > 1:
        samples = samples.reshape(-1, channels).mean(axis=1)
    return samples, rate


def resample(samples, rate, target):
    """Resample mono float32 audio; torchaudio's resampler when available (what coqui-tts uses)."""
    import numpy as np

    if rate == target:
        return samples
    try:
        import torch
        import torchaudio

        return torchaudio.functional.resample(torch.from_numpy(samples), rate, target).numpy()
    except Exception:
        pass
    try:
        from math import gcd

        from scipy.signal import resample_poly

        g = gcd(rate, target)
        return resample_poly(samples, target // g, rate // g).astype(np.float32)
    except Exception:
        n = int(round(len(samples) * target / rate))
        return np.interp(np.linspace(0, len(samples) - 1, n), np.arange(len(samples)), samples).astype(np.float32)


def allow_tts_without_torchcodec():
    """coqui-tts refuses to import with torch >= 2.9 unless torchcodec is installed, only
    because torchaudio.load needs it; XTTS reads WAV references through
    patch_xtts_audio_loader instead, so the check is satisfied before `import TTS`."""
    try:
        from transformers.utils import import_utils
    except Exception:
        return
    import_utils.is_torchcodec_available = lambda: True


def patch_xtts_audio_loader():
    """coqui-tts reads the reference voice with torchaudio.load, which from torchaudio 2.9
    needs torchcodec and FFmpeg. AMD ROCm builds of PyTorch are newer than that, so WAV
    references are read here instead; other formats still go to the original loader."""
    import torch
    from TTS.tts.models import xtts

    original = xtts.load_audio

    def load_audio(audiopath, sampling_rate):
        try:
            samples, rate = read_wav(audiopath)
        except (wave.Error, ValueError, EOFError):
            return original(audiopath, sampling_rate)
        audio = torch.from_numpy(resample(samples, rate, sampling_rate).copy()).unsqueeze(0)
        return audio.clip_(-1, 1)

    xtts.load_audio = load_audio


def pick_torch_device(torch, requested):
    """"cpu", an explicit device, or for "auto" the GPU with the most memory (a Ryzen iGPU
    is also visible to ROCm next to the Radeon card)."""
    if requested != "auto":
        return requested
    if not torch.cuda.is_available():
        return "cpu"
    count = torch.cuda.device_count()
    best = max(range(count), key=lambda i: torch.cuda.get_device_properties(i).total_memory)
    return f"cuda:{best}"


class Voice:
    """XTTS-v2 voice clone via coqui-tts."""

    def __init__(self, refs, device):
        import torch

        allow_tts_without_torchcodec()
        from TTS.api import TTS

        patch_xtts_audio_loader()
        device = pick_torch_device(torch, device)
        self.lock = threading.Lock()
        attempts = [device] if device == "cpu" else [device, "cpu"]
        last_error = None
        for dev in attempts:
            try:
                name = torch.cuda.get_device_name(dev) if dev.startswith("cuda") else "CPU"
                backend = "ROCm" if getattr(torch.version, "hip", None) else "CUDA"
                print(f"[tts] loading XTTS-v2 on {dev} ({name}) ...", flush=True)
                api = TTS(XTTS_MODEL).to(dev)
                self.model = api.synthesizer.tts_model
                print(f"[tts] reference samples: {len(refs)}", flush=True)
                self.latent, self.embedding = self.model.get_conditioning_latents(audio_path=refs)
                # a GPU that loads the model but fails in inference (driver, missing kernels) falls back here
                self.model.inference("Готов.", "ru", self.latent, self.embedding, temperature=0.7)
                self.device = "cpu" if dev == "cpu" else f"{backend} {name}"
                print(f"[tts] ready on {self.device}", flush=True)
                return
            except Exception as e:
                last_error = e
                print(f"[tts] XTTS on {dev} failed: {e}", flush=True)
                self.model = None
                if dev != "cpu":
                    torch.cuda.empty_cache()
        raise RuntimeError(f"XTTS could not be loaded: {last_error}")

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


def make_handler(recognizer=None, voice=None, profile=None):
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
                self._json(
                    200,
                    {
                        "ok": True,
                        "stt": recognizer is not None,
                        "tts": voice is not None,
                        "stt_engine": getattr(recognizer, "description", None),
                        "tts_device": getattr(voice, "device", None),
                        "gpu": (profile or {}).get("gpu"),
                        "profile": (profile or {}).get("profile"),
                    },
                )
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


def resolve_engine(args, profile):
    """(engine, model) for speech recognition: the profile decides unless set explicitly."""
    engine = args.stt_engine if args.stt_engine != "auto" else profile["stt"]
    if engine == "whispercpp" and not whispercpp_exe().exists():
        print(f"[stt] {whispercpp_exe()} not found, using faster-whisper", flush=True)
        engine = "faster-whisper"
    model = args.whisper_model
    if model == "auto":
        # the large model is too slow without a GPU; "small" keeps latency acceptable
        model = "small" if profile["profile"] == "cpu" else "large-v3-turbo"
    return engine, model


def download(args, profile):
    """Fetch model files ahead of time so the first voice command is not a multi-GB wait."""
    failed = False
    if not args.no_stt:
        engine, model = resolve_engine(args, profile)
        try:
            if engine == "whispercpp":
                path, size = whispercpp_model(model)
                print(f"[stt] downloading {path.name} ...", flush=True)
                download_file(WHISPERCPP_URL + path.name, path, size)
            else:
                from faster_whisper import download_model

                print(f"[stt] downloading Whisper {model} ...", flush=True)
                download_model(model)
        except Exception as e:
            failed = True
            print(f"[stt] download failed: {e}", flush=True)
    if not args.no_tts:
        try:
            allow_tts_without_torchcodec()
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
    ap.add_argument("--stt-engine", default="auto", help="auto (by graphics card) | faster-whisper | whispercpp")
    ap.add_argument("--whisper-model", default="auto", help="auto | large-v3-turbo | small | path to a model")
    ap.add_argument("--whisper-device", default="auto", help="faster-whisper: auto | cuda | cpu; whisper.cpp: auto | cpu")
    ap.add_argument("--whisper-compute", default="int8_float16", help="faster-whisper GPU precision: int8_float16 | float16")
    ap.add_argument("--device", default="auto", help="TTS device: auto | cuda | cuda:1 | cpu")
    ap.add_argument("--voice", action="append", help="WAV file or folder with reference samples (repeatable)")
    ap.add_argument("--download-only", action="store_true", help="download the models and exit (used by the installer)")
    args = ap.parse_args()

    profile = gpu.load_profile(fallback=gpu.detect)
    print(f"[server] graphics: {profile.get('gpu') or 'not found'} -> profile {profile['profile']}", flush=True)

    if args.download_only:
        download(args, profile)
        return

    recognizer = voice = None

    if not args.no_stt:
        engine, model = resolve_engine(args, profile)
        try:
            if engine == "whispercpp":
                use_gpu = args.whisper_device != "cpu" and profile["profile"] != "cpu"
                recognizer = WhisperCppRecognizer(model, use_gpu=use_gpu)
            else:
                recognizer = Recognizer(model, args.whisper_device, args.whisper_compute)
        except Exception as e:
            print(f"[stt] disabled: {e}", flush=True)

    if not args.no_tts:
        refs = find_reference_wavs(args.voice or DEFAULT_REFS)
        if not refs:
            print("[tts] disabled: no reference WAV files; pass --voice <folder with .wav>", flush=True)
        else:
            device = args.device
            if device == "auto" and profile["tts_device"] == "cpu":
                device = "cpu"
            try:
                voice = Voice(refs, device)
            except Exception as e:
                print(f"[tts] disabled: {e}", flush=True)

    if recognizer is None and voice is None:
        sys.exit("nothing to serve: speech recognition and voice synthesis both failed or are disabled")

    server = ThreadingHTTPServer((args.host, args.port), make_handler(recognizer, voice, profile))
    print(
        f"[server] ready on http://{args.host}:{args.port} "
        f"(stt={getattr(recognizer, 'description', None)}, tts={getattr(voice, 'device', None)})",
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
