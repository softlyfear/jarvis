"""Local voice server for Jarvis: speech recognition (Whisper) and voice-clone TTS
(Jarvis New, F5-TTS ESpeech RL-V2 on GPU).

POST /stt  body: audio/wav (16 kHz mono PCM16)       ->  {"text": "..."}
POST /tts  {"text": "...", "language": "ru", "voice": "jarvis-remaster"}  ->  audio/wav
GET  /health                                          ->  {"ok": true, "stt": bool, "tts": bool, ...}

How the models run depends on the graphics card (gpu.py, saved by the installer in
gpu-profile.json): NVIDIA uses faster-whisper on CUDA; other profiles use whisper.cpp
(Vulkan or CPU) for speech. F5 voice synthesis requires CUDA or ROCm.
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
from urllib.parse import parse_qs
import uuid
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import gpu

HERE = Path(__file__).resolve().parent

# ESpeech Russian F5-TTS fine-tune (Apache 2.0).
F5_REPO = "ESpeech/ESpeech-TTS-1_RL-V2"
F5_CHECKPOINT = "espeech_tts_rlv2.pt"
F5_VOCAB = "vocab.txt"
F5_CONFIG = dict(dim=1024, depth=22, heads=16, ff_mult=2, text_dim=512, conv_layers=4)
F5_STEPS = 16  # 32 by default: no better by ear or by the scores, twice the time
F5_REFERENCE_SECONDS = 12.0  # shorter references lose the voice, longer ones only cost time
RUACCENT_DIR = HERE / "models" / "ruaccent"

# keep downloaded models next to the server: an ASCII install path avoids native
# loaders failing on non-Latin user profile paths, and uninstall removes them
os.environ.setdefault("HF_HOME", str(HERE / "models" / "hf"))
# the embeddable Python has no tkinter: matplotlib (imported by F5) must not look for it
os.environ.setdefault("MPLBACKEND", "Agg")
# caches of voice libraries also stay here instead of the user profile
_CACHE = HERE / "models" / "cache"
for _var, _sub in (
    ("XDG_CACHE_HOME", ""),
    ("TORCH_HOME", "torch"),
    ("MPLCONFIGDIR", "matplotlib"),
    ("NUMBA_CACHE_DIR", "numba"),
    ("MIOPEN_USER_DB_PATH", "miopen"),
    ("MIOPEN_CUSTOM_CACHE_DIR", "miopen"),
    ("TRITON_CACHE_DIR", "triton"),
):
    os.environ.setdefault(_var, str(_CACHE / _sub) if _sub else str(_CACHE))
# The sole bundled voice, Jarvis New. Requests cannot select another pack or reference.
VOICES_DIR = HERE.parent.parent / "resources" / "sound" / "voices"
DEFAULT_VOICE = "jarvis-remaster"
REFERENCE_SUFFIXES = (".wav", ".mp3")
GPU_REQUIRED = "Для F5-TTS нужна NVIDIA с CUDA или поддерживаемая AMD с ROCm. Синтез на CPU отключён; записанные отклики и распознавание остаются доступны."

TTS_SAMPLE_RATE = 24000
STT_SAMPLE_RATE = 16000
MAX_STT_SECONDS = 30
MAX_REQUEST_BYTES = 4 * 1024 * 1024

# whisper.cpp (AMD, Intel, CPU): same Whisper weights as faster-whisper, 8-bit like its int8_float16
WHISPERCPP_DIR = HERE / "whispercpp"
WHISPERCPP_MODELS_DIR = HERE / "models" / "whispercpp"
WHISPERCPP_URL = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"
WHISPERCPP_MODELS = {
    "large-v3-turbo": ("ggml-large-v3-turbo-q8_0.bin", 874188075),
    "medium": ("ggml-medium-q8_0.bin", 823369779),
    "small": ("ggml-small-q8_0.bin", 264464607),
}

# vocabulary hint for Whisper: the wake word and names it misspells. Only names, no commands:
# on noise Whisper sometimes repeats its prompt, and "Громкость пятьдесят." from the old prompt
# set the volume to 50% (log of 02.10.2026)
STT_PROMPT = "Джарвис. Блокнот, Телеграм, Steam, Discord, Chrome, Dota 2."
_PROMPT_WORDS = set(re.findall(r"\w+", STT_PROMPT.lower()))

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


def valid_language(language):
    return isinstance(language, str) and re.fullmatch(r"[a-z]{2,3}(?:-[a-z]{2,4})?", language) is not None


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
    """Drop known hallucinations and an echo of the prompt, collapse whitespace."""
    text = " ".join(text.split())
    lower = text.lower()
    if any(h in lower for h in HALLUCINATIONS):
        return ""
    words = re.findall(r"\w+", lower)
    if len(words) >= 2 and all(w in _PROMPT_WORDS for w in words):
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
    if size is not None and have == size:
        os.replace(part, dest)
        return dest
    if size is not None and have > size:
        part.unlink()
        have = 0
    request = urllib.request.Request(url, headers={"Range": f"bytes={have}-"} if have else {})
    with urllib.request.urlopen(request, timeout=60) as response:
        if response.status == 206:
            content_range = response.headers.get("Content-Range", "")
            match = re.fullmatch(r"bytes (\d+)-(\d+)/(\d+|\*)", content_range)
            if not match or int(match[1]) != have:
                raise IOError("invalid Content-Range while resuming download")
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
    if total and have != total:
        raise IOError(f"{dest.name}: got {have} bytes, expected {total}")
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
        if not self.model.exists() or (size is not None and self.model.stat().st_size != size):
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
    """Resample mono float32 audio; torchaudio's resampler when available."""
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


