"""Local model comparison. Full WAV timing is not audible/streaming latency."""

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import resource
import statistics
import subprocess
import sys
import time
import traceback
import wave

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / ".codex/.cache/voice-benchmark"
OUT = ROOT / ".codex/.tmp/voice-benchmark"
REPOS = {
    "f5": ["ESpeech/ESpeech-TTS-1_RL-V2", "charactr/vocos-mel-24khz"],
    "qwen06": ["Qwen/Qwen3-TTS-12Hz-0.6B-Base"],
    "qwen17": ["Qwen/Qwen3-TTS-12Hz-1.7B-Base"],
    "vox": ["openbmb/VoxCPM2"],
    "quality": ["mobiuslabsgmbh/faster-whisper-large-v3-turbo", "speechbrain/spkrec-ecapa-voxceleb"],
}


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_batch(key, extra=False):
    """Reject stale, partial or mismatched experiment metadata before scoring."""
    directory = OUT / (key + ("-extra" if extra else ""))
    data = json.loads((directory / "results.json").read_text())
    assert data.get("model") == key and data.get("provenance") == "observed_local_synthesis"
    assert data.get("result") == "pass" and data.get("exit_code") == 0
    corpus_path = ROOT / ".codex/scenarios" / ("voice-extra-41.json" if extra else "voice-corpus.json")
    assert data["corpus_sha256"] == digest(corpus_path)
    phrases = json.loads(corpus_path.read_text())["phrases"]
    expected = {(p["id"], repeat): p["text"] for p in phrases for repeat in ([1] if extra else [1, 2, 3])}
    runs = data["runs"]
    assert len(runs) == len(expected) and {(r["phrase"], r["repeat"]) for r in runs} == set(expected)
    for run in runs:
        assert run["text"] == expected[run["phrase"], run["repeat"]]
        assert run["wav"] == run["phrase"] + f"-{run['repeat']}.wav"
        assert digest(directory / run["wav"]) == run["sha256"]
        with wave.open(str(directory / run["wav"])) as wav:
            assert wav.getnchannels() == 1 and wav.getsampwidth() == 2 and wav.getframerate() == 24000
            frames = wav.getnframes()
            pcm = wav.readframes(frames)
            assert frames > 0 and len(pcm) == frames * wav.getnchannels() * wav.getsampwidth()
            duration = frames / wav.getframerate()
        assert all(isinstance(run[field], (float, int)) and not isinstance(run[field], bool)
                   and math.isfinite(run[field]) and run[field] > 0 for field in ("seconds", "audio_seconds", "rtf"))
        assert math.isclose(run["audio_seconds"], duration, rel_tol=1e-9, abs_tol=1e-9)
        assert math.isclose(run["rtf"], run["seconds"] / duration, rel_tol=1e-9, abs_tol=1e-9)
    import server
    files, text = server.voice_reference()
    assert (OUT / "reference.txt").read_text() == text
    reference = data["reference"]
    with wave.open(str(OUT / "reference.wav")) as wav:
        seconds = wav.getnframes() / wav.getframerate()
    assert reference == {"text": text, "sha256": digest(OUT / "reference.wav"), "seconds": seconds,
        "source_clips": [{"name": Path(f).name, "sha256": digest(Path(f))} for f in files]}
    if key != "nano":
        assert set(data["models"]) == set(REPOS[key])
        assert data["models"] == json.loads((OUT / (key + "-models.json")).read_text())
    else:
        archived = ROOT / ".codex/evidence/local-voice-2026-10-06/nano.json"
        assert data["models"] == json.loads(archived.read_text())["models"]
    return data


def configure():
    CACHE.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    for name, directory in {
        "HF_HOME": "hf", "XDG_CACHE_HOME": "cache", "TORCH_HOME": "torch",
        "MPLCONFIGDIR": "matplotlib", "NUMBA_CACHE_DIR": "numba", "TRITON_CACHE_DIR": "triton",
    }.items():
        os.environ[name] = str(CACHE / directory)
    os.environ["TOKENIZERS_PARALLELISM"] = "false"
    os.environ["MPLBACKEND"] = "Agg"
    sys.path.insert(0, str(ROOT / "tools/voice-server"))


