"""Jarvis New adapter for the pinned OpenMOSS ONNX runtime (no PyTorch).

The HTTP contract remains a complete WAV. Incremental codec decoding here does
not mean the Windows client plays audio before the HTTP request completes.
"""

import hashlib
import json
from math import gcd
from pathlib import Path
import re
import threading
import time

HERE = Path(__file__).resolve().parent
MODELS = json.loads((HERE / "nano-models.json").read_text(encoding="utf-8"))["models"]
MODEL_ROOT = HERE / "models" / "nano"
MAX_FRAMES = 375
TEXT_TOKENS = 75


def file_valid(path, spec):
    if not path.is_file() or path.stat().st_size != spec["size"]:
        return False
    if spec.get("sha256"):
        with path.open("rb") as stream:
            return hashlib.file_digest(stream, "sha256").hexdigest() == spec["sha256"]
    return True


def download_models(root=MODEL_ROOT):
    """Both ONNX snapshots, including external weights; incomplete files are retried."""
    from huggingface_hub import hf_hub_download

    root = Path(root)
    for model in MODELS:
        target = root / model["directory"]
        for spec in model["files"]:
            path = target / spec["name"]
            if file_valid(path, spec):
                continue
            print(f"[tts] Nano download: {model['directory']}/{spec['name']}", flush=True)
            hf_hub_download(
                model["repo"], spec["name"], revision=model["revision"],
                local_dir=str(target), force_download=True,
            )
            if not file_valid(path, spec):
                raise RuntimeError(f"Nano model is incomplete or corrupt: {spec['name']}")
    return root / MODELS[0]["directory"]


def split_text(text, tokenizer, budget=TEXT_TOKENS):
    """Keep all text, prefer word/punctuation boundaries within the token budget."""
    remaining = re.sub(r"\s+", " ", text).strip()
    chunks = []
    while remaining:
        if len(tokenizer.encode(remaining, out_type=int)) <= budget:
            chunks.append(remaining)
            break
        # SentencePiece token counts are not strictly monotonic in prefix length.
        cut = min(len(remaining), budget)
        while cut > 1 and len(tokenizer.encode(remaining[:cut], out_type=int)) > budget:
            cut -= 1
        while cut < len(remaining) and len(tokenizer.encode(remaining[:cut + 1], out_type=int)) <= budget:
            cut += 1
        boundary = max((i + 1 for i, char in enumerate(remaining[:cut]) if char in " ,;.!?"), default=cut)
        chunks.append(remaining[:boundary].strip())
        remaining = remaining[boundary:].strip()
    return [chunk for chunk in chunks if chunk]


def resample_audio(samples, source, target):
    from scipy.signal import resample_poly

    factor = gcd(source, target)
    return resample_poly(samples, target // factor, source // factor).astype("float32")


class NanoVoice:
    engine = "MOSS-TTS-Nano ONNX"
    device = "MOSS-TTS-Nano ONNX, CPU (4 threads)"
    revision = MODELS[0]["revision"]

    def __init__(self):
        import numpy as np
        import sentencepiece
        from vendor.moss_nano.ort_cpu_runtime import OrtCpuRuntime
        from server import build_reference, voice_reference

        started = time.perf_counter()
        self.lock = threading.Lock()
        model_dir = download_models()
        print(f"[tts] loading {self.engine} revision={self.revision} on CPU ...", flush=True)
        self.runtime = OrtCpuRuntime(
            model_dir, thread_count=4, max_new_frames=MAX_FRAMES,
            sample_mode="fixed", execution_provider="cpu",
        )
        self.tokenizer = sentencepiece.SentencePieceProcessor(
            model_file=str(self.runtime.resolve_manifest_relative_path(self.runtime.manifest["model_files"]["tokenizer_model"])),
        )
        self.rate = int(self.runtime.codec_meta["codec_config"]["sample_rate"])
        files, _transcript = voice_reference()
        # Nano conditions on audio alone; F5's reference transcript is not prepended.
        samples, used = build_reference(files, rate=self.rate, resampler=resample_audio)
        if used != files:
            raise ValueError("Jarvis New reference exceeds the duration limit")
        channels = int(self.runtime.codec_meta["codec_config"]["channels"])
        reference = np.repeat(samples[None, None, :], channels, axis=1)
        outputs = self.runtime.sessions["codec_encode"].run(None, {
            "waveform": reference.astype(np.float32),
            "input_lengths": np.asarray([len(samples)], dtype=np.int32),
        })
        names = [item.name for item in self.runtime.sessions["codec_encode"].get_outputs()]
        encoded = dict(zip(names, outputs, strict=True))
        length = int(np.asarray(encoded["audio_code_lengths"]).reshape(-1)[0])
        self.prompt = np.asarray(encoded["audio_codes"])[0, :length].astype(int).tolist()
        if not self.prompt:
            raise RuntimeError("Nano produced an empty voice reference")
        self.reference_hash = hashlib.sha256(reference.tobytes()).hexdigest()
        self._infer("Готов.")
        print(f"[tts] ready: {self.device}, reference={self.reference_hash}, load_ms={(time.perf_counter()-started)*1000:.0f}", flush=True)

    def _infer(self, text):
        import numpy as np

        runtime = self.runtime
        parts = []
        for index, chunk in enumerate(split_text(text, self.tokenizer)):
            rows = runtime.build_voice_clone_request_rows(self.prompt, self.tokenizer.encode(chunk, out_type=int))
            pending, audio_parts = [], []
            runtime.codec_streaming_session.reset()

            def decode_pending(force=False):
                if not pending or (not force and len(pending) < 8):
                    return
                decoded = runtime.codec_streaming_session.run_frames(list(pending))
                pending.clear()
                if decoded is not None:
                    audio, count = decoded
                    if count > 0:
                        audio_parts.append(np.asarray(audio[0, :, :count]).mean(axis=0))

            def on_frame(_frames, _index, frame):
                pending.append(list(frame))
                decode_pending()

            try:
                frames = runtime.generate_audio_frames(rows, on_frame=on_frame)
                decode_pending(True)
                if len(frames) >= MAX_FRAMES:
                    raise RuntimeError("Nano reached its audio limit; reply may be incomplete")
                if not audio_parts:
                    raise RuntimeError("Nano produced no audio")
                if index:
                    parts.append(np.zeros(int(self.rate * 0.15), dtype=np.float32))
                parts.extend(audio_parts)
            finally:
                runtime.codec_streaming_session.reset()
        if not parts:
            raise ValueError("empty text")
        wav = np.concatenate(parts)
        if not np.isfinite(wav).all():
            raise RuntimeError("Nano produced invalid audio")
        return resample_audio(wav, self.rate, 24000)

    def synthesize(self, text, language="ru", voice_id=None):
        from server import to_wav_bytes

        with self.lock:
            started = time.perf_counter()
            samples = self._infer(text)
            elapsed = time.perf_counter() - started
            print(f"[tts] Nano generated chars={len(text)} synthesis_ms={elapsed*1000:.0f} audio_s={len(samples)/24000:.2f} rtf={elapsed/(len(samples)/24000):.3f}", flush=True)
            return to_wav_bytes(samples)
