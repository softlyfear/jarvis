"""Measure WAV HTTP synthesis; do not equate HTTP first byte with audible latency."""

import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import statistics
import platform
import time
import urllib.parse
import urllib.request
import wave

ROOT = Path(__file__).resolve().parents[2]
MAX_WAV_BYTES = 32 * 1024 * 1024


def wav_seconds(data):
    with wave.open(io.BytesIO(data), "rb") as wav:
        if not wav.getframerate() or not wav.getnframes():
            raise ValueError("Пустой WAV или неизвестная частота")
        frames = wav.getnframes()
        samples = wav.readframes(frames)
        if len(samples) != frames * wav.getnchannels() * wav.getsampwidth():
            raise ValueError("WAV оборван: фактический звук не соответствует заголовку")
        return frames / wav.getframerate()


def percentile(values, fraction):
    values = sorted(values)
    index = (len(values) - 1) * fraction
    low = int(index)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (index - low)


def request_voice(base, text, voice, timeout):
    body = json.dumps({"text": text, "language": "ru", "voice": voice}).encode()
    request = urllib.request.Request(base.rstrip("/") + "/tts", body, {"Content-Type": "application/json"})
    start = time.perf_counter()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        first = response.read(1)
        first_byte = time.perf_counter() - start
        data = first + response.read(MAX_WAV_BYTES)
    total = time.perf_counter() - start
    if len(data) > MAX_WAV_BYTES:
        raise ValueError("Ответ синтезатора превышает лимит")
    seconds = wav_seconds(data)
    return data, {"http_first_byte_seconds": first_byte, "http_complete_seconds": total, "audio_seconds": seconds, "rtf": total / seconds}


def run(base, model, profile, output, repeats, corpus, timeout=60, reference=None, fixture=False):
    if repeats < 1:
        raise ValueError("Нужно хотя бы одно повторение")
    parsed = urllib.parse.urlparse(base)
    if parsed.scheme not in ("http", "https") or parsed.hostname not in ("127.0.0.1", "localhost", "::1") or parsed.query or parsed.fragment or parsed.username:
        raise ValueError("Benchmark обращается только к локальному серверу")
    with urllib.request.urlopen(base.rstrip("/") + "/health", timeout=timeout) as response:
        health = json.load(response)
    if health.get("tts") is not True or health.get("profile") != profile:
        raise ValueError("TTS не готов или фактический аппаратный профиль не совпадает")
    device = str(health.get("tts_device") or "")
    gpu = str(health.get("gpu") or "")
    if not fixture and (not gpu or profile.lower() not in device.lower() or gpu.lower() not in device.lower() or str(health.get("tts_engine", "")).lower() in ("", "fixture", "mock", "fake")):
        raise ValueError("Сервер не сообщил согласованные GPU/движок/устройство синтеза")
    phrases = corpus.get("phrases", [])
    if not phrases or len({p["id"] for p in phrases}) != len(phrases):
        raise ValueError("Нужны фразы с уникальными ID")
    if any(not p["id"].replace("-", "").replace("_", "").isalnum() or not p["text"].strip() for p in phrases):
        raise ValueError("Небезопасный ID или пустая фраза")
    # Refuse existing output files rather than silently overwriting a previous run.
    output.mkdir(parents=True, exist_ok=False)
    _, warmup = request_voice(base, phrases[0]["text"], corpus["voice"], timeout)
    runs = []
    for phrase in phrases:
        for repeat in range(repeats):
            data, metrics = request_voice(base, phrase["text"], corpus["voice"], timeout)
            filename = phrase["id"] + "-" + str(repeat + 1) + ".wav"
            (output / filename).write_bytes(data)
            runs.append({"phrase_id": phrase["id"], "repeat": repeat + 1, "audio": filename, "sha256": hashlib.sha256(data).hexdigest(), **metrics})
    delays = [r["http_complete_seconds"] for r in runs]
    result = {
        "schema_version": 1, "provenance": "fixture" if fixture else "observed_http", "created_at": datetime.now(timezone.utc).isoformat(),
        "hardware_verified": False, "environment": {"os": platform.system()},
        "model_declared": model, "model_verified_by_server": False,
        "profile": profile, "server_health": {k: health.get(k) for k in ("tts", "tts_engine", "tts_device", "gpu", "profile")},
        "corpus_sha256": hashlib.sha256(json.dumps(corpus, ensure_ascii=False, sort_keys=True).encode()).hexdigest(),
        "reference_declared": reference, "warmup": warmup, "runs": runs,
        "summary": {"count": len(runs), "http_complete_p50_seconds": statistics.median(delays), "http_complete_p95_seconds": percentile(delays, .95)},
        "not_measured": ["Первый слышимый звук", "VRAM", "Сходство голоса", "Ошибки произношения", "WER", "Полный холодный старт сервера", "Независимая проверка GPU и загруженных весов"],
    }
    (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:5055")
    parser.add_argument("--model", required=True)
    parser.add_argument("--profile", required=True, choices=("cuda", "rocm"))
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--corpus", type=Path, default=ROOT / ".codex/scenarios/voice-corpus.json")
    parser.add_argument("--reference-audio", required=True, type=Path)
    parser.add_argument("--reference-text", required=True, type=Path)
    args = parser.parse_args()
    reference = {"audio_sha256": hashlib.sha256(args.reference_audio.read_bytes()).hexdigest(), "text_sha256": hashlib.sha256(args.reference_text.read_bytes()).hexdigest(), "verified_by_server": False}
    report = run(args.url, args.model, args.profile, args.output, args.repeats, json.loads(args.corpus.read_text(encoding="utf-8")), args.timeout, reference)
    print(json.dumps(report["summary"], ensure_ascii=False))


if __name__ == "__main__":
    main()