def read_audio(path):
    """Mono float32 samples and rate: WAV by the standard library, MP3 by soundfile
    (libsndfile decodes MP3)."""
    try:
        return read_wav(path)
    except (wave.Error, ValueError, EOFError):
        import numpy as np
        import soundfile

        samples, rate = soundfile.read(str(path), dtype="float32", always_2d=True)
        return np.ascontiguousarray(samples.mean(axis=1)), rate


def pick_torch_device(torch, requested, gfx=None):
    """Require CUDA/ROCm and prefer the discrete card over a Ryzen iGPU."""
    if requested == "cpu" or not torch.cuda.is_available() or torch.cuda.device_count() == 0:
        raise RuntimeError(GPU_REQUIRED)
    if requested != "auto":
        if not re.fullmatch(r"cuda(?::[0-9]+)?", requested):
            raise ValueError("TTS device must be auto, cuda or cuda:<index>")
        index = int(requested.split(":")[1]) if ":" in requested else 0
        if index >= torch.cuda.device_count():
            raise ValueError("TTS GPU index is unavailable")
        return requested

    def score(i):
        props = torch.cuda.get_device_properties(i)
        arch = (getattr(props, "gcnArchName", "") or "").split(":")[0]
        name = props.name or ""
        integrated = bool(getattr(props, "is_integrated", False)) or (
            "radeon" in name.lower() and not gpu.is_discrete_amd(name, arch or None)
        )
        return (bool(gfx) and arch == gfx, not integrated, props.total_memory)

    best = max(range(torch.cuda.device_count()), key=score)
    return f"cuda:{best}"


def voice_reference():
    """The fixed Jarvis New clips and their exact Russian text."""
    import tomllib

    pack = VOICES_DIR / DEFAULT_VOICE
    with open(pack / "voice.toml", "rb") as f:
        entry = tomllib.load(f)["tts"]["ru"]
    text = entry["text"].strip()
    if not text or not entry["reference"]:
        raise ValueError("Jarvis New reference must include clips and text")
    files = []
    for name in entry["reference"]:
        if not isinstance(name, str) or not re.fullmatch(r"[a-zA-Z0-9_-]+", name):
            raise ValueError("invalid Jarvis New reference name")
        found = next((pack / "ru" / f"{name}{ext}" for ext in REFERENCE_SUFFIXES
                      if (pack / "ru" / f"{name}{ext}").is_file()), None)
        if found is None:
            raise ValueError(f"Jarvis New reference clip is missing: {name}")
        files.append(str(found))
    return files, text


def trim_silence(samples, rate, threshold=0.02, pad=0.05):
    """Drops quiet edges (dub clips start and end with room tone)."""
    import numpy as np

    if len(samples) == 0:
        return samples
    loud = np.flatnonzero(np.abs(samples) > threshold * max(1e-6, float(np.abs(samples).max())))
    if loud.size == 0:
        return samples
    margin = int(pad * rate)
    return samples[max(0, loud[0] - margin): loud[-1] + margin + 1]


