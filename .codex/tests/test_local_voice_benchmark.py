"""Regressions from the independent review of the local TTS comparison."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch
import wave

ROOT = Path(__file__).resolve().parents[2]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / ".codex/checks" / filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


bench = module("benchmark_guard", "local-voice-benchmark.py")
quality = module("benchmark_metrics", "local-voice-quality.py")


class BatchIntegrity(unittest.TestCase):
    def setUp(self):
        parent = ROOT / ".codex/.tmp"
        parent.mkdir(parents=True, exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=parent)
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.out = self.root / "results"
        self.out.mkdir()
        (self.out / "reference.txt").write_text("Образец.")
        corpus = self.root / ".codex/scenarios/voice-corpus.json"
        corpus.parent.mkdir(parents=True)
        corpus.write_text(json.dumps({"phrases": [{"id": "negative", "text": "Файл не сохранён."}]}))
        with wave.open(str(self.out / "reference.wav"), "wb") as wav:
            wav.setparams((1, 2, 24000, 0, "NONE", "not compressed"))
            wav.writeframes(b"\0\0" * 240)
        self.clip = self.root / "voice.wav"
        self.clip.write_bytes((self.out / "reference.wav").read_bytes())
        target = self.out / "qwen06"
        target.mkdir()
        models = {bench.REPOS["qwen06"][0]: {"revision": "fixture", "files": []}}
        (self.out / "qwen06-models.json").write_text(json.dumps(models))
        self.data = {"model": "qwen06", "provenance": "observed_local_synthesis", "result": "pass", "exit_code": 0,
            "corpus_sha256": bench.digest(corpus), "models": models,
            "reference": {"sha256": bench.digest(self.out / "reference.wav"), "text": "Образец.", "seconds": .01,
                "source_clips": [{"name": self.clip.name, "sha256": bench.digest(self.clip)}]}, "runs": []}
        for repeat in [1, 2, 3]:
            filename = f"negative-{repeat}.wav"
            (target / filename).write_bytes(self.clip.read_bytes())
            self.data["runs"].append({"phrase": "negative", "repeat": repeat, "text": "Файл не сохранён.",
                                      "wav": filename, "sha256": bench.digest(target / filename),
                                      "seconds": .02, "audio_seconds": .01, "rtf": 2.0})
        self.target = target / "results.json"
        self.fake_server = types.SimpleNamespace(voice_reference=lambda: ([str(self.clip)], "Образец."))

    def validate(self, data):
        self.target.write_text(json.dumps(data))
        with patch.object(bench, "ROOT", self.root), patch.object(bench, "OUT", self.out), patch.dict("sys.modules", {"server": self.fake_server}):
            return bench.validate_batch("qwen06")

    def test_valid_fixture(self):
        self.assertEqual(len(self.validate(self.data)["runs"]), 3)

    def test_stale_or_mismatched_metadata_rejected(self):
        for field, value in [("model", "f5"), ("provenance", "fixture"), ("result", "fail"), ("exit_code", 1),
                             ("corpus_sha256", "0" * 64)]:
            with self.subTest(field=field):
                data = copy.deepcopy(self.data)
                data[field] = value
                with self.assertRaises(AssertionError):
                    self.validate(data)

    def test_wrong_reference_rejected(self):
        for field, value in [("sha256", "0" * 64), ("text", "Другой образец."), ("seconds", 2), ("source_clips", [])]:
            with self.subTest(field=field):
                data = copy.deepcopy(self.data)
                data["reference"][field] = value
                with self.assertRaises(AssertionError):
                    self.validate(data)

    def test_changed_text_and_duplicate_repeat_rejected(self):
        data = copy.deepcopy(self.data)
        data["runs"][0]["text"] = "Файл сохранён."
        with self.assertRaises(AssertionError):
            self.validate(data)
        data = copy.deepcopy(self.data)
        data["runs"][2]["repeat"] = 2
        with self.assertRaises(AssertionError):
            self.validate(data)

    def test_changed_audio_rejected(self):
        (self.target.parent / "negative-1.wav").write_bytes(b"corrupt")
        with self.assertRaises(AssertionError):
            self.validate(self.data)

    def test_missing_party_rejected(self):
        self.target.unlink(missing_ok=True)
        with patch.object(bench, "OUT", self.out):
            with self.assertRaises(FileNotFoundError):
                bench.validate_batch("qwen06")

    def test_invalid_metrics_rejected(self):
        for field, value in [("audio_seconds", -1), ("seconds", float("nan")), ("rtf", 0),
                             ("audio_seconds", 2), ("rtf", 3)]:
            with self.subTest(field=field, value=value):
                data = copy.deepcopy(self.data)
                data["runs"][0][field] = value
                with self.assertRaises(AssertionError):
                    self.validate(data)

    def test_truncated_pcm_rejected_even_with_updated_hash(self):
        file = self.target.parent / "negative-1.wav"
        file.write_bytes(file.read_bytes()[:-2])
        self.data["runs"][0]["sha256"] = bench.digest(file)
        with self.assertRaises(AssertionError):
            self.validate(self.data)

    def test_reference_text_file_mismatch_rejected(self):
        (self.out / "reference.txt").write_text("Другой образец.")
        with self.assertRaises(AssertionError):
            self.validate(self.data)


class TextMetrics(unittest.TestCase):
    def test_critical_word_errors(self):
        self.assertEqual(quality.errors(quality.words("файл не сохранён"), quality.words("файл сохранен"))["deletions"], 1)
        self.assertEqual(quality.errors(quality.words("открой файл"), quality.words("закрой файл"))["substitutions"], 1)
        self.assertEqual(quality.errors(["готово", "сэр"], ["готово", "мой", "сэр"])["insertions"], 1)

    @unittest.skipUnless(importlib.util.find_spec("num2words"), "numeric dependency installed in isolated evaluation environment")
    def test_numeric_equivalence_preserves_values(self):
        pairs = [("сорок пять", "45"), ("двадцать пять", "25"), ("сто двадцать три", "123"),
                 ("семь, девять, ноль, два, четыре", "7,9,0,2,4"),
                 ("две тысячи двадцать шестого года", "2026 года"), ("ноль, ноль, один", "001"),
                 ("шестое октября", "6 октября"), ("Телеграм и Стим", "Telegram и Steam"),
                 ("пэ дэ эф", "PDF")]
        for a, b in pairs:
            with self.subTest(a=a, b=b):
                self.assertEqual(quality.numeric_words(a), quality.numeric_words(b))
        self.assertNotEqual(quality.numeric_words("45"), quality.numeric_words("46"))
        self.assertNotEqual(quality.numeric_words("двадцать пять"), quality.numeric_words("26"))
        self.assertEqual(quality.numeric_words("сорок пять процентов"), quality.numeric_words("45%"))
        self.assertEqual(quality.numeric_words("резюме точка пэ дэ эф"), quality.numeric_words("резюме.pdf"))
        self.assertEqual(quality.score_text("Повторяю номер: семь, девять, ноль, два, четыре.", "Повторяю номер 79024.", "extra18")["wer"], 0)
        self.assertGreater(quality.score_text("Повторяю номер: семь, девять, ноль, два, четыре.", "Повторяю номер 79025.", "extra18")["wer"], 0)


if __name__ == "__main__":
    unittest.main()
