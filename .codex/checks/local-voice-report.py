"""Aggregate observed runs and prepare a local blind listening page."""

import html
import argparse
import importlib.util
import json
from pathlib import Path
import random
import shutil
import statistics
import zipfile

spec = importlib.util.spec_from_file_location("local_benchmark", Path(__file__).with_name("local-voice-benchmark.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
KEYS = ["f5", "nano", "qwen06", "qwen17", "vox"]


def percentile(values, fraction):
    values = sorted(values)
    pos = (len(values) - 1) * fraction
    low = int(pos)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (pos - low)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary-only", action="store_true")
    args = parser.parse_args()
    bench.configure()
    quality = json.loads((bench.OUT / "quality.json").read_text())
    assert quality["result"] == "pass" and quality["exit_code"] == 0
    assert quality["provenance"] == "observed_local_quality_proxy"
    assert quality["reference_control"]["sha256"] == bench.digest(bench.OUT / "reference.wav")
    assert set(r["model"] for r in quality["runs"]) == set(KEYS)
    summary = {"result": "pass", "exit_code": 0, "provenance": "observed_local_comparison", "models": [],
               "scope": "Full WAV generation; Linux RTX 5050 Laptop / i5-13420H. Not audible TTFA; no blind listening verdict.",
               "quality_scope": "First repetition of all 50 unique phrases. Canonical ASR WER (numbers, glyphs, corpus name/acronym transliterations) and ECAPA cosine are diagnostic proxies; raw WER saved separately.",
               "latency_scope": "27 runs: same original 9 phrases, 3 repetitions each. p95 is descriptive, not a confidence bound.",
               "reference_scope": "Same original Jarvis New clips and transcript; native resampling/preprocessing differs between production F5 and Nano; Qwen/Vox share a 24kHz WAV."}
    for key in KEYS:
        data = bench.validate_batch(key)
        extra = bench.validate_batch(key, extra=True)
        assert data["reference"] == extra["reference"]
        runs = data["runs"] + extra["runs"]
        assert len(runs) == 68 and len({r["phrase"] for r in runs}) == 50
        for r in runs:
            directory = key + ("-extra" if r["phrase"].startswith("extra") else "")
            assert bench.digest(bench.OUT / directory / r["wav"]) == r["sha256"]
        qr = [r for r in quality["runs"] if r["model"] == key and r["repeat"] == 1]
        assert len(qr) == 50
        all_qr = [r for r in quality["runs"] if r["model"] == key]
        assert len(all_qr) == 68 and {(r["phrase"], r["repeat"]) for r in all_qr} == {(r["phrase"], r["repeat"]) for r in runs}
        for row in all_qr:
            original = next(r for r in runs if r["phrase"] == row["phrase"] and r["repeat"] == row["repeat"])
            assert row["text"] == original["text"] and row["wav_sha256"] == original["sha256"]
        expected = {(r["phrase"], r["sha256"]) for r in runs if r["repeat"] == 1}
        assert {(r["phrase"], r["wav_sha256"]) for r in qr} == expected
        delays = [r["seconds"] for r in data["runs"]]
        row = {"model": key, "runs": 68, "unique_phrases": 50,
               "wav_p50_s": statistics.median(delays), "wav_p95_s": percentile(delays, .95),
               "rtf_p50": statistics.median(r["rtf"] for r in data["runs"]),
               "wer_micro": sum(r["edits"] for r in qr) / sum(r["reference_words"] for r in qr),
               "raw_wer_micro": sum(r["raw_edits"] for r in qr) / sum(r["raw_reference_words"] for r in qr),
               "negation_count_mismatches": [r["phrase"] for r in qr if r["negative_word_count_preserved"] is False],
               "speaker_cosine_median": statistics.median(r["speaker_cosine"] for r in qr),
               "speaker_cosine_median_audio_over_2s": statistics.median(r["speaker_cosine"] for r in qr
                   if next(x["audio_seconds"] for x in runs if x["phrase"] == r["phrase"] and x["repeat"] == 1) >= 2),
               "peak_vram_allocated_gib": max(data.get("peak_vram_allocated_bytes", 0), extra.get("peak_vram_allocated_bytes", 0)) / 1024**3,
               "peak_process_rss_gib": max(data["peak_rss_kib"], extra["peak_rss_kib"]) / 1024**2,
               "load_s": data["load_seconds"], "warmup_s": data["warmup_seconds"],
               "by_phrase": {p: {"wav_p50_s": statistics.median(r["seconds"] for r in data["runs"] if r["phrase"] == p),
                                  "min_s": min(r["seconds"] for r in data["runs"] if r["phrase"] == p),
                                  "max_s": max(r["seconds"] for r in data["runs"] if r["phrase"] == p)}
                             for p in {r["phrase"] for r in data["runs"]}}}
        summary["models"].append(row)
    bench.dump(bench.OUT / "summary.json", summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    if args.summary_only:
        return

    # One representative repetition of the 9 original phrases, same stable
    # anonymous model labels for every phrase; reveal only on user action.
    listen = bench.OUT / "listen"
    listen.mkdir(exist_ok=False)
    shutil.copyfile(bench.OUT / "reference.wav", listen / "reference.wav")
    shuffled = list(KEYS)
    random.Random(20261006).shuffle(shuffled)
    labels = dict(zip("ABCDE", shuffled, strict=True))
    corpus = json.loads((bench.ROOT / ".codex/scenarios/voice-corpus.json").read_text())
    sections = []
    for phrase in corpus["phrases"]:
        tracks = []
        for label, key in labels.items():
            name = f"{label}-{phrase['id']}.wav"
            shutil.copyfile(bench.OUT / key / (phrase["id"] + "-1.wav"), listen / name)
            tracks.append(f'<div><b>{label}</b><audio controls preload="none" src="{name}"></audio></div>')
        sections.append(f'<section><h2>{html.escape(phrase["text"])}</h2>{"".join(tracks)}</section>')
    document = '''<!doctype html><html lang="ru"><meta charset="utf-8"><title>Сравнение голоса Джарвиса</title>
<style>body{font:17px system-ui;max-width:1100px;margin:30px auto;background:#111;color:#ddd}section{border-top:1px solid #555;padding:16px}section div{display:inline-flex;align-items:center;gap:12px;margin:6px}h2{font-size:18px}audio{width:240px}details{margin:24px 0}</style>
<h1>Сравнение голоса Джарвиса</h1><p>Одинаковый образец и тексты. Сначала оцени сходство, произношение и естественность, затем раскрой названия моделей. Аудио обрабатывалось локально.</p>
<p>Образец Jarvis New: <audio controls src="reference.wav"></audio></p>'''
    document += "".join(sections)
    document += '<details><summary>Показать названия моделей</summary><pre>' + html.escape(json.dumps(labels, ensure_ascii=False, indent=2)) + '</pre></details></html>'
    (listen / "index.html").write_text(document, encoding="utf-8")
    with zipfile.ZipFile(bench.OUT / "listen.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(listen.iterdir()):
            archive.write(path, path.name)


if __name__ == "__main__":
    main()