def build_reference(files, rate=TTS_SAMPLE_RATE, max_seconds=F5_REFERENCE_SECONDS, gap=0.25, resampler=None):
    """The clips joined into one reference of at most max_seconds: (samples, the files used).
    A clip that does not fit is skipped rather than cut mid-word."""
    import numpy as np

    parts, used, total = [], [], 0.0
    silence = np.zeros(int(gap * rate), dtype=np.float32)
    for f in files:
        samples, r = read_audio(f)
        clip = trim_silence((resampler or resample)(samples, r, rate).astype(np.float32), rate)
        seconds = len(clip) / rate + (gap if parts else 0)
        if total + seconds > max_seconds:
            continue
        if parts:
            parts.append(silence)
        parts.append(clip)
        used.append(f)
        total += seconds
    if not parts:
        raise ValueError("no reference clip fits")
    ref = np.concatenate(parts)
    return (ref / max(1e-6, float(np.abs(ref).max())) * 0.9).astype(np.float32), used


VOCOS_REPO = "charactr/vocos-mel-24khz"


def hub_file(repo, name):
    """A model file from the local cache, the network only when it is missing: Hugging Face is
    slow or unreachable from some networks, and every check of a cached file costs a timeout."""
    from huggingface_hub import hf_hub_download

    try:
        return hf_hub_download(repo, name, local_files_only=True)
    except Exception:
        return hf_hub_download(repo, name)


def patch_torchaudio_load():
    """F5 reads its reference with torchaudio.load, which from torchaudio 2.9 needs torchcodec
    and FFmpeg (ROCm builds are that new): read it with soundfile instead."""
    import torch
    import torchaudio

    def load(path, *args, **kwargs):
        samples, rate = read_audio(path)
        return torch.from_numpy(samples.copy()).unsqueeze(0), rate

    torchaudio.load = load


class F5Voice:
    """F5-TTS (ESpeech Russian fine-tune) with stresses from RUAccent."""

    engine = "F5-TTS"

    def __init__(self, device="auto", gfx=None):
        import torch

        self.dev = pick_torch_device(torch, device, gfx)
        patch_torchaudio_load()
        from f5_tts.infer.utils_infer import load_model, load_vocoder
        from f5_tts.model import DiT

        if getattr(torch.version, "hip", None):
            torch.backends.cudnn.enabled = False  # Windows ROCm lacks MIOpen kernel compilation
        self.lock = threading.Lock()
        print(f"[tts] loading F5-TTS ({F5_REPO}) on {self.dev} ...", flush=True)
        self.model = load_model(
            DiT, F5_CONFIG, hub_file(F5_REPO, F5_CHECKPOINT), vocab_file=hub_file(F5_REPO, F5_VOCAB), device=self.dev,
        )
        vocos_dir = Path(hub_file(VOCOS_REPO, "config.yaml")).parent
        hub_file(VOCOS_REPO, "pytorch_model.bin")
        self.vocoder = load_vocoder(is_local=True, local_path=str(vocos_dir), device=self.dev)
        self.accent = load_ruaccent()
        self.reference = self._reference()
        # A failed GPU warm-up disables synthesis; there is no CPU fallback.
        self._infer("Готов.", self.reference)
        backend = "ROCm" if getattr(torch.version, "hip", None) else "CUDA"
        self.device = f"F5-TTS, {backend} {torch.cuda.get_device_name(self.dev)}"
        print(f"[tts] ready: {self.device}", flush=True)

    def _stressed(self, text):
        if self.accent is None:
            return text
        try:
            return self.accent.process_all(text)
        except Exception as e:  # a word the models choke on must not cost the whole reply
            print(f"[tts] stresses skipped: {e}", flush=True)
            return text

    def _reference(self):
        """Prepare the sole voice once; all clips must match the provided transcript."""
        from f5_tts.infer.utils_infer import preprocess_ref_audio_text

        files, text = voice_reference()
        samples, used = build_reference(files)
        if used != files:
            raise ValueError("Jarvis New reference exceeds the duration limit")
        path = HERE / "models" / "cache" / "f5-reference-jarvis-new.wav"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(to_wav_bytes(samples))
        return preprocess_ref_audio_text(str(path), self._stressed(text), show_info=lambda *a, **k: None)

    def _infer(self, text, reference):
        import numpy as np
        from f5_tts.infer.utils_infer import infer_process

        audio, ref_text = reference
        wav, rate, _ = infer_process(
            audio, ref_text, self._stressed(text), self.model, self.vocoder,
            nfe_step=F5_STEPS, show_info=lambda *a, **k: None, device=self.dev,
        )
        return resample(np.asarray(wav, dtype=np.float32), rate, TTS_SAMPLE_RATE)

    def synthesize(self, text, language="ru", voice_id=None):
        with self.lock:
            return to_wav_bytes(self._infer(text, self.reference))


