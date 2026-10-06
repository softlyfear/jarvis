"""Diagnostic ASR WER and speaker-embedding similarity, not subjective MOS."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib.util

spec = importlib.util.spec_from_file_location("local_benchmark", Path(__file__).with_name("local-voice-benchmark.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def words(text):
    return re.findall(r"[а-яa-z0-9]+", text.lower().replace("ё", "е"))


# Morphological equivalents of numerals; meanings remain distinct. The corpus
# uses spoken numbers. Whisper often writes digits, which is not a TTS error.
NUMERAL_GROUPS = [
    "один одна одно первого первый", "два две второго второй", "три третьего третий",
    "четыре четвертого четвертый", "пять пятого пятый", "шесть шестого шестой",
    "семь седьмого седьмой", "восемь восьмого восьмой", "девять девятого девятый",
    "десять десятого десятый", "одиннадцать одиннадцатого", "двенадцать двенадцатого",
    "тринадцать тринадцатого", "четырнадцать четырнадцатого", "пятнадцать пятнадцатого",
    "шестнадцать шестнадцатого", "семнадцать семнадцатого", "восемнадцать восемнадцатого",
    "девятнадцать девятнадцатого", "двадцать двадцатого",
]
NUMERAL_FORMS = {form: group.split()[0] for group in NUMERAL_GROUPS for form in group.split()}
NUMERAL_FORMS.update({"нуль": "ноль", "тысяча": "тысяча", "тысячи": "тысяча", "тысяч": "тысяча",
                       "миллиона": "миллион", "миллионов": "миллион"})
NUMERAL_FORMS.update(dict(zip(
    "первое второе третье четвертое пятое шестое седьмое восьмое девятое десятое одиннадцатое двенадцатое тринадцатое четырнадцатое пятнадцатое шестнадцатое семнадцатое восемнадцатое девятнадцатое двадцатое".split(),
    [group.split()[0] for group in NUMERAL_GROUPS], strict=True)))
ENTITY_FORMS = {"steam": "стим", "telegram": "телеграм", "firefox": "файрфокс", "youtube": "ютуб",
                "discord": "дискорд", "dota": "дота", "доту": "дота", "pdf": "пэ дэ эф", "txt": "тэ икс тэ"}


def numeric_words(text, digit_sequence=False):
    from num2words import num2words
    # Leading-zero identifiers remain a digit sequence rather than an integer.
    def expand(match):
        token = match.group()
        if digit_sequence or (len(token) > 1 and token.startswith("0")):
            return " ".join(num2words(int(char), lang="ru") for char in token)
        return num2words(int(token), lang="ru") if len(token) <= 12 else token
    text = re.sub(r"\.(pdf|txt)\b", r" точка \1 ", text, flags=re.I)
    text = text.replace("%", " процентов ")
    expanded = re.sub(r"\b[0-9]+\b", expand, text)
    tokens = []
    for token in words(expanded):
        tokens.extend(ENTITY_FORMS.get(NUMERAL_FORMS.get(token, token), NUMERAL_FORMS.get(token, token)).split())
    return tokens


def score_text(text, transcript, phrase):
    ref, hyp = words(text), words(transcript)
    raw = errors(ref, hyp)
    normalized_ref = numeric_words(text, digit_sequence=phrase == "extra18")
    normalized_hyp = numeric_words(transcript, digit_sequence=phrase == "extra18")
    score = errors(normalized_ref, normalized_hyp)
    return {"reference_words": len(normalized_ref), "wer": score["edits"] / len(normalized_ref), **score,
            "raw_wer": raw["edits"] / len(ref), "raw_edits": raw["edits"], "raw_reference_words": len(ref),
            "normalized_reference": normalized_ref, "normalized_transcript": normalized_hyp}


def errors(reference, hypothesis):
    # Levenshtein edit counts, with deterministic tie resolution.
    previous = [(i, 0, 0, i) for i in range(len(hypothesis) + 1)]
    for i, ref in enumerate(reference, 1):
        current = [(i, 0, i, 0)]
        for j, hyp in enumerate(hypothesis, 1):
            if ref == hyp:
                current.append(previous[j-1])
            else:
                total, substitutions, deletions, insertions = previous[j-1]
                choices = [(total+1, substitutions+1, deletions, insertions)]
                total, substitutions, deletions, insertions = previous[j]
                choices.append((total+1, substitutions, deletions+1, insertions))
                total, substitutions, deletions, insertions = current[j-1]
                choices.append((total+1, substitutions, deletions, insertions+1))
                current.append(min(choices, key=lambda item: item[0]))
        previous = current
    return dict(zip(("edits", "substitutions", "deletions", "insertions"), previous[-1], strict=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("models", nargs="+", choices=["nano", "f5", "qwen06", "qwen17", "vox"])
    parser.add_argument("--rescore", type=Path, help="Recompute text metrics from immutable observed transcripts, no new inference")
    args = parser.parse_args()
    bench.configure()
    if args.rescore:
        report = json.loads(args.rescore.read_text())
        assert report["result"] == "pass" and report["exit_code"] == 0
        assert report["provenance"] == "observed_local_quality_proxy"
        assert set(r["model"] for r in report["runs"]) == set(args.models)
        for key in args.models:
            bench.validate_batch(key)
            bench.validate_batch(key, extra=True)
        for row in report["runs"]:
            row.update(score_text(row["text"], row["transcript"], row["phrase"]))
        report["rescoring"] = {"source_sha256": bench.digest(args.rescore), "source": str(args.rescore.resolve().relative_to(bench.ROOT)),
            "method": "Same observed transcripts and embeddings. Canonical WER expands % and file extensions; extra18 is explicitly a digit sequence.",
            "new_inference": False}
        report["method"]["entity_forms"] = ENTITY_FORMS
        report["method"]["digit_sequence_ids"] = ["extra18"]
        bench.dump(bench.OUT / "quality.json", report)
        return
    import numpy as np
    import soundfile as sf
    import torch
    from faster_whisper import WhisperModel
    from speechbrain.inference.speaker import EncoderClassifier
    resample_audio = bench.resample_audio

    pins = bench.prepare("quality")
    whisper = WhisperModel(pins["mobiuslabsgmbh/faster-whisper-large-v3-turbo"]["path"], device="cuda", compute_type="float16")
    ecapa_dir = pins["speechbrain/spkrec-ecapa-voxceleb"]["path"]
    speaker = EncoderClassifier.from_hparams(source=ecapa_dir, savedir=str(bench.CACHE / "ecapa"),
                                             overrides={"pretrained_path": ecapa_dir}, run_opts={"device": "cuda"})

    def audio16(path):
        samples, rate = sf.read(path, dtype="float32")
        if samples.ndim > 1:
            samples = samples.mean(axis=1)
        return resample_audio(samples, rate, 16000)

    def embedding(path):
        samples = audio16(path)
        with torch.inference_mode():
            result = speaker.encode_batch(torch.from_numpy(samples).unsqueeze(0).to("cuda")).flatten()
        assert torch.isfinite(result).all() and result.norm() > 1e-8, "Invalid speaker embedding"
        return result / result.norm()

    reference_embedding = embedding(bench.OUT / "reference.wav")
    for key in args.models:
        bench.validate_batch(key)
        bench.validate_batch(key, extra=True)
    report = {"provenance": "observed_local_quality_proxy", "models": pins, "versions": bench.versions(),
              "method": {"ASR": "Whisper large-v3-turbo, ru, beam_size=5, no initial prompt, no VAD",
                         "speaker": "ECAPA VoxCeleb cosine on 16kHz; similarity proxy, not MOS or blind listening",
                         "normalization": "raw WER: lowercase, ё=е, words only; canonical WER: symmetric num2words ru + explicit numeral forms and corpus name/acronym transliterations; no synonym or value replacement",
                         "numeral_forms": NUMERAL_FORMS, "entity_forms": ENTITY_FORMS}, "runs": []}
    segments, _ = whisper.transcribe(audio16(bench.OUT / "reference.wav"), language="ru", beam_size=5,
                                     vad_filter=False, condition_on_previous_text=False)
    reference_transcript = " ".join(s.text.strip() for s in segments).strip()
    reference_text = (bench.OUT / "reference.txt").read_text()
    reference_errors = errors(words(reference_text), words(reference_transcript))
    report["reference_control"] = {"sha256": bench.digest(bench.OUT / "reference.wav"),
        "text": reference_text, "transcript": reference_transcript,
        "wer": reference_errors["edits"] / len(words(reference_text)), **reference_errors}
    for key in args.models:
        paths = [bench.OUT / key / "results.json", bench.OUT / (key + "-extra") / "results.json"]
        for result_path in paths:
            data = bench.validate_batch(key, extra=result_path.parent.name.endswith("-extra"))
            for run in data["runs"]:
                path = result_path.parent / run["wav"]
                assert bench.digest(path) == run["sha256"]
                segments, _ = whisper.transcribe(audio16(path), language="ru", beam_size=5, vad_filter=False,
                                                  condition_on_previous_text=False)
                transcript = " ".join(s.text.strip() for s in segments).strip()
                ref, hyp = words(run["text"]), words(transcript)
                score = score_text(run["text"], transcript, run["phrase"])
                cosine = float(torch.dot(reference_embedding, embedding(path)).item())
                report["runs"].append({"model": key, "batch": result_path.parent.name, "phrase": run["phrase"], "repeat": run["repeat"],
                    "wav_sha256": run["sha256"], "text": run["text"], "transcript": transcript,
                    **score,
                    "speaker_cosine": cosine,
                    "negative_word_count_preserved": hyp.count("не") == ref.count("не") if "не" in ref else None})
                print(key, run["phrase"], run["repeat"], "WER", round(score["wer"], 3), "cos", round(cosine, 3), transcript, flush=True)
                bench.dump(bench.OUT / "quality.json", report)
    report.update(result="pass", exit_code=0)
    bench.dump(bench.OUT / "quality.json", report)


if __name__ == "__main__":
    main()