def prepare(key, repair=False):
    from huggingface_hub import HfApi, snapshot_download
    pins_path = OUT / (key + "-models.json")
    previous = json.loads(pins_path.read_text()) if pins_path.exists() else {}
    if previous and not repair:
        return previous
    result = {}
    for repo in REPOS[key]:
        info = HfApi().model_info(repo, revision=previous.get(repo, {}).get("revision"), files_metadata=True)
        path = Path(snapshot_download(repo, revision=info.sha, allow_patterns=[
            "*.json", "*.txt", "*.yaml", "*.model", "*.safetensors", "*.pt", "*.pth", "*.bin", "*.ckpt",
        ]))
        files = []
        for file in sorted(path.rglob("*")):
            if file.is_file():
                files.append({"name": str(file.relative_to(path)), "size": file.stat().st_size, "sha256": digest(file)})
        result[repo] = {"revision": info.sha, "path": str(path), "files": files}
        print("Downloaded and hashed", repo, info.sha, flush=True)
    dump(pins_path, result)
    return result


def resample_audio(samples, source, target):
    from scipy.signal import resample_poly
    factor = math.gcd(source, target)
    return resample_poly(samples, target // factor, source // factor).astype("float32")


def reference():
    import server
    import soundfile as sf
    files, text = server.voice_reference()
    samples, used = server.build_reference(files, rate=24000, resampler=resample_audio)
    assert used == files
    path = OUT / "reference.wav"
    # All candidate adapters receive the same 24 kHz reference, except the
    # unchanged production Nano which constructs its pinned 48 kHz reference.
    sf.write(path, samples, 24000, subtype="PCM_16")
    (OUT / "reference.txt").write_text(text, encoding="utf-8")
    return path, text, {"sha256": digest(path), "source_clips": [
        {"name": Path(f).name, "sha256": digest(Path(f))} for f in files
    ], "text": text, "seconds": len(samples) / 24000}


def versions():
    result = {}
    for package in ["torch", "torchaudio", "f5-tts", "qwen-tts", "voxcpm", "transformers",
                    "onnxruntime", "ruaccent", "speechbrain", "faster-whisper", "num2words"]:
        try:
            result[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            pass
    return result


def run(key, repeats, corpus_path, tag):
    import numpy as np
    import soundfile as sf
    import server
    out = OUT / (key + ("-" + tag if tag else ""))
    out.mkdir(exist_ok=False)
    path, text, ref = reference()
    corpus = json.loads(corpus_path.read_text())
    import re
    phrases = corpus.get("phrases", [])
    if not phrases or len({p["id"] for p in phrases}) != len(phrases) or any(
        not re.fullmatch(r"[a-zA-Z0-9_-]+", p["id"]) or not p["text"].strip() for p in phrases
    ):
        raise ValueError("Corpus needs unique safe IDs and nonempty text")
    report = {"provenance": "observed_local_synthesis", "model": key,
              "created_at": datetime.now(timezone.utc).isoformat(), "reference": ref,
              "corpus_sha256": digest(corpus_path), "corpus_path": str(corpus_path.relative_to(ROOT)), "versions": versions(), "runs": [],
              "limits": ["No audible TTFA measured", "No blind listening", "Linux/NVIDIA is not Windows/AMD",
                         "Automated quality is a proxy, not subjective listening"]}
    torch = None
    start = time.perf_counter()
    try:
        if key == "nano":
            raise RuntimeError("Nano removed from product; reproduce its synthesis from commit a59b0e0")
        else:
            import torch
            assert torch.cuda.is_available(), "CUDA unavailable"
            torch.set_num_threads(4)
            torch.manual_seed(42)
            pins = prepare(key)
            report["models"] = pins
            report["device"] = {"name": torch.cuda.get_device_name(0), "capability": torch.cuda.get_device_capability(0),
                                "cuda": torch.version.cuda, "total_bytes": torch.cuda.get_device_properties(0).total_memory}
            torch.cuda.reset_peak_memory_stats()
            if key == "f5":
                original = server.hub_file
                server.hub_file = lambda repo, name: str(Path(pins[repo]["path"]) / name) if repo in pins else original(repo, name)
                server.RUACCENT_DIR = CACHE / "ruaccent"
                model = server.load_voice({"profile": "cuda"}, "cuda:0")
                report["settings"] = {"steps": server.F5_STEPS, "ruaccent_loaded": model.accent is not None, "default_engine": server.default_tts_engine()}
                generate = lambda phrase: model.synthesize(phrase)
            elif key.startswith("qwen"):
                from qwen_tts import Qwen3TTSModel
                model = Qwen3TTSModel.from_pretrained(next(iter(pins.values()))["path"],
                    device_map="cuda:0", dtype=torch.bfloat16, attn_implementation="sdpa")
                prompt = model.create_voice_clone_prompt(ref_audio=str(path), ref_text=text, x_vector_only_mode=False)
                report["settings"] = {"dtype": "bfloat16", "attention": "sdpa", "max_new_tokens": 2048, "cached_prompt": True}
                def generate(phrase):
                    wavs, rate = model.generate_voice_clone(text=phrase, language="Russian", voice_clone_prompt=prompt,
                                                            max_new_tokens=2048)
                    return server.to_wav_bytes(resample_audio(np.asarray(wavs[0]), rate, 24000))
            else:
                from voxcpm import VoxCPM
                model = VoxCPM.from_pretrained(next(iter(pins.values()))["path"], load_denoiser=False, optimize=False, device="cuda:0")
                report["settings"] = {"inference_timesteps": 10, "cfg_value": 2.0, "denoiser": False,
                                      "torch_compile": False, "normalize": False, "seed": 42}
                def generate(phrase):
                    wav = model.generate(text=phrase, prompt_wav_path=str(path), prompt_text=text, reference_wav_path=str(path),
                                         inference_timesteps=10, cfg_value=2.0, normalize=False)
                    return server.to_wav_bytes(resample_audio(np.asarray(wav), model.tts_model.sample_rate, 24000))
        report["load_seconds"] = time.perf_counter() - start
        if torch:
            torch.cuda.synchronize()
        start = time.perf_counter()
        generate(corpus["phrases"][0]["text"])
        if torch:
            torch.cuda.synchronize()
        report["warmup_seconds"] = time.perf_counter() - start
        for phrase in corpus["phrases"]:
            for index in range(repeats):
                if torch:
                    torch.manual_seed(42 + index)
                    torch.cuda.synchronize()
                start = time.perf_counter()
                data = generate(phrase["text"])
                if torch:
                    torch.cuda.synchronize()
                elapsed = time.perf_counter() - start
                filename = phrase["id"] + f"-{index+1}.wav"
                (out / filename).write_bytes(data)
                samples, rate = sf.read(out / filename)
                assert len(samples) and np.isfinite(samples).all()
                seconds = len(samples) / rate
                report["runs"].append({"phrase": phrase["id"], "text": phrase["text"], "repeat": index + 1,
                    "wav": filename, "sha256": hashlib.sha256(data).hexdigest(), "seconds": elapsed,
                    "audio_seconds": seconds, "rtf": elapsed / seconds})
                print(key, phrase["id"], index+1, round(elapsed, 3), "s", flush=True)
                dump(out / "results.json", report)
        report["result"] = "pass"
        report["exit_code"] = 0
    except Exception:
        report["result"] = "fail"
        report["exit_code"] = 1
        report["error"] = traceback.format_exc()
        traceback.print_exc()
    finally:
        report["peak_rss_kib"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        if torch and torch.cuda.is_available():
            report["peak_vram_allocated_bytes"] = torch.cuda.max_memory_allocated()
            report["peak_vram_reserved_bytes"] = torch.cuda.max_memory_reserved()
        dump(out / "results.json", report)
    return report["exit_code"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=["nano", "f5", "qwen06", "qwen17", "vox", "quality"], required=True)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--repair", action="store_true", help="Complete the same pinned snapshots, preserve revision")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--corpus", type=Path, default=ROOT / ".codex/scenarios/voice-corpus.json")
    parser.add_argument("--tag", default="")
    args = parser.parse_args()
    configure()
    if args.prepare:
        prepare(args.model, args.repair)
        return 0
    if args.repeats < 1 or (args.tag and not args.tag.replace("-", "").isalnum()):
        parser.error("Invalid repeats or output tag")
    return run(args.model, args.repeats, args.corpus.resolve(), args.tag)


if __name__ == "__main__":
    raise SystemExit(main())