def load_ruaccent():
    """Stresses for F5 ("з+амок"): without them it guesses and often misses. Optional."""
    try:
        from ruaccent import RUAccent

        accent = RUAccent()
        RUACCENT_DIR.mkdir(parents=True, exist_ok=True)
        accent.load(omograph_model_size="turbo3.1", use_dictionary=True, workdir=str(RUACCENT_DIR))
        accent.process_all("Проверка.")
        return accent
    except Exception as e:
        print(f"[tts] RUAccent unavailable ({e}), speaking without stress marks", flush=True)
        return None


def load_voice(profile, device="auto", factory=None, engine="f5"):
    """F5 is the sole synthesizer and requires a working CUDA/ROCm GPU."""
    if engine != "f5":
        raise ValueError("F5 is the only supported TTS engine")
    if profile["profile"] not in ("cuda", "rocm") or device == "cpu":
        raise RuntimeError(GPU_REQUIRED)
    return (factory or F5Voice)(device, profile.get("gfx"))


# ---------------------------------------------------------------- HTTP


class RequestError(ValueError):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def make_handler(recognizer=None, voice=None, profile=None, tts_error=None):
    tts_state = {"error": tts_error}

    class Handler(BaseHTTPRequestHandler):
        def setup(self):
            super().setup()
            self.connection.settimeout(15)

        def _send(self, code, body, content_type):
            try:
                self.send_response(code)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except (ConnectionError, TimeoutError):
                # A cancelled client cannot receive a second error response.
                self.close_connection = True

        def _json(self, code, obj):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self._send(code, body, "application/json; charset=utf-8")

        def _body(self):
            if self.headers.get("Transfer-Encoding") is not None:
                raise RequestError(400, "Transfer-Encoding is unsupported")
            raw = self.headers.get("Content-Length")
            if raw is None:
                raise RequestError(411, "Content-Length is required")
            if not re.fullmatch(r"[0-9]+", raw):
                raise RequestError(400, "invalid Content-Length")
            length = int(raw)
            if length > MAX_REQUEST_BYTES:
                raise RequestError(413, "request body is too large")
            try:
                body = self.rfile.read(length)
            except TimeoutError:
                raise RequestError(408, "request body timed out") from None
            if len(body) != length:
                raise RequestError(400, "incomplete request body")
            return body

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
                        "tts_engine": getattr(voice, "engine", None),
                        "tts_revision": getattr(voice, "revision", None),
                        "tts_reference": getattr(voice, "reference_hash", None),
                        "tts_streaming": False,
                        "tts_error": tts_state["error"],
                        "gpu": (profile or {}).get("gpu"),
                        "profile": (profile or {}).get("profile"),
                    },
                )
            else:
                self._send(404, b"not found", "text/plain")

        def do_POST(self):
            path, _, query = self.path.partition("?")
            try:
                # Only native Jarvis clients use these endpoints. A website must not
                # drive GPU inference or choose reference paths via the local server.
                if self.headers.get("Origin") is not None:
                    raise RequestError(403, "browser requests are not allowed")
                if path == "/stt":
                    if recognizer is None:
                        self._json(503, {"error": "speech recognition is disabled"})
                        return
                    language = parse_qs(query).get("language", ["ru"])[0]
                    if not valid_language(language):
                        raise RequestError(400, "invalid language")
                    text = recognizer.transcribe_wav(self._body(), language)
                    self._json(200, {"text": text})
                elif path == "/tts":
                    if voice is None:
                        self._json(503, {"error": tts_error or "Синтез голоса выключен."})
                        return
                    data = json.loads(self._body() or b"{}")
                    if not isinstance(data, dict) or not isinstance(data.get("text"), str):
                        raise RequestError(400, "expected an object with string text")
                    text = data["text"].strip()
                    if not text:
                        self._json(400, {"error": "empty text"})
                        return
                    language = data.get("language", "ru")
                    if not valid_language(language):
                        raise RequestError(400, "invalid language")
                    if language != "ru" or data.get("voice") not in (None, "", DEFAULT_VOICE):
                        raise RequestError(400, "Only Jarvis New in Russian is supported")
                    try:
                        audio = voice.synthesize(text[:2000], language, DEFAULT_VOICE)
                    except Exception as e:
                        tts_state["error"] = str(e)
                        print(f"[tts] synthesis failed ({getattr(voice, 'engine', 'unknown')}): {e}", flush=True)
                        raise RuntimeError(str(e)) from e
                    tts_state["error"] = None
                    self._send(200, audio, "audio/wav")
                else:
                    self._send(404, b"not found", "text/plain")
            except RequestError as e:
                self._json(e.code, {"error": str(e)})
            except (ValueError, wave.Error, EOFError) as e:
                self._json(400, {"error": str(e)})
            except (ConnectionError, TimeoutError):
                self.close_connection = True
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
    if not args.no_tts and profile["profile"] in ("cuda", "rocm"):
        try:
            print("[tts] downloading F5-TTS (ESpeech) for Jarvis New ...", flush=True)
            for name in (F5_CHECKPOINT, F5_VOCAB):
                hub_file(F5_REPO, name)
            hub_file(VOCOS_REPO, "config.yaml")
            hub_file(VOCOS_REPO, "pytorch_model.bin")
            print("[tts] downloading RUAccent ...", flush=True)
            if load_ruaccent() is None:
                failed = True
        except Exception as e:
            failed = True
            print(f"[tts] F5 download failed: {e}", flush=True)
    if not args.no_tts and profile["profile"] not in ("cuda", "rocm"):
        print(f"[tts] unavailable: {GPU_REQUIRED}", flush=True)
    print("[server] download finished" + (" with errors" if failed else ""), flush=True)
    if failed:
        sys.exit(1)


class Server(ThreadingHTTPServer):
    # a second copy of the server must fail to bind instead of sharing the port
    # (Windows lets SO_REUSEADDR steal a bound port; SO_EXCLUSIVEADDRUSE forbids it)
    allow_reuse_address = False
    daemon_threads = True

    def server_bind(self):
        if os.name == "nt" and hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        super().server_bind()


def reserve_port(host, port):
    """Binds the port before the models load (minutes): a second copy exits at once, and
    until listen() clients get "connection refused" instead of hanging."""
    server = Server((host, port), BaseHTTPRequestHandler, bind_and_activate=False)
    try:
        server.server_bind()
    except OSError:
        server.server_close()
        return None
    return server


def app_is_running(name):
    if os.name == "nt":
        out = subprocess.run(
            ["tasklist", "/FI", f"IMAGENAME eq {name}.exe", "/NH", "/FO", "CSV"],
            capture_output=True, text=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        ).stdout
        return f'"{name}.exe"'.lower() in out.lower()
    return subprocess.run(["pgrep", "-x", name], capture_output=True).returncode == 0


def exit_with_app(name, grace=20, poll=5, is_running=app_is_running):
    """Started by Jarvis: quit when no Jarvis process is left. The grace period lets the
    server live through a restart of Jarvis (settings saved) without reloading the models."""
    def watch():
        gone_since = None
        while True:
            time.sleep(poll)
            try:
                alive = is_running(name)
            except OSError:
                alive = True
            if alive:
                gone_since = None
            elif gone_since is None:
                gone_since = time.monotonic()
            elif time.monotonic() - gone_since >= grace:
                print(f"[server] {name} is not running, exiting", flush=True)
                os._exit(0)

    threading.Thread(target=watch, daemon=True).start()


def default_tts_engine():
    # Ignore selection files left by old installations; updates migrate them to F5.
    return "f5"


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
    ap.add_argument("--tts-engine", choices=("f5",), default=default_tts_engine(), help=argparse.SUPPRESS)
    ap.add_argument("--device", default="auto", help="F5 GPU: auto | cuda | cuda:1 (CUDA or ROCm required)")
    ap.add_argument("--download-only", action="store_true", help="download the models and exit (used by the installer)")
    ap.add_argument("--exit-with-app", metavar="NAME", help="quit when no process NAME is running (used by Jarvis)")
    args = ap.parse_args()

    profile = gpu.load_profile(fallback=gpu.detect)
    print(f"[server] graphics: {profile.get('gpu') or 'not found'} -> profile {profile['profile']}", flush=True)

    if args.download_only:
        download(args, profile)
        return

    server = reserve_port(args.host, args.port)
    if server is None:
        print(f"[server] port {args.port} is taken: the voice server is already running", flush=True)
        return
    if args.exit_with_app:
        exit_with_app(args.exit_with_app)

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

    tts_error = "Синтез голоса выключен." if args.no_tts else None
    if not args.no_tts:
        try:
            voice = load_voice(profile, args.device, engine=args.tts_engine)
        except Exception as e:
            tts_error = str(e)
            print(f"[tts] disabled: {tts_error}", flush=True)

    # Even with both models unavailable, /health explains the failure to the settings UI.
    server.RequestHandlerClass = make_handler(recognizer, voice, profile, tts_error)
    server.server_activate()
    print(
        f"[server] ready on http://{args.host}:{args.port} "
        f"(stt={getattr(recognizer, 'description', None)}, tts={getattr(voice, 'device', None)})",
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
